//! Per-generation completion ownership; background drains cannot steal replies.
use crate::runtime::AppState;
use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::oneshot;

static NEXT_TURN: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Stored,
    Proposed,
    Rejected,
    Unknown,
    Cancelled,
    Unobserved,
    LocalRefused,
}

impl Decision {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Stored => "Memory: stored locally after the gate appended its receipt.",
            Self::Proposed => {
                "Memory: stored locally after the gate appended its receipt. The proposed replacement still needs your confirmation; the old fact is unchanged."
            }
            Self::Rejected => "Memory: the gate rejected the proposal. Nothing was stored locally.",
            Self::Unknown => {
                "Memory: admission is unknown because the gate did not return a valid receipt. Nothing was stored locally."
            }
            Self::Cancelled => {
                "Memory: the request was cancelled before admission. Nothing was stored locally. Try `/remember` when Abbey is available."
            }
            Self::Unobserved => {
                "Memory: the final local outcome could not be observed. Check `/facts` before trying again."
            }
            Self::LocalRefused => {
                "Memory: the gate admitted the proposal, but the local fact was not added because local state changed. No replacement was applied."
            }
        }
    }
}

#[derive(Debug)]
pub struct Completion {
    turn_id: u64,
    sender: oneshot::Sender<Decision>,
}

impl Completion {
    pub fn finish(self, decision: Decision) {
        if self.sender.send(decision).is_err() {
            tracing::info!(
                turn_id = self.turn_id,
                "memory decision recipient no longer available"
            );
        }
    }

    pub(super) fn belongs_to(&self, turn: &MemoryTurn) -> bool {
        self.turn_id == turn.id
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    attempted: usize,
    failed: usize,
}

impl DeliveryReport {
    pub const fn attempted(self) -> usize {
        self.attempted
    }

    pub const fn failed(self) -> usize {
        self.failed
    }
}

pub struct MemoryTurn {
    id: u64,
    pending: Mutex<Vec<oneshot::Receiver<Decision>>>,
}

impl Default for MemoryTurn {
    fn default() -> Self {
        Self {
            id: NEXT_TURN.fetch_add(1, Ordering::Relaxed),
            pending: Mutex::new(Vec::new()),
        }
    }
}

impl MemoryTurn {
    pub fn completion(&self) -> Completion {
        let (sender, receiver) = oneshot::channel();
        AppState::lock(&self.pending).push(receiver);
        Completion {
            turn_id: self.id,
            sender,
        }
    }

    /// Called after generation has stopped enqueueing and either the original
    /// response was delivered or this turn's pending queue entries were
    /// cancelled. A periodic drain may already own the remaining senders.
    pub async fn decisions(self) -> Vec<Decision> {
        let pending = self.pending.into_inner().unwrap_or_else(|e| e.into_inner());
        let mut decisions = Vec::with_capacity(pending.len());
        for receiver in pending {
            decisions.push(receiver.await.unwrap_or(Decision::Unobserved));
        }
        decisions
    }

    /// Consume this turn's terminal outcomes and attempt every response. A
    /// failed response never retries the underlying memory effect and never
    /// prevents a later outcome from being delivered.
    pub async fn deliver<E, F, Fut>(self, mut send: F) -> DeliveryReport
    where
        F: FnMut(Decision) -> Fut,
        Fut: Future<Output = Result<(), E>>,
    {
        let mut report = DeliveryReport::default();
        for decision in self.decisions().await {
            report.attempted += 1;
            if send(decision).await.is_err() {
                report.failed += 1;
            }
        }
        report
    }
}
