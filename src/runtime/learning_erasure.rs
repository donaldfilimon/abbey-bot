//! Retained learning deletion and atomic canonical publication. The preparation
//! owner serializes Work/engagement/checkpoint writes. No lock spans network IO.
use super::*;
use crate::{
    brain::erasure::{self, LearningEraseReport, LearningEraseState},
    work::WorkError,
};

impl AppState {
    pub async fn erase_personal_learning(
        &self,
        scope: String,
        member: u64,
    ) -> Result<LearningEraseReport, WorkError> {
        self.commit_learning_erasure(scope, Some(member), None)
            .await
    }
    /// Called only after the native shell consumes an actor/scope-bound fresh
    /// manager confirmation. The runtime never manufactures Discord authority.
    pub(crate) async fn reset_learning_scope(
        &self,
        scope: String,
        authorized_at: u64,
    ) -> Result<LearningEraseReport, WorkError> {
        self.commit_learning_erasure(scope, None, Some(authorized_at))
            .await
    }
    async fn commit_learning_erasure(
        &self,
        scope: String,
        member: Option<u64>,
        authorized_at: Option<u64>,
    ) -> Result<LearningEraseReport, WorkError> {
        if self.data_dir.is_none() {
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
                    let admitted = now();
                    if authorized_at.is_some_and(|at| admitted < at || admitted - at > 60) {
                        return Err(WorkError::Stale);
                    }
                    // Publish the live fence before disk IO, so in-flight repair,
                    // settlement and caches cannot recreate a deleted row. On disk
                    // failure report failure; keep the protective in-memory deletion.
                    let continuity = state.erase_continuity_owned(&scope, member).await?;
                    let mut report = state.finish_learning_erasure(
                        &scope,
                        member,
                        authorized_at,
                        admitted,
                        now(),
                    )?;
                    report.continuity = continuity;
                    let snapshot = state.snapshot_without_proposals();
                    let expected =
                        serde_json::to_vec(&snapshot.stores).map_err(|_| WorkError::Persistence)?;
                    let path =
                        Stores::state_path(state.data_dir.as_ref().ok_or(WorkError::Persistence)?);
                    let persisted = state
                        .persistence_requests
                        .get()
                        .ok_or(WorkError::Persistence)?
                        .submit(snapshot)
                        .await
                        .map_err(|_| WorkError::Persistence)?;
                    // Sink success alone is insufficient for a privacy receipt.
                    // Exact owned bytes also reconcile a write-then-error without
                    // restoring erased authority or blessing unrelated disk data.
                    let exact = tokio::task::spawn_blocking(move || {
                        std::fs::read(path).is_ok_and(|actual| actual == expected)
                    })
                    .await
                    .map_err(|_| WorkError::Persistence)?;
                    if exact {
                        let directory = state.data_dir.clone().ok_or(WorkError::Persistence)?;
                        let disk = tokio::task::spawn_blocking(move || Stores::load(&directory))
                            .await
                            .map_err(|_| WorkError::Persistence)?
                            .map_err(|_| WorkError::Persistence)?;
                        Self::lock(&state.stores)
                            .canonical_base
                            .set(disk.canonical_base.get());
                    }
                    if !exact
                        || persisted.canonical_state
                            != crate::persist::PersistComponentOutcome::Committed
                    {
                        return Err(WorkError::Persistence);
                    }
                    Ok(report)
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        result.await.map_err(|_| WorkError::Persistence)?
    }
    fn finish_learning_erasure(
        &self,
        scope: &str,
        member: Option<u64>,
        authorized_at: Option<u64>,
        _started: u64,
        finished: u64,
    ) -> Result<LearningEraseReport, WorkError> {
        if authorized_at.is_some_and(|at| finished < at || finished - at > 60) {
            return Err(WorkError::Stale);
        }
        self.erase_learning_now(scope, member, finished)
    }
    pub(crate) fn erase_learning_now(
        &self,
        scope: &str,
        member: Option<u64>,
        now: u64,
    ) -> Result<LearningEraseReport, WorkError> {
        let _mutation = Self::lock(&self.personal_memory_mutation);
        let mut stores = Self::lock(&self.stores);
        let mut brains = Self::lock(&self.brains);
        let mut social = Self::lock(&self.social);
        let mut rewards = Self::lock(&self.rewards);
        let mut state = LearningEraseState {
            stores: &mut stores,
            rewards: &mut rewards,
            social: &mut social,
        };
        let report = match member {
            Some(m) => erasure::erase_member(&mut state, scope, m, now)?,
            None => erasure::reset_scope(&mut state, scope, true, now)?,
        };
        if member.is_none() {
            brains.reset(scope);
            // Removing a checkpoint does not admit a replacement. Fresh weights
            // must pass the existing cumulative-budget gate before persistence.
            Self::lock(&self.checkpoints).remove(scope);
        }
        Ok(report)
    }
}
#[cfg(test)]
mod tests;
