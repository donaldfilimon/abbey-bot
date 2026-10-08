//! Owned content-free measurement retained until an actual delivery receipt.
use crate::{
    observability::{
        EventCode, EventOutcome, OperationalErrorCategory, OperationalEvent, TextStage,
    },
    provider::ProviderId,
    runtime::AppState,
    service::telemetry::TelemetryRequests,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct DeliveryTiming(Arc<Measurement>);
impl std::fmt::Debug for DeliveryTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeliveryTiming")
    }
}
struct Measurement {
    events: Option<TelemetryRequests>,
    discord: bool,
    started: tokio::time::Instant,
    provider: Mutex<Option<ProviderId>>,
    first: AtomicBool,
    final_delivered: AtomicBool,
    #[cfg(test)]
    pub(super) observed: Arc<Mutex<Vec<(EventCode, u64)>>>,
}
impl DeliveryTiming {
    pub(super) fn new(state: &AppState, discord: bool) -> Self {
        Self(Arc::new(Measurement {
            events: state.operational_events().cloned(),
            discord,
            started: tokio::time::Instant::now(),
            provider: Mutex::new(None),
            first: AtomicBool::new(false),
            final_delivered: AtomicBool::new(false),
            #[cfg(test)]
            observed: Arc::new(Mutex::new(Vec::new())),
        }))
    }
    pub(super) fn elapsed(&self) -> std::time::Duration {
        self.0.started.elapsed()
    }
    pub(super) fn provider(&self, provider: Option<ProviderId>) {
        *AppState::lock(&self.0.provider) = provider;
    }
    pub(super) fn posted(&self) {
        if self.0.discord && !self.0.first.swap(true, Ordering::Relaxed) {
            self.emit(
                TextStage::FirstVisible.code(),
                EventOutcome::Succeeded,
                None,
                self.elapsed(),
            );
        }
    }
    /// Only the delivery owner calls this after a nonempty successful native receipt.
    pub(crate) fn delivered(&self) {
        if self.0.discord && !self.0.final_delivered.swap(true, Ordering::Relaxed) {
            self.posted();
            self.emit(
                TextStage::FinalDelivered.code(),
                EventOutcome::Succeeded,
                None,
                self.elapsed(),
            );
        }
    }
    pub(crate) fn failed(&self, error: OperationalErrorCategory) {
        if self.0.discord {
            self.emit(
                TextStage::DeliveryFailed.code(),
                EventOutcome::Failed,
                Some(error),
                self.elapsed(),
            );
        }
    }
    pub(super) fn emit(
        &self,
        code: EventCode,
        outcome: EventOutcome,
        error: Option<OperationalErrorCategory>,
        duration: std::time::Duration,
    ) {
        #[cfg(test)]
        AppState::lock(&self.0.observed).push((code, duration.as_millis() as u64));
        if let Some(events) = &self.0.events
            && let Ok(mut event) = OperationalEvent::new(
                crate::runtime::now_millis(),
                super::timing::component(code),
                code,
                outcome,
            )
        {
            event = event.with_duration(duration);
            if let Some(provider) = AppState::lock(&self.0.provider).clone() {
                event = event.with_provider(provider);
            }
            if let Some(error) = error {
                event = event.with_error(error);
            }
            let _ = events.event(event);
        }
    }
    #[cfg(test)]
    pub(super) fn observed(&self) -> Arc<Mutex<Vec<(EventCode, u64)>>> {
        self.0.observed.clone()
    }
}
