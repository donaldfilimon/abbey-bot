use super::*;
use crate::engagement::{CommunityFeature, GuildFeaturePolicy, IntroductionState};
#[derive(Clone, Copy)]
enum Change {
    None,
    Withdraw,
    Stop,
    Access,
    InitialAccess,
    Policy,
    Feature,
    Uncertain,
}
struct IntroductionFake {
    state: Arc<AppState>,
    dir: PathBuf,
    auth: AtomicUsize,
    sends: AtomicUsize,
    change: Change,
}
impl EngagementTransport for IntroductionFake {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        assert!(self.state.stores.try_lock().is_ok());
        let snapshot = r.introduction.as_ref().unwrap();
        assert_eq!(snapshot.introduction.members, [2, 5]);
        assert_eq!(r.destination, DestinationPreference::Origin);
        let call = self.auth.fetch_add(1, Ordering::SeqCst);
        if call == 0 && matches!(self.change, Change::InitialAccess) {
            return Err(WorkError::Denied);
        }
        if call == 1 {
            match self.change {
                Change::Access => return Err(WorkError::Denied),
                Change::Withdraw => {
                    self.state
                        .commit_work_owned(|s| s.engagement.withdraw_introduction(1, 5, 2))
                        .await?;
                }
                Change::Stop => {
                    self.state
                        .commit_work_owned(|s| {
                            crate::commands_engage::stop_for_test(&mut s.engagement, 5, &r.scope)
                        })
                        .await?;
                }
                Change::Policy => {
                    self.state
                        .commit_work_owned(|s| {
                            s.engagement.member_policies.get_mut(&5).unwrap().revision += 1;
                            Ok(())
                        })
                        .await?;
                }
                Change::Feature => {
                    self.state
                        .commit_work_owned(|s| {
                            s.engagement.guild_features.get_mut(&7).unwrap().revision += 1;
                            Ok(())
                        })
                        .await?;
                }
                _ => {}
            }
        }
        Ok(AuthorizedDestination {
            channel: 3,
            member: None,
            scope: r.scope.clone(),
        })
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        panic!("Introduction has no source/history hydration");
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        panic!("Introduction must not retrieve private facts");
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        panic!("Introduction must not call a model");
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        assert_eq!(channel, 3);
        assert_eq!(
            body,
            "Two members have approved sharing these introductions here:\n\nI build compilers.\n\nI build runtimes.\n\nYou’re welcome to connect around what you’ve shared."
        );
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(Stores::state_path(&self.dir)).unwrap()).unwrap();
        assert_eq!(
            disk["work"]["engagement"]["candidates"]["2"]["state"],
            "Reserved"
        );
        assert_eq!(
            disk["work"]["engagement"]["charges"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        self.sends.fetch_add(1, Ordering::SeqCst);
        if matches!(self.change, Change::Uncertain) {
            Err(SendFailure::Uncertain)
        } else {
            Ok(123)
        }
    }
}
fn setup(h: &Harness, approve: bool) {
    let mut stores = AppState::lock(&h.state.stores);
    AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |p| {
        p.unsolicited = true;
        p.learning_enabled = true;
        p.unsolicited_per_hour = 4;
        p.reply_cooldown_seconds = 0;
    });
    let e = &mut stores.work.engagement;
    *e = crate::engagement::EngagementStore::default();
    e.guild_features.insert(
        7,
        GuildFeaturePolicy {
            revision: 1,
            enabled: [CommunityFeature::Introductions].into(),
            channels: [(CommunityFeature::Introductions, [3].into())].into(),
        },
    );
    for member in [2, 5] {
        e.member_policies.insert(
            member,
            MemberPolicy {
                revision: 1,
                daily_limit: Some(1),
                timezone: Some("UTC".into()),
                quiet_start: 0,
                quiet_end: 0,
                ..Default::default()
            },
        );
    }
    let scope = EngagementScope::Guild {
        guild: 7,
        channel: 3,
    };
    let id = e
        .create_introduction([2, 5], scope.clone(), "I build compilers.".into(), NOW)
        .unwrap();
    e.member_policies
        .get_mut(&2)
        .unwrap()
        .destinations
        .insert(scope, DestinationPreference::Private);
    e.edit_introduction(id, 5, 1, "I build runtimes.".into(), None)
        .unwrap();
    e.approve_introduction(id, 2, 2).unwrap();
    if approve {
        e.approve_introduction(id, 5, 2).unwrap();
    }
}
#[tokio::test]
async fn introductions_retained_delivery_double_charge_no_model_and_deduplicated() {
    let h = Harness::new();
    setup(&h, true);
    let f = IntroductionFake {
        state: h.state.clone(),
        dir: h.dir.clone(),
        auth: AtomicUsize::new(0),
        sends: AtomicUsize::new(0),
        change: Change::None,
    };
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
    assert_eq!(f.sends.load(Ordering::SeqCst), 1);
    let loaded = Stores::load(&h.dir).unwrap();
    assert_eq!(
        loaded.work.engagement.candidates[&2].state,
        CandidateState::Sent
    );
    assert_eq!(
        loaded.work.engagement.introductions[&1].state,
        IntroductionState::Consumed
    );
    h.state
        .commit_work_owned(|s| {
            crate::commands_engage::stop_for_test(
                &mut s.engagement,
                5,
                &EngagementScope::Guild {
                    guild: 7,
                    channel: 3,
                },
            )
        })
        .await
        .unwrap();
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .member_policies[&5]
            .global_stop
    );
    h.finish().await;
}
#[tokio::test]
async fn introductions_final_both_access_withdraw_stop_and_revision_rechecks() {
    for change in [
        Change::Withdraw,
        Change::Stop,
        Change::Access,
        Change::InitialAccess,
        Change::Policy,
        Change::Feature,
    ] {
        let h = Harness::new();
        setup(&h, true);
        let f = IntroductionFake {
            state: h.state.clone(),
            dir: h.dir.clone(),
            auth: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            change,
        };
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        assert_eq!(f.sends.load(Ordering::SeqCst), 0);
        {
            let stores = AppState::lock(&h.state.stores);
            assert_eq!(
                stores.work.engagement.charges.len(),
                if matches!(change, Change::InitialAccess) {
                    0
                } else {
                    2
                }
            );
            assert_eq!(
                stores.work.engagement.introductions[&1].state,
                IntroductionState::Cancelled
            );
            assert_eq!(
                stores.work.engagement.introductions[&1].approvals,
                [None; 2]
            );
            assert!(matches!(
                stores.work.engagement.candidates[&2].state,
                CandidateState::Cancelled | CandidateState::Rejected
            ));
        }
        h.finish().await;
    }
}
#[tokio::test]
async fn introductions_half_approved_never_contacts_and_uncertain_never_rearms() {
    for approved in [false, true] {
        let h = Harness::new();
        setup(&h, approved);
        let f = IntroductionFake {
            state: h.state.clone(),
            dir: h.dir.clone(),
            auth: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            change: Change::Uncertain,
        };
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        h.state
            .clone()
            .deliver_engagement(&f, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        {
            let stores = AppState::lock(&h.state.stores);
            if approved {
                assert_eq!(f.sends.load(Ordering::SeqCst), 1);
                assert_eq!(
                    stores.work.engagement.candidates[&2].state,
                    CandidateState::ReviewRequired
                );
                assert_eq!(stores.work.engagement.charges.len(), 2);
            } else {
                assert_eq!(f.auth.load(Ordering::SeqCst), 0);
                assert!(stores.work.engagement.charges.is_empty());
            }
        }
        h.finish().await;
    }
}
