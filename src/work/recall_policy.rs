//! Durable scope opt-in and bounded operator rollout. Neither grants access
//! to native sources. No history is indexed by a policy transition.
use super::*;

const MAX_POLICIES: usize = 10_000;
const MAX_POLICY_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROLLOUT_SCOPES: usize = 1_000;
const MAX_ROLLOUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecallRollout(BTreeSet<WorkScope>);

impl RecallRollout {
    pub fn parse(value: Option<&str>) -> Result<Self, WorkError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        if value.len() > MAX_ROLLOUT_BYTES {
            return Err(WorkError::Full);
        }
        let scopes: Vec<WorkScope> = serde_json::from_str(value).map_err(|_| WorkError::Invalid)?;
        if scopes.len() > MAX_ROLLOUT_SCOPES {
            return Err(WorkError::Full);
        }
        let mut allowed = BTreeSet::new();
        for scope in scopes {
            if !valid_scope(&scope) || !allowed.insert(scope) {
                return Err(WorkError::Invalid);
            }
        }
        Ok(Self(allowed))
    }

    pub fn contains(&self, scope: &WorkScope) -> bool {
        self.0.contains(scope)
    }
}

fn valid_scope(scope: &WorkScope) -> bool {
    match scope {
        WorkScope::Personal { owner } => *owner != 0,
        WorkScope::Team { guild, channel } => *guild != 0 && *channel != 0,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallPolicy {
    pub enabled: bool,
    pub configured_by: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecallPolicies(
    #[serde(with = "super::recall::entries")] BTreeMap<WorkScope, RecallPolicy>,
);

impl RecallPolicies {
    pub(super) fn validate(&self) -> Result<(), WorkError> {
        if self.0.len() > MAX_POLICIES {
            return Err(WorkError::Full);
        }
        if self.0.iter().any(|(scope, policy)| {
            !valid_scope(scope)
                || policy.configured_by == 0
                || policy.revision == 0
                || matches!(scope, WorkScope::Personal { owner } if *owner != policy.configured_by)
        }) {
            return Err(WorkError::Invalid);
        }
        // Reserve all future in-place principal/revision widths, including the
        // larger false encoding. Disabling cannot fail merely due to growth.
        let reserved = Self(
            self.0
                .keys()
                .map(|scope| {
                    (
                        scope.clone(),
                        RecallPolicy {
                            enabled: false,
                            configured_by: u64::MAX,
                            revision: u64::MAX,
                        },
                    )
                })
                .collect(),
        );
        if serde_json::to_vec(&reserved)
            .map_err(|_| WorkError::Invalid)?
            .len()
            > MAX_POLICY_BYTES
        {
            return Err(WorkError::Full);
        }
        Ok(())
    }
}

impl WorkStore {
    /// Scope comes only from authenticated current channel/actor facts. A
    /// scope-wide policy needs every project's manager, including new projects.
    pub fn configure_recall(
        &mut self,
        access: WorkAccess,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<RecallPolicy, WorkError> {
        let scope = access.scope();
        self.scope_projects(&scope, access, true)?;
        let old = self
            .recall_policies
            .0
            .get(&scope)
            .cloned()
            .unwrap_or_default();
        if old.revision != expected_revision {
            return Err(WorkError::Stale);
        }
        if old.revision != 0 && old.enabled == enabled && old.configured_by == access.actor {
            return Ok(old);
        }
        let policy = RecallPolicy {
            enabled,
            configured_by: access.actor,
            revision: old.revision.checked_add(1).ok_or(WorkError::Full)?,
        };
        let mut candidate = self.recall_policies.clone();
        candidate.0.insert(scope, policy.clone());
        candidate.validate()?;
        self.recall_policies = candidate;
        Ok(policy)
    }

    /// Authorized metadata only; never returns source payloads or receipts.
    pub fn recall_policy(&self, access: WorkAccess) -> Result<RecallPolicy, WorkError> {
        let scope = access.scope();
        self.scope_projects(&scope, access, false)?;
        Ok(self
            .recall_policies
            .0
            .get(&scope)
            .cloned()
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests;
