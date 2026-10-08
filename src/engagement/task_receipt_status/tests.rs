// Apply as src/engagement/task_receipt_status/tests.rs.
use super::*;
use crate::engagement::{
    ContactCharge, DestinationPreference, EngagementKind, EngagementStore, SourceRef,
};
use crate::work::{WorkContentRef, follow_up::task_key, follow_up::work_scope};

const NOW: u64 = 1_790_683_200;
fn origin() -> EngagementScope {
    EngagementScope::Dm {
        member: 7,
        channel: 2,
    }
}
fn receipt(id: u64, state: CandidateState) -> Candidate {
    let scope = origin();
    let reference = WorkContentRef::Task {
        project: 11,
        id: 20 + id,
        revision: 0,
    };
    Candidate {
        id,
        kind: EngagementKind::FollowUp,
        source: Some(SourceRef {
            scope: scope.clone(),
            message: 100 + id,
            author: 7,
            revision: 1,
            at: NOW - 86_400,
        }),
        member: Some(7),
        scope: scope.clone(),
        due_at: NOW,
        revision: 1,
        state,
        dedupe_key: task_key(&work_scope(&scope), &reference).unwrap(),
        policy_revision: 0,
        destination: DestinationPreference::Origin,
        message_id: (state == CandidateState::Sent).then_some(500 + id),
        introduction_id: None,
        work_ref: Some(reference),
        expires_at: Some(NOW + 3600),
        follow_up_reason: None,
    }
}
fn store(rows: impl IntoIterator<Item = Candidate>) -> EngagementStore {
    let mut s = EngagementStore::default();
    for c in rows {
        s.sequence = s.sequence.max(c.id);
        if matches!(
            c.state,
            CandidateState::Reserved | CandidateState::Sent | CandidateState::ReviewRequired
        ) {
            s.charges.push(ContactCharge {
                candidate_id: c.id,
                member: 7,
                local_day: "2026-10-01".into(),
                local_week: "2026-09-28".into(),
                at: NOW,
            });
        }
        s.candidates.insert(c.id, c);
    }
    s.validate().unwrap();
    s
}
fn explicit(s: &mut EngagementStore, id: u64, kind: FeedbackKind) {
    s.feedback.entry(id).or_default().insert(
        7,
        ExplicitFeedback {
            actor: 7,
            at: NOW + 1,
            kind,
        },
    );
    s.validate().unwrap();
}
fn aggregate(s: &EngagementStore) -> TaskReceiptAggregate {
    let mut counts = TaskReceiptAggregate::default();
    for c in s.candidates.values() {
        counts.observe(
            7,
            &origin(),
            c,
            s.feedback.get(&c.id).and_then(|rows| rows.get(&7)),
        );
    }
    counts
}

#[test]
fn usefulness_denominator_counts_stops_and_failures() {
    let mut stopped = receipt(2, CandidateState::Cancelled);
    stopped.follow_up_reason = Some(FollowUpDecision::OptedOut);
    let mut s = store([
        receipt(1, CandidateState::Sent),
        stopped,
        receipt(3, CandidateState::Rejected),
        receipt(4, CandidateState::ReviewRequired),
        receipt(5, CandidateState::Sent),
    ]);
    explicit(&mut s, 1, FeedbackKind::Useful);
    let before = s.clone();
    let counts = aggregate(&s);
    assert_eq!(counts.total(), 5);
    assert_eq!(
        counts.total(),
        counts.useful + counts.stopped + counts.failed + counts.unanswered
    );
    assert_eq!(
        (
            counts.useful,
            counts.stopped,
            counts.failed,
            counts.unanswered
        ),
        (1, 1, 2, 1)
    );
    assert_eq!(counts.active, 0);
    assert_eq!(s, before, "inspection does not mutate receipts or charges");
}

#[test]
fn task_receipt_aggregate_covers_each_delivery_state_without_inferring_useful() {
    for (state, active, failed, unanswered) in [
        (CandidateState::Pending, 1, 0, 0),
        (CandidateState::Reserved, 1, 0, 0),
        (CandidateState::Sent, 0, 0, 1),
        (CandidateState::Cancelled, 0, 1, 0),
        (CandidateState::Rejected, 0, 1, 0),
        (CandidateState::ReviewRequired, 0, 1, 0),
    ] {
        let s = store([receipt(1, state)]);
        let counts = aggregate(&s);
        assert_eq!(counts.active, active, "{state:?}");
        assert_eq!(counts.failed, failed, "{state:?}");
        assert_eq!(counts.unanswered, unanswered, "{state:?}");
        assert_eq!(counts.useful, 0, "{state:?} is not feedback");
        assert_eq!(counts.stopped, 0, "a reasonless cancellation is unknown");
        assert_eq!(counts.total(), failed + unanswered);
    }
}

#[test]
fn task_receipt_aggregate_dismissed_feedback_is_non_useful_never_unanswered() {
    let mut s = store([receipt(1, CandidateState::Sent)]);
    explicit(&mut s, 1, FeedbackKind::Dismissed);
    let counts = aggregate(&s);
    assert_eq!(counts.total(), 1);
    assert_eq!(counts.failed, 1);
    assert_eq!(
        (counts.useful, counts.unanswered, counts.stopped),
        (0, 0, 0)
    );
    let text = counts.render();
    assert!(text.contains("failed/non-useful 1"));
    assert!(text.contains("Dismissed feedback"));
    assert!(text.contains("does not prove no reply"));
    assert!(text.contains("counts cover the shown readable receipts"));
    assert!(text.contains("erased or inaccessible outcomes are omitted"));
}

#[test]
fn task_receipt_aggregate_current_policy_cannot_rewrite_historical_causes() {
    let s = store([receipt(1, CandidateState::Cancelled)]);
    let mut stopped_now = s.clone();
    stopped_now
        .member_policies
        .entry(7)
        .or_default()
        .global_stop = true;
    assert_eq!(aggregate(&s), aggregate(&stopped_now));
    assert_eq!(aggregate(&s).stopped, 0);
    assert_eq!(aggregate(&s).failed, 1);
    let mut recorded = s;
    recorded.candidates.get_mut(&1).unwrap().follow_up_reason = Some(FollowUpDecision::Expired);
    assert_eq!(aggregate(&recorded).failed, 1);
    assert_eq!(aggregate(&recorded).stopped, 0);
}

#[test]
fn task_receipt_aggregate_rejects_foreign_scope_actor_and_invalid_linked_metadata() {
    let c = receipt(1, CandidateState::Sent);
    let bad_feedback = ExplicitFeedback {
        actor: 8,
        at: NOW,
        kind: FeedbackKind::Useful,
    };
    let mut counts = TaskReceiptAggregate::default();
    assert!(!counts.observe(8, &origin(), &c, None));
    assert!(!counts.observe(
        7,
        &EngagementScope::Dm {
            member: 7,
            channel: 3
        },
        &c,
        None
    ));
    assert!(!counts.observe(7, &origin(), &c, Some(&bad_feedback)));
    for invalid in 0..4 {
        let mut malformed = c.clone();
        match invalid {
            0 => malformed.work_ref = None,
            1 => malformed.source.as_mut().unwrap().author = 8,
            2 => malformed.dedupe_key = "unlinked-source".into(),
            _ => malformed.message_id = None,
        }
        assert!(!counts.observe(7, &origin(), &malformed, None));
    }
    assert_eq!(counts.total(), 0);
    assert_eq!(counts.active, 0);
    assert!(counts.render().is_empty());
}

#[test]
fn task_receipt_aggregate_has_five_row_bound_and_no_duplicate_denominator() {
    let mut counts = TaskReceiptAggregate::default();
    let first = receipt(1, CandidateState::Sent);
    assert!(counts.observe(7, &origin(), &first, None));
    assert!(!counts.observe(7, &origin(), &first, None));
    for id in 2..=7 {
        assert_eq!(
            counts.observe(7, &origin(), &receipt(id, CandidateState::Sent), None),
            id <= 5
        );
    }
    assert_eq!(counts.total(), 5);
    assert_eq!(counts.unanswered, 5);
}

#[test]
fn task_receipt_aggregate_erasure_does_not_reconstruct_outcomes_from_minimized_charges() {
    let mut s = store([receipt(1, CandidateState::Sent)]);
    explicit(&mut s, 1, FeedbackKind::Useful);
    assert_eq!(aggregate(&s).useful, 1);
    assert!(s.erase_learning("discord:dm:7", Some(7)).unwrap() > 0);
    assert!(s.candidates.is_empty());
    assert!(s.feedback.is_empty());
    assert_eq!(s.erased_contact_charges.len(), 1);
    assert!(!s.erased_identities.is_empty());
    let counts = aggregate(&s);
    assert_eq!(counts.total(), 0);
    assert_eq!((counts.active, counts.useful, counts.failed), (0, 0, 0));
    assert!(counts.render().is_empty());
}
