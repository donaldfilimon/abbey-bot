use super::*;
use crate::engagement::{
    CommunityFeature, GuildFeaturePolicy,
    community::{CommunityEvidence, CommunityFact, CommunityFacts},
};

#[derive(Clone, Copy, Debug)]
enum Path {
    Message,
    Exchange,
    CommunityMessage,
    Welcome,
    Project,
}
struct PlanTransport {
    path: Path,
    response: u64,
    revoke_after_proof: bool,
    calls: std::sync::Mutex<Vec<&'static str>>,
    dir: PathBuf,
}
impl PlanTransport {
    fn record(&self, call: &'static str) {
        self.calls.lock().unwrap().push(call);
    }
}
impl EngagementTransport for PlanTransport {
    async fn authorize(
        &self,
        reservation: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        let final_check = !self.calls.lock().unwrap().is_empty();
        self.record(if final_check {
            "final_authorize"
        } else {
            "preflight"
        });
        if final_check && self.revoke_after_proof {
            return Err(WorkError::Denied);
        }
        Ok(AuthorizedDestination {
            channel: 3,
            member: reservation.member,
            scope: reservation.scope.clone(),
        })
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        assert!(matches!(
            self.path,
            Path::Message | Path::Exchange | Path::CommunityMessage
        ));
        self.record("source");
        Ok(true)
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        assert!(matches!(self.path, Path::Message));
        self.record("hydrate_message");
        Ok("transient message context".into())
    }
    async fn hydrate_exchange(&self, _: &Candidate, response: u64) -> Result<String, WorkError> {
        assert!(matches!(self.path, Path::Exchange));
        assert_eq!(response, self.response);
        self.record("hydrate_exchange");
        Ok("transient exchange context".into())
    }
    async fn hydrate_community(&self, _: &AppState, _: &Candidate) -> Result<String, WorkError> {
        assert!(matches!(
            self.path,
            Path::CommunityMessage | Path::Welcome | Path::Project
        ));
        self.record("hydrate_community");
        Ok("transient community context".into())
    }
    async fn candidate_current(
        &self,
        _: &Candidate,
        response: Option<u64>,
    ) -> Result<bool, WorkError> {
        match self.path {
            Path::Message => assert_eq!(response, None),
            Path::Exchange => assert_eq!(response, Some(self.response)),
            _ => panic!("community uses its own evidence proof"),
        }
        self.record("exchange_current");
        Ok(true)
    }
    async fn community_current(&self, _: &AppState, _: &Candidate) -> Result<bool, WorkError> {
        assert!(matches!(
            self.path,
            Path::CommunityMessage | Path::Welcome | Path::Project
        ));
        self.record("community_current");
        Ok(true)
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        context: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        assert!(context.starts_with("transient "));
        self.record("generate");
        Ok("How is the project going?".into())
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        assert_eq!(channel, 3);
        assert_eq!(body, "How is the project going?");
        assert_eq!(self.calls.lock().unwrap().last(), Some(&"final_authorize"));
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(Stores::state_path(&self.dir)).unwrap()).unwrap();
        assert_eq!(
            persisted["work"]["engagement"]["candidates"]["1"]["state"],
            "Reserved"
        );
        self.record("send");
        Ok(123)
    }
}
fn setup(h: &Harness, path: Path) {
    let mut stores = AppState::lock(&h.state.stores);
    if matches!(path, Path::Message) {
        return;
    }
    if matches!(path, Path::Exchange) {
        stores.work.engagement.responses.insert(4, 8);
        return;
    }
    AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |settings| {
        settings.unsolicited = true;
        settings.learning_enabled = true;
        settings.unsolicited_per_hour = 4;
        settings.reply_cooldown_seconds = 0;
    });
    let (kind, feature, evidence, at) = match path {
        Path::CommunityMessage => (
            EngagementKind::UnansweredQuestion,
            CommunityFeature::Questions,
            CommunityEvidence::Message,
            NOW - 86_400,
        ),
        Path::Welcome => (
            EngagementKind::Welcome,
            CommunityFeature::Welcomes,
            CommunityEvidence::Join {
                member: 5,
                joined_at: NOW - 10,
            },
            NOW - 10,
        ),
        Path::Project => (
            EngagementKind::ProjectCheckIn,
            CommunityFeature::Projects,
            CommunityEvidence::Project {
                project: 9,
                revision: 1,
                actor: 2,
                audience: [2].into(),
                content: [crate::work::WorkContentRef::Task {
                    project: 9,
                    id: 10,
                    revision: 1,
                }]
                .into(),
            },
            NOW,
        ),
        _ => unreachable!(),
    };
    let store = &mut stores.work.engagement;
    store.candidates.clear();
    store.sequence = 0;
    store.guild_features.insert(
        7,
        GuildFeaturePolicy {
            revision: 1,
            enabled: [feature].into(),
            channels: [(feature, [3].into())].into(),
        },
    );
    let scope = EngagementScope::Guild {
        guild: 7,
        channel: 3,
    };
    store
        .propose_community(
            &CommunityFacts {
                rows: vec![CommunityFact {
                    kind,
                    scope: scope.clone(),
                    source: matches!(path, Path::CommunityMessage).then_some(SourceRef {
                        scope,
                        message: 4,
                        author: 2,
                        revision: 1,
                        at,
                    }),
                    evidence,
                    at,
                    useful: true,
                    current: true,
                }],
                join_events_available: true,
            },
            NOW,
        )
        .unwrap();
}

#[tokio::test]
async fn engagement_delivery_plan_source_exchange_and_community_reauthorize_after_external_proofs()
{
    for path in [
        Path::Message,
        Path::Exchange,
        Path::CommunityMessage,
        Path::Welcome,
        Path::Project,
    ] {
        for revoke_after_proof in [false, true] {
            let h = Harness::new();
            setup(&h, path);
            let transport = PlanTransport {
                path,
                response: 8,
                revoke_after_proof,
                calls: std::sync::Mutex::default(),
                dir: h.dir.clone(),
            };
            h.state
                .clone()
                .deliver_engagement(&transport, CancellationToken::new(), || NOW)
                .await
                .unwrap();
            let mut expected = match path {
                Path::Message => vec![
                    "preflight",
                    "hydrate_message",
                    "generate",
                    "source",
                    "exchange_current",
                    "final_authorize",
                ],
                Path::Exchange => vec![
                    "preflight",
                    "hydrate_exchange",
                    "generate",
                    "source",
                    "exchange_current",
                    "final_authorize",
                ],
                Path::CommunityMessage => vec![
                    "preflight",
                    "hydrate_community",
                    "generate",
                    "source",
                    "community_current",
                    "final_authorize",
                ],
                Path::Welcome | Path::Project => vec![
                    "preflight",
                    "hydrate_community",
                    "generate",
                    "community_current",
                    "final_authorize",
                ],
            };
            if !revoke_after_proof {
                expected.push("send");
            }
            assert_eq!(*transport.calls.lock().unwrap(), expected, "{path:?}");
            assert_eq!(
                h.status(),
                if revoke_after_proof {
                    CandidateState::Rejected
                } else {
                    CandidateState::Sent
                },
                "{path:?}"
            );
            let persisted = Stores::load(&h.dir).unwrap();
            assert_eq!(
                persisted.work.engagement.charges.len(),
                usize::from(matches!(
                    path,
                    Path::Message | Path::Exchange | Path::Project
                )),
                "{path:?}"
            );
            h.state
                .clone()
                .deliver_engagement(&transport, CancellationToken::new(), || NOW)
                .await
                .unwrap();
            assert_eq!(
                *transport.calls.lock().unwrap(),
                expected,
                "settled {path:?} was replayed"
            );
            h.finish().await;
        }
    }
}

struct ReplacedExchange {
    inner: PlanTransport,
    state: Arc<AppState>,
    calls: AtomicUsize,
    replace_on: usize,
}
impl EngagementTransport for ReplacedExchange {
    async fn authorize(
        &self,
        reservation: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        let destination = self.inner.authorize(reservation).await?;
        if self.calls.fetch_add(1, Ordering::SeqCst) == self.replace_on {
            self.state
                .commit_work_owned(|store| {
                    store.engagement.responses.insert(4, 9);
                    Ok(())
                })
                .await?;
        }
        Ok(destination)
    }
    async fn source_exists(&self, source: &SourceRef) -> Result<bool, WorkError> {
        self.inner.source_exists(source).await
    }
    async fn hydrate(&self, candidate: &Candidate) -> Result<String, WorkError> {
        self.inner.hydrate(candidate).await
    }
    async fn hydrate_exchange(
        &self,
        candidate: &Candidate,
        response: u64,
    ) -> Result<String, WorkError> {
        self.inner.hydrate_exchange(candidate, response).await
    }
    async fn candidate_current(
        &self,
        candidate: &Candidate,
        response: Option<u64>,
    ) -> Result<bool, WorkError> {
        self.inner.candidate_current(candidate, response).await
    }
    async fn generate(
        &self,
        state: &AppState,
        candidate: &Candidate,
        context: &str,
        now: u64,
    ) -> Result<String, WorkError> {
        self.inner.generate(state, candidate, context, now).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        self.inner.send(channel, body).await
    }
}

#[tokio::test]
async fn engagement_delivery_plan_changed_exchange_defers_before_charge_or_rejects_before_send() {
    for replace_on in [0, 1] {
        let h = Harness::new();
        setup(&h, Path::Exchange);
        let transport = ReplacedExchange {
            inner: PlanTransport {
                path: Path::Exchange,
                response: 8,
                revoke_after_proof: false,
                calls: std::sync::Mutex::default(),
                dir: h.dir.clone(),
            },
            state: h.state.clone(),
            calls: AtomicUsize::new(0),
            replace_on,
        };
        h.state
            .clone()
            .deliver_engagement(&transport, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        let state = if replace_on == 0 {
            CandidateState::Pending
        } else {
            CandidateState::Rejected
        };
        assert_eq!(h.status(), state);
        assert_eq!(
            AppState::lock(&h.state.stores)
                .work
                .engagement
                .charges
                .len(),
            replace_on
        );
        let calls = transport.inner.calls.lock().unwrap().clone();
        assert!(!calls.contains(&"send"));
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == "final_authorize")
                .count(),
            replace_on
        );
        let healthy = PlanTransport {
            path: Path::Exchange,
            response: 9,
            revoke_after_proof: false,
            calls: std::sync::Mutex::default(),
            dir: h.dir.clone(),
        };
        h.state
            .clone()
            .deliver_engagement(&healthy, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        assert_eq!(
            healthy.calls.lock().unwrap().contains(&"send"),
            replace_on == 0
        );
        let persisted = Stores::load(&h.dir).unwrap();
        assert_eq!(persisted.work.engagement.charges.len(), 1);
        assert_eq!(persisted.work.engagement.responses[&4], 9);
        h.finish().await;
    }
}
