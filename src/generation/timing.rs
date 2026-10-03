//! Content-free timing of the canonical text path. Voice has its own pipeline.
use crate::{
    observability::{
        EventCode, EventComponent, EventOutcome, OperationalErrorCategory, OperationalEvent,
        TextStage,
    },
    runtime::AppState,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) struct Timing {
    observer: std::sync::Arc<ProducerTiming>,
    terminal: AtomicBool,
}
impl std::ops::Deref for Timing {
    type Target = ProducerTiming;
    fn deref(&self) -> &Self::Target {
        &self.observer
    }
}
pub(super) struct ProducerTiming {
    discord: bool,
    started: tokio::time::Instant,
    generation: std::sync::Mutex<GenerationTime>,
    first_text: AtomicBool,
    delivery_failed: AtomicBool,
    pub(super) delivery: super::delivery_timing::DeliveryTiming,
    #[cfg(test)]
    pub(super) observed: std::sync::Arc<std::sync::Mutex<Vec<(EventCode, u64)>>>,
}
#[derive(Default)]
struct GenerationTime {
    started: Option<tokio::time::Instant>,
}
impl Timing {
    pub(super) fn new(state: &AppState, discord: bool) -> Self {
        let delivery = super::delivery_timing::DeliveryTiming::new(state, discord);
        Self {
            observer: std::sync::Arc::new(ProducerTiming {
                discord,
                started: tokio::time::Instant::now(),
                generation: std::sync::Mutex::new(GenerationTime::default()),
                first_text: AtomicBool::new(false),
                delivery_failed: AtomicBool::new(false),
                delivery: delivery.clone(),
                #[cfg(test)]
                observed: delivery.observed(),
            }),
            terminal: AtomicBool::new(false),
        }
    }
    pub(super) fn observer(&self) -> std::sync::Arc<ProducerTiming> {
        self.observer.clone()
    }
}
impl ProducerTiming {
    fn emit(
        &self,
        code: EventCode,
        outcome: EventOutcome,
        error: Option<OperationalErrorCategory>,
    ) {
        let duration = if code == EventCode::GenerationFirstText {
            let generation = AppState::lock(&self.generation);
            generation
                .started
                .map_or_else(|| self.started.elapsed(), |start| start.elapsed())
        } else {
            self.started.elapsed()
        };
        self.emit_duration(code, outcome, error, duration);
    }
    fn emit_duration(
        &self,
        code: EventCode,
        outcome: EventOutcome,
        error: Option<OperationalErrorCategory>,
        duration: Duration,
    ) {
        self.delivery.emit(code, outcome, error, duration);
    }
    pub(super) fn admitted(&self, wait: Duration, result: &Result<(), crate::llm::LlmError>) {
        self.emit_duration(
            TextStage::QueueWait.code(),
            if result.is_ok() {
                EventOutcome::Succeeded
            } else if result.as_ref().is_err_and(|error| {
                error.provider_failure() == crate::provider::ProviderFailureKind::Cancelled
            }) {
                EventOutcome::Cancelled
            } else {
                EventOutcome::Failed
            },
            result
                .as_ref()
                .err()
                .filter(|error| {
                    error.provider_failure() != crate::provider::ProviderFailureKind::Cancelled
                })
                .map(|error| failure_category(error.provider_failure())),
            wait,
        );
    }
    pub(super) fn provider_started(&self) {
        AppState::lock(&self.generation).started = Some(tokio::time::Instant::now());
    }
    pub(super) fn text(&self, text: &str) {
        if !text.trim().is_empty() && !self.first_text.swap(true, Ordering::Relaxed) {
            self.emit(
                TextStage::ProviderFirstText.code(),
                EventOutcome::Succeeded,
                None,
            );
        }
    }
    pub(super) fn posted(&self) {
        self.delivery.posted();
    }
    pub(super) fn post_failed(&self, failure: &crate::outbound_failure::OutboundFailure) {
        self.delivery_failed.store(true, Ordering::Relaxed);
        if self.discord {
            self.emit(
                TextStage::DeliveryFailed.code(),
                EventOutcome::Failed,
                Some(delivery_category(failure)),
            );
        }
    }
}
impl Timing {
    pub(super) fn finish(
        &self,
        result: &Result<
            (
                String,
                Option<String>,
                crate::persona::Persona,
                &'static str,
            ),
            crate::llm::LlmError,
        >,
    ) {
        if self.terminal.swap(true, Ordering::Relaxed) {
            return;
        }
        // Actual outbound faults were recorded at the network boundary.
        // Local producer capacity is replay-forbidden too, but it never made
        // a Discord request. Only this request owner emits its terminal event.
        if let Some(failure) = result
            .as_ref()
            .err()
            .and_then(|error| error.outbound_failure())
        {
            if failure.category() == crate::outbound_failure::OutboundFailureCategory::Capacity
                && !self.delivery_failed.load(Ordering::Relaxed)
            {
                self.emit(
                    EventCode::GenerationFailure,
                    EventOutcome::Failed,
                    Some(OperationalErrorCategory::Capacity),
                );
            }
            return;
        }
        if result.as_ref().is_err_and(|error| {
            error.provider_failure() == crate::provider::ProviderFailureKind::Cancelled
        }) {
            self.emit(TextStage::Cancelled.code(), EventOutcome::Cancelled, None);
            return;
        }
        self.emit(
            if result.is_ok() {
                EventCode::GenerationCompleted
            } else {
                EventCode::GenerationFailure
            },
            if result.is_ok() {
                EventOutcome::Succeeded
            } else {
                EventOutcome::Failed
            },
            result
                .as_ref()
                .err()
                .map(|error| failure_category(error.provider_failure())),
        );
    }
}
impl Drop for Timing {
    fn drop(&mut self) {
        if !self.terminal.load(Ordering::Relaxed) {
            self.emit(TextStage::Cancelled.code(), EventOutcome::Cancelled, None);
        }
    }
}
pub(super) async fn observe(
    stream: impl std::future::Future<Output = Result<crate::llm::ModelTurn, crate::llm::LlmError>>,
    mut deltas: crate::generation::stream_owner::DeltaReceiver,
    timing: Option<&Timing>,
) -> Result<crate::llm::ModelTurn, crate::llm::LlmError> {
    let mut stream = std::pin::pin!(stream);
    let overflow = deltas.cancellation();
    loop {
        tokio::select! {
            biased;
            _ = overflow.cancelled() => {
                let result = if deltas.retained { (&mut stream).await }
                    else { Err(super::stream_owner::capacity()) };
                return if deltas.failed() { Err(super::stream_owner::capacity()) } else { result };
            }
            Some(delta) = deltas.recv() => {
                if let Some(timing) = timing { timing.text(&delta); }
            }
            result = &mut stream => {
                while let Ok(delta) = deltas.try_recv() {
                    if let Some(timing) = timing { timing.text(&delta); }
                }
                return if deltas.failed() { Err(super::stream_owner::capacity()) } else { result.and_then(super::stream_owner::validate) };
            }
        }
    }
}

pub(crate) fn record(
    state: &AppState,
    code: EventCode,
    outcome: EventOutcome,
    duration: Duration,
    error: Option<OperationalErrorCategory>,
) {
    if let Some(events) = state.operational_events()
        && let Ok(mut event) =
            OperationalEvent::new(crate::runtime::now_millis(), component(code), code, outcome)
    {
        event = event.with_duration(duration);
        if let Some(error) = error {
            event = event.with_error(error);
        }
        let _ = events.event(event);
    }
}

pub(super) fn component(code: EventCode) -> EventComponent {
    match code {
        EventCode::DiscordFirstPost
        | EventCode::DiscordFinalDelivered
        | EventCode::DiscordPostFailure => EventComponent::Discord,
        EventCode::EngagementQueue
        | EventCode::EngagementCompleted
        | EventCode::EngagementFailure => EventComponent::Scheduler,
        _ => EventComponent::Provider,
    }
}

fn failure_category(failure: crate::provider::ProviderFailureKind) -> OperationalErrorCategory {
    use crate::provider::ProviderFailureKind as F;
    match failure {
        F::Timeout => OperationalErrorCategory::Timeout,
        F::Authentication => OperationalErrorCategory::Authentication,
        F::Authorization | F::Cancelled => OperationalErrorCategory::Authorization,
        F::Configuration | F::ExecutableIdentity | F::ModelIdentity | F::SandboxIdentity => {
            OperationalErrorCategory::Configuration
        }
        F::ToolSchema | F::ResponseSchema | F::ProtocolDrift | F::InvalidRequest => {
            OperationalErrorCategory::Protocol
        }
        F::RateLimited | F::Busy => OperationalErrorCategory::Capacity,
        F::Success | F::TransportUnavailable | F::Http5xx => OperationalErrorCategory::Unavailable,
    }
}

pub(crate) fn delivery_category(
    failure: &crate::outbound_failure::OutboundFailure,
) -> OperationalErrorCategory {
    use crate::outbound_failure::OutboundFailureCategory as F;
    match failure.category() {
        F::Permission => OperationalErrorCategory::Authorization,
        F::RateLimited | F::Capacity => OperationalErrorCategory::Capacity,
        F::Transport => OperationalErrorCategory::Unavailable,
        F::Internal => OperationalErrorCategory::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn poisoned_timing_state_recovers_without_affecting_generation() {
        let state = AppState::in_memory();
        let timing = Timing::new(&state, false);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = timing.generation.lock().unwrap();
                panic!("synthetic timing poison");
            }))
            .is_err()
        );
        assert!(timing.generation.is_poisoned());
        timing.admitted(Duration::ZERO, &Ok(()));
        timing.text("incremental delta");
        assert_eq!(AppState::lock(&timing.observed).len(), 2);
    }
}
