// Apply as src/commands_engage/tests/task_receipt_stop.rs; add in tests.rs:
// #[path = "tests/task_receipt_stop.rs"] mod task_receipt_stop;
// These exercise the existing stop_store API before changing its implementation.
use super::*;
use crate::work::{
    WorkContentRef,
    follow_up::{FollowUpDecision, task_key, work_scope},
};

const NOW: u64 = 1_790_683_200;
fn linked(state: CandidateState) -> EngagementStore {
    let mut s = receipt(state);
    let c = s.candidates.get_mut(&1).unwrap();
    let reference = WorkContentRef::Task {
        project: 11,
        id: 21,
        revision: 0,
    };
    c.kind = EngagementKind::FollowUp;
    c.source = Some(SourceRef {
        scope: c.scope.clone(),
        message: 101,
        author: 7,
        revision: 1,
        at: NOW - 86_400,
    });
    c.due_at = NOW;
    c.dedupe_key = task_key(&work_scope(&c.scope), &reference).unwrap();
    c.work_ref = Some(reference);
    c.expires_at = Some(NOW + 3600);
    s.validate().unwrap();
    s
}

#[test]
fn task_follow_up_stop_then_resume_retains_recorded_opted_out_cause_and_charge() {
    let mut s = linked(CandidateState::Reserved);
    let original = s.candidates[&1].clone();
    let origin = original.scope.clone();
    let charges = s.charges.clone();
    stop_store(&mut s, 7, &origin, StopScope::CurrentConversation).unwrap();
    assert_eq!(s.candidates[&1].state, CandidateState::Cancelled);
    assert_eq!(
        s.candidates[&1].follow_up_reason,
        Some(FollowUpDecision::OptedOut),
        "a new explicit stop must remain distinguishable after scoped resume"
    );
    let p = s.member_policies.get_mut(&7).unwrap();
    set_stop(p, &origin, StopScope::CurrentConversation, false).unwrap();
    assert!(!blocked_scope(p, &origin));
    assert_eq!(s.candidates[&1].state, CandidateState::Cancelled);
    assert_eq!(
        s.candidates[&1].follow_up_reason,
        Some(FollowUpDecision::OptedOut)
    );
    assert_eq!(s.candidates[&1].dedupe_key, original.dedupe_key);
    assert_eq!(s.charges, charges, "recording a stop never refunds contact");
    assert!(s.follow_up_source_attempted(original.source.as_ref().unwrap(), 7));
    assert!(s.task_follow_up_attempted(&work_scope(&origin), original.work_ref.as_ref().unwrap()));
    s.validate().unwrap();
    let reopened: EngagementStore =
        serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(
        reopened.candidates[&1].follow_up_reason,
        Some(FollowUpDecision::OptedOut)
    );
    assert_eq!(reopened.charges, charges);
}

#[test]
fn task_follow_up_stop_preserves_old_unknown_and_known_terminal_causes() {
    let mut s = linked(CandidateState::Pending);
    let origin = s.candidates[&1].scope.clone();
    for (id, reason) in [(2, None), (3, Some(FollowUpDecision::Expired))] {
        let mut old = s.candidates[&1].clone();
        old.id = id;
        old.source.as_mut().unwrap().message = 100 + id;
        let reference = WorkContentRef::Task {
            project: 11,
            id: 20 + id,
            revision: 0,
        };
        old.dedupe_key = task_key(&work_scope(&origin), &reference).unwrap();
        old.work_ref = Some(reference);
        old.state = CandidateState::Cancelled;
        old.follow_up_reason = reason;
        s.candidates.insert(id, old);
    }
    let mut generic = receipt(CandidateState::Pending)
        .candidates
        .remove(&1)
        .unwrap();
    generic.id = 4;
    generic.dedupe_key = "weekly-4".into();
    s.candidates.insert(4, generic);
    s.sequence = 4;
    s.validate().unwrap();
    stop_store(&mut s, 7, &origin, StopScope::CurrentConversation).unwrap();
    assert_eq!(
        s.candidates[&1].follow_up_reason,
        Some(FollowUpDecision::OptedOut)
    );
    assert_eq!(
        s.candidates[&2].follow_up_reason, None,
        "do not reconstruct historical stops"
    );
    assert_eq!(
        s.candidates[&3].follow_up_reason,
        Some(FollowUpDecision::Expired)
    );
    assert_eq!(s.candidates[&4].state, CandidateState::Cancelled);
    assert_eq!(
        s.candidates[&4].follow_up_reason, None,
        "legacy candidates retain their shape"
    );
    s.validate().unwrap();
}

#[test]
fn task_follow_up_feedback_authority_records_only_explicit_sent_recipient_feedback() {
    let mut s = linked(CandidateState::Sent);
    assert!(feedback(&mut s, 8, 1, FeedbackKind::Useful, NOW + 1).is_err());
    assert!(s.feedback.is_empty());
    feedback(&mut s, 7, 1, FeedbackKind::Useful, NOW + 1).unwrap();
    assert_eq!(
        s.feedback[&1][&7],
        ExplicitFeedback {
            actor: 7,
            at: NOW + 1,
            kind: FeedbackKind::Useful
        }
    );
    assert!(feedback(&mut s, 7, 1, FeedbackKind::Dismissed, NOW + 2).is_err());
    let pending = &mut linked(CandidateState::Pending);
    assert!(feedback(pending, 7, 1, FeedbackKind::Useful, NOW + 1).is_err());
    s.validate().unwrap();
}
