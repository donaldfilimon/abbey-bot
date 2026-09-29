use super::*;
use crate::{
    persist::{FsPersistenceSink, Stores},
    service::{ServiceSupervisor, persistence::PersistenceWriter},
    work::{DeliveryState, WorkAutomationPolicy, WorkStatus, WorkStore, WorkTask},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
const NOW: u64 = 1_790_683_200; // Injected time; quiet hours disabled below.
fn access() -> WorkAccess {
    WorkAccess {
        actor: 1,
        guild: None,
        channel: 2,
        can_view: true,
        can_manage: false,
    }
}
fn fixture() -> WorkStore {
    let mut store = WorkStore::default();
    let project = store
        .create_project(access(), "Synthetic", "project")
        .unwrap();
    store
        .add_task(
            access(),
            WorkTask {
                id: 0,
                project_id: project,
                title: "Synthetic task".into(),
                owner: 0,
                assignee: None,
                goal_id: None,
                priority: 2,
                status: WorkStatus::Open,
                due_at: None,
                remind_at: Some(NOW - 1),
                reminder_revision: 0,
                snoozed_until: None,
                source: None,
                github: None,
                revision: 0,
            },
            "task",
        )
        .unwrap();
    store
        .configure_automation(
            &WorkScope::Personal { owner: 1 },
            access(),
            WorkAutomationPolicy {
                enabled: true,
                destination: Some(2),
                timezone: "UTC".into(),
                quiet_start: 0,
                quiet_end: 0,
                ..Default::default()
            },
        )
        .unwrap();
    store
}
struct Harness {
    state: Arc<AppState>,
    writer: PersistenceWriter,
    supervisor: ServiceSupervisor,
    dir: PathBuf,
}
impl Harness {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-work-delivery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let state =
            AppState::in_memory_with_persistence(Some(dir.clone()), Arc::new(FsPersistenceSink));
        AppState::lock(&state.stores).work = fixture();
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = state.attach_service(supervisor.operations());
        Self {
            state,
            writer,
            supervisor,
            dir,
        }
    }
    async fn finish(mut self) {
        self.writer.stop();
        self.writer.joined().await.unwrap();
        std::fs::remove_dir_all(self.dir).unwrap();
    }
}
struct Fake {
    state: Arc<AppState>,
    path: PathBuf,
    auth: AtomicUsize,
    sends: AtomicUsize,
    deny_second: bool,
    mutate_second: bool,
    uncertain: bool,
    stall_send: bool,
    cancel: CancellationToken,
}
impl Fake {
    fn new(h: &Harness) -> Self {
        Self {
            state: h.state.clone(),
            path: h.dir.clone(),
            auth: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            deny_second: false,
            mutate_second: false,
            uncertain: false,
            stall_send: false,
            cancel: CancellationToken::new(),
        }
    }
}
impl WorkDeliveryTransport for Fake {
    async fn authorize(
        &self,
        _: &WorkScope,
        _: u64,
        _: u64,
        _: &WorkDestination,
        _: &BTreeSet<u64>,
    ) -> Result<(WorkAccess, u64), WorkError> {
        let call = self.auth.fetch_add(1, Ordering::SeqCst);
        if call == 1 && self.deny_second {
            return Err(WorkError::Denied);
        }
        if call == 1 && self.mutate_second {
            self.state
                .commit_work_owned(|store| {
                    store
                        .scope_automation
                        .get_mut("personal:1")
                        .unwrap()
                        .enabled = false;
                    Ok(())
                })
                .await?;
        }
        Ok((access(), 2))
    }
    async fn send(&self, _: u64, body: &str) -> Result<u64, WorkError> {
        // Read the actual atomic file, without startup repair, BEFORE recording send.
        let persisted: Stores =
            serde_json::from_slice(&std::fs::read(Stores::state_path(&self.path)).unwrap())
                .unwrap();
        assert_eq!(persisted.work.deliveries.len(), 1);
        assert_eq!(
            persisted.work.deliveries.values().next().unwrap().state,
            DeliveryState::Attempting
        );
        assert!(body.contains("Synthetic task"));
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.stall_send {
            self.cancel.cancel();
            std::future::pending::<()>().await;
        }
        if self.uncertain {
            Err(WorkError::Denied)
        } else {
            Ok(123)
        }
    }
}
#[tokio::test]
async fn concurrent_ticks_reserve_before_send_and_record_success_without_duplicates() {
    let h = Harness::new();
    let fake = Fake::new(&h);
    let (a, b) = tokio::join!(
        h.state
            .clone()
            .deliver_work(&fake, fake.cancel.clone(), || NOW),
        h.state
            .clone()
            .deliver_work(&fake, fake.cancel.clone(), || NOW)
    );
    a.unwrap();
    b.unwrap();
    h.state
        .clone()
        .deliver_work(&fake, fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
    let loaded = Stores::load(&h.dir).unwrap();
    let receipt = loaded.work.deliveries.values().next().unwrap();
    assert_eq!(
        (receipt.state, receipt.message_id),
        (DeliveryState::Sent, Some(123))
    );
    h.finish().await;
}
#[tokio::test]
async fn late_permissions_and_policy_changes_block_send_and_consume_attempt() {
    for policy in [false, true] {
        let h = Harness::new();
        let mut fake = Fake::new(&h);
        fake.deny_second = !policy;
        fake.mutate_second = policy;
        h.state
            .clone()
            .deliver_work(&fake, fake.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
        assert_eq!(
            Stores::load(&h.dir)
                .unwrap()
                .work
                .deliveries
                .values()
                .next()
                .unwrap()
                .state,
            DeliveryState::ReviewRequired
        );
        h.finish().await;
    }
}
#[tokio::test]
async fn disconnected_or_cancelled_send_settles_for_review_without_retry() {
    for cancelled in [false, true] {
        let h = Harness::new();
        let mut fake = Fake::new(&h);
        fake.uncertain = !cancelled;
        fake.stall_send = cancelled;
        h.state
            .clone()
            .deliver_work(&fake, fake.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
        let loaded = Stores::load(&h.dir).unwrap();
        assert_eq!(
            loaded.work.deliveries.values().next().unwrap().state,
            DeliveryState::ReviewRequired
        );
        AppState::lock(&h.state.stores).work = loaded.work;
        h.state
            .clone()
            .deliver_work(&fake, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
        h.finish().await;
    }
}
#[tokio::test]
async fn pre_cancelled_tick_does_not_authorize_reserve_or_send() {
    let h = Harness::new();
    let fake = Fake::new(&h);
    fake.cancel.cancel();
    h.state
        .clone()
        .deliver_work(&fake, fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(fake.auth.load(Ordering::SeqCst), 0);
    assert!(AppState::lock(&h.state.stores).work.deliveries.is_empty());
    // No persistence was requested, so there need not be a directory.
    std::fs::create_dir_all(&h.dir).unwrap();
    h.finish().await;
}
#[tokio::test]
async fn failed_reservation_never_reaches_transport() {
    let h = Harness::new();
    let fake = Fake::new(&h);
    // An ordinary file at the directory path makes atomic publication fail.
    std::fs::write(&h.dir, b"fixture").unwrap();
    assert_eq!(
        h.state
            .clone()
            .deliver_work(&fake, fake.cancel.clone(), || NOW)
            .await,
        Err(WorkError::Persistence)
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
    assert!(AppState::lock(&h.state.stores).work.deliveries.is_empty());
    std::fs::remove_file(&h.dir).unwrap();
    std::fs::create_dir(&h.dir).unwrap();
    h.finish().await;
}
#[tokio::test]
async fn restart_repairs_interrupted_receipt_preserving_dedupe_and_outcomes_are_terminal() {
    let h = Harness::new();
    h.state
        .commit_work_owned(|store| {
            let batch = store
                .next_batch(&WorkScope::Personal { owner: 1 }, access(), NOW)?
                .unwrap();
            store.reserve_batch(access(), &batch, NOW)
        })
        .await
        .unwrap();
    let mut loaded = Stores::load(&h.dir).unwrap();
    let id = *loaded.work.deliveries.keys().next().unwrap();
    assert_eq!(
        loaded.work.deliveries[&id].state,
        DeliveryState::ReviewRequired
    );
    assert_eq!(
        loaded.work.settle_delivery(id, Some(99)),
        Err(WorkError::Stale)
    );
    assert!(
        loaded
            .work
            .next_batch(&WorkScope::Personal { owner: 1 }, access(), NOW)
            .unwrap()
            .is_none()
    );
    h.finish().await;
}
#[tokio::test]
async fn supervisor_abort_retains_delivery_until_cancelled_send_is_settled() {
    let mut h = Harness::new();
    let mut fake = Fake::new(&h);
    fake.stall_send = true;
    let fake = Arc::new(fake);
    let transport = fake.clone();
    let state = h.state.clone();
    h.supervisor
        .operations()
        .spawn_operation(
            crate::service::OperationKind::WorkDelivery,
            move |cancel| async move {
                // Tie the test's cancellation to the retained operation's token.
                let state_task = state.clone();
                state_task
                    .deliver_work(transport.as_ref(), cancel, || NOW)
                    .await
                    .unwrap();
                crate::service::TaskExit::Returned
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while fake.sends.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    h.supervisor.begin_draining(
        crate::service::ShutdownReason::Signal,
        tokio::time::Instant::now(),
    );
    h.supervisor.request_abort();
    let completed = h.supervisor.next_completion().await;
    assert!(!completed.abort_requested);
    assert_eq!(
        Stores::load(&h.dir)
            .unwrap()
            .work
            .deliveries
            .values()
            .next()
            .unwrap()
            .state,
        DeliveryState::ReviewRequired
    );
    h.finish().await;
}

#[tokio::test(start_paused = true)]
async fn child_timeout_is_bounded_and_cancellation_wins_ready_work() {
    let token = CancellationToken::new();
    let started = tokio::time::Instant::now();
    let result = bounded(&token, std::future::pending::<Result<(), WorkError>>()).await;
    assert_eq!(result, Err(WorkError::Denied));
    assert_eq!(started.elapsed(), Duration::from_secs(30));
    token.cancel();
    let polled = AtomicUsize::new(0);
    assert_eq!(
        bounded(&token, async {
            polled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await,
        Err(WorkError::Denied)
    );
    assert_eq!(polled.load(Ordering::SeqCst), 0);
}

#[test]
fn private_resolution_never_falls_back_to_origin_and_personal_requires_same_dm() {
    let store = fixture();
    let mut batch = store
        .next_batch(&WorkScope::Personal { owner: 1 }, access(), NOW)
        .unwrap()
        .unwrap();
    assert!(resolved_destination(&batch, 2));
    assert!(!resolved_destination(&batch, 3));
    batch.target = WorkDestination::TeamPrivate { principal: 1 };
    assert!(!resolved_destination(&batch, 0));
    assert!(!resolved_destination(&batch, 2));
    assert!(resolved_destination(&batch, 3));
}

#[tokio::test]
async fn no_due_work_or_revoked_project_authority_never_resolves_a_dm() {
    for revoked in [false, true] {
        let h = Harness::new();
        {
            let mut stores = AppState::lock(&h.state.stores);
            if revoked {
                stores
                    .work
                    .projects
                    .values_mut()
                    .next()
                    .unwrap()
                    .managers
                    .clear();
            } else {
                stores.work.tasks.clear();
            }
        }
        let fake = Fake::new(&h);
        h.state
            .clone()
            .deliver_work(&fake, fake.cancel.clone(), || NOW)
            .await
            .unwrap();
        assert_eq!(fake.auth.load(Ordering::SeqCst), 0);
        assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
        std::fs::create_dir_all(&h.dir).unwrap();
        h.finish().await;
    }
}
