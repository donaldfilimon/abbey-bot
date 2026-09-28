//! Fail-closed schema, capacity, identity and receipt validation.
use super::*;

impl WorkRecallState {
    pub(crate) fn validate(&self) -> Result<(), WorkError> {
        if self.schema_version != 1 {
            return Err(WorkError::Invalid);
        }
        if self.records.len() > MAX_ROWS
            || self.source_versions.len() > MAX_SOURCES
            || self.scope_controls.len() > MAX_SCOPES
            || self.attempts.len() > MAX_ATTEMPTS
            || self.terminal.len() > MAX_TERMINALS
        {
            return Err(WorkError::Full);
        }
        let mut payload_total = 0usize;
        for (id, row) in &self.records {
            row.validate()?;
            if *id != row.id
                || *id > self.sequence
                || !self.source_versions.contains_key(&row.payload.source)
                || !self.scope_controls.contains_key(&row.payload.scope)
            {
                return Err(WorkError::Invalid);
            }
            payload_total = payload_total
                .checked_add(row.payload.encoded()?.len())
                .ok_or(WorkError::Full)?;
        }
        for (key, value) in &self.source_versions {
            if !key.valid() {
                return Err(WorkError::Invalid);
            }
            if bytes(key)?.len() > 256
                || bytes(&serde_json::json!({"key": key, "value": value}))?.len() > 512
            {
                return Err(WorkError::Full);
            }
        }
        for (scope, value) in &self.scope_controls {
            if !scope_valid(scope) {
                return Err(WorkError::Invalid);
            }
            if bytes(&serde_json::json!({"key": scope, "value": value}))?.len() > 128 {
                return Err(WorkError::Full);
            }
        }
        let mut sources = BTreeSet::new();
        let mut reserved_rows = BTreeSet::new();
        for (id, attempt) in &self.attempts {
            if *id == 0
                || *id != attempt.id
                || self.records.contains_key(id)
                || *id > self.sequence
                || !attempt.source.valid()
                || !valid_digest(&attempt.payload_digest)
                || !valid_digest(&attempt.config_digest)
                || attempt.payload_bytes == 0
                || attempt.payload_bytes > MAX_PAYLOAD_BYTES
                || !self.source_versions.contains_key(&attempt.source)
                || !sources.insert(&attempt.source)
                || bytes(attempt)?.len() > 1024
            {
                return Err(WorkError::Invalid);
            }
            if let Some(admission) = &attempt.observed_admission {
                if attempt.state != AttemptState::Unknown {
                    return Err(WorkError::Invalid);
                }
                if let WorkAdmission::Appended { digest_hex, .. } = admission
                    && !valid_digest(digest_hex)
                {
                    return Err(WorkError::Invalid);
                }
            }
            match attempt.operation {
                ProjectionOperation::Add { row } => {
                    if row == 0
                        || row > self.sequence
                        || row == *id
                        || self.records.contains_key(&row)
                        || self.attempts.contains_key(&row)
                        || !reserved_rows.insert(row)
                    {
                        return Err(WorkError::Invalid);
                    }
                    payload_total = payload_total
                        .checked_add(attempt.payload_bytes)
                        .ok_or(WorkError::Full)?;
                }
                ProjectionOperation::Forget { row } => {
                    let record = self.records.get(&row).ok_or(WorkError::Invalid)?;
                    if record.payload.source != attempt.source
                        || record.payload_digest != attempt.payload_digest
                        || record.payload.revision != attempt.revision
                        || record.payload.generation != attempt.generation
                        || record.payload.encoded()?.len() != attempt.payload_bytes
                    {
                        return Err(WorkError::Invalid);
                    }
                }
            }
        }
        if self.records.len() + reserved_rows.len() > MAX_ROWS || payload_total > MAX_TOTAL_PAYLOAD
        {
            return Err(WorkError::Full);
        }
        let mut terminal_ids = BTreeSet::new();
        for item in &self.terminal {
            if item.attempt == 0
                || item.attempt > self.sequence
                || !terminal_ids.insert(item.attempt)
                || self.attempts.contains_key(&item.attempt)
                || self.records.contains_key(&item.attempt)
                || reserved_rows.contains(&item.attempt)
                || bytes(item)?.len() > 256
            {
                return Err(WorkError::Invalid);
            }
        }
        // Fixed worst-case reservations make all native controls and terminal
        // transitions safe at capacity, including decimal counter growth.
        let metadata = self.source_versions.len() * 512
            + self.scope_controls.len() * 128
            + self.attempts.len() * 1024
            + MAX_TERMINALS * 256
            + (self.records.len() + reserved_rows.len()) * 512;
        if metadata > MAX_METADATA {
            return Err(WorkError::Full);
        }
        Ok(())
    }
    pub(in crate::work) fn recover_prepared(&mut self) {
        for attempt in self.attempts.values_mut() {
            attempt.state = AttemptState::Unknown;
        }
    }
}

impl WorkStore {
    pub(in crate::work) fn validate_recall_joins(&self) -> Result<(), WorkError> {
        for key in self.recall.source_versions.keys() {
            let scope = self.recall_source_scope(key)?;
            if !self.recall.scope_controls.contains_key(&scope) {
                return Err(WorkError::Invalid);
            }
        }
        for row in self.recall.records.values() {
            let p = &row.payload;
            if self.recall_source_scope(&p.source)? != p.scope
                || p.contributing_projects
                    .iter()
                    .any(|id| self.projects.get(id).is_none_or(|v| v.scope != p.scope))
            {
                return Err(WorkError::Invalid);
            }
            match &p.source {
                WorkSourceKey::Task { id, .. } => {
                    let task = &self.tasks[id];
                    if p.revision > task.revision || p.actor != Some(task.owner) {
                        return Err(WorkError::Invalid);
                    }
                }
                WorkSourceKey::Decision { id, .. } => {
                    let d = &self.decisions[id];
                    if p.revision != 1 || p.actor != Some(d.author) || p.at != Some(d.at) {
                        return Err(WorkError::Invalid);
                    }
                }
                WorkSourceKey::Preference {
                    scope,
                    delivery,
                    actor,
                } => {
                    let receipt = &self.deliveries[delivery];
                    let provenance = receipt.provenance.as_ref().ok_or(WorkError::Invalid)?;
                    provenance.validate()?;
                    if receipt.state != DeliveryState::Sent
                        || receipt.message_id.is_none()
                        || receipt.kind.is_none()
                        || p.at.is_none_or(|at| at < receipt.at)
                        || provenance.contributing_projects != p.contributing_projects
                        || p.delivered_source_digest != Some(digest(&bytes(provenance)?))
                        || self.preferences.get(&scope.key()).is_none_or(|profile| {
                            !profile.observed_deliveries.contains(&(*delivery, *actor))
                                && !profile.evidence.iter().any(|e| {
                                    e.actor == Some(*actor)
                                        && e.delivery_id == *delivery
                                        && e.scope.as_ref() == Some(scope)
                                })
                        })
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
                }
            }
        }
        for attempt in self.recall.attempts.values() {
            let scope = self
                .recall_source_scope(&attempt.source)?
                .recall_gate_scope()
                .0;
            if let Some(
                WorkAdmission::Appended { scoped_guild, .. }
                | WorkAdmission::Uncovered { scoped_guild },
            ) = &attempt.observed_admission
                && *scoped_guild != scope
            {
                return Err(WorkError::Invalid);
            }
        }
        Ok(())
    }
    pub(crate) fn recall_source_scope(&self, key: &WorkSourceKey) -> Result<WorkScope, WorkError> {
        match key {
            WorkSourceKey::Task { project, id } => {
                if self.tasks.get(id).is_none_or(|t| t.project_id != *project) {
                    return Err(WorkError::Invalid);
                }
                self.projects
                    .get(project)
                    .map(|p| p.scope.clone())
                    .ok_or(WorkError::Invalid)
            }
            WorkSourceKey::Decision { project, id } => {
                if self
                    .decisions
                    .get(id)
                    .is_none_or(|t| t.project_id != *project)
                {
                    return Err(WorkError::Invalid);
                }
                self.projects
                    .get(project)
                    .map(|p| p.scope.clone())
                    .ok_or(WorkError::Invalid)
            }
            WorkSourceKey::Preference {
                scope, delivery, ..
            } => {
                if self
                    .deliveries
                    .get(delivery)
                    .is_none_or(|r| r.scope.as_ref() != Some(scope))
                {
                    return Err(WorkError::Invalid);
                }
                Ok(scope.clone())
            }
        }
    }
}
