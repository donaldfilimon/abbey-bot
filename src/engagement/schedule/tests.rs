use super::*;
use crate::engagement::lifecycle::{DeliveryOutcome, EngagementReservation};
fn fixture() -> (EngagementStore, CandidateProposal) {
    let scope = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let source = SourceRef {
        scope: scope.clone(),
        message: 3,
        author: 4,
        revision: 1,
        at: 1,
    };
    let mut s = EngagementStore::default();
    s.member_policies.insert(
        4,
        MemberPolicy {
            revision: 1,
            daily_limit: Some(1),
            timezone: Some("UTC".into()),
            quiet_start: 0,
            quiet_end: 0,
            ..Default::default()
        },
    );
    s.eligibility.insert(4, BTreeSet::from([source.clone()]));
    (
        s,
        CandidateProposal {
            kind: EngagementKind::FollowUp,
            source: Some(source),
            member: Some(4),
            scope,
            due_at: 100,
            introduction_id: None,
        },
    )
}
#[test]
fn consumed_source_never_rearms() {
    let (mut s, p) = fixture();
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    s.cancel_source(&p.scope, 3);
    assert!(s.propose(p, 100).unwrap().is_none());
    assert!(s.due(100).is_empty());
    assert_eq!(s.candidates[&id].state, CandidateState::Cancelled);
}
#[test]
fn reserved_sent_and_duplicate_tick() {
    let (mut s, p) = fixture();
    let id = s.propose(p, 100).unwrap().unwrap();
    s.reserve(id, 1, 100).unwrap();
    assert!(s.reserve(id, 1, 100).is_err());
    s.settle(id, DeliveryOutcome::Sent { message_id: 8 })
        .unwrap();
    assert_eq!(s.candidates[&id].message_id, Some(8));
    assert_eq!(s.charges.len(), 1);
    s.validate().unwrap();
}
#[test]
fn revision_cancellation_and_restart() {
    let (mut s, p) = fixture();
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    let r: EngagementReservation = s.reserve(id, 1, 100).unwrap();
    s.member_policies.get_mut(&4).unwrap().revision += 1;
    assert!(s.validate_reserved(&r, 100).is_err());
    let loaded: EngagementStore =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(loaded.candidates[&id].state, CandidateState::ReviewRequired);
    assert!(loaded.due(100).is_empty());
    s.cancel_source(&p.scope, 3);
    assert!(s.validate_reserved(&r, 100).is_err());
}
#[test]
fn default_disabled_and_overflow() {
    let (mut s, mut p) = fixture();
    s.member_policies.insert(4, MemberPolicy::default());
    assert!(s.propose(p.clone(), 100).is_ok());
    p.due_at = u64::MAX;
    assert!(s.propose(p, 100).is_err());
}
#[test]
fn invalid_source_time_rejected() {
    let (mut s, mut p) = fixture();
    let source = p.source.as_mut().unwrap();
    source.at = u64::MAX;
    s.eligibility.insert(4, BTreeSet::from([source.clone()]));
    assert!(s.propose(p, 100).is_err());
}
fn instant(s: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp()
        .try_into()
        .unwrap()
}
fn another(
    s: &mut EngagementStore,
    p: &CandidateProposal,
    message: u64,
    scope: EngagementScope,
) -> CandidateProposal {
    let mut p = p.clone();
    p.scope = scope.clone();
    let source = p.source.as_mut().unwrap();
    source.message = message;
    source.scope = scope;
    s.eligibility.get_mut(&4).unwrap().insert(source.clone());
    p
}
#[test]
fn global_daily_covers_two_guilds_and_dm() {
    let (mut s, p) = fixture();
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    s.reserve(id, 1, 100).unwrap();
    for (message, scope) in [
        (
            5,
            EngagementScope::Guild {
                guild: 2,
                channel: 8,
            },
        ),
        (
            6,
            EngagementScope::Dm {
                member: 4,
                channel: 9,
            },
        ),
    ] {
        let p = another(&mut s, &p, message, scope);
        let id = s.propose(p, 100).unwrap().unwrap();
        assert!(s.reserve(id, 1, 100).is_err());
        assert_eq!(s.candidates[&id].state, CandidateState::Pending);
    }
    assert_eq!(s.charges.len(), 1);
}
#[test]
fn weekly_cap_no_refund_and_timezone_edit() {
    let (mut s, p) = fixture();
    let now = instant("2026-10-01T12:00:00Z");
    let policy = s.member_policies.get_mut(&4).unwrap();
    policy.weekly_limit = Some(1);
    let id = s.propose(p.clone(), now).unwrap().unwrap();
    s.reserve(id, 1, now).unwrap();
    s.settle(id, DeliveryOutcome::Rejected).unwrap();
    let p = another(
        &mut s,
        &p,
        5,
        EngagementScope::Guild {
            guild: 2,
            channel: 8,
        },
    );
    let id = s.propose(p, now).unwrap().unwrap();
    let policy = s.member_policies.get_mut(&4).unwrap();
    policy.timezone = Some("Pacific/Auckland".into());
    policy.revision += 1;
    assert!(s.reserve(id, 1, now + 86_400).is_err());
    assert_eq!(s.charges.len(), 1);
}
#[test]
fn quiet_wrap_snooze_and_current_policy_capture() {
    let (mut s, p) = fixture();
    let id = s.propose(p, 100).unwrap().unwrap();
    let policy = s.member_policies.get_mut(&4).unwrap();
    policy.quiet_start = 22;
    policy.quiet_end = 8;
    policy.revision = 2;
    assert!(s.reserve(id, 1, instant("2026-10-01T23:00:00Z")).is_err());
    assert!(s.reserve(id, 1, instant("2026-10-02T07:59:00Z")).is_err());
    let now = instant("2026-10-02T08:00:00Z");
    s.member_policies.get_mut(&4).unwrap().snoozed_until = Some(now + 1);
    assert!(s.reserve(id, 1, now).is_err());
    let r = s.reserve(id, 1, now + 1).unwrap();
    assert_eq!(r.policy_revision, 2);
    s.validate_reserved(&r, now + 1).unwrap();
    s.member_policies.get_mut(&4).unwrap().global_stop = true;
    assert!(s.validate_reserved(&r, now + 1).is_err());
}
#[test]
fn dst_gap_overlap_and_missed_weeks() {
    let (mut s, p) = fixture();
    let policy = s.member_policies.get_mut(&4).unwrap();
    policy.timezone = Some("America/New_York".into());
    policy.weekly_subscription = Some(WeeklySubscription {
        weekday: 6,
        hour: 2,
        scope: p.scope.clone(),
        destination: DestinationPreference::Origin,
    });
    assert_eq!(
        s.next_weekly(4, instant("2026-03-08T06:00:00Z")).unwrap(),
        Some(instant("2026-03-08T07:00:00Z"))
    );
    s.member_policies
        .get_mut(&4)
        .unwrap()
        .weekly_subscription
        .as_mut()
        .unwrap()
        .hour = 1;
    assert_eq!(
        s.next_weekly(4, instant("2026-11-01T04:00:00Z")).unwrap(),
        Some(instant("2026-11-01T05:00:00Z"))
    );
    assert_eq!(
        s.next_weekly(4, instant("2026-11-09T12:00:00Z")).unwrap(),
        Some(instant("2026-11-15T06:00:00Z"))
    );
}
#[test]
fn source_identity_ignores_destination_and_arbitrary_key() {
    let (mut s, p) = fixture();
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    s.candidates.get_mut(&id).unwrap().dedupe_key = "arbitrary".into();
    s.member_policies
        .get_mut(&4)
        .unwrap()
        .destinations
        .insert(p.scope.clone(), DestinationPreference::Private);
    assert!(s.propose(p.clone(), 100).unwrap().is_none());
    s.cancel_member_origin(4, &p.scope, 2);
    assert!(s.propose(p, 100).unwrap().is_none());
}
#[test]
fn reserved_recovery_and_settlement_validation() {
    let (mut s, p) = fixture();
    let id = s.propose(p, 100).unwrap().unwrap();
    s.reserve(id, 1, 100).unwrap();
    assert!(
        s.settle(id, DeliveryOutcome::Sent { message_id: 0 })
            .is_err()
    );
    assert_eq!(s.recover_reserved(), 1);
    assert_eq!(s.recover_reserved(), 0);
    assert!(s.due(100).is_empty());
    assert!(s.settle(id, DeliveryOutcome::ReviewRequired).is_err());
    assert_eq!(s.charges.len(), 1);
}
#[test]
fn cancelled_admitted_attempt_preserves_remote_receipt() {
    let (mut s, p) = fixture();
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    let r = s.reserve(id, 1, 100).unwrap();
    s.cancel_source(&p.scope, 3);
    assert!(s.validate_reserved(&r, 100).is_err());
    s.settle(id, DeliveryOutcome::Sent { message_id: 9 })
        .unwrap();
    assert_eq!(s.candidates[&id].message_id, Some(9));
    assert!(s.propose(p, 100).unwrap().is_none());
    s.validate().unwrap();
}
#[test]
fn cancelled_before_reserve_cannot_claim_delivery() {
    let (mut s, p) = fixture();
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    s.cancel_source(&p.scope, 3);
    assert!(
        s.settle(id, DeliveryOutcome::Sent { message_id: 9 })
            .is_err()
    );
}
fn weekly_fixture(due: u64) -> (EngagementStore, CandidateProposal) {
    let (mut s, mut p) = fixture();
    p.kind = EngagementKind::WeeklyCheckIn;
    p.source = None;
    p.due_at = due;
    s.member_policies.get_mut(&4).unwrap().weekly_subscription = Some(WeeklySubscription {
        weekday: 6,
        hour: 1,
        scope: p.scope.clone(),
        destination: DestinationPreference::Origin,
    });
    (s, p)
}
#[test]
fn persisted_pending_weekly_does_not_replay_after_restart() {
    let (mut s, p) = weekly_fixture(100);
    let id = s.propose(p, 100).unwrap().unwrap();
    let mut loaded: EngagementStore =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert!(loaded.due(100 + 7 * 86400).is_empty());
    assert!(loaded.reserve(id, 1, 100 + 7 * 86400).is_err());
    assert!(loaded.charges.is_empty());
}
#[test]
fn weekly_snooze_never_extends_occurrence_hour() {
    let (mut s, p) = weekly_fixture(100);
    let id = s.propose(p, 100).unwrap().unwrap();
    s.member_policies.get_mut(&4).unwrap().snoozed_until = Some(3700);
    assert!(s.reserve(id, 1, 3700).is_err());
    assert!(s.due(3700).is_empty());
}
#[test]
fn weekly_first_fallback_hour_excludes_second_occurrence() {
    let first = instant("2026-11-01T05:00:00Z");
    let (mut s, p) = weekly_fixture(first);
    let id = s.propose(p, first).unwrap().unwrap();
    let r = s.reserve(id, 1, first + 3599).unwrap();
    s.validate_reserved(&r, first + 3599).unwrap();
    assert!(
        s.validate_reserved(&r, instant("2026-11-01T06:00:00Z"))
            .is_err()
    );
}
#[test]
fn weekly_scheduler_delay_inside_hour_remains_admitted() {
    let (mut s, p) = weekly_fixture(100);
    let id = s.propose(p, 100).unwrap().unwrap();
    assert_eq!(s.due(3699), vec![id]);
    let r = s.reserve(id, 1, 3699).unwrap();
    s.validate_reserved(&r, 3699).unwrap();
}
#[test]
fn missed_weekly_expiration_preserves_charges_and_dedupe() {
    let (mut s, p) = weekly_fixture(100);
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    let r = s.reserve(id, 1, 100).unwrap();
    assert_eq!(s.expire_missed_weekly(3699).unwrap(), 0);
    assert_eq!(s.expire_missed_weekly(3700).unwrap(), 1);
    assert_eq!(s.expire_missed_weekly(3701).unwrap(), 0);
    assert_eq!(s.candidates[&id].state, CandidateState::Cancelled);
    assert!(s.validate_reserved(&r, 3700).is_err());
    assert_eq!(s.charges.len(), 1);
    assert!(s.propose(p, 3700).unwrap().is_none());
    s.validate().unwrap();
}
#[test]
fn missed_pending_weekly_expires_while_snoozed_but_followup_survives() {
    let (mut s, p) = weekly_fixture(100);
    let weekly = s.propose(p.clone(), 100).unwrap().unwrap();
    let (_, follow) = fixture();
    let follow_id = s.propose(follow, 100).unwrap().unwrap();
    s.member_policies.get_mut(&4).unwrap().snoozed_until = Some(100000);
    assert_eq!(s.expire_missed_weekly(3700).unwrap(), 1);
    assert_eq!(s.candidates[&weekly].state, CandidateState::Cancelled);
    assert_eq!(s.candidates[&follow_id].state, CandidateState::Pending);
    assert!(s.charges.is_empty());
}
#[test]
fn weekly_deadline_overflow_is_rejected_without_partial_expiration() {
    let (mut s, mut p) = weekly_fixture(100);
    let id = s.propose(p.clone(), 100).unwrap().unwrap();
    p.due_at = u64::MAX;
    assert!(s.propose(p, 100).is_err());
    s.candidates.get_mut(&id).unwrap().due_at = u64::MAX;
    assert!(s.expire_missed_weekly(3700).is_err());
    assert_eq!(s.candidates[&id].state, CandidateState::Pending);
    assert!(s.reserve(id, 1, 3700).is_err());
    assert!(s.due(3700).is_empty());
}
