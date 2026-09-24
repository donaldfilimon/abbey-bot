//! Staged exact-record deletion bookkeeping; the coordinator must supply a
//! positively observed gate result. This module never calls or retries a gate.
use super::*;
impl WorkStore {
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
        // Explicit forget must first disable a still-current native source.
        if self
            .recall_candidate(&record.payload.source, access)
            .is_ok_and(|p| p == record.payload)
        {
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
