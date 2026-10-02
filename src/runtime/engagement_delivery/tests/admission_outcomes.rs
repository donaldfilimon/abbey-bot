//! Final recipient authorization follows every external source proof.
use super::*;
use std::sync::atomic::AtomicBool;

struct PrivateAccess {
    inner: Fake,
    accessible: AtomicBool,
    revoke: bool,
    proofs: AtomicUsize,
}
impl EngagementTransport for PrivateAccess {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        assert!(self.inner.state.stores.try_lock().is_ok());
        self.inner.auth.fetch_add(1, Ordering::SeqCst);
        assert_eq!(r.destination, DestinationPreference::Private);
        if !self.accessible.load(Ordering::SeqCst) {
            return Err(WorkError::Denied);
        }
        Ok(AuthorizedDestination {
            channel: 9,
            member: r.member,
            scope: r.scope.clone(),
        })
    }
    async fn source_exists(&self, s: &SourceRef) -> Result<bool, WorkError> {
        self.inner.source_exists(s).await
    }
    async fn hydrate(&self, c: &Candidate) -> Result<String, WorkError> {
        self.inner.hydrate(c).await
    }
    async fn candidate_current(&self, _: &Candidate, _: Option<u64>) -> Result<bool, WorkError> {
        assert!(self.inner.state.stores.try_lock().is_ok());
        self.proofs.fetch_add(1, Ordering::SeqCst);
        if self.revoke {
            self.accessible.store(false, Ordering::SeqCst);
        }
        Ok(true)
    }
    async fn generate(
        &self,
        s: &AppState,
        c: &Candidate,
        text: &str,
        now: u64,
    ) -> Result<String, WorkError> {
        self.inner.generate(s, c, text, now).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        assert_eq!(channel, 9);
        self.inner.send(channel, body).await
    }
}
#[tokio::test]
async fn engagement_delivery_recipient_access_revoked_during_final_source_check_prevents_send() {
    for revoke in [false, true] {
        let h = Harness::new();
        super::feedback_tests::guild_candidate(&h);
        {
            let mut stores = AppState::lock(&h.state.stores);
            let store = &mut stores.work.engagement;
            let scope = store.candidates[&1].scope.clone();
            store
                .member_policies
                .get_mut(&2)
                .unwrap()
                .destinations
                .insert(scope, DestinationPreference::Private);
            store.candidates.get_mut(&1).unwrap().destination = DestinationPreference::Private;
        }
        let fake = PrivateAccess {
            inner: Fake::new(&h, Race::None),
            accessible: AtomicBool::new(true),
            revoke,
            proofs: AtomicUsize::new(0),
        };
        h.state
            .clone()
            .deliver_engagement(&fake, fake.inner.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(fake.proofs.load(Ordering::SeqCst), 1);
        assert_eq!(fake.inner.auth.load(Ordering::SeqCst), 2);
        assert_eq!(
            fake.inner.sends.load(Ordering::SeqCst),
            usize::from(!revoke)
        );
        assert_eq!(
            h.status(),
            if revoke {
                CandidateState::Rejected
            } else {
                CandidateState::Sent
            }
        );
        let loaded = Stores::load(&h.dir).unwrap();
        assert_eq!(loaded.work.engagement.charges.len(), 1);
        assert_eq!(
            loaded.work.engagement.candidates[&1].message_id,
            if revoke { None } else { Some(123) }
        );
        // Restoring access does not retry or reroute a consumed source.
        fake.accessible.store(true, Ordering::SeqCst);
        h.state
            .clone()
            .deliver_engagement(&fake, fake.inner.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(
            fake.inner.sends.load(Ordering::SeqCst),
            usize::from(!revoke)
        );
        assert_eq!(fake.inner.auth.load(Ordering::SeqCst), 2);
        h.finish().await;
    }
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum Observation {
    GenerationDenied,
    GenerationUnavailable,
    Empty,
    Oversized,
    Cancel,
    Rejected,
    Uncertain,
    FinalSourceDenied,
    FinalProofDenied,
    FinalProofUnavailable,
    FinalAuthorizeDenied,
    FinalAuthorizeTimeout,
    FinalPolicyDenied,
}
#[cfg(unix)]
struct OutcomeTransport {
    inner: Fake,
    observation: Observation,
    final_entered: tokio::sync::Notify,
}
#[cfg(unix)]
impl EngagementTransport for OutcomeTransport {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        if self.inner.auth.load(Ordering::SeqCst) == 1 {
            match self.observation {
                Observation::FinalAuthorizeDenied => {
                    self.inner.auth.fetch_add(1, Ordering::SeqCst);
                    return Err(WorkError::Denied);
                }
                Observation::FinalAuthorizeTimeout => {
                    self.inner.auth.fetch_add(1, Ordering::SeqCst);
                    self.final_entered.notify_one();
                    tokio::time::sleep(std::time::Duration::from_secs(31)).await;
                    panic!("the final authorization must be bounded to 30 seconds");
                }
                _ => {}
            }
        }
        self.inner.authorize(r).await
    }
    async fn source_exists(&self, s: &SourceRef) -> Result<bool, WorkError> {
        if matches!(self.observation, Observation::FinalSourceDenied) {
            Err(WorkError::Denied)
        } else {
            self.inner.source_exists(s).await
        }
    }
    async fn candidate_current(
        &self,
        c: &Candidate,
        response: Option<u64>,
    ) -> Result<bool, WorkError> {
        match self.observation {
            Observation::FinalProofDenied => Err(WorkError::Denied),
            Observation::FinalProofUnavailable => Err(WorkError::Missing),
            _ => self.inner.candidate_current(c, response).await,
        }
    }
    async fn hydrate(&self, c: &Candidate) -> Result<String, WorkError> {
        self.inner.hydrate(c).await
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        match self.observation {
            Observation::GenerationDenied => Err(WorkError::Denied),
            Observation::GenerationUnavailable => Err(WorkError::Missing),
            Observation::Empty => Ok(String::new()),
            Observation::Oversized => Ok("x".repeat(1901)),
            Observation::Cancel => {
                self.inner.cancel.cancel();
                Ok("transient source question".into())
            }
            _ => Ok("transient source question".into()),
        }
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        self.inner.send(channel, body).await
    }
}
#[cfg(unix)]
#[tokio::test]
async fn engagement_delivery_observability_uses_observed_failure_stages_and_cancellation() {
    for (observation, category, event_outcome, status, sends) in [
        (
            Observation::GenerationDenied,
            None,
            "failed",
            CandidateState::Rejected,
            0,
        ),
        (
            Observation::GenerationUnavailable,
            Some("unavailable"),
            "failed",
            CandidateState::Rejected,
            0,
        ),
        (
            Observation::Empty,
            Some("protocol"),
            "failed",
            CandidateState::Rejected,
            0,
        ),
        (
            Observation::Oversized,
            Some("protocol"),
            "failed",
            CandidateState::Rejected,
            0,
        ),
        (
            Observation::Cancel,
            None,
            "cancelled",
            CandidateState::Cancelled,
            0,
        ),
        (
            Observation::Rejected,
            Some("unavailable"),
            "failed",
            CandidateState::Rejected,
            1,
        ),
        (
            Observation::Uncertain,
            None,
            "failed",
            CandidateState::ReviewRequired,
            1,
        ),
    ] {
        retained_outcome(observation, category, event_outcome, status, sends).await;
    }
}
#[cfg(unix)]
async fn retained_outcome(
    observation: Observation,
    category: Option<&str>,
    event_outcome: &str,
    status: CandidateState,
    sends: usize,
) {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new();
    // A synthetic private HOME gives the actual retained telemetry writer a
    // file sink; this fixture never loads operator env or starts a service.
    let home = h.dir.canonicalize().unwrap().join("telemetry-home");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = home.join(".config/abbey-bot");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        config.join("env"),
        b"DISCORD_TOKEN=SYNTHETIC_UNUSED_FIXTURE\n",
    )
    .unwrap();
    std::fs::set_permissions(config.join("env"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let managed = crate::managed_service::begin(&home).unwrap();
    let mut telemetry = crate::service::telemetry::TelemetryWriter::start(
        managed.log,
        managed.publisher,
        managed.fatal,
    );
    assert!(h.state.operational_events.set(telemetry.requests()).is_ok());
    let race = match observation {
        Observation::Rejected => Race::BlockedDm,
        Observation::Uncertain => Race::Uncertain,
        Observation::FinalPolicyDenied => Race::Stop,
        _ => Race::None,
    };
    let fake = OutcomeTransport {
        inner: Fake::new(&h, race),
        observation,
        final_entered: tokio::sync::Notify::new(),
    };
    let delivery = h
        .state
        .clone()
        .deliver_engagement(&fake, fake.inner.cancel.clone(), || NOW);
    if matches!(observation, Observation::FinalAuthorizeTimeout) {
        let deadline = async {
            fake.final_entered.notified().await;
            tokio::time::advance(std::time::Duration::from_secs(30)).await;
        };
        let (delivered, ()) = tokio::join!(delivery, deadline);
        delivered.unwrap();
    } else {
        delivery.await.unwrap();
    }
    assert_eq!(h.status(), status);
    assert_eq!(fake.inner.sends.load(Ordering::SeqCst), sends);
    assert!(
        AppState::lock(&h.state.stores).work.engagement.candidates[&1]
            .message_id
            .is_none()
    );
    let persisted = Stores::load(&h.dir).unwrap();
    assert_eq!(persisted.work.engagement.candidates[&1].state, status);
    assert!(
        persisted.work.engagement.candidates[&1]
            .message_id
            .is_none()
    );
    assert_eq!(persisted.work.engagement.charges.len(), 1);
    let authorizations = fake.inner.auth.load(Ordering::SeqCst);
    if matches!(
        observation,
        Observation::FinalSourceDenied
            | Observation::FinalProofDenied
            | Observation::FinalProofUnavailable
            | Observation::FinalAuthorizeDenied
            | Observation::FinalAuthorizeTimeout
            | Observation::FinalPolicyDenied
    ) {
        assert_eq!(authorizations, 2);
    }
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .len(),
        1
    );
    h.state
        .clone()
        .deliver_engagement(&fake, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(fake.inner.sends.load(Ordering::SeqCst), sends);
    assert_eq!(fake.inner.auth.load(Ordering::SeqCst), authorizations);
    telemetry.stop();
    telemetry.joined().await.unwrap();
    let log = std::fs::read_to_string(home.join("Library/Logs/abbey-bot/abbey-bot.events.jsonl"))
        .unwrap();
    assert!(!log.contains("transient source") && !log.contains("SYNTHETIC_UNUSED_FIXTURE"));
    let events: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let failures: Vec<_> = events
        .iter()
        .filter(|event| event["code"] == "engagement_failure")
        .collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["outcome"], event_outcome);
    assert_eq!(
        failures[0].get("error_category").and_then(|v| v.as_str()),
        category
    );
    assert!(
        events
            .iter()
            .all(|event| event["code"] != "discord_first_post"
                && event["code"] != "engagement_completed")
    );
    if matches!(observation, Observation::FinalAuthorizeTimeout) {
        assert!(failures[0]["duration_ms"].as_u64().unwrap() >= 30_000);
    }
    h.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn engagement_delivery_final_checks_ambiguous_proofs_omit_cause() {
    for observation in [
        Observation::FinalProofDenied,
        Observation::FinalSourceDenied,
    ] {
        retained_outcome(observation, None, "failed", CandidateState::Rejected, 0).await;
    }
}
#[cfg(unix)]
#[tokio::test]
async fn engagement_delivery_final_checks_ambiguous_authorization_omits_cause() {
    retained_outcome(
        Observation::FinalAuthorizeDenied,
        None,
        "failed",
        CandidateState::Rejected,
        0,
    )
    .await;
}
#[cfg(unix)]
#[tokio::test]
async fn engagement_delivery_final_checks_known_unavailability_retains_category() {
    retained_outcome(
        Observation::FinalProofUnavailable,
        Some("unavailable"),
        "failed",
        CandidateState::Rejected,
        0,
    )
    .await;
}
#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn engagement_delivery_final_checks_authorization_deadline_is_timeout() {
    retained_outcome(
        Observation::FinalAuthorizeTimeout,
        Some("timeout"),
        "failed",
        CandidateState::Rejected,
        0,
    )
    .await;
}
#[cfg(unix)]
#[tokio::test]
async fn engagement_delivery_final_checks_known_policy_denial_is_authorization() {
    retained_outcome(
        Observation::FinalPolicyDenied,
        Some("authorization"),
        "failed",
        CandidateState::Rejected,
        0,
    )
    .await;
}
