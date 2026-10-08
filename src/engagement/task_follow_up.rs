//! Validated optional native-task metadata and durable replay coverage.
use super::*;
use crate::calendar::utc;
use crate::work::{
    WorkContentRef, WorkScope,
    follow_up::{FollowUpDecision, FollowUpIntent, follow_up_due, task_key, work_scope},
};

impl Candidate {
    pub(crate) fn validate_task_follow_up(&self) -> Result<(), WorkError> {
        match (&self.work_ref, self.expires_at) {
            (None, None) if self.follow_up_reason.is_none() => Ok(()),
            (Some(task), Some(expires)) => {
                let source = self.source.as_ref().ok_or(WorkError::Invalid)?;
                let member = self.member.ok_or(WorkError::Invalid)?;
                let key = task_key(&work_scope(&self.scope), task)?;
                utc(expires)?;
                if self.kind != EngagementKind::FollowUp
                    || source.scope != self.scope
                    || source.author != member
                    || self.due_at != follow_up_due(source)?
                    || expires <= self.due_at
                    || self.dedupe_key != key
                    || self.introduction_id.is_some()
                    || self.follow_up_reason.is_some_and(|reason| {
                        reason == FollowUpDecision::Allowed
                            || !matches!(
                                self.state,
                                CandidateState::Cancelled | CandidateState::Rejected
                            )
                    })
                {
                    return Err(WorkError::Invalid);
                }
                Ok(())
            }
            _ => Err(WorkError::Invalid),
        }
    }
}

impl EngagementStore {
    pub(crate) fn task_follow_up_erased(&self, candidate: &Candidate) -> bool {
        candidate.validate_task_follow_up().is_ok()
            && candidate.work_ref.as_ref().is_some_and(|task| {
                self.erased_identities
                    .contains(&erasure_identity::task_follow_up(
                        &work_scope(&candidate.scope),
                        task,
                    ))
                    && self
                        .erased_identities
                        .contains(&erasure_identity::candidate(candidate))
            })
    }
    pub(crate) fn task_follow_up_attempted(
        &self,
        scope: &WorkScope,
        task: &WorkContentRef,
    ) -> bool {
        if task_key(scope, task).is_err() {
            return true;
        }
        self.erased_identities
            .contains(&erasure_identity::task_follow_up(scope, task))
            || self
                .candidates
                .values()
                .any(|c| c.work_ref.as_ref() == Some(task) && work_scope(&c.scope) == *scope)
    }

    pub(crate) fn follow_up_source_attempted(&self, source: &SourceRef, member: u64) -> bool {
        let proposal = schedule::CandidateProposal {
            kind: EngagementKind::FollowUp,
            source: Some(source.clone()),
            member: Some(member),
            scope: source.scope.clone(),
            due_at: 0,
            introduction_id: None,
        };
        self.erased_identities
            .contains(&erasure_identity::proposal(&proposal))
            || self.candidates.values().any(|c| {
                c.kind == EngagementKind::FollowUp
                    && c.member == Some(member)
                    && c.scope == source.scope
                    && c.source
                        .as_ref()
                        .is_some_and(|s| s.message == source.message)
            })
    }

    /// The intermediate generic candidate exists only in a private clone. The
    /// complete linked record validates before the caller's store is replaced.
    pub(crate) fn propose_linked_task_follow_up(
        &mut self,
        intent: &FollowUpIntent,
        source: SourceRef,
        member: u64,
        due: u64,
        now: u64,
    ) -> Result<Option<u64>, WorkError> {
        if self.task_follow_up_attempted(&intent.scope, &intent.task)
            || self.follow_up_source_attempted(&source, member)
        {
            return Ok(None);
        }
        let mut next = self.clone();
        let Some(id) = next.propose(
            schedule::CandidateProposal {
                kind: EngagementKind::FollowUp,
                source: Some(source.clone()),
                member: Some(member),
                scope: source.scope,
                due_at: due,
                introduction_id: None,
            },
            now,
        )?
        else {
            return Ok(None);
        };
        let candidate = next.candidates.get_mut(&id).ok_or(WorkError::Missing)?;
        candidate.work_ref = Some(intent.task.clone());
        candidate.expires_at = Some(intent.expires_at);
        candidate.dedupe_key = task_key(&intent.scope, &intent.task)?;
        candidate.follow_up_reason = None;
        next.validate()?;
        *self = next;
        Ok(Some(id))
    }

    /// Retire expiry/stale native task/destination in the WorkStore-owned tick
    /// before expensive proof/reservation. The candidate remains a replay guard.
    pub(crate) fn cancel_task_follow_up(
        &mut self,
        id: u64,
        revision: u64,
        reason: FollowUpDecision,
    ) -> Result<bool, WorkError> {
        if reason == FollowUpDecision::Allowed {
            return Err(WorkError::Invalid);
        }
        let candidate = self.candidates.get_mut(&id).ok_or(WorkError::Missing)?;
        if candidate.revision != revision
            || candidate.work_ref.is_none()
            || !matches!(
                candidate.state,
                CandidateState::Pending | CandidateState::Reserved
            )
        {
            return Ok(false);
        }
        candidate.state = CandidateState::Cancelled;
        candidate.follow_up_reason = Some(reason);
        Ok(true)
    }
}
