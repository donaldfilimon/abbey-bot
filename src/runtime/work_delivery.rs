//! Retained, single-flight work delivery with durable reservation before I/O.
use super::AppState;
use crate::work::{WorkAccess, WorkBatch, WorkDestination, WorkError, WorkScope};
use std::{collections::BTreeSet, future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// The shell supplies fresh origin permissions and proves the entire audience.
/// Errors carry no response bodies, work text, or credentials.
pub(crate) trait WorkDeliveryTransport: Send + Sync {
    fn authorize(
        &self,
        scope: &WorkScope,
        actor: u64,
        origin: u64,
        target: &WorkDestination,
        audience: &BTreeSet<u64>,
    ) -> impl Future<Output = Result<(WorkAccess, u64), WorkError>> + Send;
    fn send(&self, channel: u64, body: &str)
    -> impl Future<Output = Result<u64, WorkError>> + Send;
}

async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = Result<T, WorkError>>,
) -> Result<T, WorkError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(WorkError::Denied),
        result = tokio::time::timeout(Duration::from_secs(30), future) =>
            result.unwrap_or(Err(WorkError::Denied)),
    }
}

impl AppState {
    /// Only called by a retained WorkDelivery operation. Settlement remains
    /// owned after cancellation and does not request new service admission.
    pub(crate) async fn deliver_work<T: WorkDeliveryTransport>(
        self: Arc<Self>,
        transport: &T,
        cancel: CancellationToken,
        now: impl Fn() -> u64,
    ) -> Result<(), WorkError> {
        let Ok(_single) = self.work_delivery_running.try_lock() else {
            return Ok(());
        };
        let candidates = Self::lock(&self.stores).work.delivery_candidates();
        for (scope, actor, origin, target, audience) in candidates {
            if cancel.is_cancelled() {
                break;
            }
            // Cheap local preflight avoids REST/DM resolution when no work is
            // due or the saved actor has lost project authority. These assumed
            // channel facts never authorize a send; fresh shell facts follow.
            let preview_access = WorkAccess {
                actor,
                guild: match scope {
                    WorkScope::Team { guild, .. } => Some(guild),
                    _ => None,
                },
                channel: origin,
                can_view: true,
                can_manage: false,
            };
            if !matches!(
                Self::lock(&self.stores)
                    .work
                    .next_batch(&scope, preview_access, now()),
                Ok(Some(_))
            ) {
                continue;
            }
            let Ok((access, destination)) = bounded(
                &cancel,
                transport.authorize(&scope, actor, origin, &target, &audience),
            )
            .await
            else {
                continue;
            };
            let batch = Self::lock(&self.stores)
                .work
                .next_batch(&scope, access, now());
            let Ok(Some(batch)) = batch else { continue };
            if batch.target != target || !resolved_destination(&batch, destination) {
                continue;
            }
            let reserved = self
                .commit_work_owned(|store| {
                    if cancel.is_cancelled() || store.delivery_audience(&scope) != audience {
                        return Err(WorkError::Denied);
                    }
                    if matches!(batch.target, WorkDestination::TeamPrivate { .. }) {
                        store.reserve_private_batch(access, &batch, destination, now())
                    } else {
                        store.reserve_batch(access, &batch, now())
                    }
                })
                .await;
            let (id, _) = match reserved {
                Ok(receipt) => receipt,
                Err(WorkError::Persistence) => return Err(WorkError::Persistence),
                Err(_) => continue,
            };
            // Permissions may change while the disk write is pending. Recheck
            // remotely and then compare current local policy and source revisions.
            let fresh = bounded(
                &cancel,
                transport.authorize(&scope, actor, origin, &target, &audience),
            )
            .await;
            let valid = match fresh {
                Ok((fresh_access, fresh_destination)) if fresh_destination == destination => {
                    let stores = Self::lock(&self.stores);
                    stores.work.delivery_audience(&scope) == audience
                        && stores
                            .work
                            .validate_reserved_batch(fresh_access, &batch, now())
                            .is_ok()
                }
                _ => false,
            };
            let message = if valid {
                bounded(&cancel, transport.send(destination, &batch.rendered_body))
                    .await
                    .ok()
            } else {
                None
            };
            // A timeout/disconnect/cancellation might follow server acceptance.
            // Keep coverage consumed and record ReviewRequired, never retry.
            self.commit_work_owned(|store| store.settle_delivery(id, message))
                .await?;
        }
        Ok(())
    }
}

fn resolved_destination(batch: &WorkBatch, destination: u64) -> bool {
    destination != 0
        && match batch.target {
            WorkDestination::TeamPrivate { .. } => destination != batch.destination,
            _ => destination == batch.destination,
        }
}

#[cfg(test)]
mod tests;
