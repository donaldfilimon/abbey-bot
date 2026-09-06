//! Retained FIFO writer. Only owned snapshots cross into the blocking actor.
use crate::persist::{
    PersistComponentOutcome, PersistReport, PersistenceSink, Stores, persist_canonical,
    persist_projection,
};
use crate::wdbx::Recall;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Debug, Default, Clone, Copy)]
pub struct ComponentProgress {
    pub canonical_state: Option<PersistComponentOutcome>,
    pub wdbx_projection: Option<PersistComponentOutcome>,
}
pub struct Snapshot {
    pub stores: Stores,
    pub recall: Recall,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    Draining,
    WriterUnavailable,
    DeadlineExpired,
}
impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DeadlineExpired => {
                "The final persistence deadline expired before writing started."
            }
            Self::Draining => {
                "The service is shutting down; this persistence request was not started."
            }
            Self::WriterUnavailable => {
                "The persistence worker is unavailable; no completed result is known."
            }
        })
    }
}
impl std::error::Error for RequestError {}
struct WriteRequest {
    snapshot: Snapshot,
    result: oneshot::Sender<Result<PersistReport, RequestError>>,
    deadline: Option<tokio::time::Instant>,
}
enum Message {
    Write(Box<WriteRequest>),
    Stop,
}
struct State {
    accepting: bool,
    final_started: bool,
    queued: usize,
    completed: Option<PersistReport>,
    final_progress: ComponentProgress,
}
#[derive(Clone)]
pub struct PersistenceRequests {
    sender: mpsc::Sender<Message>,
    state: Arc<Mutex<State>>,
}
pub struct PersistenceWriter {
    requests: PersistenceRequests,
    handle: JoinHandle<()>,
    failure: Arc<super::failure::FailureSignal>,
}
impl PersistenceWriter {
    pub fn start(dir: Option<PathBuf>, sink: Arc<dyn PersistenceSink>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            accepting: true,
            final_started: false,
            queued: 0,
            completed: None,
            final_progress: ComponentProgress::default(),
        }));
        let owned = state.clone();
        let failure = Arc::new(super::failure::FailureSignal::default());
        let failed = failure.clone();
        let handle = tokio::task::spawn_blocking(move || {
            struct Guard(Arc<Mutex<State>>, Arc<super::failure::FailureSignal>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    if self
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .accepting
                    {
                        self.1.trigger();
                    }
                }
            }
            let _guard = Guard(owned.clone(), failed);
            while let Ok(message) = receiver.recv() {
                match message {
                    Message::Write(request) => {
                        let report = if request
                            .deadline
                            .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                        {
                            Err(RequestError::DeadlineExpired)
                        } else {
                            Ok(write_snapshot_observed(
                                dir.as_deref(),
                                &*sink,
                                request.snapshot,
                                |progress| {
                                    if request.deadline.is_some() {
                                        owned
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .final_progress = progress;
                                    }
                                },
                            ))
                        };
                        let mut state = owned
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        state.queued -= 1;
                        if let Ok(completed) = report {
                            state.completed = Some(completed);
                        }
                        drop(state);
                        let _ = request.result.send(report);
                    }
                    Message::Stop => break,
                }
            }
        });
        Self {
            requests: PersistenceRequests { sender, state },
            handle,
            failure,
        }
    }
    pub fn failure(&self) -> Arc<super::failure::FailureSignal> {
        self.failure.clone()
    }
    pub fn requests(&self) -> PersistenceRequests {
        self.requests.clone()
    }
    pub fn close_admission(&self) {
        self.requests
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting = false;
    }
    pub fn idle(&self) -> bool {
        self.requests
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .queued
            == 0
    }
    pub fn finished(&self) -> bool {
        self.handle.is_finished()
    }
    pub fn final_progress(&self) -> ComponentProgress {
        self.requests
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .final_progress
    }
    pub fn last_completed(&self) -> Option<PersistReport> {
        self.requests
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .completed
    }
    /// Final write uses the same FIFO only after root has proven quiescence.
    pub fn final_snapshot(
        &self,
        snapshot: Snapshot,
        deadline: tokio::time::Instant,
    ) -> Result<oneshot::Receiver<Result<PersistReport, RequestError>>, RequestError> {
        if !self.idle() {
            return Err(RequestError::WriterUnavailable);
        }
        self.requests.enqueue(snapshot, Some(deadline))
    }
    pub fn stop(&self) {
        self.close_admission();
        let _ = self.requests.sender.send(Message::Stop);
    }
    pub async fn joined(&mut self) -> Result<(), tokio::task::JoinError> {
        (&mut self.handle).await
    }
}
impl PersistenceRequests {
    pub async fn submit(&self, snapshot: Snapshot) -> Result<PersistReport, RequestError> {
        self.enqueue(snapshot, None)?
            .await
            .map_err(|_| RequestError::WriterUnavailable)?
    }
    fn enqueue(
        &self,
        snapshot: Snapshot,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<oneshot::Receiver<Result<PersistReport, RequestError>>, RequestError> {
        let final_write = deadline.is_some();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if final_write {
            if state.accepting || state.final_started || state.queued != 0 {
                return Err(RequestError::WriterUnavailable);
            }
            state.final_started = true;
        }
        if !state.accepting && !final_write {
            return Err(RequestError::Draining);
        }
        let (result, receive) = oneshot::channel();
        self.sender
            .send(Message::Write(Box::new(WriteRequest {
                snapshot,
                result,
                deadline,
            })))
            .map_err(|_| RequestError::WriterUnavailable)?;
        state.queued += 1;
        Ok(receive)
    }
}
pub fn write_snapshot(
    dir: Option<&std::path::Path>,
    sink: &dyn PersistenceSink,
    snapshot: Snapshot,
) -> PersistReport {
    write_snapshot_observed(dir, sink, snapshot, |_| {})
}
fn write_snapshot_observed(
    dir: Option<&std::path::Path>,
    sink: &dyn PersistenceSink,
    snapshot: Snapshot,
    mut observed: impl FnMut(ComponentProgress),
) -> PersistReport {
    let Some(dir) = dir else {
        observed(ComponentProgress {
            canonical_state: Some(PersistComponentOutcome::NotConfigured),
            wdbx_projection: Some(PersistComponentOutcome::NotConfigured),
        });
        return PersistReport::memory_only();
    };
    if let Err(category) = persist_canonical(sink, dir, &snapshot.stores) {
        observed(ComponentProgress {
            canonical_state: Some(PersistComponentOutcome::Failed(category)),
            wdbx_projection: Some(PersistComponentOutcome::SkippedCanonicalFailure),
        });
        return PersistReport::from_components(
            PersistComponentOutcome::Failed(category),
            PersistComponentOutcome::SkippedCanonicalFailure,
        );
    }
    observed(ComponentProgress {
        canonical_state: Some(PersistComponentOutcome::Committed),
        wdbx_projection: None,
    });
    let projection = persist_projection(sink, dir, &snapshot.recall)
        .map_or_else(PersistComponentOutcome::Failed, |()| {
            PersistComponentOutcome::Committed
        });
    observed(ComponentProgress {
        canonical_state: Some(PersistComponentOutcome::Committed),
        wdbx_projection: Some(projection),
    });
    PersistReport::from_components(PersistComponentOutcome::Committed, projection)
}

impl Drop for PersistenceWriter {
    fn drop(&mut self) {
        self.stop();
    }
}
