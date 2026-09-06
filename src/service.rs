//! Root-owned task handles and atomic admission for the service lifecycle.
//!
//! `OperationRegistry` clones may admit work, but only `ServiceSupervisor` polls
//! or retires its actual handles. The root must retain the supervisor through
//! terminal containment when a budget expires. Dropping a waiter does not drop
//! a task handle; an abort request never counts as a join.
//!
//! Serenity's borrowed Framework::init remains inside root-owned client
//! construction. Call `finish_startup` only after that future has finished or
//! been dropped under the root's control. The entire owned Framework::dispatch
//! future (including Ready/setup and all hooks) belongs in `spawn_operation`.
use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub mod failure;
pub mod framework;
pub mod persistence;
pub mod refresh;
pub mod scheduler;
pub mod shutdown;
pub mod status;
pub mod telemetry;

pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(20);
pub const STAGE_BUDGET: Duration = Duration::from_secs(5);
const REAP_RESERVE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskName {
    Scheduler,
    Telegram,
    Slack,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskExit {
    Cancelled,
    Returned,
    Panicked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    Signal,
    DiscordClientReturned,
    DiscordClientFailed,
    SupervisedTaskFailed(TaskName),
    OperationalInvariantFailed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePhase {
    Starting,
    Running,
    Draining,
    Frozen,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    FrameworkDispatch,
    ConnectorEvent,
    Summary,
    Episode,
    Voice,
    ConsentPersistence,
    ProviderPersistence,
    ProviderProcess,
    MemoryDrain,
    PersistencePreparation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedTaskKind {
    Service(TaskName),
    Operation(OperationKind),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OperationId(u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    NotRunning,
    DuplicateService,
    IdentityExhausted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeError {
    NotDraining,
    StartupStillOwned,
    TasksNotJoined,
    WriterNotIdle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskCompletion {
    pub id: OperationId,
    pub kind: OwnedTaskKind,
    pub exit: TaskExit,
    pub abort_requested: bool,
    pub fatal: Option<ShutdownReason>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutstandingTask {
    pub id: OperationId,
    pub kind: OwnedTaskKind,
    pub abort_requested: bool,
}
/// Receipt cancellation only drops a notification receiver, never the task.
pub struct OperationReceipt {
    pub id: OperationId,
    completion: oneshot::Receiver<TaskCompletion>,
}
impl OperationReceipt {
    /// Root must continuously call `next_completion`; it sends receipts only
    /// after observing the actual JoinHandle result, including cancellation.
    pub async fn joined(self) -> Result<TaskCompletion, oneshot::error::RecvError> {
        self.completion.await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ShutdownBudget {
    started: Instant,
    deadline: Instant,
}
impl ShutdownBudget {
    #[cfg(test)]
    pub fn started(self) -> Instant {
        self.started
    }
    #[cfg(test)]
    pub fn deadline(self) -> Instant {
        self.deadline
    }
    pub fn elapsed(self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }
    pub fn remaining(self, now: Instant) -> Duration {
        self.deadline.saturating_duration_since(now)
    }
    pub fn stage(self, now: Instant) -> StageBudget {
        let deadline = (now + STAGE_BUDGET).min(self.deadline);
        let remaining = deadline.saturating_duration_since(now);
        // Reserve cleanup within the same stage, even with little time left.
        let reserve = REAP_RESERVE.min(remaining / 2);
        StageBudget {
            deadline,
            abort_at: deadline - reserve,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct StageBudget {
    pub deadline: Instant,
    pub abort_at: Instant,
}
#[derive(Debug, Clone, Copy)]
pub struct ShutdownStart {
    pub reason: ShutdownReason,
    pub budget: ShutdownBudget,
    pub first_trigger: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapOutcome {
    Joined,
    TimedOut,
}
#[derive(Debug)]
pub struct ReapReport {
    pub outcome: ReapOutcome,
    pub joined: Vec<TaskCompletion>,
    pub outstanding: Vec<OutstandingTask>,
}

struct OwnedTask {
    kind: OwnedTaskKind,
    token: CancellationToken,
    handle: JoinHandle<TaskExit>,
    abort_requested: bool,
    receipt: oneshot::Sender<TaskCompletion>,
}
struct RegistryState {
    phase: ServicePhase,
    startup_owned: bool,
    shutdown: Option<ShutdownStart>,
    next_id: u64,
    tasks: BTreeMap<OperationId, OwnedTask>,
    // Exactly one root observer, enforced by next_completion's &mut self.
    observer: Option<Waker>,
    shutdown_completions: Vec<TaskCompletion>,
}
struct Shared {
    state: Mutex<RegistryState>,
    cancellation: CancellationToken,
}
fn lock(shared: &Shared) -> MutexGuard<'_, RegistryState> {
    shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
#[derive(Clone)]
pub struct OperationRegistry(Arc<Shared>);
/// There is one root owner. Admission clones cannot join or remove handles.
pub struct ServiceSupervisor {
    shared: Arc<Shared>,
}
impl Default for ServiceSupervisor {
    fn default() -> Self {
        Self::new()
    }
}
impl ServiceSupervisor {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(RegistryState {
                    phase: ServicePhase::Starting,
                    startup_owned: true,
                    shutdown: None,
                    next_id: 0,
                    tasks: BTreeMap::new(),
                    observer: None,
                    shutdown_completions: Vec::new(),
                }),
                cancellation: CancellationToken::new(),
            }),
        }
    }
    pub fn operations(&self) -> OperationRegistry {
        OperationRegistry(self.shared.clone())
    }
    pub fn phase(&self) -> ServicePhase {
        lock(&self.shared).phase
    }
    /// The root has observed completion/drop of its borrowed construction
    /// future. A concurrent shutdown cannot reopen admission.
    pub fn finish_startup(&mut self) {
        let mut state = lock(&self.shared);
        state.startup_owned = false;
        if state.phase == ServicePhase::Starting {
            state.phase = ServicePhase::Running;
        }
    }
    pub fn spawn_service<F, Fut>(
        &mut self,
        name: TaskName,
        work: F,
    ) -> Result<OperationReceipt, AdmissionError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = TaskExit> + Send + 'static,
    {
        register(
            &self.shared,
            OwnedTaskKind::Service(name),
            |token, start| {
                tokio::spawn(async move {
                    if start.await.is_err() {
                        return TaskExit::Cancelled;
                    }
                    work(token).await
                })
            },
        )
    }
    /// Close admission and fix the first trigger while holding the same lock
    /// used to insert every handle. `now` is sampled before readiness/log I/O.
    pub fn begin_draining(&mut self, reason: ShutdownReason, now: Instant) -> ShutdownStart {
        let mut state = lock(&self.shared);
        if let Some(previous) = state.shutdown {
            return ShutdownStart {
                first_trigger: false,
                ..previous
            };
        }
        let started = ShutdownStart {
            reason,
            budget: ShutdownBudget {
                started: now,
                deadline: now + SHUTDOWN_BUDGET,
            },
            first_trigger: true,
        };
        state.phase = ServicePhase::Draining;
        state.shutdown = Some(started);
        started
    }
    pub fn request_cancellation(&self) {
        self.shared.cancellation.cancel();
    }
    pub fn request_abort(&mut self) {
        for task in lock(&self.shared).tasks.values_mut() {
            task.token.cancel();
            // These owners contain cancellation-independent writes or child
            // cleanup. Retain them to completion or the terminal boundary.
            if !matches!(
                task.kind,
                OwnedTaskKind::Operation(
                    OperationKind::ProviderProcess
                        | OperationKind::MemoryDrain
                        | OperationKind::PersistencePreparation
                        | OperationKind::Voice
                        | OperationKind::Episode
                )
            ) {
                task.abort_requested = true;
                task.handle.abort();
            }
        }
    }
    pub fn outstanding(&self) -> Vec<OutstandingTask> {
        lock(&self.shared)
            .tasks
            .iter()
            .map(|(id, task)| OutstandingTask {
                id: *id,
                kind: task.kind,
                abort_requested: task.abort_requested,
            })
            .collect()
    }
    /// Cancellation-safe observation: each actual handle stays in the registry
    /// while Pending. Only Ready joins retire an entry and publish a receipt.
    pub async fn next_completion(&mut self) -> TaskCompletion {
        poll_fn(|cx| self.poll_completion(cx)).await
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<TaskCompletion> {
        let mut state = lock(&self.shared);
        state.observer = Some(cx.waker().clone());
        let mut completed = None;
        for (id, task) in &mut state.tasks {
            if let Poll::Ready(result) = Pin::new(&mut task.handle).poll(cx) {
                let exit = match result {
                    Ok(exit) => exit,
                    Err(error) if error.is_cancelled() => TaskExit::Cancelled,
                    Err(_) => TaskExit::Panicked,
                };
                completed = Some((*id, exit));
                break;
            }
        }
        let Some((id, exit)) = completed else {
            return Poll::Pending;
        };
        let task = state.tasks.remove(&id).expect("observed registered handle");
        let fatal = match (task.kind, exit, task.token.is_cancelled()) {
            (OwnedTaskKind::Service(_), TaskExit::Cancelled, true) => None,
            (OwnedTaskKind::Service(name), _, _) => {
                Some(ShutdownReason::SupervisedTaskFailed(name))
            }
            (OwnedTaskKind::Operation(_), TaskExit::Panicked, _) => {
                Some(ShutdownReason::OperationalInvariantFailed)
            }
            _ => None,
        };
        let completion = TaskCompletion {
            id,
            kind: task.kind,
            exit,
            abort_requested: task.abort_requested,
            fatal,
        };
        if state.phase == ServicePhase::Draining {
            state.shutdown_completions.push(completion);
        }
        let _ = task.receipt.send(completion);
        Poll::Ready(completion)
    }
    fn joined_during_shutdown(&self) -> Vec<TaskCompletion> {
        lock(&self.shared).shutdown_completions.clone()
    }
    pub async fn cancel_and_reap(&mut self, budget: StageBudget) -> ReapReport {
        self.request_cancellation();
        let mut aborted = false;
        loop {
            if self.outstanding().is_empty() {
                return ReapReport {
                    outcome: if Instant::now() > budget.deadline {
                        ReapOutcome::TimedOut
                    } else {
                        ReapOutcome::Joined
                    },
                    joined: self.joined_during_shutdown(),
                    outstanding: Vec::new(),
                };
            }
            let now = Instant::now();
            if now >= budget.deadline {
                return ReapReport {
                    outcome: ReapOutcome::TimedOut,
                    joined: self.joined_during_shutdown(),
                    outstanding: self.outstanding(),
                };
            }
            if !aborted && now >= budget.abort_at {
                self.request_abort();
                aborted = true;
            }
            let until = if aborted {
                budget.deadline
            } else {
                budget.abort_at
            };
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(until) => {}
                _ = self.next_completion() => {},
            }
        }
    }
    /// `writer_idle` must come from the retained writer's observed state, not
    /// from a timed-out request waiter. Frozen only follows actual task joins.
    pub fn try_freeze(&mut self, writer_idle: bool) -> Result<(), FreezeError> {
        let mut state = lock(&self.shared);
        if state.phase != ServicePhase::Draining {
            return Err(FreezeError::NotDraining);
        }
        if state.startup_owned {
            return Err(FreezeError::StartupStillOwned);
        }
        if !state.tasks.is_empty() {
            return Err(FreezeError::TasksNotJoined);
        }
        if !writer_idle {
            return Err(FreezeError::WriterNotIdle);
        }
        state.phase = ServicePhase::Frozen;
        Ok(())
    }
}
impl OperationRegistry {
    pub fn is_running(&self) -> bool {
        lock(&self.0).phase == ServicePhase::Running
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.0.cancellation.clone()
    }
    pub fn spawn_result<T, Fut>(
        &self,
        kind: OperationKind,
        work: Fut,
    ) -> Result<oneshot::Receiver<T>, AdmissionError>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let (send, receive) = oneshot::channel();
        self.spawn_operation(kind, move |_| async move {
            let result = work.await;
            let _ = send.send(result);
            TaskExit::Returned
        })?;
        Ok(receive)
    }
    pub fn blocking_result<T, F>(
        &self,
        kind: OperationKind,
        work: F,
    ) -> Result<oneshot::Receiver<T>, AdmissionError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (send, receive) = oneshot::channel();
        self.spawn_blocking(kind, move |_| {
            let result = work();
            let _ = send.send(result);
            TaskExit::Returned
        })?;
        Ok(receive)
    }
    pub fn spawn_operation<F, Fut>(
        &self,
        kind: OperationKind,
        work: F,
    ) -> Result<OperationReceipt, AdmissionError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = TaskExit> + Send + 'static,
    {
        register(&self.0, OwnedTaskKind::Operation(kind), |token, start| {
            tokio::spawn(async move {
                if start.await.is_err() {
                    return TaskExit::Cancelled;
                }
                work(token).await
            })
        })
    }
    /// The registered handle is the actual blocking task, not an async proxy.
    /// Once started, abort may have no effect; the handle stays outstanding.
    pub fn spawn_blocking<F>(
        &self,
        kind: OperationKind,
        work: F,
    ) -> Result<OperationReceipt, AdmissionError>
    where
        F: FnOnce(CancellationToken) -> TaskExit + Send + 'static,
    {
        register(&self.0, OwnedTaskKind::Operation(kind), |token, start| {
            tokio::task::spawn_blocking(move || {
                if start.blocking_recv().is_err() {
                    return TaskExit::Cancelled;
                }
                work(token)
            })
        })
    }
}
fn register(
    shared: &Shared,
    kind: OwnedTaskKind,
    spawn: impl FnOnce(CancellationToken, oneshot::Receiver<()>) -> JoinHandle<TaskExit>,
) -> Result<OperationReceipt, AdmissionError> {
    let mut state = lock(shared);
    if state.phase != ServicePhase::Running {
        return Err(AdmissionError::NotRunning);
    }
    if let OwnedTaskKind::Service(name) = kind
        && state
            .tasks
            .values()
            .any(|task| task.kind == OwnedTaskKind::Service(name))
    {
        return Err(AdmissionError::DuplicateService);
    }
    let next = state
        .next_id
        .checked_add(1)
        .ok_or(AdmissionError::IdentityExhausted)?;
    let id = OperationId(next);
    let token = shared.cancellation.child_token();
    let (admit, start) = oneshot::channel();
    let handle = spawn(token.clone(), start);
    let (send, completion) = oneshot::channel();
    state.tasks.insert(
        id,
        OwnedTask {
            kind,
            token,
            handle,
            abort_requested: false,
            receipt: send,
        },
    );
    state.next_id = next;
    let observer = state.observer.take();
    drop(state);
    let _ = admit.send(());
    if let Some(observer) = observer {
        observer.wake();
    }
    Ok(OperationReceipt { id, completion })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod persistence_tests;
