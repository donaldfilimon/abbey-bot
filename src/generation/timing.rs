//! Content-free timing of the canonical text path. Voice has its own pipeline.
use crate::{
    observability::{
        EventCode, EventComponent, EventOutcome, OperationalErrorCategory, OperationalEvent,
    },
    runtime::AppState,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) struct Timing<'a> {
    state: &'a AppState,
    discord: bool,
    started: tokio::time::Instant,
    generation: std::sync::Mutex<GenerationTime>,
    first_text: AtomicBool,
    first_post: AtomicBool,
    terminal: AtomicBool,
    #[cfg(test)]
    pub(super) observed: std::sync::Mutex<Vec<(EventCode, u64)>>,
}
#[derive(Default)]
struct GenerationTime {
    started: Option<tokio::time::Instant>,
    later_waits: Duration,
}
impl<'a> Timing<'a> {
    pub(super) fn new(state: &'a AppState, discord: bool) -> Self {
        Self {
            state,
            discord,
            started: tokio::time::Instant::now(),
            generation: std::sync::Mutex::new(GenerationTime::default()),
            first_text: AtomicBool::new(false),
            first_post: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
            #[cfg(test)]
            observed: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn emit(
        &self,
        code: EventCode,
        outcome: EventOutcome,
        error: Option<OperationalErrorCategory>,
    ) {
        let duration = if code == EventCode::GenerationFirstText {
            let generation = AppState::lock(&self.generation);
            generation.started.map_or_else(
                || self.started.elapsed(),
                |start| start.elapsed().saturating_sub(generation.later_waits),
            )
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
        #[cfg(test)]
        AppState::lock(&self.observed).push((code, duration.as_millis() as u64));
        record(self.state, code, outcome, duration, error);
    }
    pub(super) fn admitted(&self, wait: Duration, result: &Result<(), crate::llm::LlmError>) {
        {
            let mut generation = AppState::lock(&self.generation);
            if generation.started.is_some() {
                generation.later_waits += wait;
            } else {
                generation.started = Some(tokio::time::Instant::now());
            }
        }
        self.emit_duration(
            EventCode::GenerationQueue,
            if result.is_ok() {
                EventOutcome::Succeeded
            } else {
                EventOutcome::Failed
            },
            result
                .as_ref()
                .err()
                .map(|error| failure_category(error.provider_failure())),
            wait,
        );
    }
    pub(super) fn text(&self, text: &str) {
        if !text.trim().is_empty() && !self.first_text.swap(true, Ordering::Relaxed) {
            self.emit(
                EventCode::GenerationFirstText,
                EventOutcome::Succeeded,
                None,
            );
        }
    }
    pub(super) fn posted(&self) {
        if self.discord && !self.first_post.swap(true, Ordering::Relaxed) {
            self.emit(EventCode::DiscordFirstPost, EventOutcome::Succeeded, None);
        }
    }
    pub(super) fn post_failed(&self) {
        if self.discord {
            self.emit(
                EventCode::DiscordPostFailure,
                EventOutcome::Failed,
                Some(OperationalErrorCategory::Unavailable),
            );
        }
    }
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
        self.terminal.store(true, Ordering::Relaxed);
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
impl Drop for Timing<'_> {
    fn drop(&mut self) {
        if !self.terminal.load(Ordering::Relaxed) {
            self.emit(EventCode::GenerationFailure, EventOutcome::Cancelled, None);
        }
    }
}
pub(super) async fn observe(
    stream: impl std::future::Future<Output = Result<crate::llm::ModelTurn, crate::llm::LlmError>>,
    mut deltas: tokio::sync::mpsc::UnboundedReceiver<String>,
    timing: Option<&Timing<'_>>,
) -> Result<crate::llm::ModelTurn, crate::llm::LlmError> {
    let mut stream = std::pin::pin!(stream);
    loop {
        tokio::select! {
            biased;
            Some(delta) = deltas.recv() => {
                if let Some(timing) = timing { timing.text(&delta); }
            }
            result = &mut stream => {
                while let Ok(delta) = deltas.try_recv() {
                    if let Some(timing) = timing { timing.text(&delta); }
                }
                return result;
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
        && let Ok(mut event) = OperationalEvent::new(
            crate::runtime::now_millis(),
            match code {
                EventCode::DiscordFirstPost | EventCode::DiscordPostFailure => {
                    EventComponent::Discord
                }
                EventCode::EngagementQueue
                | EventCode::EngagementCompleted
                | EventCode::EngagementFailure => EventComponent::Scheduler,
                _ => EventComponent::Provider,
            },
            code,
            outcome,
        )
    {
        event = event.with_duration(duration);
        if let Some(error) = error {
            event = event.with_error(error);
        }
        let _ = events.event(event);
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
