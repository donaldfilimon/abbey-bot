//! Durable native work transitions, retained through canonical publication.
//! Generic memory and checkpoint proposals belong to the scheduled drain.

use super::AppState;

impl AppState {
    /// Retain preparation through publication even if the caller is cancelled.
    /// Apply a work transition to an owned snapshot. Publish it in memory only
    /// after the canonical JSON write has completed. The preparation mutex also
    /// orders this write against scheduled snapshots, preventing an older
    /// scheduler snapshot from overwriting a just-acknowledged work change.
    pub async fn commit_work<R: Send + 'static>(
        &self,
        change: impl FnOnce(&mut crate::work::WorkStore) -> Result<R, crate::work::WorkError>
        + Send
        + 'static,
    ) -> Result<R, crate::work::WorkError> {
        use crate::persist::PersistComponentOutcome;
        use crate::work::WorkError;

        if self.data_dir.is_none() || self.service.get().is_none() {
            return Err(WorkError::Persistence);
        }
        let state = self.owned_state().ok_or(WorkError::Persistence)?;
        let result = self
            .service
            .get()
            .ok_or(WorkError::Persistence)?
            .spawn_result(
                crate::service::OperationKind::PersistencePreparation,
                async move {
                    let _serial = state.persistence_preparation.lock().await;
                    let crate::service::persistence::Snapshot { mut stores, recall } =
                        state.snapshot_without_proposals();
                    let value = change(&mut stores.work)?;
                    let requests = state
                        .persistence_requests
                        .get()
                        .ok_or(WorkError::Persistence)?;
                    let report = requests
                        .submit(crate::service::persistence::Snapshot {
                            stores: stores.clone(),
                            recall,
                        })
                        .await
                        .map_err(|_| WorkError::Persistence)?;
                    if report.canonical_state != PersistComponentOutcome::Committed {
                        return Err(WorkError::Persistence);
                    }
                    Self::lock(&state.stores).work = stores.work;
                    Ok(value)
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        result.await.map_err(|_| WorkError::Persistence)?
    }
}
