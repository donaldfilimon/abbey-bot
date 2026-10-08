//! Actual interaction delivery receipts and queued-memory ownership.
use super::*;

/// Finish delivery before admitting queued model writes. Failed delivery
/// cancels only this turn's still-queued writes and waits for any drain that
/// already owns one; neither path can replay an effect. Both interactive
/// generation surfaces use this boundary, while the pipeline owns its
/// corresponding outbound boundary.
pub(crate) async fn deliver_generated_reply<T, E>(
    state: &AppState,
    memory: crate::memory_gate::MemoryTurn,
    delivery: impl std::future::Future<Output = Result<T, E>>,
) -> Result<(T, crate::memory_gate::MemoryTurn), E> {
    match delivery.await {
        Ok(delivered) => {
            if state.episode_gate.is_some() {
                crate::memory_gate::drain(state).await;
            }
            Ok((delivered, memory))
        }
        Err(error) => {
            crate::memory_gate::cancel_pending(state, &memory);
            // An already-running drain is absent from the queue and remains
            // authoritative. Wait for its bounded terminal result before this
            // turn owner exits; the failed response leaves nowhere to post it.
            let _ = memory.decisions().await;
            Err(error)
        }
    }
}

pub(crate) fn observe_generated_receipt(
    timing: Option<&generation::DeliveryTiming>,
    id: u64,
) -> Result<(), Error> {
    if id == 0 {
        return Err(crate::outbound_failure::OutboundFailure::new(
            crate::outbound_failure::OutboundFailureCategory::Internal,
            crate::outbound_failure::DeliveryCertainty::PossiblySent,
            None,
        )
        .into());
    }
    if let Some(timing) = timing {
        timing.delivered();
    }
    Ok(())
}
