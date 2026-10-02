use super::*;
#[test]
fn community_missing_join_capability_is_honest() {
    assert!(!join_events_available());
}
use crate::{
    engagement::{lifecycle::EngagementReservation, schedule::CandidateProposal, *},
    persist::{FsPersistenceSink, Stores},
    runtime::engagement_delivery::{AuthorizedDestination, SendFailure},
    service::{ServiceSupervisor, persistence::PersistenceWriter},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;
const NOW: u64 = 1_790_683_200;
struct Harness {
    state: Arc<AppState>,
    writer: PersistenceWriter,
    dir: PathBuf,
    _supervisor: ServiceSupervisor,
}
impl Harness {
    fn new() -> Self {
        Self::new_quiet(false)
    }
    fn new_quiet(quiet: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-community-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state =
            AppState::in_memory_with_persistence(Some(dir.clone()), Arc::new(FsPersistenceSink));
        Arc::get_mut(&mut state).unwrap().quiet = quiet;
        {
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.guilds).update("discord:7", &mut *stores, |s| {
                s.unsolicited = true;
                s.learning_enabled = true;
                s.unsolicited_per_hour = 1;
                s.reply_cooldown_seconds = 0;
            });
            let e = &mut stores.work.engagement;
            e.guild_features.insert(
                7,
                GuildFeaturePolicy {
                    revision: 1,
                    enabled: std::collections::BTreeSet::from([CommunityFeature::Questions]),
                    channels: std::collections::BTreeMap::from([(
                        CommunityFeature::Questions,
                        std::collections::BTreeSet::from([9]),
                    )]),
                },
            );
            let source = SourceRef {
                scope: EngagementScope::Guild {
                    guild: 7,
                    channel: 9,
                },
                message: 10,
                author: 2,
                revision: 1,
                at: NOW - 86_400,
            };
            e.propose_community(
                &CommunityFacts {
                    rows: vec![CommunityFact {
                        kind: EngagementKind::UnansweredQuestion,
                        scope: source.scope.clone(),
                        source: Some(source),
                        evidence: CommunityEvidence::Message,
                        at: NOW - 86_400,
                        useful: true,
                        current: true,
                    }],
                    join_events_available: false,
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
    fn state(&self) -> CandidateState {
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
    PolicyOff,
    Reply,
    Deleted,
    Uncertain,
}
struct Fake {
    state: Arc<AppState>,
    auth: AtomicUsize,
    sends: AtomicUsize,
    race: Race,
    dir: PathBuf,
}
impl Fake {
    fn new(h: &Harness, race: Race) -> Self {
        Self {
            state: h.state.clone(),
            auth: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            race,
            dir: h.dir.clone(),
        }
    }
}
impl EngagementTransport for Fake {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        assert!(self.state.stores.try_lock().is_ok());
        if self.auth.fetch_add(1, Ordering::SeqCst) == 1 {
            match self.race {
                Race::PolicyOff => {
                    self.state
                        .commit_work_owned(|s| {
                            let p = s.engagement.guild_features.get_mut(&7).unwrap();
                            p.enabled.clear();
                            p.revision += 1;
                            Ok(())
                        })
                        .await?;
                }
                Race::Reply => {
                    self.state
                        .observe_engagement(SourceRef {
                            scope: r.scope.clone(),
                            message: 11,
                            author: 3,
                            revision: 1,
                            at: NOW,
                        })
                        .await?;
                }
                Race::Deleted => {
                    self.state.delete_engagement_source(9, 10).await?;
                }
                _ => {}
            }
        }
        Ok(AuthorizedDestination {
            channel: 9,
            member: r.member,
            scope: r.scope.clone(),
        })
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        Ok(true)
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        Ok("Synthetic substantive forum question".into())
    }
    async fn candidate_current(&self, _: &Candidate, _: Option<u64>) -> Result<bool, WorkError> {
        Ok(true)
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        Ok("Which part of the experiment is still blocked?".into())
    }
    async fn send(&self, channel: u64, _: &str) -> Result<u64, SendFailure> {
        assert_eq!(channel, 9);
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(Stores::state_path(&self.dir)).unwrap()).unwrap();
        assert_eq!(
            disk["work"]["engagement"]["candidates"]["1"]["state"],
            "Reserved"
        );
        self.sends.fetch_add(1, Ordering::SeqCst);
        if matches!(self.race, Race::Uncertain) {
            Err(SendFailure::Uncertain)
        } else {
            Ok(12)
        }
    }
}
#[tokio::test]
async fn community_retained_duplicate_ticks_send_once_without_member_charge() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::None);
    let (a, b) = tokio::join!(
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW),
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
    );
    a.unwrap();
    b.unwrap();
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    assert_eq!(h.state(), CandidateState::Sent);
    assert!(
        Stores::load(&h.dir)
            .unwrap()
            .work
            .engagement
            .charges
            .is_empty()
    );
    h.finish().await;
}
#[tokio::test]
async fn community_fresh_feature_off_human_reply_or_delete_prevents_send() {
    for race in [Race::PolicyOff, Race::Reply, Race::Deleted] {
        let h = Harness::new();
        let f = Fake::new(&h, race);
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        assert_eq!(f.sends.load(Ordering::SeqCst), 0);
        assert!(matches!(
            h.state(),
            CandidateState::Cancelled | CandidateState::Rejected
        ));
        h.finish().await;
    }
}
#[tokio::test]
async fn community_uncertain_is_durable_review_without_retry() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::Uncertain);
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(h.state(), CandidateState::ReviewRequired);
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
#[tokio::test]
async fn community_and_personal_share_one_guild_budget() {
    let h = Harness::new();
    let f = Fake::new(&h, Race::None);
    h.state
        .commit_engagement(|s| {
            s.member_policies.insert(
                2,
                MemberPolicy {
                    daily_limit: Some(1),
                    timezone: Some("UTC".into()),
                    quiet_start: 0,
                    quiet_end: 0,
                    ..Default::default()
                },
            );
            let source = SourceRef {
                scope: EngagementScope::Guild {
                    guild: 7,
                    channel: 9,
                },
                message: 20,
                author: 2,
                revision: 1,
                at: NOW - 86_400,
            };
            s.eligibility.entry(2).or_default().insert(source.clone());
            s.propose(
                CandidateProposal {
                    kind: EngagementKind::FollowUp,
                    source: Some(source.clone()),
                    member: Some(2),
                    scope: source.scope,
                    due_at: NOW,
                    introduction_id: None,
                },
                NOW,
            )?;
            Ok(())
        })
        .await
        .unwrap();
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        AppState::lock(&h.state.stores).work.engagement.candidates[&2].state,
        CandidateState::Pending
    );
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    h.finish().await;
}
#[tokio::test]
async fn community_act_quiet_and_health_block_public_delivery() {
    for blocker in 0..3 {
        let h = Harness::new_quiet(blocker == 0);
        if blocker == 0 {
        } else if blocker == 2 {
            h.state
                .engagement_events_healthy
                .store(false, Ordering::SeqCst);
        } else {
            let mut stores = AppState::lock(&h.state.stores);
            AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |s| {
                s.unsolicited = false;
            });
        }
        let f = Fake::new(&h, Race::None);
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        assert_eq!(f.sends.load(Ordering::SeqCst), 0);
        assert_eq!(h.state(), CandidateState::Pending);
        h.finish().await;
    }
}
#[tokio::test]
async fn community_learning_off_preserves_public_delivery_and_shared_budget() {
    let h = Harness::new();
    {
        let mut stores = AppState::lock(&h.state.stores);
        AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |s| {
            s.learning_enabled = false;
        });
    }
    let f = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&f, CancellationToken::new(), || NOW)
        .await
        .unwrap();
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    assert_eq!(h.state(), CandidateState::Sent);
    assert!(!h.state.engagement_guild_gate(
        &EngagementScope::Guild {
            guild: 7,
            channel: 8,
        },
        NOW,
        true,
    ));
    h.finish().await;
}
#[tokio::test]
async fn community_join_switch_owns_new_path_but_legacy_off_remains() {
    let h = Harness::new();
    assert!(!h.state.community_join(7, 2, NOW, false).await.unwrap());
    h.state
        .commit_engagement(|s| {
            let p = s.guild_features.get_mut(&7).unwrap();
            p.enabled.insert(CommunityFeature::Welcomes);
            p.channels.insert(
                CommunityFeature::Welcomes,
                std::collections::BTreeSet::from([9]),
            );
            Ok(())
        })
        .await
        .unwrap();
    assert!(h.state.community_join(7, 2, NOW, false).await.unwrap());
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .candidates
            .len(),
        1
    );
    h.finish().await;
}
#[test]
fn community_bot_source_never_observed_or_eligible() {
    let event = crate::platform::SocialEvent {
        network: crate::platform::SocialNetwork::Discord,
        kind: crate::platform::EventKind::Message {
            text: "quoted synthetic question?".into(),
            attachments: Vec::new(),
        },
        native_message_id: "10".into(),
        native_channel_id: "9".into(),
        native_guild_id: Some("7".into()),
        native_user_id: "2".into(),
        user_display_name: "Synthetic".into(),
        is_bot: true,
        timestamp: NOW,
    };
    assert_eq!(
        crate::runtime::engagement_candidates::event_source(&event),
        Err(WorkError::Denied)
    );
}
