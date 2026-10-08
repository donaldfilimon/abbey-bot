//! Bounded coalesced text and actual producer join ownership.
use crate::{
    llm,
    outbound_failure::{DeliveryCertainty, OutboundFailure, OutboundFailureCategory},
};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub(crate) const MAX_TEXT_BYTES: usize = 65_536;
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BufferFull;
#[derive(Default)]
pub(crate) struct CoalescedText(String);
impl CoalescedText {
    pub(crate) fn append(&mut self, delta: &str) -> Result<(), BufferFull> {
        if delta.len() > MAX_TEXT_BYTES.saturating_sub(self.0.len()) {
            return Err(BufferFull);
        }
        self.0.push_str(delta);
        Ok(())
    }
    pub(crate) fn snapshot(&self) -> &str {
        &self.0
    }
}
pub(crate) fn capacity() -> llm::LlmError {
    llm::LlmError::delivery(OutboundFailure::new(
        OutboundFailureCategory::Capacity,
        DeliveryCertainty::NotSent,
        None,
    ))
}
pub(crate) fn validate(turn: llm::ModelTurn) -> Result<llm::ModelTurn, llm::LlmError> {
    if turn.text.len() > MAX_TEXT_BYTES {
        Err(capacity())
    } else if turn.text.trim().is_empty() && turn.calls.is_empty() {
        Err(llm::LlmError::backend(
            "the response carried no answer text".into(),
        ))
    } else {
        Ok(turn)
    }
}
struct State {
    text: CoalescedText,
    version: u64,
    failed: bool,
}
type TextObserver = Arc<dyn Fn(&str) + Send + Sync>;
struct Shared {
    observer: Mutex<Option<TextObserver>>,
    state: Mutex<State>,
    changed: Notify,
    cancellation: CancellationToken,
}
#[derive(Clone)]
pub struct DeltaSender(Arc<Shared>);
pub(crate) struct DeltaReceiver {
    shared: Arc<Shared>,
    seen: u64,
    pub(super) retained: bool,
}
pub(crate) fn channel() -> (DeltaSender, DeltaReceiver) {
    let shared = Arc::new(Shared {
        observer: Mutex::new(None),
        state: Mutex::new(State {
            text: CoalescedText::default(),
            version: 0,
            failed: false,
        }),
        changed: Notify::new(),
        cancellation: CancellationToken::new(),
    });
    (
        DeltaSender(shared.clone()),
        DeltaReceiver {
            shared,
            seen: 0,
            retained: false,
        },
    )
}
impl DeltaSender {
    pub(crate) fn observe(&self, observer: TextObserver) {
        *crate::runtime::AppState::lock(&self.0.observer) = Some(observer);
    }
    pub(crate) fn send(&self, delta: String) -> Result<(), llm::LlmError> {
        if delta.is_empty() {
            return Ok(());
        }
        let mut state = crate::runtime::AppState::lock(&self.0.state);
        if state.failed || state.text.append(&delta).is_err() {
            state.failed = true;
            self.0.cancellation.cancel();
            self.0.changed.notify_one();
            return Err(capacity());
        }
        state.version += 1;
        drop(state);
        if let Some(observer) = &*crate::runtime::AppState::lock(&self.0.observer) {
            observer(&delta);
        }
        self.0.changed.notify_one();
        Ok(())
    }
    pub(crate) fn check_capacity(&self) -> Result<(), llm::LlmError> {
        if crate::runtime::AppState::lock(&self.0.state).failed {
            Err(capacity())
        } else {
            Ok(())
        }
    }
    pub(crate) fn snapshot(&self) -> Result<String, llm::LlmError> {
        let state = crate::runtime::AppState::lock(&self.0.state);
        if state.failed {
            Err(capacity())
        } else {
            Ok(state.text.snapshot().to_owned())
        }
    }
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.0.cancellation.clone()
    }
}
impl DeltaReceiver {
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.shared.cancellation.clone()
    }
    pub(crate) fn try_recv(&mut self) -> Result<String, ()> {
        let state = crate::runtime::AppState::lock(&self.shared.state);
        if state.version == self.seen {
            return Err(());
        }
        self.seen = state.version;
        Ok(state.text.snapshot().to_owned())
    }
    pub(crate) async fn recv(&mut self) -> Option<String> {
        loop {
            let shared = self.shared.clone();
            let changed = shared.changed.notified();
            if let Ok(text) = self.try_recv() {
                return Some(text);
            }
            changed.await;
        }
    }
    pub(crate) fn failed(&self) -> bool {
        crate::runtime::AppState::lock(&self.shared.state).failed
    }
}

pub(crate) struct StreamOwner<T> {
    handle: tokio::task::JoinHandle<T>,
    cancellation: CancellationToken,
}
impl<T> StreamOwner<T> {
    pub(crate) fn new(handle: tokio::task::JoinHandle<T>, cancellation: CancellationToken) -> Self {
        Self {
            handle,
            cancellation,
        }
    }
    pub(crate) async fn cancel_and_join(self) -> Result<T, tokio::task::JoinError> {
        self.cancellation.cancel();
        self.join().await
    }
    pub(crate) async fn completed(&mut self) -> Result<T, tokio::task::JoinError> {
        (&mut self.handle).await
    }
    pub(crate) async fn join(self) -> Result<T, tokio::task::JoinError> {
        self.handle.await
    }
}

pub(crate) fn cancelled() -> llm::LlmError {
    llm::LlmError::classified(
        "provider generation cancelled",
        crate::provider::ProviderFailureKind::Cancelled,
    )
}
