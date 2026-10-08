// Apply as src/runtime/engagement_follow_up/tests.rs; add #[cfg(test)] mod tests
// to engagement_follow_up.rs. This draft does not perform network/provider IO.
use super::*;
use crate::{
    engagement::{CandidateState, DestinationPreference, MemberPolicy, SourceRef},
    persist::{FsPersistenceSink, Stores},
    runtime::engagement_delivery::{AuthorizedDestination, SendFailure},
    service::{
        OperationKind, OwnedTaskKind, ReapOutcome, ServiceSupervisor, ShutdownReason,
        persistence::PersistenceWriter,
    },
    work::{WorkAccess, WorkDestination, WorkScope, WorkStatus, WorkTask},
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

const NOW: u64 = 1_790_683_200;
fn origin() -> EngagementScope {
    EngagementScope::Dm {
        member: 2,
        channel: 3,
    }
}
fn source() -> SourceRef {
    SourceRef {
        scope: origin(),
        message: 4,
        author: 2,
        revision: 1,
        at: NOW - 86_400,
    }
}
fn access() -> WorkAccess {
    WorkAccess {
        actor: 2,
        guild: None,
        channel: 3,
        can_view: true,
        can_manage: false,
    }
}

struct RequestHarness {
    state: Arc<AppState>,
    writer: PersistenceWriter,
    supervisor: ServiceSupervisor,
    dir: PathBuf,
    task: u64,
    project: u64,
}
impl RequestHarness {
    fn new(destination: DestinationPreference) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-task-follow-up-request-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state =
            AppState::in_memory_with_persistence(Some(dir.clone()), Arc::new(FsPersistenceSink));
        let (project, task) = {
            let mut stores = AppState::lock(&state.stores);
            let work = &mut stores.work;
            let project = work
                .create_project(access(), "Synthetic personal project", "request-project")
                .unwrap();
            let task = work
                .add_task(
                    access(),
                    WorkTask {
                        id: 0,
                        project_id: project,
                        title: "Retain the native task".into(),
                        owner: 2,
                        assignee: None,
                        goal_id: None,
                        priority: 0,
                        status: WorkStatus::Open,
                        due_at: None,
                        remind_at: None,
                        reminder_revision: 0,
                        snoozed_until: None,
                        source: None,
                        github: None,
                        revision: 0,
                    },
                    "request-task",
                )
                .unwrap();
            let e = &mut work.engagement;
            let mut policy = MemberPolicy {
                revision: 1,
                daily_limit: Some(1),
                timezone: Some("UTC".into()),
                quiet_start: 0,
                quiet_end: 0,
                ..Default::default()
            };
            policy.destinations.insert(origin(), destination);
            e.member_policies.insert(2, policy);
            e.eligibility.entry(2).or_default().insert(source());
            e.observations
                .entry(origin())
                .or_default()
                .insert(2, source());
            e.responses.insert(4, 5);
            e.validate().unwrap();
            (project, task)
        };
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = state.attach_service(supervisor.operations());
        Self {
            state,
            writer,
            supervisor,
            dir,
            task,
            project,
        }
    }
    fn request(&self) -> TaskFollowUpRequest {
        TaskFollowUpRequest {
            member: 2,
            origin: origin(),
            task: self.task,
            revision: 0,
            source_message: 4,
            expiry_seconds: 3600,
        }
    }
    fn native_task(&self) -> WorkTask {
        AppState::lock(&self.state.stores).work.tasks[&self.task].clone()
    }
    async fn persist_seed(&self) {
        self.state.commit_work_owned(|_| Ok(())).await.unwrap();
    }
    fn assert_no_request_effect(&self, transport: &MockEngagementTransport) {
        let stores = AppState::lock(&self.state.stores);
        assert!(stores.work.engagement.candidates.is_empty());
        assert!(stores.work.engagement.charges.is_empty());
        assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
        assert_eq!(transport.sends.load(Ordering::SeqCst), 0);
    }
    async fn finish(mut self) {
        let shutdown = self
            .supervisor
            .begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
        let report = self
            .supervisor
            .cancel_and_reap(shutdown.budget.stage(tokio::time::Instant::now()))
            .await;
        assert_eq!(report.outcome, ReapOutcome::Joined);
        self.writer.stop();
        self.writer.joined().await.unwrap();
        std::fs::remove_dir_all(self.dir).unwrap();
    }
}

struct MockEngagementTransport {
    state: Arc<AppState>,
    work_calls: AtomicUsize,
    hydrations: AtomicUsize,
    source_calls: AtomicUsize,
    current_calls: AtomicUsize,
    generations: AtomicUsize,
    sends: AtomicUsize,
    deny_work: AtomicBool,
    source_live: AtomicBool,
    exchange_live: AtomicBool,
    hold_exchange: bool,
    entered: Notify,
    release: Notify,
}
impl MockEngagementTransport {
    fn new(h: &RequestHarness, hold_exchange: bool) -> Arc<Self> {
        Arc::new(Self {
            state: h.state.clone(),
            work_calls: AtomicUsize::new(0),
            hydrations: AtomicUsize::new(0),
            source_calls: AtomicUsize::new(0),
            current_calls: AtomicUsize::new(0),
            generations: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            deny_work: AtomicBool::new(false),
            source_live: AtomicBool::new(true),
            exchange_live: AtomicBool::new(true),
            hold_exchange,
            entered: Notify::new(),
            release: Notify::new(),
        })
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.entered.notified())
            .await
            .unwrap();
    }
}
impl EngagementTransport for MockEngagementTransport {
    async fn authorize_work(
        &self,
        scope: &WorkScope,
        actor: u64,
        origin_channel: u64,
        target: &WorkDestination,
        audience: &BTreeSet<u64>,
    ) -> Result<(WorkAccess, u64), WorkError> {
        assert!(self.state.stores.try_lock().is_ok());
        self.work_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(scope, &WorkScope::Personal { owner: 2 });
        assert_eq!((actor, origin_channel), (2, 3));
        assert_eq!(target, &WorkDestination::Personal { principal: 2 });
        assert_eq!(audience, &BTreeSet::from([2]));
        if self.deny_work.load(Ordering::SeqCst) {
            Err(WorkError::Denied)
        } else {
            Ok((access(), 3))
        }
    }
    async fn authorize(
        &self,
        _: &crate::engagement::lifecycle::EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        panic!("request must not reserve or deliver")
    }
    async fn source_exists(&self, value: &SourceRef) -> Result<bool, WorkError> {
        self.source_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(value, &source());
        Ok(self.source_live.load(Ordering::SeqCst))
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        panic!("request must verify the real exchange")
    }
    async fn hydrate_exchange(
        &self,
        candidate: &Candidate,
        response: u64,
    ) -> Result<String, WorkError> {
        assert!(self.state.stores.try_lock().is_ok());
        self.hydrations.fetch_add(1, Ordering::SeqCst);
        assert_eq!(candidate.source, Some(source()));
        assert_eq!(response, 5);
        if self.hold_exchange {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok("Synthetic human task question and an actual synthetic Abbey response".into())
    }
    async fn candidate_current(
        &self,
        candidate: &Candidate,
        response: Option<u64>,
    ) -> Result<bool, WorkError> {
        self.current_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(candidate.source, Some(source()));
        assert_eq!(response, Some(5));
        Ok(self.exchange_live.load(Ordering::SeqCst))
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        self.generations.fetch_add(1, Ordering::SeqCst);
        panic!("request is metadata only")
    }
    async fn send(&self, _: u64, _: &str) -> Result<u64, SendFailure> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        panic!("request must not contact the member")
    }
}

#[tokio::test]
async fn task_follow_up_request_is_retained_metadata_only_and_reopens_exact_native_task() {
    let h = RequestHarness::new(DestinationPreference::Origin);
    h.persist_seed().await;
    let before = h.native_task();
    let transport = MockEngagementTransport::new(&h, false);
    assert_eq!(
        h.state
            .request_task_follow_up(h.request(), transport.clone(), || NOW)
            .await
            .unwrap(),
        TaskFollowUpResult::Saved {
            candidate: 1,
            due_at: NOW,
            expires_at: NOW + 3600
        }
    );
    let reopened = Stores::load(&h.dir).unwrap();
    assert!(
        !std::fs::read_to_string(Stores::state_path(&h.dir))
            .unwrap()
            .contains("Synthetic human task question")
    );
    let c = &reopened.work.engagement.candidates[&1];
    assert_eq!(
        c.work_ref,
        Some(WorkContentRef::Task {
            project: h.project,
            id: h.task,
            revision: 0
        })
    );
    assert_eq!(c.source, Some(source()));
    assert_eq!(c.state, CandidateState::Pending);
    assert_eq!(c.destination, DestinationPreference::Origin);
    assert_eq!(reopened.work.tasks[&h.task], before);
    assert_eq!(h.native_task(), before);
    assert!(reopened.work.engagement.charges.is_empty());
    assert_eq!(transport.work_calls.load(Ordering::SeqCst), 2);
    assert_eq!(transport.hydrations.load(Ordering::SeqCst), 1);
    assert_eq!(transport.source_calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.current_calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
    assert_eq!(transport.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        h.state
            .request_task_follow_up(h.request(), transport.clone(), || NOW)
            .await
            .unwrap(),
        TaskFollowUpResult::Refused(FollowUpDecision::AlreadyAttempted)
    );
    assert_eq!(transport.hydrations.load(Ordering::SeqCst), 1);
    h.finish().await;
}

#[derive(Clone, Copy, Debug)]
enum RequestRace {
    TaskRevision,
    TaskCompletion,
    Source,
    Response,
    Stop,
    Destination,
    Audience,
    Erasure,
}
#[tokio::test]
async fn task_follow_up_request_rechecks_canonical_authority_after_exchange_await() {
    for race in [
        RequestRace::TaskRevision,
        RequestRace::TaskCompletion,
        RequestRace::Source,
        RequestRace::Response,
        RequestRace::Stop,
        RequestRace::Destination,
        RequestRace::Audience,
        RequestRace::Erasure,
    ] {
        let h = RequestHarness::new(DestinationPreference::Origin);
        h.persist_seed().await;
        let transport = MockEngagementTransport::new(&h, true);
        let state = h.state.clone();
        let request = h.request();
        let t = transport.clone();
        let waiter =
            tokio::spawn(async move { state.request_task_follow_up(request, t, || NOW).await });
        transport.entered().await;
        if matches!(race, RequestRace::Erasure) {
            h.state
                .erase_personal_learning("discord:dm:2".into(), 2)
                .await
                .unwrap();
        } else {
            let task = h.task;
            let project = h.project;
            h.state
                .commit_work_owned(move |w| {
                    match race {
                        RequestRace::TaskRevision => w.tasks.get_mut(&task).unwrap().revision += 1,
                        RequestRace::TaskCompletion => {
                            w.tasks.get_mut(&task).unwrap().status = WorkStatus::Done
                        }
                        RequestRace::Source => {
                            w.engagement
                                .eligibility
                                .get_mut(&2)
                                .unwrap()
                                .remove(&source());
                            w.engagement
                                .observations
                                .get_mut(&origin())
                                .unwrap()
                                .remove(&2);
                        }
                        RequestRace::Response => {
                            w.engagement.responses.insert(4, 6);
                        }
                        RequestRace::Stop => {
                            w.engagement
                                .member_policies
                                .get_mut(&2)
                                .unwrap()
                                .global_stop = true
                        }
                        RequestRace::Destination => {
                            w.engagement
                                .member_policies
                                .get_mut(&2)
                                .unwrap()
                                .destinations
                                .remove(&origin());
                        }
                        RequestRace::Audience => {
                            w.projects.get_mut(&project).unwrap().members.insert(9);
                        }
                        RequestRace::Erasure => unreachable!(),
                    }
                    Ok(())
                })
                .await
                .unwrap();
        }
        transport.release.notify_one();
        let result = waiter.await.unwrap().unwrap();
        let expected = match race {
            RequestRace::Stop => FollowUpDecision::OptedOut,
            RequestRace::Destination | RequestRace::Erasure => FollowUpDecision::OptedOut,
            RequestRace::Audience => FollowUpDecision::AccessDenied,
            _ => FollowUpDecision::StaleTask,
        };
        assert_eq!(result, TaskFollowUpResult::Refused(expected), "{race:?}");
        h.assert_no_request_effect(&transport);
        let disk = Stores::load(&h.dir).unwrap();
        assert!(disk.work.engagement.candidates.is_empty(), "{race:?}");
        assert!(disk.work.engagement.charges.is_empty());
        h.finish().await;
    }
}

#[tokio::test]
async fn task_follow_up_request_refreshes_native_work_source_and_exchange_proof() {
    for failure in 0..3 {
        let h = RequestHarness::new(DestinationPreference::Origin);
        h.persist_seed().await;
        let transport = MockEngagementTransport::new(&h, true);
        let state = h.state.clone();
        let request = h.request();
        let t = transport.clone();
        let waiter =
            tokio::spawn(async move { state.request_task_follow_up(request, t, || NOW).await });
        transport.entered().await;
        match failure {
            0 => transport.deny_work.store(true, Ordering::SeqCst),
            1 => transport.source_live.store(false, Ordering::SeqCst),
            _ => transport.exchange_live.store(false, Ordering::SeqCst),
        }
        transport.release.notify_one();
        let expected = if failure == 0 {
            FollowUpDecision::AccessDenied
        } else {
            FollowUpDecision::StaleTask
        };
        assert_eq!(
            waiter.await.unwrap().unwrap(),
            TaskFollowUpResult::Refused(expected)
        );
        h.assert_no_request_effect(&transport);
        h.finish().await;
    }
}

#[tokio::test]
async fn task_follow_up_request_drop_keeps_service_owned_publication() {
    let mut h = RequestHarness::new(DestinationPreference::Origin);
    h.persist_seed().await;
    let before = h.native_task();
    let transport = MockEngagementTransport::new(&h, true);
    let state = h.state.clone();
    let request = h.request();
    let t = transport.clone();
    let waiter =
        tokio::spawn(async move { state.request_task_follow_up(request, t, || NOW).await });
    transport.entered().await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    transport.release.notify_one();
    let completion = tokio::time::timeout(Duration::from_secs(2), h.supervisor.next_completion())
        .await
        .unwrap();
    assert_eq!(
        completion.kind,
        OwnedTaskKind::Operation(OperationKind::EngagementDelivery)
    );
    let reopened = Stores::load(&h.dir).unwrap();
    assert_eq!(reopened.work.engagement.candidates.len(), 1);
    assert!(reopened.work.engagement.charges.is_empty());
    assert_eq!(reopened.work.tasks[&h.task], before);
    assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
    assert_eq!(transport.sends.load(Ordering::SeqCst), 0);
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_request_service_cancellation_is_observed_without_publication() {
    let mut h = RequestHarness::new(DestinationPreference::Origin);
    h.persist_seed().await;
    let before = h.native_task();
    let transport = MockEngagementTransport::new(&h, true);
    let state = h.state.clone();
    let request = h.request();
    let t = transport.clone();
    let waiter =
        tokio::spawn(async move { state.request_task_follow_up(request, t, || NOW).await });
    transport.entered().await;
    let shutdown = h
        .supervisor
        .begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    let report = h
        .supervisor
        .cancel_and_reap(shutdown.budget.stage(tokio::time::Instant::now()))
        .await;
    assert_eq!(report.outcome, ReapOutcome::Joined);
    assert!(report.outstanding.is_empty());
    assert!(matches!(
        waiter.await.unwrap(),
        Ok(TaskFollowUpResult::Refused(_)) | Err(WorkError::Denied | WorkError::Persistence)
    ));
    h.assert_no_request_effect(&transport);
    let reopened = Stores::load(&h.dir).unwrap();
    assert!(reopened.work.engagement.candidates.is_empty());
    assert!(reopened.work.engagement.charges.is_empty());
    assert_eq!(reopened.work.tasks[&h.task], before);
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_request_preserves_explicit_private_preference_in_owner_dm() {
    let h = RequestHarness::new(DestinationPreference::Private);
    h.persist_seed().await;
    let transport = MockEngagementTransport::new(&h, false);
    assert!(matches!(
        h.state
            .request_task_follow_up(h.request(), transport.clone(), || NOW)
            .await
            .unwrap(),
        TaskFollowUpResult::Saved { .. }
    ));
    let c = &Stores::load(&h.dir).unwrap().work.engagement.candidates[&1];
    assert_eq!(c.destination, DestinationPreference::Private);
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    assert_eq!(transport.sends.load(Ordering::SeqCst), 0);
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_request_missing_native_task_or_wrong_actor_never_reads_exchange() {
    for wrong_actor in [false, true] {
        let h = RequestHarness::new(DestinationPreference::Origin);
        h.persist_seed().await;
        let transport = MockEngagementTransport::new(&h, false);
        let mut request = h.request();
        if wrong_actor {
            request.member = 9;
        } else {
            request.task = u64::MAX;
        }
        assert_eq!(
            h.state
                .request_task_follow_up(request, transport.clone(), || NOW)
                .await
                .unwrap(),
            TaskFollowUpResult::Refused(FollowUpDecision::AccessDenied)
        );
        assert_eq!(transport.hydrations.load(Ordering::SeqCst), 0);
        h.assert_no_request_effect(&transport);
        h.finish().await;
    }
}

#[tokio::test]
async fn task_follow_up_request_original_expiry_is_not_extended_or_revalidated_as_a_new_minimum() {
    for elapsed in [1, 60] {
        let h = RequestHarness::new(DestinationPreference::Origin);
        h.persist_seed().await;
        let transport = MockEngagementTransport::new(&h, true);
        let clock = Arc::new(AtomicU64::new(NOW));
        let mut request = h.request();
        request.expiry_seconds = 60;
        let state = h.state.clone();
        let t = transport.clone();
        let request_clock = clock.clone();
        let waiter = tokio::spawn(async move {
            state
                .request_task_follow_up(request, t, move || request_clock.load(Ordering::SeqCst))
                .await
        });
        transport.entered().await;
        clock.store(NOW + elapsed, Ordering::SeqCst);
        transport.release.notify_one();
        let result = waiter.await.unwrap().unwrap();
        if elapsed == 1 {
            assert_eq!(
                result,
                TaskFollowUpResult::Saved {
                    candidate: 1,
                    due_at: NOW,
                    expires_at: NOW + 60
                }
            );
            assert_eq!(
                Stores::load(&h.dir).unwrap().work.engagement.candidates[&1].expires_at,
                Some(NOW + 60)
            );
        } else {
            assert_eq!(
                result,
                TaskFollowUpResult::Refused(FollowUpDecision::Expired)
            );
            h.assert_no_request_effect(&transport);
        }
        assert!(
            AppState::lock(&h.state.stores)
                .work
                .engagement
                .charges
                .is_empty()
        );
        h.finish().await;
    }
}

#[path = "tests/contact_reason.rs"]
mod contact_reason;
