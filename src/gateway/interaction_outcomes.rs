//! Content-free Discord interaction outcome observation, shared by command adapters.
use crate::observability::{EventCode, EventComponent, EventOutcome, OperationalErrorCategory};

pub(crate) fn record_failure(
    state: &crate::runtime::AppState,
    code: EventCode,
    category: OperationalErrorCategory,
) {
    tracing::warn!(?code, ?category, "interaction outcome unavailable");
    if let Some(events) = state.operational_events() {
        let _ = events.record(
            EventComponent::Discord,
            code,
            EventOutcome::Failed,
            Some(category),
        );
    }
}

/// Record a failed delivery attempt after its error has been consumed.
/// The operation has already run; this function cannot replay it.
pub(crate) fn delivery_failed(state: &crate::runtime::AppState) {
    record_failure(
        state,
        EventCode::ResponseDelivery,
        OperationalErrorCategory::Unavailable,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tracing_subscriber::prelude::*;

    struct CountEvents(Arc<AtomicUsize>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CountEvents {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event
                .metadata()
                .target()
                .ends_with("gateway::interaction_outcomes")
            {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn one_observation_per_failed_attempt_does_not_replay_completed_mutation() {
        let state = crate::runtime::AppState::in_memory();
        let count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(CountEvents(count.clone()));
        tracing::subscriber::with_default(subscriber, || {
            let mut mutations = 0;
            mutations += 1;
            for result in [Ok(()), Err("first attempt"), Err("response-only retry")] {
                if result.is_err() {
                    delivery_failed(&state);
                }
            }
            assert_eq!(mutations, 1);
        });
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
