//! Operator rollout is read once at construction. Commands cannot supply an
//! enabled-scope set; native opt-in uses the retained no-proposal commit path.
use super::{AppState, StartupError};
use crate::work::{
    WorkAccess, WorkError,
    recall_policy::{RecallPolicy, RecallRollout},
};

pub(super) fn rollout_from_env() -> Result<RecallRollout, StartupError> {
    let value = std::env::var("ABBEY_WORK_RECALL_SCOPES");
    let value = match value {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(StartupError(
                "ABBEY_WORK_RECALL_SCOPES must be UTF-8 JSON".into(),
            ));
        }
    };
    RecallRollout::parse(value.as_deref()).map_err(|_| StartupError(
        "ABBEY_WORK_RECALL_SCOPES must be a unique array of positive typed scopes (at most 1000 scopes and 64 KiB)".into()
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub struct RecallPolicyStatus {
    pub policy: RecallPolicy,
    pub operator_available: bool,
}

impl AppState {
    pub async fn configure_work_recall(
        &self,
        access: WorkAccess,
        enabled: bool,
        revision: u64,
    ) -> Result<RecallPolicyStatus, WorkError> {
        let operator_available = self.work_recall_rollout.contains(&access.scope());
        let policy = self
            .commit_work(move |store| {
                // Authorize inside serialization even if operator availability is
                // off; the rollout never substitutes for manager membership.
                store.scope_projects(&access.scope(), access, true)?;
                if enabled && !operator_available {
                    return Err(WorkError::Denied);
                }
                store.configure_recall(access, enabled, revision)
            })
            .await?;
        Ok(RecallPolicyStatus {
            policy,
            operator_available,
        })
    }

    pub fn work_recall_policy(&self, access: WorkAccess) -> Result<RecallPolicyStatus, WorkError> {
        let policy = Self::lock(&self.stores).work.recall_policy(access)?;
        Ok(RecallPolicyStatus {
            policy,
            operator_available: self.work_recall_rollout.contains(&access.scope()),
        })
    }
}

#[cfg(test)]
mod tests;
