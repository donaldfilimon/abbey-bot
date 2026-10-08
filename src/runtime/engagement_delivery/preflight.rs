//! No attempted capacity is charged until recipient proof is available.
use super::*;

pub(super) enum PreflightOutcome {
    Authorized(AuthorizedDestination),
    Rejected,
    Unavailable,
    Cancelled,
    Timeout,
}

pub(super) async fn authorize<T: EngagementTransport>(
    transport: &T,
    reservation: &EngagementReservation,
    cancel: &CancellationToken,
) -> PreflightOutcome {
    match final_bounded(cancel, transport.authorize(reservation)).await {
        Ok(destination) if matches(reservation, &destination) => {
            PreflightOutcome::Authorized(destination)
        }
        Ok(_) | Err(FinalCheckFailure::Rejected) => PreflightOutcome::Rejected,
        Err(FinalCheckFailure::Failed(
            WorkError::Denied | WorkError::Invalid | WorkError::Stale,
        )) => PreflightOutcome::Rejected,
        Err(FinalCheckFailure::Failed(
            WorkError::Missing | WorkError::Persistence | WorkError::Full,
        )) => PreflightOutcome::Unavailable,
        Err(FinalCheckFailure::Cancelled) => PreflightOutcome::Cancelled,
        Err(FinalCheckFailure::Timeout) => PreflightOutcome::Timeout,
    }
}
