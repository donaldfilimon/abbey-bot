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
                    state
                        .commit_work_owned(change)
                        .await
                        .map(|(value, _)| value)
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        result.await.map_err(|_| WorkError::Persistence)?
    }
    /// Only an existing retained mutation owner may call this seam. It never
    /// requests a second service admission, so settlement survives draining.
    pub(super) async fn commit_work_owned<R>(
        &self,
        change: impl FnOnce(&mut crate::work::WorkStore) -> Result<R, crate::work::WorkError>,
    ) -> Result<(R, crate::persist::PersistReport), crate::work::WorkError> {
        use crate::{persist::PersistComponentOutcome, work::WorkError};
        let _serial = self.persistence_preparation.lock().await;
        let mut snapshot = self.snapshot_without_proposals();
        let value = change(&mut snapshot.stores.work)?;
        let work = snapshot.stores.work.clone();
        let report = self
            .persistence_requests
            .get()
            .ok_or(WorkError::Persistence)?
            .submit(snapshot)
            .await
            .map_err(|_| WorkError::Persistence)?;
        if report.canonical_state != PersistComponentOutcome::Committed {
            return Err(WorkError::Persistence);
        }
        // Never publish an old whole Stores/Recall: generic memory can change
        // while the writer is busy. Only this work authority is serialized.
        let mut stores = Self::lock(&self.stores);
        stores.work = work;
        let mut recall = Self::lock(&self.recall);
        let _projection_available = recall.reconcile_work_evidence(&stores.work.recall).is_ok();
        *Self::lock(&self.work_recall_disk) = (report.wdbx_projection
            == PersistComponentOutcome::Committed)
            .then_some(stores.work.recall.projection_revision);
        Ok((value, report))
    }
    pub(super) fn observe_work_projection(
        &self,
        revision: u64,
        report: crate::persist::PersistReport,
    ) {
        let stores = Self::lock(&self.stores);
        if report.canonical_state == crate::persist::PersistComponentOutcome::Committed {
            *Self::lock(&self.work_recall_disk) = (report.wdbx_projection
                == crate::persist::PersistComponentOutcome::Committed
                && revision == stores.work.recall.projection_revision)
                .then_some(revision);
        }
    }
    /// Compare the loaded disk image before any in-memory repair. An unreadable
    /// WDBX wire file remains the existing fail-closed startup error; malformed
    /// work envelopes inside a readable file are repairable projection debt.
    pub(super) fn restore_work_projection(
        stores: &crate::persist::Stores,
        recall: &mut crate::wdbx::Recall,
    ) -> Option<u64> {
        let disk = recall
            .work_projection_current(&stores.work.recall)
            .unwrap_or(false)
            .then_some(stores.work.recall.projection_revision);
        let _available = recall.reconcile_work_evidence(&stores.work.recall).is_ok();
        disk
    }
}
