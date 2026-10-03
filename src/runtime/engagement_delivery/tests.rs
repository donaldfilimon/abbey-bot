use super::*;
#[test]
fn engagement_delivery_destinations_require_exact_scope_member_and_origin() {
    let r = EngagementReservation {
        introduction: None,
        candidate_id: 1,
        revision: 1,
        policy_revision: 1,
        scope: EngagementScope::Dm {
            member: 2,
            channel: 3,
        },
        member: Some(2),
        destination: DestinationPreference::Origin,
    };
    let mut d = AuthorizedDestination {
        channel: 3,
        member: Some(2),
        scope: r.scope.clone(),
    };
    assert!(matches(&r, &d));
    d.channel = 4;
    assert!(!matches(&r, &d));
    d.channel = 3;
    d.member = Some(4);
    assert!(!matches(&r, &d));
}
use crate::{
    engagement::{EngagementKind, MemberPolicy, schedule::CandidateProposal},
    persist::{FsPersistenceSink, Stores},
    service::{ServiceSupervisor, persistence::PersistenceWriter},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
const NOW: u64 = 1_790_683_200;
struct Harness {
    state: Arc<AppState>,
    writer: PersistenceWriter,
    dir: PathBuf,
    _supervisor: ServiceSupervisor,
}
impl Harness {
    fn new() -> Self {
        Self::new_with_voice(false)
    }
    fn new_with_voice(voice: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-engagement-delivery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state =
            AppState::in_memory_with_persistence(Some(dir.clone()), Arc::new(FsPersistenceSink));
        if voice {
            Arc::get_mut(&mut state)
                .unwrap()
                .providers
                .register_test_adapter(Arc::new(InvitationProvider(
                    crate::provider::ProviderId::parse("local").unwrap(),
                )));
            let backend = if cfg!(target_os = "macos") {
                crate::voice::VoiceBackendConfig::Local(
                    crate::offline_voice::OfflineVoiceConfig::from_values(
                        None, None, None, None, None,
                    )
                    .unwrap(),
                )
            } else {
                crate::voice::VoiceBackendConfig::Disabled
            };
            state
                .voice_registry
                .configure(
                    crate::voice::VoiceConfig::selected_only(7, 9, backend, true).template(),
                    None,
                    None,
                    Arc::new(crate::inspect::VoiceInspectRegistry::default()),
                )
                .unwrap();
        }
        {
            let mut stores = AppState::lock(&state.stores);
            let e = &mut stores.work.engagement;
            e.member_policies.insert(
                2,
                MemberPolicy {
                    revision: 1,
                    daily_limit: Some(1),
                    timezone: Some("UTC".into()),
                    quiet_start: 0,
                    quiet_end: 0,
                    ..Default::default()
                },
            );
            let source = SourceRef {
                scope: EngagementScope::Dm {
                    member: 2,
                    channel: 3,
                },
                message: 4,
                author: 2,
                revision: 1,
                at: NOW - 86400,
            };
            e.eligibility.entry(2).or_default().insert(source.clone());
            e.propose(
                CandidateProposal {
                    kind: EngagementKind::FollowUp,
                    source: Some(source.clone()),
                    member: Some(2),
                    scope: source.scope,
                    due_at: NOW,
                    introduction_id: None,
                },
                NOW,
            )
            .unwrap();
        }
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = state.attach_service(supervisor.operations());
        Self {
            state,
            writer,
            dir,
            _supervisor: supervisor,
        }
    }
    async fn finish(mut self) {
        self.writer.stop();
        self.writer.joined().await.unwrap();
        std::fs::remove_dir_all(self.dir).unwrap();
    }
    fn status(&self) -> CandidateState {
        AppState::lock(&self.state.stores)
            .work
            .engagement
            .candidates[&1]
            .state
    }
}
#[derive(Clone, Copy)]
enum Race {
    None,
    Stop,
    Reply,
    Deleted,
    Denied,
    Mismatch,
    Uncertain,
    Shutdown,
    BlockedDm,
    Timeout,
    AcceptedReply,
}
struct Fake {
    state: Arc<AppState>,
    dir: PathBuf,
    auth: AtomicUsize,
    sends: AtomicUsize,
    race: Race,
    cancel: CancellationToken,
}
impl Fake {
    fn new(h: &Harness, race: Race) -> Self {
        Self {
            state: h.state.clone(),
            dir: h.dir.clone(),
            auth: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            race,
            cancel: CancellationToken::new(),
        }
    }
}
impl EngagementTransport for Fake {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        assert!(self.state.stores.try_lock().is_ok());
        let call = self.auth.fetch_add(1, Ordering::SeqCst);
        if call == 1 {
            match self.race {
                Race::Stop => {
                    self.state
                        .commit_work_owned(|s| {
                            s.engagement
                                .member_policies
                                .get_mut(&2)
                                .unwrap()
                                .global_stop = true;
                            Ok(())
                        })
                        .await?;
                }
                Race::Reply => {
                    self.state
                        .commit_work_owned(|s| {
                            s.engagement.cancel_member_origin(2, &r.scope, NOW);
                            Ok(())
                        })
                        .await?;
                }
                Race::Denied => return Err(WorkError::Denied),
                _ => {}
            }
        }
        Ok(AuthorizedDestination {
            channel: if call == 1 && matches!(self.race, Race::Mismatch) {
                9
            } else {
                3
            },
            member: r.member,
            scope: r.scope.clone(),
        })
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        Ok(!matches!(self.race, Race::Deleted))
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        Ok("synthetic unresolved project question".into())
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        Ok("How did the synthetic project test go?".into())
    }
    async fn send(&self, _: u64, _: &str) -> Result<u64, SendFailure> {
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(Stores::state_path(&self.dir)).unwrap()).unwrap();
        assert_eq!(
            disk["work"]["engagement"]["candidates"]["1"]["state"],
            "Reserved"
        );
        assert_eq!(
            disk["work"]["engagement"]["charges"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        self.sends.fetch_add(1, Ordering::SeqCst);
        match self.race {
            Race::Uncertain => Err(SendFailure::Uncertain),
            Race::BlockedDm => Err(SendFailure::Rejected),
            Race::AcceptedReply => {
                self.state
                    .commit_work_owned(|s| {
                        s.engagement.cancel_member_origin(
                            2,
                            &EngagementScope::Dm {
                                member: 2,
                                channel: 3,
                            },
                            NOW,
                        );
                        Ok(())
                    })
                    .await
                    .unwrap();
                Ok(123)
            }
            Race::Timeout => std::future::pending().await,
            Race::Shutdown => {
                self.cancel.cancel();
                std::future::pending().await
            }
            _ => Ok(123),
        }
    }
}
#[tokio::test]
async fn engagement_delivery_duplicate_and_concurrent_ticks_charge_and_send_once() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::None);
    let (a, b) = tokio::join!(
        h.state
            .clone()
            .deliver_engagement(&f, f.cancel.clone(), || NOW),
        h.state
            .clone()
            .deliver_engagement(&f, f.cancel.clone(), || NOW)
    );
    a.unwrap();
    b.unwrap();
    h.state
        .clone()
        .deliver_engagement(&f, f.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    assert_eq!(h.status(), CandidateState::Sent);
    let loaded = Stores::load(&h.dir).unwrap();
    assert_eq!(loaded.work.engagement.candidates[&1].message_id, Some(123));
    h.finish().await;
}
#[tokio::test]
async fn engagement_delivery_late_stop_reply_source_access_and_destination_prevent_send() {
    for race in [
        Race::Stop,
        Race::Reply,
        Race::Deleted,
        Race::Denied,
        Race::Mismatch,
    ] {
        let h = Harness::new();
        let f = Fake::new(&h, race);
        h.state
            .clone()
            .deliver_engagement(&f, f.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(f.sends.load(Ordering::SeqCst), 0);
        assert!(matches!(
            h.status(),
            CandidateState::Rejected | CandidateState::Cancelled
        ));
        assert_eq!(
            AppState::lock(&h.state.stores)
                .work
                .engagement
                .charges
                .len(),
            1
        );
        h.finish().await;
    }
}
#[tokio::test]
async fn engagement_delivery_remote_uncertainty_and_shutdown_are_review_only_without_retry() {
    for race in [Race::Uncertain, Race::Shutdown] {
        let h = Harness::new();
        let f = Fake::new(&h, race);
        h.state
            .clone()
            .deliver_engagement(&f, f.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(h.status(), CandidateState::ReviewRequired);
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        assert_eq!(f.sends.load(Ordering::SeqCst), 1);
        assert_eq!(
            Stores::load(&h.dir).unwrap().work.engagement.candidates[&1].state,
            CandidateState::ReviewRequired
        );
        h.finish().await;
    }
}
#[tokio::test]
async fn engagement_delivery_and_pipeline_share_atomic_guild_capacity() {
    let h = Harness::new();
    let settings = {
        let mut stores = AppState::lock(&h.state.stores);
        AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |s| {
            s.unsolicited = true;
            s.learning_enabled = true;
            s.unsolicited_per_hour = 1;
            s.reply_cooldown_seconds = 0;
        })
    };
    let scope = EngagementScope::Guild {
        guild: 7,
        channel: 3,
    };
    let limits = crate::pipeline::RateLimits {
        budget: &h.state.budget,
        cooldown: &h.state.cooldown,
    };
    let (a, b) = tokio::join!(
        async { h.state.engagement_guild_gate(&scope, NOW, true) },
        async {
            limits
                .try_acquire("discord:7", "discord:8", &settings, NOW)
                .is_ok()
        }
    );
    assert_eq!(usize::from(a) + usize::from(b), 1);
    assert!(!h.state.engagement_guild_gate(&scope, NOW, true));
    h.finish().await;
}
#[tokio::test]
async fn engagement_delivery_known_destination_rejection_is_terminal() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::BlockedDm);
    h.state
        .clone()
        .deliver_engagement(&f, f.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(h.status(), CandidateState::Rejected);
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    h.finish().await;
}
#[tokio::test]
async fn engagement_delivery_retained_owner_settles_while_service_drains() {
    let mut h = Harness::new();
    let state = h.state.clone();
    let dir = h.dir.clone();
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    struct Draining {
        fake: Fake,
        started: Arc<tokio::sync::Notify>,
    }
    impl EngagementTransport for Draining {
        async fn authorize(
            &self,
            r: &EngagementReservation,
        ) -> Result<AuthorizedDestination, WorkError> {
            self.fake.authorize(r).await
        }
        async fn source_exists(&self, s: &SourceRef) -> Result<bool, WorkError> {
            self.fake.source_exists(s).await
        }
        async fn hydrate(&self, c: &Candidate) -> Result<String, WorkError> {
            self.fake.hydrate(c).await
        }
        async fn generate(
            &self,
            s: &AppState,
            c: &Candidate,
            t: &str,
            n: u64,
        ) -> Result<String, WorkError> {
            self.fake.generate(s, c, t, n).await
        }
        async fn send(&self, _: u64, _: &str) -> Result<u64, SendFailure> {
            self.started.notify_one();
            std::future::pending().await
        }
    }
    let receipt = h
        ._supervisor
        .operations()
        .spawn_operation(
            crate::service::OperationKind::EngagementDelivery,
            move |cancel| async move {
                let transport = Draining {
                    fake: Fake {
                        state: state.clone(),
                        dir,
                        auth: AtomicUsize::new(0),
                        sends: AtomicUsize::new(0),
                        race: Race::None,
                        cancel: cancel.clone(),
                    },
                    started: signal,
                };
                state
                    .deliver_engagement(&transport, cancel, || NOW)
                    .await
                    .unwrap();
                crate::service::TaskExit::Returned
            },
        )
        .unwrap();
    drop(receipt);
    started.notified().await;
    let shutdown = h._supervisor.begin_draining(
        crate::service::ShutdownReason::Signal,
        tokio::time::Instant::now(),
    );
    let report = h
        ._supervisor
        .cancel_and_reap(shutdown.budget.stage(tokio::time::Instant::now()))
        .await;
    assert_eq!(report.outcome, crate::service::ReapOutcome::Joined);
    assert_eq!(h.status(), CandidateState::ReviewRequired);
    assert_eq!(
        Stores::load(&h.dir).unwrap().work.engagement.candidates[&1].state,
        CandidateState::ReviewRequired
    );
    h.finish().await;
}
#[tokio::test(start_paused = true)]
async fn engagement_delivery_timeout_after_remote_acceptance_keeps_charge_and_review() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::Timeout);
    let delivery = h
        .state
        .clone()
        .deliver_engagement(&f, f.cancel.clone(), || NOW);
    tokio::pin!(delivery);
    tokio::select! {
        result = &mut delivery => panic!("send should be pending: {result:?}"),
        () = async {
            while f.sends.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
            tokio::time::advance(Duration::from_secs(31)).await;
        } => {}
    }
    delivery.await.unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    assert_eq!(h.status(), CandidateState::ReviewRequired);
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .len(),
        1
    );
    h.finish().await;
}
#[tokio::test]
async fn engagement_delivery_member_budget_is_shared_across_origins() {
    let h = Harness::new();
    h.state
        .commit_work_owned(|s| {
            let e = &mut s.engagement;
            let source = SourceRef {
                scope: EngagementScope::Dm {
                    member: 2,
                    channel: 5,
                },
                message: 6,
                author: 2,
                revision: 1,
                at: NOW - 86400,
            };
            e.eligibility.get_mut(&2).unwrap().insert(source.clone());
            e.propose(
                CandidateProposal {
                    kind: EngagementKind::FollowUp,
                    source: Some(source.clone()),
                    scope: source.scope,
                    member: Some(2),
                    due_at: NOW,
                    introduction_id: None,
                },
                NOW,
            )
            .map(|_| ())
        })
        .await
        .unwrap();
    let f = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&f, f.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    {
        let stores = AppState::lock(&h.state.stores);
        let e = &stores.work.engagement;
        assert_eq!(e.charges.len(), 1);
        assert_eq!(e.candidates[&2].state, CandidateState::Pending);
    }
    h.finish().await;
}
#[tokio::test]
async fn engagement_delivery_accepted_receipt_reconciles_concurrent_cancellation() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::AcceptedReply);
    h.state
        .clone()
        .deliver_engagement(&f, f.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(h.status(), CandidateState::Sent);
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .len(),
        1
    );
    assert_eq!(
        Stores::load(&h.dir).unwrap().work.engagement.candidates[&1].message_id,
        Some(123)
    );
    h.finish().await;
}

/// Fake authorization/send surround the real Discord invitation body/hydration;
/// no gateway, voice activation, provider call or participant consent occurs.
struct InvitationAdapter {
    real: crate::gateway::engagement_delivery::DiscordEngagementDelivery,
    auth: AtomicUsize,
    bodies: std::sync::Mutex<Vec<String>>,
}
impl EngagementTransport for InvitationAdapter {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        self.auth.fetch_add(1, Ordering::SeqCst);
        Ok(AuthorizedDestination {
            channel: 3,
            member: r.member,
            scope: r.scope.clone(),
        })
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        panic!("A slash request must not invent a human message source")
    }
    async fn hydrate(&self, c: &Candidate) -> Result<String, WorkError> {
        self.real.hydrate(c).await
    }
    async fn candidate_current(
        &self,
        c: &Candidate,
        response: Option<u64>,
    ) -> Result<bool, WorkError> {
        self.real.candidate_current(c, response).await
    }
    async fn generate(
        &self,
        state: &AppState,
        c: &Candidate,
        source: &str,
        now: u64,
    ) -> Result<String, WorkError> {
        self.real.generate(state, c, source, now).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        assert_eq!(channel, 3);
        self.bodies.lock().unwrap().push(body.into());
        Ok(123)
    }
}
#[cfg(target_os = "macos")]
#[tokio::test]
async fn invitation_real_voice_body_uses_retained_delivery_without_source_or_consent() {
    let h = Harness::new_with_voice(true);
    let id = h
        .state
        .commit_work_owned(|s| {
            s.engagement.candidates.get_mut(&1).unwrap().state = CandidateState::Cancelled;
            s.engagement
                .request_invitation(
                    EngagementKind::VoiceInvite,
                    crate::engagement::InvitationRequest {
                        interaction: 8,
                        member: 2,
                        scope: EngagementScope::Dm {
                            member: 2,
                            channel: 3,
                        },
                        at: NOW,
                        activity: None,
                    },
                )
                .map(|id| id.unwrap())
        })
        .await
        .unwrap()
        .0;
    let adapter = InvitationAdapter {
        real: crate::gateway::engagement_delivery::DiscordEngagementDelivery(Arc::new(
            serenity::all::Http::new("synthetic-offline-fixture"),
        )),
        auth: AtomicUsize::new(0),
        bodies: std::sync::Mutex::default(),
    };
    let policies_before = AppState::lock(&h.state.stores)
        .work
        .engagement
        .member_policies
        .clone();
    h.state
        .clone()
        .deliver_engagement(&adapter, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert!(h.state.voice_registry.get(7).is_none());
    assert!(!h.dir.join("voice-consent.json").exists());
    let body = adapter.bodies.lock().unwrap()[0].clone();
    assert_eq!(body, crate::engagement::invitations::voice_invitation());
    assert_eq!(adapter.auth.load(Ordering::SeqCst), 2);
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .member_policies,
        policies_before
    );
    let stored = Stores::load(&h.dir).unwrap();
    assert_eq!(
        stored.work.engagement.candidates[&id].state,
        CandidateState::Sent
    );
    assert_eq!(stored.work.engagement.candidates[&id].message_id, Some(123));
    assert_eq!(stored.work.engagement.charges.len(), 1);
    assert!(stored.work.engagement.candidates[&id].source.is_none());
    h.state
        .clone()
        .deliver_engagement(&adapter, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(adapter.bodies.lock().unwrap().len(), 1);
    h.finish().await;
}

struct InvitationProvider(crate::provider::ProviderId);
impl crate::provider::TurnAdapter for InvitationProvider {
    fn provider_id(&self) -> &crate::provider::ProviderId {
        &self.0
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [crate::llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> crate::provider::TurnFuture<'a> {
        panic!("Invitations must not call the model")
    }
}
#[tokio::test]
async fn invitation_unconfigured_voice_is_rejected_before_charging_or_sending() {
    let h = Harness::new();
    let id = h
        .state
        .commit_work_owned(|s| {
            s.engagement.candidates.get_mut(&1).unwrap().state = CandidateState::Cancelled;
            s.engagement
                .request_invitation(
                    EngagementKind::VoiceInvite,
                    crate::engagement::InvitationRequest {
                        interaction: 8,
                        member: 2,
                        scope: EngagementScope::Dm {
                            member: 2,
                            channel: 3,
                        },
                        at: NOW,
                        activity: None,
                    },
                )
                .map(|id| id.unwrap())
        })
        .await
        .unwrap()
        .0;
    let f = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        AppState::lock(&h.state.stores).work.engagement.candidates[&id].state,
        CandidateState::Rejected
    );
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    assert!(h.state.voice_registry.get(7).is_none());
    h.finish().await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn invitation_disabled_guild_voice_rejects_configured_backend_without_charge() {
    let h = Harness::new_with_voice(true);
    let scope = EngagementScope::Guild {
        guild: 7,
        channel: 3,
    };
    crate::guild::GuildConfigStore::save(
        &mut *AppState::lock(&h.state.stores),
        "discord:7",
        &crate::guild::GuildSettings {
            voice_enabled: false,
            unsolicited: true,
            learning_enabled: true,
            ..Default::default()
        },
    );
    assert!(
        !h.state.voice_invitation_available(&scope),
        "guild voice opt-out is current authority despite global local backend"
    );
    let id = h
        .state
        .commit_work_owned(move |s| {
            s.engagement.candidates.get_mut(&1).unwrap().state = CandidateState::Cancelled;
            s.engagement
                .request_invitation(
                    EngagementKind::VoiceInvite,
                    crate::engagement::InvitationRequest {
                        interaction: 8,
                        member: 2,
                        scope,
                        at: NOW,
                        activity: None,
                    },
                )
                .map(|id| id.unwrap())
        })
        .await
        .unwrap()
        .0;
    let f = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
    assert_eq!(f.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        AppState::lock(&h.state.stores).work.engagement.candidates[&id].state,
        CandidateState::Rejected
    );
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    assert!(h.state.voice_registry.get(7).is_none());
    h.finish().await;
}
#[path = "tests/introductions.rs"]
mod introductions;

#[path = "feedback_tests.rs"]
mod feedback_tests;

#[path = "tests/admission_outcomes.rs"]
mod admission_outcomes;

mod plans;
