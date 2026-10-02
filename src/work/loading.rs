//! Backward-compatible work snapshot loading and delivery-policy validation.
use super::*;

#[derive(Default, Deserialize)]
#[serde(remote = "WorkStore", default)]
struct LoadedWorkStore {
    engagement: crate::engagement::EngagementStore,
    recall: recall::WorkRecallState,
    recall_policies: recall_policy::RecallPolicies,
    sequence: u64,
    projects: BTreeMap<u64, WorkProject>,
    goals: BTreeMap<u64, WorkGoal>,
    tasks: BTreeMap<u64, WorkTask>,
    decisions: BTreeMap<u64, WorkDecision>,
    automation: BTreeMap<u64, WorkAutomationPolicy>,
    scope_automation: BTreeMap<String, WorkAutomationPolicy>,
    scope_automation_actors: BTreeMap<String, u64>,
    change_coverage: BTreeMap<String, BTreeSet<WorkContentRef>>,
    change_fingerprints: BTreeMap<String, BTreeMap<u64, String>>,
    preferences: BTreeMap<String, WorkPreferenceProfile>,
    deliveries: BTreeMap<u64, WorkDeliveryReceipt>,
    request_ids: BTreeMap<String, u64>,
    actions: crate::action_approval::ActionStore,
    github_snapshots: BTreeMap<String, GitHubSnapshot>,
}

impl<'de> Deserialize<'de> for WorkStore {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut store = LoadedWorkStore::deserialize(deserializer)?;
        store
            .recall_policies
            .validate()
            .map_err(serde::de::Error::custom)?;
        store.recall.validate().map_err(serde::de::Error::custom)?;
        store
            .validate_recall_joins()
            .map_err(serde::de::Error::custom)?;
        store.recall.recover_prepared();
        store
            .validate_delivery_state()
            .map_err(serde::de::Error::custom)?;
        let scopes: Vec<_> = store.projects.values().map(|p| p.scope.clone()).collect();
        for scope in scopes {
            if store
                .scope_automation
                .get(&scope.key())
                .is_some_and(|policy| policy.enabled)
                && !store.change_coverage.contains_key(&scope.key())
            {
                store
                    .change_coverage
                    .insert(scope.key(), store.content_refs(&scope));
            }
            if store
                .scope_automation
                .get(&scope.key())
                .is_some_and(|policy| policy.enabled)
                && !store.change_fingerprints.contains_key(&scope.key())
            {
                store
                    .change_fingerprints
                    .insert(scope.key(), store.content_fingerprints(&scope));
            }
        }
        Ok(store)
    }
}

impl WorkStore {
    pub(super) fn validate_delivery_state(&self) -> Result<(), WorkError> {
        for (key, policy) in &self.scope_automation {
            policy.validate()?;
            let scope = &self
                .projects
                .values()
                .find(|p| &p.scope.key() == key)
                .ok_or(WorkError::Invalid)?
                .scope;
            if let WorkScope::Team { channel, .. } = scope
                && policy.enabled
                && policy.destination != Some(*channel)
            {
                return Err(WorkError::Invalid);
            }
            if let Some(target) = &policy.delivery_target {
                let actor =
                    self.scope_automation_actors
                        .get(key)
                        .copied()
                        .unwrap_or(match target {
                            WorkDestination::Personal { principal }
                            | WorkDestination::TeamPrivate { principal } => *principal,
                            WorkDestination::TeamChannel { .. } => 1,
                        });
                target.validate(scope, actor)?;
                if let WorkScope::Team { channel, .. } = scope
                    && policy.destination != Some(*channel)
                {
                    return Err(WorkError::Invalid);
                }
            }
        }
        for receipt in self.deliveries.values() {
            if let Some(provenance) = &receipt.provenance {
                provenance.validate()?;
                // Project association and native kind are immutable. Delivered
                // revisions may be older than current state, never from its future.
                for source in &provenance.source_refs {
                    let valid = match source {
                        WorkContentRef::Task {
                            project,
                            id,
                            revision,
                        } => self.tasks.get(id).is_some_and(|task| {
                            task.project_id == *project && *revision <= task.revision
                        }),
                        WorkContentRef::Decision {
                            project,
                            id,
                            revision,
                        } => self.decisions.get(id).is_some_and(|decision| {
                            decision.project_id == *project && *revision == 1
                        }),
                    };
                    if !valid {
                        return Err(WorkError::Invalid);
                    }
                }
                let tasks: BTreeSet<_> = provenance
                    .source_refs
                    .iter()
                    .filter_map(|r| match r {
                        WorkContentRef::Task { id, .. } => Some(*id),
                        _ => None,
                    })
                    .collect();
                if tasks != receipt.task_ids.iter().copied().collect()
                    || receipt.coverage.iter().any(|c| !tasks.contains(&c.task_id))
                    || provenance.contributing_projects.iter().any(|id| {
                        self.projects
                            .get(id)
                            .is_none_or(|p| Some(&p.scope) != receipt.scope.as_ref())
                    })
                {
                    return Err(WorkError::Invalid);
                }
            }
            if let Some(target) = &receipt.destination {
                let scope = receipt.scope.as_ref().ok_or(WorkError::Invalid)?;
                let actor = match target {
                    WorkDestination::Personal { principal }
                    | WorkDestination::TeamPrivate { principal } => *principal,
                    WorkDestination::TeamChannel { .. } => 1,
                };
                target.validate(scope, actor)?;
                let wrong_channel = match (target, scope) {
                    (WorkDestination::TeamChannel { channel }, _) => receipt.recipient != *channel,
                    (WorkDestination::TeamPrivate { .. }, WorkScope::Team { channel, .. }) => {
                        receipt.recipient == *channel
                    }
                    _ => false,
                };
                if receipt.recipient == 0 || wrong_channel {
                    return Err(WorkError::Invalid);
                }
            }
        }
        Ok(())
    }
}
