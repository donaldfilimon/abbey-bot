//! Staged exact-record deletion bookkeeping; the coordinator must supply a
//! positively observed gate result. This module never calls or retries a gate.
use super::*;
impl WorkStore {
    /// Inspect canonical native state independently of learning/query controls.
    /// Missing/corrupt provenance is an error, never evidence of retirement.
    #[cfg(test)]
    fn recall_native_obsolete(&self, payload: &WorkEvidencePayload) -> Result<bool, WorkError> {
        let version = self
            .recall
            .source_versions
            .get(&payload.source)
            .ok_or(WorkError::Invalid)?;
        let native = match &payload.source {
            WorkSourceKey::Task { project, id } => {
                let Some(task) = self.tasks.get(id) else {
                    return Ok(true);
                };
                if task.project_id != *project {
                    return Err(WorkError::Invalid);
                }
                if task.revision != payload.revision {
                    return Ok(true);
                }
                bytes(task)?
            }
            WorkSourceKey::Decision { project, id } => {
                let Some(decision) = self.decisions.get(id) else {
                    return Ok(true);
                };
                if decision.project_id != *project || payload.revision != 1 {
                    return Err(WorkError::Invalid);
                }
                bytes(decision)?
            }
            WorkSourceKey::Preference {
                scope,
                delivery,
                actor,
            } => {
                let control = self
                    .recall
                    .scope_controls
                    .get(scope)
                    .ok_or(WorkError::Invalid)?;
                if control.generation != payload.generation || version.revision != payload.revision
                {
                    return Ok(true);
                }
                let observation = self.preferences.get(&scope.key()).and_then(|p| {
                    p.evidence.iter().find(|e| {
                        e.scope.as_ref() == Some(scope)
                            && e.delivery_id == *delivery
                            && e.actor == Some(*actor)
                    })
                });
                let Some(observation) = observation else {
                    return Ok(true);
                };
                self.validate_delivery_state()?;
                let receipt = self.deliveries.get(delivery).ok_or(WorkError::Invalid)?;
                let provenance = receipt.provenance.as_ref().ok_or(WorkError::Invalid)?;
                if receipt.scope.as_ref() != Some(scope) || receipt.kind != observation.kind {
                    return Err(WorkError::Invalid);
                }
                bytes(&(observation, provenance))?
            }
        };
        Ok(digest(&native) != payload.native_digest)
    }
    #[cfg(test)]
    pub fn prepare_recall_forget(
        &mut self,
        row: u64,
        access: WorkAccess,
        at: u64,
        nonce: u64,
        config_digest: String,
    ) -> Result<u64, WorkError> {
        let record = self.recall.records.get(&row).ok_or(WorkError::Missing)?;
        self.scope_projects(&record.payload.scope, access, true)?;
        // Query suppression (especially learning disable) is reversible. Only
        // durable native change or explicit source disable permits deletion.
        let version = self
            .recall
            .source_versions
            .get(&record.payload.source)
            .ok_or(WorkError::Invalid)?;
        if version.recall_enabled && !self.recall_native_obsolete(&record.payload)? {
            return Err(WorkError::Denied);
        }
        let mut next = self.recall.clone();
        let id = next.sequence.checked_add(1).ok_or(WorkError::Full)?;
        next.sequence = id;
        next.attempts.insert(
            id,
            ProjectionAttempt {
                id,
                source: record.payload.source.clone(),
                operation: ProjectionOperation::Forget { row },
                revision: record.payload.revision,
                generation: record.payload.generation,
                payload_digest: record.payload_digest.clone(),
                payload_bytes: record.payload.encoded()?.len(),
                at,
                nonce,
                config_digest,
                state: AttemptState::Prepared,
                observed_admission: None,
            },
        );
        next.validate()?;
        self.recall = next;
        Ok(id)
    }
}
impl WorkRecallState {
    /// `admission` is an observed outcome with the original scope. Covered versus
    /// uncovered policy and receipt-ledger edge verification belong to the adapter.
    #[cfg(test)]
    pub fn settle_forget(
        &mut self,
        id: u64,
        admission: WorkAdmission,
        at: u64,
    ) -> Result<(), WorkError> {
        let attempt = self.attempts.get(&id).ok_or(WorkError::Missing)?.clone();
        let ProjectionOperation::Forget { row } = attempt.operation else {
            return Err(WorkError::Invalid);
        };
        let record = self.records.get(&row).ok_or(WorkError::Missing)?;
        let mut proof = record.clone();
        proof.admission = admission;
        proof.validate()?;
        let outcome = match proof.admission {
            WorkAdmission::Appended { .. } => TerminalOutcome::Appended,
            WorkAdmission::Uncovered { .. } => TerminalOutcome::Uncovered,
        };
        let mut next = self.clone();
        next.records.remove(&row);
        next.attempts.remove(&id);
        if next.terminal.len() == MAX_TERMINALS {
            next.terminal.pop_front();
        }
        next.terminal.push_back(TerminalSummary {
            attempt: id,
            at,
            outcome,
        });
        match next.projection_revision.checked_add(1) {
            Some(v) => next.projection_revision = v,
            None => next.revision_exhausted = true,
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}
