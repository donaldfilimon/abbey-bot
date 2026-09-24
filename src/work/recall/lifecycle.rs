//! In-place native invalidation plus staged, bounded admission bookkeeping.
use super::*;
impl WorkRecallState {
    fn changed(&mut self) {
        match self.projection_revision.checked_add(1) {
            Some(next) => self.projection_revision = next,
            None => self.revision_exhausted = true,
        }
    }
    pub(in crate::work) fn task_changed(&mut self, project: u64, id: u64, revision: u64) {
        if let Some(value) = self
            .source_versions
            .get_mut(&WorkSourceKey::Task { project, id })
        {
            value.revision = revision;
            self.changed();
        }
    }
    pub(in crate::work) fn preference_changed(
        &mut self,
        scope: &WorkScope,
        delivery: u64,
        actor: u64,
    ) {
        let key = WorkSourceKey::Preference {
            scope: scope.clone(),
            delivery,
            actor,
        };
        if let Some(value) = self.source_versions.get_mut(&key) {
            match value.revision.checked_add(1) {
                Some(next) => value.revision = next,
                None => value.recall_disabled_exhausted = true,
            }
            self.changed();
        }
    }
    pub(in crate::work) fn preferences_reset(&mut self, scope: &WorkScope) {
        if let Some(control) = self.scope_controls.get_mut(scope) {
            match control.generation.checked_add(1) {
                Some(next) => control.generation = next,
                None => control.recall_disabled_exhausted = true,
            }
            self.changed();
        }
    }
    pub(in crate::work) fn learning_changed(&mut self, scope: &WorkScope) {
        if self.scope_controls.contains_key(scope) {
            self.changed();
        }
    }
    #[cfg(test)]
    fn allocate(&mut self) -> Result<u64, WorkError> {
        self.sequence = self.sequence.checked_add(1).ok_or(WorkError::Full)?;
        Ok(self.sequence)
    }
    #[cfg(test)]
    fn finish(&mut self, attempt: ProjectionAttempt, outcome: TerminalOutcome, at: u64) {
        self.attempts.remove(&attempt.id);
        if self.terminal.len() == MAX_TERMINALS {
            self.terminal.pop_front();
        }
        self.terminal.push_back(TerminalSummary {
            attempt: attempt.id,
            at,
            outcome,
        });
        self.changed();
    }
    /// Future coordinator seam: settlement retains an appended original even if
    /// native authority changed during admission. Eligibility handles retirement.
    #[cfg(test)]
    pub fn settle_add(
        &mut self,
        attempt_id: u64,
        payload: WorkEvidencePayload,
        admission: WorkAdmission,
        at: u64,
    ) -> Result<u64, WorkError> {
        let attempt = self
            .attempts
            .get(&attempt_id)
            .ok_or(WorkError::Missing)?
            .clone();
        let ProjectionOperation::Add { row } = attempt.operation else {
            return Err(WorkError::Invalid);
        };
        if attempt.source != payload.source
            || attempt.revision != payload.revision
            || attempt.generation != payload.generation
            || digest(&payload.encoded()?) != attempt.payload_digest
            || payload.encoded()?.len() != attempt.payload_bytes
        {
            return Err(WorkError::Invalid);
        }
        let outcome = match admission {
            WorkAdmission::Appended { .. } => TerminalOutcome::Appended,
            WorkAdmission::Uncovered { .. } => TerminalOutcome::Uncovered,
        };
        let record = AdmittedWorkEvidence {
            id: row,
            payload,
            payload_digest: attempt.payload_digest.clone(),
            admission,
        };
        record.validate()?;
        let mut next = self.clone();
        next.finish(attempt, outcome, at);
        next.records.insert(row, record);
        next.validate()?;
        *self = next;
        Ok(row)
    }
    #[cfg(test)]
    pub fn reject(&mut self, attempt_id: u64, at: u64) -> Result<(), WorkError> {
        let attempt = self
            .attempts
            .get(&attempt_id)
            .ok_or(WorkError::Missing)?
            .clone();
        self.finish(attempt, TerminalOutcome::Rejected, at);
        Ok(())
    }
}
impl WorkStore {
    /// Explicit projection preparation only; loading and native hooks never call it.
    #[cfg(test)]
    pub fn prepare_recall(
        &mut self,
        key: &WorkSourceKey,
        access: WorkAccess,
        at: u64,
        nonce: u64,
        config_digest: String,
    ) -> Result<(u64, WorkEvidencePayload), WorkError> {
        let payload = self.recall_candidate(key, access)?;
        let mut next = self.recall.clone();
        next.scope_controls
            .entry(payload.scope.clone())
            .or_default();
        next.source_versions
            .entry(key.clone())
            .or_insert(SourceVersion {
                revision: payload.revision,
                generation: payload.generation,
                recall_enabled: true,
                recall_disabled_exhausted: false,
            });
        if let Some(v) = next.source_versions.get_mut(key) {
            v.generation = payload.generation;
        }
        let row = next.allocate()?;
        let id = next.allocate()?;
        let attempt = ProjectionAttempt {
            id,
            source: key.clone(),
            operation: ProjectionOperation::Add { row },
            revision: payload.revision,
            generation: payload.generation,
            payload_digest: digest(&payload.encoded()?),
            payload_bytes: payload.encoded()?.len(),
            at,
            nonce,
            config_digest,
            state: AttemptState::Prepared,
        };
        next.attempts.insert(id, attempt);
        next.validate()?;
        next.changed();
        self.recall = next;
        Ok((id, payload))
    }
    #[cfg(test)]
    pub fn disable_recall_source(
        &mut self,
        key: &WorkSourceKey,
        access: WorkAccess,
    ) -> Result<(), WorkError> {
        // Re-authorize even when already disabled. Existing native state remains.
        let scope = self.recall_source_scope(key)?;
        match key {
            WorkSourceKey::Task { project, .. } | WorkSourceKey::Decision { project, .. } => {
                self.project(*project, access)?.authorize(access, true)?;
            }
            WorkSourceKey::Preference { .. } => {
                self.scope_projects(&scope, access, true)?;
            }
        }
        let mut next = self.recall.clone();
        if !next.source_versions.contains_key(key) {
            let payload = self.recall_candidate(key, access)?;
            next.scope_controls.entry(scope).or_default();
            next.source_versions.insert(
                key.clone(),
                SourceVersion {
                    revision: payload.revision,
                    generation: payload.generation,
                    recall_enabled: false,
                    recall_disabled_exhausted: false,
                },
            );
        } else {
            next.source_versions
                .get_mut(key)
                .ok_or(WorkError::Missing)?
                .recall_enabled = false;
        }
        next.validate()?;
        next.changed();
        self.recall = next;
        Ok(())
    }
    /// Caller supplies durable disk cleanup and every retained delivery draft's
    /// source set; absence of either proof cannot compact authority.
    #[cfg(test)]
    pub fn compact_recall(
        &mut self,
        disk_revision: Option<u64>,
        retained_drafts: &BTreeSet<WorkSourceKey>,
    ) {
        if disk_revision != Some(self.recall.projection_revision) || self.recall.revision_exhausted
        {
            return;
        }
        let keep: BTreeSet<_> = self
            .recall
            .source_versions
            .iter()
            .filter_map(|(key, value)| {
                let native_exists = match key {
                    WorkSourceKey::Preference {
                        scope,
                        delivery,
                        actor,
                    } => self.preferences.get(&scope.key()).is_some_and(|p| {
                        p.evidence
                            .iter()
                            .any(|e| e.delivery_id == *delivery && e.actor == Some(*actor))
                    }),
                    _ => self.recall_source_scope(key).is_ok(),
                };
                (retained_drafts.contains(key)
                    || self
                        .recall
                        .records
                        .values()
                        .any(|r| &r.payload.source == key)
                    || self.recall.attempts.values().any(|a| &a.source == key)
                    || (native_exists
                        && (!value.recall_enabled || value.recall_disabled_exhausted)))
                    .then_some(key.clone())
            })
            .collect();
        let scopes: BTreeSet<_> = keep
            .iter()
            .filter_map(|k| self.recall_source_scope(k).ok())
            .collect();
        self.recall.source_versions.retain(|k, _| keep.contains(k));
        // Exhausted scope controls must remain sticky even with no rows.
        self.recall
            .scope_controls
            .retain(|s, v| scopes.contains(s) || v.recall_disabled_exhausted);
    }
}
