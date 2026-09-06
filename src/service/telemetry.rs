//! Bounded admission to one retained blocking owner for all managed output I/O.
//! Queueing a line is not a durability receipt; readiness/removal have explicit receipts.
use crate::{
    managed_log::ManagedLog,
    managed_service::ManagedFatalSignal,
    observability::{ManagedFailure, OperationalEvent},
    readiness::{ReadinessDocument, ReadinessPublisher},
};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::{sync::oneshot, task::JoinHandle};

const QUEUE_CAPACITY: usize = 1024;
type Receipt = oneshot::Receiver<Result<(), ManagedFailure>>;
type Response = oneshot::Sender<Result<(), ManagedFailure>>;
enum Message {
    Event(OperationalEvent),
    Readiness(ReadinessDocument, Response),
    Remove(Response),
}
struct State {
    accepting: bool,
    pending: usize,
    failure: Option<ManagedFailure>,
}
type FailureHook = Arc<dyn Fn() + Send + Sync>;
#[derive(Clone)]
pub struct TelemetryRequests {
    sender: mpsc::SyncSender<Message>,
    state: Arc<Mutex<State>>,
    fatal: FailureHook,
}
pub struct TelemetryWriter {
    requests: TelemetryRequests,
    handle: JoinHandle<()>,
}

trait Sink: Send + 'static {
    fn event(&self, event: &OperationalEvent) -> Result<(), ManagedFailure>;
    fn readiness(&self, document: &ReadinessDocument) -> Result<(), ManagedFailure>;
    fn remove(&self) -> Result<(), ManagedFailure>;
}
struct ManagedSink {
    log: Arc<ManagedLog>,
    publisher: ReadinessPublisher,
}
impl Sink for ManagedSink {
    fn event(&self, event: &OperationalEvent) -> Result<(), ManagedFailure> {
        self.log.write(event)
    }
    fn readiness(&self, document: &ReadinessDocument) -> Result<(), ManagedFailure> {
        self.publisher.publish(document)?;
        if document.is_ready() {
            self.publisher.remove_bootstrap()?;
        }
        // This event describes observed publication, never merely queue admission.
        let event = OperationalEvent::new(
            crate::runtime::now_millis(),
            crate::observability::EventComponent::Process,
            crate::observability::EventCode::ReadinessPublished,
            crate::observability::EventOutcome::Succeeded,
        )?;
        self.log.write(&event)
    }
    fn remove(&self) -> Result<(), ManagedFailure> {
        // Both removals independently enforce current PID+nonce ownership.
        self.publisher.remove_readiness()?;
        self.publisher.remove_bootstrap()?;
        Ok(())
    }
}
impl TelemetryWriter {
    pub fn start(
        log: Arc<ManagedLog>,
        publisher: ReadinessPublisher,
        fatal: Arc<ManagedFatalSignal>,
    ) -> Self {
        Self::start_sink(
            ManagedSink { log, publisher },
            Arc::new(move || fatal.trigger()),
        )
    }
    fn start_sink(sink: impl Sink, fatal: FailureHook) -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let state = Arc::new(Mutex::new(State {
            accepting: true,
            pending: 0,
            failure: None,
        }));
        let owned = state.clone();
        let failed = fatal.clone();
        let handle = tokio::task::spawn_blocking(move || {
            struct UnexpectedExit(Arc<Mutex<State>>, FailureHook);
            impl Drop for UnexpectedExit {
                fn drop(&mut self) {
                    let unexpected = {
                        let mut state = self
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let unexpected = state.accepting || state.pending != 0;
                        if unexpected {
                            state.failure = Some(ManagedFailure::WriterPoisoned);
                        }
                        unexpected
                    };
                    if unexpected {
                        (self.1)();
                    }
                }
            }
            let _exit = UnexpectedExit(owned.clone(), failed.clone());
            loop {
                let message = match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let state = owned
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if !state.accepting && state.pending == 0 {
                            break;
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let previous = owned
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .failure;
                let (result, response) = match message {
                    Message::Event(event) => {
                        (previous.map_or_else(|| sink.event(&event), Err), None)
                    }
                    Message::Readiness(document, response) => (
                        previous.map_or_else(|| sink.readiness(&document), Err),
                        Some(response),
                    ),
                    Message::Remove(response) => {
                        (previous.map_or_else(|| sink.remove(), Err), Some(response))
                    }
                };
                {
                    let mut state = owned
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.pending -= 1;
                    if let Err(error) = result {
                        state.failure = Some(error);
                    }
                }
                if result.is_err() {
                    failed();
                }
                if let Some(response) = response {
                    let _ = response.send(result);
                }
            }
        });
        Self {
            requests: TelemetryRequests {
                sender,
                state,
                fatal,
            },
            handle,
        }
    }
    pub fn requests(&self) -> TelemetryRequests {
        self.requests.clone()
    }
    pub fn idle(&self) -> bool {
        self.requests
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            == 0
    }
    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.handle.is_finished()
    }
    pub fn stop(&self) {
        self.requests
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting = false;
    }
    pub async fn joined(&mut self) -> Result<(), tokio::task::JoinError> {
        (&mut self.handle).await
    }
}
impl Drop for TelemetryWriter {
    fn drop(&mut self) {
        self.stop();
    }
}
impl TelemetryRequests {
    pub fn record(
        &self,
        component: crate::observability::EventComponent,
        code: crate::observability::EventCode,
        outcome: crate::observability::EventOutcome,
        error: Option<crate::observability::OperationalErrorCategory>,
    ) -> Result<(), ManagedFailure> {
        let mut event =
            match OperationalEvent::new(crate::runtime::now_millis(), component, code, outcome) {
                Ok(event) => event,
                Err(error) => {
                    (self.fatal)();
                    return Err(error);
                }
            };
        if let Some(error) = error {
            event = event.with_error(error);
        }
        self.event(event)
    }
    fn enqueue(&self, message: Message) -> Result<(), ManagedFailure> {
        let result = (|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ManagedFailure::WriterPoisoned)?;
            if let Some(failure) = state.failure {
                return Err(failure);
            }
            if !state.accepting {
                return Err(ManagedFailure::Write);
            }
            if self.sender.try_send(message).is_err() {
                state.failure = Some(ManagedFailure::Write);
                return Err(ManagedFailure::Write);
            }
            state.pending += 1;
            Ok(())
        })();
        if result.is_err() {
            (self.fatal)();
        }
        result
    }
    pub fn event(&self, event: OperationalEvent) -> Result<(), ManagedFailure> {
        self.enqueue(Message::Event(event))
    }
    pub fn readiness(&self, document: ReadinessDocument) -> Result<Receipt, ManagedFailure> {
        let (response, receive) = oneshot::channel();
        self.enqueue(Message::Readiness(document, response))?;
        Ok(receive)
    }
    pub fn remove_final(&self) -> Result<Receipt, ManagedFailure> {
        let (response, receive) = oneshot::channel();
        self.enqueue(Message::Remove(response))?;
        Ok(receive)
    }
}

#[cfg(test)]
mod tests;
