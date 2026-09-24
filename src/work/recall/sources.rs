//! Staged deterministic candidate and pre-scoring eligibility seams. All native
//! joins are checked again; a vector or historical row never grants authority.
use super::*;

impl WorkStore {
    #[cfg(test)]
    pub fn recall_candidate(
        &self,
        key: &WorkSourceKey,
        access: WorkAccess,
    ) -> Result<WorkEvidencePayload, WorkError> {
        if self.recall.revision_exhausted {
            return Err(WorkError::Full);
        }
        let scope = self.recall_source_scope(key)?;
        let control = self
            .recall
            .scope_controls
            .get(&scope)
            .cloned()
            .unwrap_or_default();
        if control.recall_disabled_exhausted {
            return Err(WorkError::Full);
        }
        let version = self.recall.source_versions.get(key);
        if version.is_some_and(|v| !v.recall_enabled || v.recall_disabled_exhausted) {
            return Err(WorkError::Denied);
        }
        let (revision, generation, projects, delivered, actor, at, native, text) = match key {
            WorkSourceKey::Task { project, id } => {
                self.project(*project, access)?;
                let task = &self.tasks[id];
                (
                    task.revision,
                    0,
                    BTreeSet::from([*project]),
                    None,
                    Some(task.owner),
                    None,
                    bytes(task)?,
                    format!(
                        "Advisory task #{id}: {}. Status: {}; priority: {}; due: {}.",
                        task.title,
                        task.status.label(),
                        task.priority,
                        task.due_at.map_or("none".into(), |t| t.to_string())
                    ),
                )
            }
            WorkSourceKey::Decision { project, id } => {
                self.project(*project, access)?;
                let d = &self.decisions[id];
                (
                    1,
                    0,
                    BTreeSet::from([*project]),
                    None,
                    Some(d.author),
                    Some(d.at),
                    bytes(d)?,
                    format!("Decision #{id}: {}", d.text),
                )
            }
            WorkSourceKey::Preference {
                delivery, actor, ..
            } => {
                self.scope_projects(&scope, access, false)?;
                let profile = self
                    .preferences
                    .get(&scope.key())
                    .ok_or(WorkError::Missing)?;
                if !profile.learning_enabled {
                    return Err(WorkError::Denied);
                }
                let observation = profile
                    .evidence
                    .iter()
                    .find(|e| {
                        e.delivery_id == *delivery
                            && e.actor == Some(*actor)
                            && e.scope.as_ref() == Some(&scope)
                    })
                    .ok_or(WorkError::Missing)?;
                let receipt = &self.deliveries[delivery];
                if receipt.state != DeliveryState::Sent
                    || receipt.message_id.is_none()
                    || observation.kind.is_none()
                    || receipt.kind != observation.kind
                    || observation.at < receipt.at
                {
                    return Err(WorkError::Invalid);
                }
                if let Some(
                    WorkDestination::Personal { principal }
                    | WorkDestination::TeamPrivate { principal },
                ) = receipt.destination
                    && principal != *actor
                {
                    return Err(WorkError::Invalid);
                }
                let provenance = receipt.provenance.as_ref().ok_or(WorkError::Missing)?;
                provenance.validate()?;
                // Also catches in-memory corrupt provenance, not only reloaded JSON.
                self.validate_delivery_state()?;
                for project in &provenance.contributing_projects {
                    self.project(*project, access)?;
                }
                let feedback = match observation.feedback {
                    WorkFeedback::Useful => "marked useful".to_string(),
                    WorkFeedback::Dismissed => "dismissed".to_string(),
                    WorkFeedback::Snoozed { hour: 0..=23 } => {
                        if let WorkFeedback::Snoozed { hour } = observation.feedback {
                            format!("snoozed to hour {hour}")
                        } else {
                            unreachable!()
                        }
                    }
                    WorkFeedback::Snoozed { .. } => return Err(WorkError::Invalid),
                };
                (
                    version.map_or(1, |v| v.revision),
                    control.generation,
                    provenance.contributing_projects.clone(),
                    Some(digest(&bytes(provenance)?)),
                    Some(*actor),
                    Some(observation.at),
                    bytes(&(observation, provenance))?,
                    format!(
                        "Feedback on the whole delivery #{delivery} for projects {:?}: {feedback}.",
                        provenance.contributing_projects
                    ),
                )
            }
        };
        if version.is_some_and(|v| v.revision != revision) {
            return Err(WorkError::Stale);
        }
        let mut text = text;
        if text.chars().count() > 1024 {
            text = text.chars().take(1010).collect::<String>() + "… [shortened]";
        }
        let payload = WorkEvidencePayload {
            version: 1,
            source: key.clone(),
            revision,
            generation,
            scope,
            contributing_projects: projects,
            delivered_source_digest: delivered,
            actor,
            at,
            native_digest: digest(&native),
            text,
        };
        payload.encoded()?;
        Ok(payload)
    }
    /// Returns only IDs. Retired content/receipts never cross the query boundary.
    /// The caller must use this allowlist BEFORE any vector scoring.
    #[cfg(test)]
    pub fn eligible_recall_ids(
        &self,
        access: WorkAccess,
        project: u64,
        audience: RecallAudience,
    ) -> Result<BTreeSet<u64>, WorkError> {
        self.project(project, access)?;
        if !matches!(audience,RecallAudience::Private { principal } if principal == access.actor) {
            return Err(WorkError::Denied);
        }
        self.recall.validate()?;
        self.validate_recall_joins()?;
        let mut allowed = BTreeSet::new();
        for (id, row) in &self.recall.records {
            if !row.payload.contributing_projects.contains(&project) {
                continue;
            }
            if row
                .payload
                .contributing_projects
                .iter()
                .any(|p| self.project(*p, access).is_err())
            {
                continue;
            }
            if self
                .recall_candidate(&row.payload.source, access)
                .is_ok_and(|p| p == row.payload)
            {
                allowed.insert(*id);
            }
        }
        Ok(allowed)
    }
}
