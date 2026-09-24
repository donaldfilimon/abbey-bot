//! Retained, explicitly requested work admission. No scheduler, startup or
//! shutdown path calls admission. Native controls use the ordinary work commit.
use super::AppState;
#[cfg(test)]
use crate::{
    work::{WorkAccess, WorkError, recall::*},
    work_recall_gate::{self as gate, Gate, Outcome},
};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tokio_util::sync::CancellationToken;

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionResult {
    Committed {
        row: Option<u64>,
        disk_cleanup_required: bool,
    },
    Rejected,
    Unknown,
}
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub struct RecallStatus {
    pub retained_rows: usize,
    pub unresolved: usize,
    pub disk_cleanup_required: bool,
}
#[cfg(test)]
#[derive(Debug)]
pub struct RecallResult {
    pub revision: u64,
    pub evidence: Vec<WorkEvidencePayload>,
}

impl AppState {
    /// Called only by a fresh authorized explicit reindex/native action. The
    /// caller supplies an enabled-scope set, default empty; it grants no access.
    #[cfg(test)]
    pub async fn admit_work_recall(
        &self,
        source: WorkSourceKey,
        access: WorkAccess,
        at: u64,
        enabled: &std::collections::BTreeSet<crate::work::WorkScope>,
    ) -> Result<AdmissionResult, WorkError> {
        let payload = Self::lock(&self.stores)
            .work
            .recall_candidate(&source, access)?;
        if !enabled.contains(&payload.scope) {
            return Err(WorkError::Denied);
        }
        let gate = gate::selected(self, &payload);
        self.start_work_recall(source, access, at, gate, None).await
    }

    #[cfg(test)]
    async fn start_work_recall(
        &self,
        source: WorkSourceKey,
        access: WorkAccess,
        at: u64,
        gate: Option<Arc<dyn Gate>>,
        forget: Option<u64>,
    ) -> Result<AdmissionResult, WorkError> {
        if self.data_dir.is_none() {
            return Err(WorkError::Persistence);
        }
        let state = self.owned_state().ok_or(WorkError::Persistence)?;
        let registry = self.service.get().ok_or(WorkError::Persistence)?;
        let (send, receive) = tokio::sync::oneshot::channel();
        registry
            .spawn_operation(
                crate::service::OperationKind::WorkRecall,
                move |cancel| async move {
                    let result = state
                        .work_recall_owned(source, access, at, gate, forget, cancel)
                        .await;
                    state.observe_work_recall(&result);
                    let _ = send.send(result);
                    crate::service::TaskExit::Returned
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        receive.await.map_err(|_| WorkError::Persistence)?
    }

    #[cfg(test)]
    async fn work_recall_owned(
        &self,
        source: WorkSourceKey,
        access: WorkAccess,
        at: u64,
        gate: Option<Arc<dyn Gate>>,
        forget: Option<u64>,
        cancel: CancellationToken,
    ) -> Result<AdmissionResult, WorkError> {
        if cancel.is_cancelled() {
            return Err(WorkError::Persistence);
        }
        let (nonce, fingerprint) = gate::identity(gate.as_ref());
        let ((attempt, payload, previous), _) = self
            .commit_work_owned(move |store| {
                if let Some(row) = forget {
                    let previous = store
                        .recall
                        .records
                        .get(&row)
                        .ok_or(WorkError::Missing)?
                        .clone();
                    if previous.payload.source != source {
                        return Err(WorkError::Invalid);
                    }
                    let attempt =
                        store.prepare_recall_forget(row, access, at, nonce, fingerprint)?;
                    Ok((attempt, previous.payload.clone(), Some(previous)))
                } else {
                    // Superseding targets exactly one same-source old receipt. An
                    // unchanged candidate does not charge another episode.
                    let candidate = store.recall_candidate(&source, access)?;
                    if store
                        .recall
                        .records
                        .values()
                        .any(|r| r.payload == candidate)
                    {
                        return Err(WorkError::Stale);
                    }
                    let previous = store
                        .recall
                        .records
                        .values()
                        .rev()
                        .find(|r| r.payload.source == source)
                        .cloned();
                    let (attempt, payload) =
                        store.prepare_recall(&source, access, at, nonce, fingerprint)?;
                    Ok((attempt, payload, previous))
                }
            })
            .await?;
        // Serializer released: reset, disable and native edits can commit while
        // this owner waits. Cancellation never grants a positive receipt.
        let outcome = crate::service::cancellation::complete_or_cancelled(
            Some(cancel),
            gate::admit(
                gate,
                &payload,
                previous.as_ref(),
                forget.is_some(),
                at,
                nonce,
            ),
        )
        .await
        .unwrap_or(Ok(Outcome::Unknown))
        .unwrap_or(Outcome::Unknown);
        let observed_admission = match &outcome {
            Outcome::Admitted(admission) => Some(admission.clone()),
            _ => None,
        };
        let result = self
            .commit_work_owned(move |store| {
                match outcome {
                    Outcome::Admitted(admission) => {
                        if forget.is_some() {
                            store.recall.settle_forget(attempt, admission, at)?;
                            Ok((None, false, false))
                        } else {
                            let row = store.recall.settle_add(attempt, payload, admission, at)?;
                            if let Some(previous) = previous {
                                // Exact supersedes or existing uncovered policy only;
                                // never remove other retained versions of the source.
                                store.recall.records.remove(&previous.id);
                            }
                            Ok((Some(row), false, false))
                        }
                    }
                    Outcome::Rejected => {
                        store.recall.reject(attempt, at)?;
                        Ok((None, true, false))
                    }
                    Outcome::Unknown => {
                        store
                            .recall
                            .attempts
                            .get_mut(&attempt)
                            .ok_or(WorkError::Missing)?
                            .state = AttemptState::Unknown;
                        Ok((None, false, true))
                    }
                }
            })
            .await;
        match result {
            Ok(((row, false, false), report)) => Ok(AdmissionResult::Committed {
                row,
                disk_cleanup_required: report.wdbx_projection
                    != crate::persist::PersistComponentOutcome::Committed,
            }),
            Ok(((_, true, _), _)) => Ok(AdmissionResult::Rejected),
            Ok(((_, _, true), _)) => Ok(AdmissionResult::Unknown),
            Err(_) => {
                // If this write also fails, durable Prepared remains the restart
                // Unknown marker. Never retry the external operation.
                let _marked = self
                    .commit_work_owned(|s| {
                        let pending = s
                            .recall
                            .attempts
                            .get_mut(&attempt)
                            .ok_or(WorkError::Missing)?;
                        pending.state = AttemptState::Unknown;
                        pending.observed_admission = observed_admission;
                        Ok(())
                    })
                    .await;
                Ok(AdmissionResult::Unknown)
            }
        }
    }

    #[cfg(test)]
    fn observe_work_recall(&self, result: &Result<AdmissionResult, WorkError>) {
        use crate::observability::{EventCode, EventComponent, EventOutcome};
        let (code, outcome) = match result {
            Ok(AdmissionResult::Committed {
                disk_cleanup_required: false,
                ..
            }) => (EventCode::WorkRecallAdmission, EventOutcome::Succeeded),
            Ok(AdmissionResult::Committed { .. }) => {
                (EventCode::WorkRecallAdmission, EventOutcome::Degraded)
            }
            Ok(AdmissionResult::Unknown) => (EventCode::WorkRecallUnknown, EventOutcome::Degraded),
            _ => (EventCode::WorkRecallAdmission, EventOutcome::Failed),
        };
        if let Some(events) = self.operational_events() {
            let _ = events.record(EventComponent::WorkRecall, code, outcome, None);
        }
    }

    /// Disable is durable before any gate traffic; zero-row requests still
    /// repair disk debt without proposing. Each human invocation is bounded.
    #[cfg(test)]
    pub async fn forget_work_recall(
        &self,
        source: WorkSourceKey,
        access: WorkAccess,
        at: u64,
    ) -> Result<RecallStatus, WorkError> {
        let state = self.owned_state().ok_or(WorkError::Persistence)?;
        let (send, receive) = tokio::sync::oneshot::channel();
        self.service
            .get()
            .ok_or(WorkError::Persistence)?
            .spawn_operation(
                crate::service::OperationKind::WorkRecall,
                move |cancel| async move {
                    let result = state.forget_work_owned(source, access, at, cancel).await;
                    let _ = send.send(result);
                    crate::service::TaskExit::Returned
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        receive.await.map_err(|_| WorkError::Persistence)?
    }

    #[cfg(test)]
    async fn forget_work_owned(
        &self,
        source: WorkSourceKey,
        access: WorkAccess,
        at: u64,
        cancel: CancellationToken,
    ) -> Result<RecallStatus, WorkError> {
        self.commit_work_owned(|s| s.disable_recall_source(&source, access))
            .await?;
        let rows: Vec<_> = Self::lock(&self.stores)
            .work
            .recall
            .records
            .values()
            .filter(|r| r.payload.source == source)
            .take(16)
            .cloned()
            .collect();
        for row in rows {
            if cancel.is_cancelled() {
                break;
            }
            let selected = gate::selected(self, &row.payload);
            match self
                .work_recall_owned(
                    source.clone(),
                    access,
                    at,
                    selected,
                    Some(row.id),
                    cancel.clone(),
                )
                .await
            {
                Ok(AdmissionResult::Committed { .. }) => (),
                _ => break,
            }
        }
        self.commit_work_owned(|_| Ok(())).await?;
        self.work_recall_status(&source, access)
    }

    #[cfg(test)]
    pub fn work_recall_status(
        &self,
        source: &WorkSourceKey,
        access: WorkAccess,
    ) -> Result<RecallStatus, WorkError> {
        let stores = Self::lock(&self.stores);
        let scope = stores.work.recall_source_scope(source)?;
        stores.work.scope_projects(&scope, access, true)?;
        Ok(RecallStatus {
            retained_rows: stores
                .work
                .recall
                .records
                .values()
                .filter(|r| &r.payload.source == source)
                .count(),
            unresolved: stores
                .work
                .recall
                .attempts
                .values()
                .filter(|a| &a.source == source)
                .count(),
            disk_cleanup_required: *Self::lock(&self.work_recall_disk)
                != Some(stores.work.recall.projection_revision),
        })
    }

    /// Fresh shell access in original team scope, even on a private transport.
    /// Result revision must be rechecked by the later send barrier consumer.
    #[cfg(test)]
    pub fn recall_project(
        &self,
        access: WorkAccess,
        project: u64,
        principal: u64,
        query: &str,
    ) -> Result<RecallResult, WorkError> {
        let stores = Self::lock(&self.stores);
        let allowed = stores.work.eligible_recall_ids(
            access,
            project,
            RecallAudience::Private { principal },
        )?;
        let recall = Self::lock(&self.recall);
        let hits = recall
            .search_work_evidence(&stores.work.recall, &allowed, query, 8)
            .map_err(|_| WorkError::Invalid)?;
        let mut evidence = Vec::new();
        let mut remaining = 4000;
        for hit in hits {
            let payload = &stores.work.recall.records[&hit.evidence_id].payload;
            let count = payload.text.chars().count();
            if count > remaining {
                continue;
            }
            remaining -= count;
            evidence.push(payload.clone());
        }
        Ok(RecallResult {
            revision: stores.work.recall.projection_revision,
            evidence,
        })
    }
}
#[cfg(test)]
mod tests;
