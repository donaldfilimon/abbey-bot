use super::*;
use crate::{
    engagement::{
        Candidate, EngagementKind, EngagementScope, MemberPolicy, SourceRef,
        lifecycle::EngagementReservation, schedule::CandidateProposal,
    },
    persist::{FsPersistenceSink, Stores},
    runtime::{
        engagement_delivery::{AuthorizedDestination, EngagementTransport, SendFailure},
        work_delivery::WorkDeliveryTransport,
    },
    service::{OperationKind, OwnedTaskKind, ServiceSupervisor, ShutdownReason, TaskName},
    work::{
        DeliveryState, WorkAccess, WorkAutomationPolicy, WorkDestination, WorkError, WorkScope,
        WorkStatus, WorkTask,
    },
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tokio::sync::{Notify, Semaphore, watch};
use tokio_util::sync::CancellationToken;

struct Harness {
    state: Arc<AppState>,
    writer: crate::service::persistence::PersistenceWriter,
    supervisor: ServiceSupervisor,
    dir: PathBuf,
}
impl Harness {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-scheduler-jobs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let state =
            AppState::in_memory_with_persistence(Some(dir.clone()), Arc::new(FsPersistenceSink));
        let access = access();
        {
            let mut stores = AppState::lock(&state.stores);
            let project = stores
                .work
                .create_project(access, "Synthetic", "project")
                .unwrap();
            stores
                .work
                .add_task(
                    access,
                    WorkTask {
                        id: 0,
                        project_id: project,
                        title: "Synthetic reminder".into(),
                        owner: 0,
                        assignee: None,
                        goal_id: None,
                        priority: 2,
                        status: WorkStatus::Open,
                        due_at: None,
                        remind_at: Some(1),
                        reminder_revision: 0,
                        snoozed_until: None,
                        source: None,
                        github: None,
                        revision: 0,
                    },
                    "task",
                )
                .unwrap();
            stores
                .work
                .configure_automation(
                    &WorkScope::Personal { owner: 1 },
                    access,
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
            let engagement = &mut stores.work.engagement;
            engagement.member_policies.insert(
                3,
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
                    member: 3,
                    channel: 4,
                },
                message: 5,
                author: 3,
                revision: 1,
                at: 1,
            };
            engagement
                .eligibility
                .entry(3)
                .or_default()
                .insert(source.clone());
            engagement
                .propose(
                    CandidateProposal {
                        kind: EngagementKind::FollowUp,
                        source: Some(source.clone()),
                        member: Some(3),
                        scope: source.scope,
                        due_at: 1,
                        introduction_id: None,
                    },
                    1,
                )
                .unwrap();
        }
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
fn access() -> WorkAccess {
    WorkAccess {
        actor: 1,
        guild: None,
        channel: 2,
        can_view: true,
        can_manage: false,
    }
}
struct PendingDrop<'a>(&'a AtomicBool);
impl Drop for PendingDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct HeldWork {
    started: watch::Sender<bool>,
    dropped: AtomicBool,
}
impl WorkDeliveryTransport for HeldWork {
    async fn authorize(
        &self,
        _: &WorkScope,
        _: u64,
        _: u64,
        _: &WorkDestination,
        _: &BTreeSet<u64>,
    ) -> Result<(WorkAccess, u64), WorkError> {
        Ok((access(), 2))
    }
    async fn send(&self, _: u64, _: &str) -> Result<u64, WorkError> {
        let _dropped = PendingDrop(&self.dropped);
        self.started.send_replace(true);
        std::future::pending().await
    }
}
struct HeldEngagement {
    started: Notify,
    dropped: AtomicBool,
}
impl EngagementTransport for HeldEngagement {
    async fn authorize(
        &self,
        _: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        let _dropped = PendingDrop(&self.dropped);
        self.started.notify_one();
        std::future::pending().await
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        panic!("held preflight must not reach source validation")
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        panic!("held preflight must not reach hydration")
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        panic!("held preflight must not reach generation")
    }
    async fn send(&self, _: u64, _: &str) -> Result<u64, SendFailure> {
        panic!("held preflight must not reach send")
    }
}
struct Maintenance {
    work_started: watch::Receiver<bool>,
    progressed: Notify,
    cancelled: Notify,
    cleanup: Semaphore,
    retain_cleanup: bool,
}
impl CommunityMaintenance for Maintenance {
    async fn maintain(
        &self,
        _: Arc<AppState>,
        cancel: CancellationToken,
        _: u64,
    ) -> Result<(), &'static str> {
        let mut work_started = self.work_started.clone();
        work_started.wait_for(|started| *started).await.unwrap();
        self.progressed.notify_one();
        if self.retain_cleanup {
            cancel.cancelled().await;
            self.cancelled.notify_one();
            self.cleanup.acquire().await.unwrap().forget();
        }
        Ok(())
    }
}
async fn started(
    h: &mut Harness,
    retain_cleanup: bool,
) -> (Arc<HeldWork>, Arc<HeldEngagement>, Arc<Maintenance>) {
    let (work_started, observed) = watch::channel(false);
    let work = Arc::new(HeldWork {
        started: work_started,
        dropped: AtomicBool::new(false),
    });
    let engagement = Arc::new(HeldEngagement {
        started: Notify::new(),
        dropped: AtomicBool::new(false),
    });
    let maintenance = Arc::new(Maintenance {
        work_started: observed,
        progressed: Notify::new(),
        cancelled: Notify::new(),
        cleanup: Semaphore::new(0),
        retain_cleanup,
    });
    let (state, w, e, m) = (
        h.state.clone(),
        work.clone(),
        engagement.clone(),
        maintenance.clone(),
    );
    h.supervisor
        .spawn_service(TaskName::Scheduler, move |cancel| {
            state.run_scheduler(cancel, w, e, m)
        })
        .unwrap();
    // Construct the scheduler's timers before advancing its first Work tick.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    // Real disk owners must not cause virtual time to jump to delivery timeout.
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), async {
        maintenance.progressed.notified().await;
        engagement.started.notified().await;
    })
    .await
    .unwrap();
    (work, engagement, maintenance)
}
async fn drain(h: &mut Harness) -> Vec<crate::service::TaskCompletion> {
    let mut joined = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !h.supervisor.outstanding().is_empty() {
            joined.push(h.supervisor.next_completion().await);
        }
    })
    .await
    .unwrap();
    joined
}
#[tokio::test(start_paused = true)]
async fn maintenance_progresses_while_work_send_is_pending() {
    let mut h = Harness::new();
    let (work, _, _) = started(&mut h, false).await;
    assert!(!work.dropped.load(Ordering::Acquire));
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .deliveries
            .values()
            .next()
            .unwrap()
            .state,
        DeliveryState::Attempting
    );
    let completion = h.supervisor.next_completion().await;
    assert_eq!(
        completion.kind,
        OwnedTaskKind::Operation(OperationKind::CommunityMaintenance)
    );
    assert!(
        h.supervisor
            .outstanding()
            .iter()
            .any(|task| { task.kind == OwnedTaskKind::Operation(OperationKind::WorkDelivery) })
    );
    h.supervisor
        .begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    h.supervisor.request_cancellation();
    drain(&mut h).await;
    h.finish().await;
}
#[tokio::test(start_paused = true)]
async fn scheduler_jobs_keep_distinct_owners_until_cancelled_cleanup_is_joined() {
    let mut h = Harness::new();
    let (work, engagement, maintenance) = started(&mut h, true).await;
    let before = h.supervisor.outstanding();
    let jobs = [
        OperationKind::WorkDelivery,
        OperationKind::EngagementDelivery,
        OperationKind::CommunityMaintenance,
    ];
    let ids: Vec<_> = jobs
        .iter()
        .map(|kind| {
            let matching: Vec<_> = before
                .iter()
                .filter(|task| task.kind == OwnedTaskKind::Operation(*kind))
                .collect();
            assert_eq!(matching.len(), 1, "{kind:?}");
            matching[0].id
        })
        .collect();
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
    assert_ne!(ids[0], ids[2]);
    h.supervisor
        .begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    h.supervisor.request_abort();
    tokio::time::timeout(Duration::from_secs(5), maintenance.cancelled.notified())
        .await
        .unwrap();
    assert!(
        h.supervisor
            .outstanding()
            .iter()
            .any(|task| { task.id == ids[2] && !task.abort_requested })
    );
    maintenance.cleanup.add_permits(1);
    let joined = drain(&mut h).await;
    for (kind, id) in jobs.into_iter().zip(ids) {
        let completion = joined
            .iter()
            .find(|completion| completion.id == id)
            .unwrap();
        assert_eq!(completion.kind, OwnedTaskKind::Operation(kind));
        assert_eq!(completion.exit, crate::service::TaskExit::Returned);
        assert!(!completion.abort_requested);
        assert!(completion.fatal.is_none());
    }
    assert!(work.dropped.load(Ordering::Acquire));
    assert!(engagement.dropped.load(Ordering::Acquire));
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
