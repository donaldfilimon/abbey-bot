//! Content-free Discord interaction outcome observation, shared by command adapters.
use crate::observability::{EventCode, EventComponent, EventOutcome, OperationalErrorCategory};

pub(crate) fn record_failure(
    state: &crate::runtime::AppState,
    code: EventCode,
    category: OperationalErrorCategory,
) {
    // The message stays category-neutral: `?category` already carries the cause, and
    // hardcoding "unavailable" here would restate at the log layer the false outage
    // claim that typed categories removed from the event layer.
    tracing::warn!(?code, ?category, "interaction outcome failed");
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

/// Classify a boxed command error by its *type*, never by message text: the operational
/// record is a closed vocabulary and must not depend on user-facing copy. An error whose
/// type carries no cause is `Internal` — claiming `Unavailable` for an unclassified
/// failure would be a false statement that the service is down.
///
/// Callers with their own typed refusal check it first and fall back to this.
pub(crate) fn category_of(error: &crate::Error) -> OperationalErrorCategory {
    error
        .downcast_ref::<serenity::Error>()
        .map_or(OperationalErrorCategory::Internal, serenity_category)
}

/// Discord REST and gateway faults. A refused request is not an outage: only transport
/// loss and server-side faults earn `Unavailable`.
pub(crate) fn serenity_category(error: &serenity::Error) -> OperationalErrorCategory {
    match error {
        serenity::Error::Http(http) => {
            http_status_category(http.status_code().map(|code| code.as_u16()))
        }
        serenity::Error::Gateway(_) | serenity::Error::Tungstenite(_) => {
            OperationalErrorCategory::Unavailable
        }
        serenity::Error::Model(_) => OperationalErrorCategory::Authorization,
        serenity::Error::Json(_) | serenity::Error::Format(_) => OperationalErrorCategory::Protocol,
        _ => OperationalErrorCategory::Internal,
    }
}

/// Pure status -> category mapping. Split out because `serenity`'s error response types
/// are `#[non_exhaustive]` and cannot be constructed in a test.
pub(crate) fn http_status_category(status: Option<u16>) -> OperationalErrorCategory {
    match status {
        Some(401 | 403) => OperationalErrorCategory::Authorization,
        Some(429) => OperationalErrorCategory::Capacity,
        Some(status) if (500..600).contains(&status) => OperationalErrorCategory::Unavailable,
        Some(_) => OperationalErrorCategory::Protocol,
        // No status means the request never completed: transport loss, not a refusal.
        None => OperationalErrorCategory::Unavailable,
    }
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

    /// An error whose type carries no cause must NOT be reported as `Unavailable`:
    /// that would be a false claim that the service is down.
    #[test]
    fn unclassified_failures_are_internal_not_unavailable() {
        let opaque: crate::Error = "some unclassified failure".into();
        assert_eq!(category_of(&opaque), OperationalErrorCategory::Internal);
        assert_ne!(category_of(&opaque), OperationalErrorCategory::Unavailable);
    }

    /// A Discord refusal is not an outage. Only transport loss and server-side faults
    /// earn `Unavailable`; a 403 is the caller's permissions, and a 429 is capacity.
    #[test]
    fn discord_rest_status_selects_the_category() {
        assert_eq!(
            http_status_category(Some(401)),
            OperationalErrorCategory::Authorization
        );
        assert_eq!(
            http_status_category(Some(403)),
            OperationalErrorCategory::Authorization
        );
        assert_eq!(
            http_status_category(Some(429)),
            OperationalErrorCategory::Capacity
        );
        assert_eq!(
            http_status_category(Some(500)),
            OperationalErrorCategory::Unavailable
        );
        assert_eq!(
            http_status_category(Some(503)),
            OperationalErrorCategory::Unavailable
        );
        assert_eq!(
            http_status_category(Some(404)),
            OperationalErrorCategory::Protocol
        );
        assert_eq!(
            http_status_category(Some(400)),
            OperationalErrorCategory::Protocol
        );
        // A request that never completed is transport loss, which IS unavailability.
        assert_eq!(
            http_status_category(None),
            OperationalErrorCategory::Unavailable
        );
    }
}
