// Apply as src/runtime/engagement_delivery/tests/task_follow_up/delivery_tests.rs.
// In tests/task_follow_up.rs add #[path = "task_follow_up/delivery_tests.rs"] mod delivery_tests;
// Uses that parent's real seed_task and its parent Harness/Fake; no live IO.
use super::*;
use crate::work::{WorkDestination, WorkScope, follow_up::FollowUpDecision};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, AtomicU64},
};
use tokio::sync::Notify;

struct WorkProven {
    fake: Fake,
    work_calls: AtomicUsize,
    exchanges: AtomicUsize,
    source_proofs: AtomicUsize,
    exchange_proofs: AtomicUsize,
    generations: AtomicUsize,
    deny_work: AtomicBool,
    source_live: AtomicBool,
    exchange_live: AtomicBool,
    hold_generation: bool,
    status_delay: Option<Duration>,
    entered: Notify,
    release: Notify,
}
impl WorkProven {
    fn new(h: &Harness, race: Race, hold_generation: bool) -> Self {
        Self {
            fake: Fake::new(h, race),
            work_calls: AtomicUsize::new(0),
            exchanges: AtomicUsize::new(0),
            source_proofs: AtomicUsize::new(0),
            exchange_proofs: AtomicUsize::new(0),
            generations: AtomicUsize::new(0),
            deny_work: AtomicBool::new(false),
            source_live: AtomicBool::new(true),
            exchange_live: AtomicBool::new(true),
            hold_generation,
            status_delay: None,
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.entered.notified())
            .await
            .unwrap();
    }
}
impl EngagementTransport for WorkProven {
    async fn authorize_work(
        &self,
        scope: &WorkScope,
        actor: u64,
        origin: u64,
        target: &WorkDestination,
        audience: &BTreeSet<u64>,
    ) -> Result<(WorkAccess, u64), WorkError> {
        assert!(self.fake.state.stores.try_lock().is_ok());
        self.work_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.status_delay {
            tokio::time::sleep(delay).await;
        }
        assert_eq!(scope, &WorkScope::Personal { owner: 2 });
        assert_eq!((actor, origin), (2, 3));
        assert_eq!(target, &WorkDestination::Personal { principal: 2 });
        assert_eq!(audience, &BTreeSet::from([2]));
        if self.deny_work.load(Ordering::SeqCst) {
            Err(WorkError::Denied)
        } else {
            Ok((
                WorkAccess {
                    actor,
                    guild: None,
                    channel: origin,
                    can_view: true,
                    can_manage: false,
                },
                3,
            ))
        }
    }
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        self.fake.authorize(r).await
    }
    async fn source_exists(&self, source: &SourceRef) -> Result<bool, WorkError> {
        self.source_proofs.fetch_add(1, Ordering::SeqCst);
        assert_eq!((source.message, source.author), (4, 2));
        Ok(self.source_live.load(Ordering::SeqCst) && self.fake.source_exists(source).await?)
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        panic!("linked task delivery needs its real source-response exchange")
    }
    async fn hydrate_exchange(&self, c: &Candidate, response: u64) -> Result<String, WorkError> {
        assert!(self.fake.state.stores.try_lock().is_ok());
        assert_eq!(c.source.as_ref().unwrap().message, 4);
        assert_eq!(response, 5);
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        self.fake.hydrate(c).await
    }
    async fn candidate_current(
        &self,
        c: &Candidate,
        response: Option<u64>,
    ) -> Result<bool, WorkError> {
        self.exchange_proofs.fetch_add(1, Ordering::SeqCst);
        assert_eq!(c.source.as_ref().unwrap().message, 4);
        assert_eq!(response, Some(5));
        Ok(self.exchange_live.load(Ordering::SeqCst))
    }
    async fn generate(
        &self,
        s: &AppState,
        c: &Candidate,
        text: &str,
        now: u64,
    ) -> Result<String, WorkError> {
        assert!(s.stores.try_lock().is_ok());
        self.generations.fetch_add(1, Ordering::SeqCst);
        assert!(
            text.contains("Current Task"),
            "the native task must be included as quoted data"
        );
        assert!(text.contains("Qualify the allocator"));
        assert!(text.contains("synthetic unresolved project question"));
        if self.hold_generation {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.fake.generate(s, c, text, now).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        assert_eq!(channel, 3);
        // Fake::send reads the actual JSON and verifies Reserved + exactly one
        // canonical charge before reporting a remote receipt.
        self.fake.send(channel, body).await
    }
}

fn charges(h: &Harness) -> usize {
    AppState::lock(&h.state.stores)
        .work
        .engagement
        .charges
        .len()
}
fn linked(h: &Harness) -> Candidate {
    AppState::lock(&h.state.stores).work.engagement.candidates[&1].clone()
}

#[tokio::test]
async fn task_follow_up_delivery_proves_work_and_exchange_then_sends_one_canonical_attempt() {
    let h = Harness::new();
    let task = seed_task(&h);
    let before = AppState::lock(&h.state.stores).work.tasks[&task].clone();
    let native_ref = linked(&h).work_ref;
    let transport = WorkProven::new(&h, Race::None, false);
    h.state
        .clone()
        .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 1);
    assert!(transport.work_calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(transport.exchanges.load(Ordering::SeqCst), 1);
    assert!(transport.source_proofs.load(Ordering::SeqCst) >= 1);
    assert!(transport.exchange_proofs.load(Ordering::SeqCst) >= 1);
    assert_eq!(transport.generations.load(Ordering::SeqCst), 1);
    assert_eq!(charges(&h), 1);
    assert_eq!(h.status(), CandidateState::Sent);
    let disk = Stores::load(&h.dir).unwrap();
    assert_eq!(
        disk.work.engagement.candidates[&1].state,
        CandidateState::Sent
    );
    assert_eq!(disk.work.engagement.candidates[&1].message_id, Some(123));
    assert_eq!(disk.work.engagement.candidates[&1].work_ref, native_ref);
    assert_eq!(disk.work.engagement.candidates[&1].follow_up_reason, None);
    assert_eq!(disk.work.tasks[&task], before);
    assert_eq!(disk.work.engagement.charges.len(), 1);
    h.finish().await;
}

#[derive(Clone, Copy, Debug)]
enum DeliveryRace {
    TaskRevision,
    TaskCompletion,
    Expiry,
    Stop,
    Destination,
    Source,
    Response,
    NativeWork,
    NativeSource,
    NativeExchange,
    Erasure,
}
#[tokio::test]
async fn task_follow_up_delivery_rechecks_task_policy_expiry_and_native_proofs_after_generation() {
    for race in [
        DeliveryRace::TaskRevision,
        DeliveryRace::TaskCompletion,
        DeliveryRace::Expiry,
        DeliveryRace::Stop,
        DeliveryRace::Destination,
        DeliveryRace::Source,
        DeliveryRace::Response,
        DeliveryRace::NativeWork,
        DeliveryRace::NativeSource,
        DeliveryRace::NativeExchange,
        DeliveryRace::Erasure,
    ] {
        let h = Harness::new();
        let task = seed_task(&h);
        // The public erasure owner uses wall time when pruning retained safety
        // charges. Keep this race in the same clock domain so the reservation
        // remains inside its retention window regardless of the calendar date.
        let admitted_at = if matches!(race, DeliveryRace::Erasure) {
            let at = crate::runtime::now();
            AppState::lock(&h.state.stores)
                .work
                .engagement
                .candidates
                .get_mut(&1)
                .unwrap()
                .expires_at = Some(at + 3600);
            at
        } else {
            NOW
        };
        let transport = Arc::new(WorkProven::new(&h, Race::None, true));
        let time = Arc::new(AtomicU64::new(admitted_at));
        let state = h.state.clone();
        let t = transport.clone();
        let clock = time.clone();
        let attempt = tokio::spawn(async move {
            state
                .deliver_engagement(t.as_ref(), t.fake.cancel.clone(), || {
                    clock.load(Ordering::SeqCst)
                })
                .await
        });
        transport.entered().await;
        assert_eq!(charges(&h), 1);
        assert_eq!(h.status(), CandidateState::Reserved);
        match race {
            DeliveryRace::Expiry => time.store(NOW + 3600, Ordering::SeqCst),
            DeliveryRace::NativeWork => transport.deny_work.store(true, Ordering::SeqCst),
            DeliveryRace::NativeSource => transport.source_live.store(false, Ordering::SeqCst),
            DeliveryRace::NativeExchange => transport.exchange_live.store(false, Ordering::SeqCst),
            DeliveryRace::Erasure => {
                h.state
                    .erase_personal_learning("discord:dm:2".into(), 2)
                    .await
                    .unwrap();
            }
            _ => {
                h.state
                    .commit_work_owned(move |w| {
                        match race {
                            DeliveryRace::TaskRevision => {
                                w.tasks.get_mut(&task).unwrap().revision += 1
                            }
                            DeliveryRace::TaskCompletion => {
                                w.tasks.get_mut(&task).unwrap().status = WorkStatus::Done
                            }
                            DeliveryRace::Stop => {
                                w.engagement
                                    .member_policies
                                    .get_mut(&2)
                                    .unwrap()
                                    .global_stop = true
                            }
                            DeliveryRace::Destination => {
                                w.engagement
                                    .member_policies
                                    .get_mut(&2)
                                    .unwrap()
                                    .destinations
                                    .remove(&EngagementScope::Dm {
                                        member: 2,
                                        channel: 3,
                                    });
                            }
                            DeliveryRace::Source => {
                                let c = w.engagement.candidates[&1].clone();
                                let source = c.source.unwrap();
                                w.engagement
                                    .eligibility
                                    .get_mut(&2)
                                    .unwrap()
                                    .remove(&source);
                                w.engagement
                                    .observations
                                    .get_mut(&c.scope)
                                    .unwrap()
                                    .remove(&2);
                            }
                            DeliveryRace::Response => {
                                w.engagement.responses.insert(4, 6);
                            }
                            _ => unreachable!(),
                        }
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
        }
        transport.release.notify_one();
        attempt.await.unwrap().unwrap();
        assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 0, "{race:?}");
        let disk = Stores::load(&h.dir).unwrap();
        if matches!(race, DeliveryRace::Erasure) {
            assert!(!disk.work.engagement.candidates.contains_key(&1));
            assert_eq!(
                disk.work.engagement.charges.len()
                    + disk.work.engagement.erased_contact_charges.len(),
                1
            );
        } else {
            assert!(
                matches!(
                    h.status(),
                    CandidateState::Cancelled | CandidateState::Rejected
                ),
                "{race:?}"
            );
            assert_eq!(
                charges(&h),
                1,
                "an attempted slot cannot be refunded: {race:?}"
            );
            assert_eq!(disk.work.engagement.charges.len(), 1);
            assert_eq!(disk.work.engagement.candidates[&1].message_id, None);
            if matches!(
                race,
                DeliveryRace::TaskRevision
                    | DeliveryRace::TaskCompletion
                    | DeliveryRace::Expiry
                    | DeliveryRace::Stop
                    | DeliveryRace::Destination
            ) {
                let expected = match race {
                    DeliveryRace::TaskRevision | DeliveryRace::TaskCompletion => {
                        FollowUpDecision::StaleTask
                    }
                    DeliveryRace::Expiry => FollowUpDecision::Expired,
                    DeliveryRace::Stop => FollowUpDecision::OptedOut,
                    DeliveryRace::Destination => FollowUpDecision::OptedOut,
                    _ => unreachable!(),
                };
                assert_eq!(
                    disk.work.engagement.candidates[&1].follow_up_reason,
                    Some(expected),
                    "{race:?}"
                );
            }
        }
        h.finish().await;
    }
}

#[tokio::test]
async fn task_follow_up_delivery_stale_expired_stopped_or_unproven_pending_candidates_do_not_charge()
 {
    for failure in 0..4 {
        let h = Harness::new();
        let task = seed_task(&h);
        let transport = WorkProven::new(&h, Race::None, false);
        let at = if failure == 1 { NOW + 3600 } else { NOW };
        if failure == 3 {
            transport.deny_work.store(true, Ordering::SeqCst);
        } else {
            let mut stores = AppState::lock(&h.state.stores);
            match failure {
                0 => stores.work.tasks.get_mut(&task).unwrap().revision += 1,
                2 => {
                    stores
                        .work
                        .engagement
                        .member_policies
                        .get_mut(&2)
                        .unwrap()
                        .global_stop = true
                }
                _ => {}
            }
        }
        h.state
            .clone()
            .deliver_engagement(&transport, transport.fake.cancel.clone(), || at)
            .await
            .unwrap();
        assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 0);
        assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
        assert_eq!(transport.exchanges.load(Ordering::SeqCst), 0);
        assert_eq!(charges(&h), 0);
        assert_ne!(h.status(), CandidateState::Reserved);
        assert_eq!(
            Stores::load(&h.dir).unwrap().work.engagement.charges.len(),
            0
        );
        h.finish().await;
    }
}

#[tokio::test]
async fn task_follow_up_delivery_concurrent_ticks_preserve_one_charge_and_send() {
    let h = Harness::new();
    seed_task(&h);
    let transport = WorkProven::new(&h, Race::None, false);
    let (a, b) = tokio::join!(
        h.state
            .clone()
            .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW),
        h.state
            .clone()
            .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
    );
    a.unwrap();
    b.unwrap();
    h.state
        .clone()
        .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 1);
    assert_eq!(transport.generations.load(Ordering::SeqCst), 1);
    assert_eq!(charges(&h), 1);
    assert_eq!(
        Stores::load(&h.dir).unwrap().work.engagement.charges.len(),
        1
    );
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_delivery_uncertain_receipt_is_not_replayed_after_canonical_reopen() {
    let h = Harness::new();
    seed_task(&h);
    let transport = WorkProven::new(&h, Race::Uncertain, false);
    h.state
        .clone()
        .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(h.status(), CandidateState::ReviewRequired);
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 1);
    let disk = Stores::load(&h.dir).unwrap();
    assert_eq!(
        disk.work.engagement.candidates[&1].state,
        CandidateState::ReviewRequired
    );
    assert_eq!(disk.work.engagement.candidates[&1].follow_up_reason, None);
    *AppState::lock(&h.state.stores) = disk;
    h.state
        .clone()
        .deliver_engagement(&transport, CancellationToken::new(), || NOW + 1)
        .await
        .unwrap();
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 1);
    assert_eq!(charges(&h), 1);
    assert_eq!(h.status(), CandidateState::ReviewRequired);
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_status_is_owned_exact_origin_and_rechecks_work_before_metadata() {
    let h = Harness::new();
    seed_task(&h);
    let transport = WorkProven::new(&h, Race::None, false);
    let origin = EngagementScope::Dm {
        member: 2,
        channel: 3,
    };
    for (member, scope) in [
        (9, origin.clone()),
        (
            2,
            EngagementScope::Dm {
                member: 2,
                channel: 8,
            },
        ),
        (
            2,
            EngagementScope::Guild {
                guild: 7,
                channel: 3,
            },
        ),
    ] {
        assert!(
            h.state
                .task_follow_up_status(member, &scope, &transport, || NOW)
                .await
                .is_empty()
        );
    }
    assert_eq!(transport.work_calls.load(Ordering::SeqCst), 0);
    let shown = h
        .state
        .task_follow_up_status(2, &origin, &transport, || NOW)
        .await;
    assert!(shown.contains("Your task follow-ups in this origin:"));
    assert!(shown.contains("Candidate 1:"));
    assert!(!shown.contains("Qualify the allocator"));
    assert!(!shown.contains("Checked personal project"));
    assert_eq!(transport.work_calls.load(Ordering::SeqCst), 1);
    transport.deny_work.store(true, Ordering::SeqCst);
    let hidden = h
        .state
        .task_follow_up_status(2, &origin, &transport, || NOW)
        .await;
    assert!(hidden.contains("current access could not be confirmed"));
    assert!(!hidden.contains("Candidate 1:"));
    assert!(!hidden.contains("Qualify the allocator"));
    assert!(!hidden.contains("expires <t:"));
    assert_eq!(transport.exchanges.load(Ordering::SeqCst), 0);
    assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
    assert_eq!(charges(&h), 0);
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_status_expiry_uses_time_after_native_permission_proof() {
    let h = Harness::new();
    seed_task(&h);
    let start = crate::runtime::now();
    AppState::lock(&h.state.stores)
        .work
        .engagement
        .candidates
        .get_mut(&1)
        .unwrap()
        .expires_at = Some(start + 1);
    let mut transport = WorkProven::new(&h, Race::None, false);
    transport.status_delay = Some(Duration::from_millis(2100));
    let shown = h
        .state
        .task_follow_up_status(
            2,
            &EngagementScope::Dm {
                member: 2,
                channel: 3,
            },
            &transport,
            crate::runtime::now,
        )
        .await;
    assert!(
        shown.contains(FollowUpDecision::Expired.message()),
        "{shown}"
    );
    assert!(!shown.contains(FollowUpDecision::Allowed.message()));
    assert_eq!(charges(&h), 0);
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_status_global_quiet_never_claims_current_eligibility() {
    let h = Harness::new();
    seed_task(&h);
    let mut state = AppState::in_memory();
    Arc::get_mut(&mut state).unwrap().quiet = true;
    *AppState::lock(&state.stores) = AppState::lock(&h.state.stores).clone();
    let transport = WorkProven::new(&h, Race::None, false);
    let shown = state
        .task_follow_up_status(
            2,
            &EngagementScope::Dm {
                member: 2,
                channel: 3,
            },
            &transport,
            || NOW,
        )
        .await;
    assert!(shown.contains(FollowUpDecision::Quiet.message()), "{shown}");
    assert!(!shown.contains(FollowUpDecision::Allowed.message()));
    assert_eq!(charges(&h), 0);
    h.finish().await;
}

#[path = "delivery_tests/receipt_status.rs"]
mod receipt_status;
