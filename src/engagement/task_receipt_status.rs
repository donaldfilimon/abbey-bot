//! Read-only classification of at most five freshly authorized linked-task receipts.
//! A counter is retained receipt evidence, never a live usefulness qualification.
use super::{Candidate, CandidateState, EngagementScope, ExplicitFeedback, FeedbackKind};
use crate::work::follow_up::FollowUpDecision;
use std::collections::BTreeSet;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TaskReceiptAggregate {
    pub(crate) useful: usize,
    pub(crate) stopped: usize,
    pub(crate) failed: usize,
    pub(crate) unanswered: usize,
    pub(crate) active: usize,
    inspected: BTreeSet<u64>,
}

impl TaskReceiptAggregate {
    /// The caller supplies only rows whose existing exact-origin, current Work
    /// and canonical-record checks just succeeded. This is not an access grant.
    /// Feedback is the existing candidate/recipient entry, read under that lock.
    pub(crate) fn observe(
        &mut self,
        actor: u64,
        origin: &EngagementScope,
        candidate: &Candidate,
        feedback: Option<&ExplicitFeedback>,
    ) -> bool {
        if actor == 0
            || candidate.id == 0
            || candidate.revision == 0
            || candidate.member != Some(actor)
            || &candidate.scope != origin
            || candidate.work_ref.is_none()
            || candidate.validate_task_follow_up().is_err()
            || candidate
                .source
                .as_ref()
                .is_none_or(|s| s.validate().is_err())
            || (candidate.state == CandidateState::Sent) != candidate.message_id.is_some()
            || candidate.message_id == Some(0)
            || feedback.is_some_and(|f| f.actor != actor || candidate.state != CandidateState::Sent)
            || self.inspected.len() >= 5
            || self.inspected.contains(&candidate.id)
        {
            return false;
        }
        self.inspected.insert(candidate.id);
        match candidate.state {
            CandidateState::Pending | CandidateState::Reserved => self.active += 1,
            CandidateState::Sent => match feedback.map(|f| f.kind) {
                Some(FeedbackKind::Useful) => self.useful += 1,
                Some(FeedbackKind::Dismissed) => self.failed += 1,
                None => self.unanswered += 1,
            },
            CandidateState::Cancelled
                if candidate.follow_up_reason == Some(FollowUpDecision::OptedOut) =>
            {
                self.stopped += 1;
            }
            CandidateState::Cancelled
            | CandidateState::Rejected
            | CandidateState::ReviewRequired => {
                self.failed += 1;
            }
        }
        true
    }

    pub(crate) fn total(&self) -> usize {
        self.useful + self.stopped + self.failed + self.unanswered
    }

    /// Do not emit zero counters when no private row passed fresh authorization.
    pub(crate) fn render(&self) -> String {
        if self.inspected.is_empty() {
            return String::new();
        }
        format!(
            "Shown readable task receipts (up to 5): terminal {} = useful {} + stopped/opted out {} + failed/non-useful {} + no explicit feedback {}; active {}.\nUseful requires your explicit Useful feedback. Failed/non-useful includes refusals, uncertainty and Dismissed feedback; no explicit feedback does not prove no reply. These counts cover the shown readable receipts; erased or inaccessible outcomes are omitted.",
            self.total(),
            self.useful,
            self.stopped,
            self.failed,
            self.unanswered,
            self.active
        )
    }
}

#[cfg(test)]
#[path = "task_receipt_status/tests.rs"]
mod tests;
