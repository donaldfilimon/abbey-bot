//! Draft for src/engagement/lifecycle/task_follow_up_tests.rs.
//! Requires the coordinated Candidate provenance fields and FollowUpDecision enum.
use super::*;
use crate::work::{WorkContentRef, follow_up::FollowUpDecision};
const NOW: u64 = 1_790_683_200;
const MEMBER: u64 = 4;
fn task_fixture() -> (EngagementStore, u64) {
    let scope = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let source = SourceRef {
        scope: scope.clone(),
        message: 3,
        author: MEMBER,
        revision: 1,
        at: NOW - 86_400,
    };
    let candidate = Candidate {
        id: 1,
        kind: EngagementKind::FollowUp,
        source: Some(source.clone()),
        member: Some(MEMBER),
        scope: scope.clone(),
        due_at: NOW,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: crate::work::follow_up::task_key(
            &crate::work::follow_up::work_scope(&scope),
            &WorkContentRef::Task {
                project: 1,
                id: 5,
                revision: 1,
            },
        )
        .unwrap(),
        policy_revision: 1,
        destination: DestinationPreference::Origin,
        message_id: None,
        introduction_id: None,
        work_ref: Some(WorkContentRef::Task {
            project: 1,
            id: 5,
            revision: 1,
        }),
        expires_at: Some(NOW + 3600),
        follow_up_reason: None,
    };
    let mut store = EngagementStore {
        sequence: 1,
        ..Default::default()
    };
    store.member_policies.insert(
        MEMBER,
        MemberPolicy {
            revision: 1,
            daily_limit: Some(1),
            timezone: Some("UTC".into()),
            quiet_start: 0,
            quiet_end: 0,
            destinations: BTreeMap::from([(scope, DestinationPreference::Origin)]),
            ..Default::default()
        },
    );
    store.eligibility.insert(MEMBER, BTreeSet::from([source]));
    store.candidates.insert(1, candidate);
    (store, 1)
}
fn decision(store: &EngagementStore, id: u64, now: u64) -> Result<FollowUpDecision, WorkError> {
    store.task_follow_up_member_decision(&store.candidates[&id], now)
}
fn another_scope(store: &mut EngagementStore, id: u64, scope: EngagementScope) {
    let mut candidate = store.candidates[&1].clone();
    candidate.id = id;
    candidate.scope = scope.clone();
    candidate.state = CandidateState::Pending;
    candidate.message_id = None;
    candidate.work_ref = Some(WorkContentRef::Task {
        project: 1,
        id: id + 4,
        revision: 1,
    });
    candidate.dedupe_key = crate::work::follow_up::task_key(
        &crate::work::follow_up::work_scope(&scope),
        candidate.work_ref.as_ref().unwrap(),
    )
    .unwrap();
    let source = candidate.source.as_mut().unwrap();
    source.scope = scope.clone();
    source.message = id + 2;
    store
        .eligibility
        .get_mut(&MEMBER)
        .unwrap()
        .insert(source.clone());
    store
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(scope, DestinationPreference::Origin);
    store.sequence = store.sequence.max(id);
    store.candidates.insert(id, candidate);
}
#[test]
fn task_follow_up_reason_distinguishes_disabled_configuration_from_stop() {
    let (store, id) = task_fixture();
    assert_eq!(decision(&store, id, NOW), Ok(FollowUpDecision::Allowed));
    let mut missing = store.clone();
    missing.member_policies.remove(&MEMBER);
    assert_eq!(decision(&missing, id, NOW), Ok(FollowUpDecision::Disabled));
    missing
        .member_policies
        .insert(MEMBER, MemberPolicy::default());
    assert_eq!(decision(&missing, id, NOW), Ok(FollowUpDecision::Disabled));
    missing
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .global_stop = true;
    assert_eq!(decision(&missing, id, NOW), Ok(FollowUpDecision::OptedOut));
    let mut no_timezone = store.clone();
    no_timezone
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .timezone = None;
    assert_eq!(
        decision(&no_timezone, id, NOW),
        Ok(FollowUpDecision::Disabled)
    );
    let mut malformed = store;
    malformed.member_policies.get_mut(&MEMBER).unwrap().timezone = Some("not/a-zone".into());
    assert_eq!(decision(&malformed, id, NOW), Err(WorkError::Invalid));
}
#[test]
fn task_follow_up_stop_and_destination_are_exactly_scoped() {
    let (store, id) = task_fixture();
    for stop in [0, 1, 2] {
        let mut stopped = store.clone();
        let scope = stopped.candidates[&id].scope.clone();
        let policy = stopped.member_policies.get_mut(&MEMBER).unwrap();
        match stop {
            0 => policy.global_stop = true,
            1 => {
                policy.stopped_guilds.insert(1);
            }
            _ => {
                policy.stopped_scopes.insert(scope);
            }
        }
        assert_eq!(decision(&stopped, id, NOW), Ok(FollowUpDecision::OptedOut));
    }
    let mut other = store.clone();
    let policy = other.member_policies.get_mut(&MEMBER).unwrap();
    policy.stopped_guilds.insert(9);
    policy.stopped_scopes.insert(EngagementScope::Guild {
        guild: 1,
        channel: 99,
    });
    assert_eq!(decision(&other, id, NOW), Ok(FollowUpDecision::Allowed));
    // A task request needs a positive saved choice for the exact origin.
    let mut origin_default = store.clone();
    origin_default
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .clear();
    assert_eq!(
        decision(&origin_default, id, NOW),
        Ok(FollowUpDecision::OptedOut)
    );
    let before = origin_default.clone();
    assert_eq!(origin_default.reserve(id, 1, NOW), Err(WorkError::Denied));
    assert_eq!(origin_default, before);
    assert!(origin_default.charges.is_empty());
    for private_request in [false, true] {
        let mut changed = store.clone();
        let scope = changed.candidates[&id].scope.clone();
        if private_request {
            changed.candidates.get_mut(&id).unwrap().destination = DestinationPreference::Private;
            changed
                .member_policies
                .get_mut(&MEMBER)
                .unwrap()
                .destinations
                .remove(&scope);
        } else {
            changed
                .member_policies
                .get_mut(&MEMBER)
                .unwrap()
                .destinations
                .insert(scope, DestinationPreference::Private);
        }
        assert_eq!(decision(&changed, id, NOW), Ok(FollowUpDecision::OptedOut));
        let before = changed.clone();
        assert_eq!(changed.reserve(id, 1, NOW), Err(WorkError::Denied));
        assert_eq!(
            changed, before,
            "a changed destination cannot charge or retarget the request"
        );
    }
}
#[test]
fn task_follow_up_quiet_and_snooze_use_existing_local_policy() {
    let (mut store, id) = task_fixture();
    store
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .snoozed_until = Some(NOW + 1);
    assert_eq!(decision(&store, id, NOW), Ok(FollowUpDecision::Quiet));
    assert_eq!(decision(&store, id, NOW + 1), Ok(FollowUpDecision::Allowed));
    let hour = utc(NOW)
        .unwrap()
        .with_timezone(&"Pacific/Auckland".parse::<Tz>().unwrap())
        .hour() as u8;
    let policy = store.member_policies.get_mut(&MEMBER).unwrap();
    policy.snoozed_until = None;
    policy.timezone = Some("Pacific/Auckland".into());
    policy.quiet_start = hour;
    policy.quiet_end = (hour + 1) % 24;
    assert_eq!(decision(&store, id, NOW), Ok(FollowUpDecision::Quiet));
    let before = store.clone();
    assert_eq!(store.reserve(id, 1, NOW), Err(WorkError::Denied));
    assert_eq!(store, before);
}
#[test]
fn task_follow_up_uses_shared_charges_across_two_guilds_and_dm() {
    let (mut store, id) = task_fixture();
    another_scope(
        &mut store,
        2,
        EngagementScope::Guild {
            guild: 9,
            channel: 10,
        },
    );
    another_scope(
        &mut store,
        3,
        EngagementScope::Dm {
            member: MEMBER,
            channel: 12,
        },
    );
    store.reserve(id, 1, NOW).unwrap();
    store.settle(id, DeliveryOutcome::Rejected).unwrap();
    for next in [2, 3] {
        assert_eq!(decision(&store, next, NOW), Ok(FollowUpDecision::Budget));
        assert_eq!(store.reserve(next, 1, NOW), Err(WorkError::Denied));
    }
    assert_eq!(
        store.charges.len(),
        1,
        "failed reservations and rejection never refund capacity"
    );
    let charge = store.charges.pop().unwrap();
    store
        .erased_contact_charges
        .push(crate::engagement::erasure::ErasedContactCharge {
            member: charge.member,
            local_day: charge.local_day,
            local_week: charge.local_week,
            at: charge.at,
        });
    assert_eq!(decision(&store, 3, NOW), Ok(FollowUpDecision::Budget));
}
#[test]
fn task_follow_up_reserved_final_allowance_validates_without_another_charge() {
    let (mut store, id) = task_fixture();
    let reservation = store.reserve(id, 1, NOW).unwrap();
    assert_eq!(decision(&store, id, NOW), Ok(FollowUpDecision::Budget));
    assert_eq!(store.validate_reserved(&reservation, NOW), Ok(()));
    assert_eq!(store.charges.len(), 1);
    let scope = store.candidates[&id].scope.clone();
    store
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(scope, DestinationPreference::Private);
    assert_eq!(
        store.validate_reserved(&reservation, NOW),
        Err(WorkError::Denied)
    );
    assert_eq!(store.charges.len(), 1);
}
#[test]
fn task_follow_up_expiry_is_exclusive_before_and_after_reservation() {
    let (store, id) = task_fixture();
    for expiry in [None, Some(NOW), Some(NOW + 3600)] {
        let mut expired = store.clone();
        expired.candidates.get_mut(&id).unwrap().expires_at = expiry;
        let at = expiry.unwrap_or(NOW);
        assert!(expired.reserve(id, 1, at).is_err());
        assert!(expired.charges.is_empty());
        assert_eq!(expired.candidates[&id].state, CandidateState::Pending);
    }
    let mut reserved = store;
    let reservation = reserved.reserve(id, 1, NOW + 3599).unwrap();
    assert_eq!(reserved.validate_reserved(&reservation, NOW + 3599), Ok(()));
    assert_eq!(
        reserved.validate_reserved(&reservation, NOW + 3600),
        Err(WorkError::Stale)
    );
    assert_eq!(reserved.charges.len(), 1);
    reserved.candidates.get_mut(&id).unwrap().expires_at = None;
    assert_eq!(
        reserved.validate_reserved(&reservation, NOW),
        Err(WorkError::Invalid)
    );
}
#[test]
fn task_follow_up_policy_and_reservation_fail_closed_before_pruned_safety() {
    let (mut store, id) = task_fixture();
    store.safety_pruned_through = NOW + 1;
    assert_eq!(decision(&store, id, NOW), Ok(FollowUpDecision::Budget));
    assert_eq!(store.reserve(id, 1, NOW), Err(WorkError::Denied));
    assert!(store.charges.is_empty());
    store.safety_pruned_through = NOW;
    let reservation = store.reserve(id, 1, NOW).unwrap();
    store.safety_pruned_through = NOW + 1;
    assert_eq!(
        store.validate_reserved(&reservation, NOW),
        Err(WorkError::Denied)
    );
    assert_eq!(store.charges.len(), 1);
}
#[test]
fn task_follow_up_requires_same_dm_owner() {
    let (mut store, id) = task_fixture();
    store.candidates.get_mut(&id).unwrap().scope = EngagementScope::Dm {
        member: 99,
        channel: 12,
    };
    assert_eq!(
        decision(&store, id, NOW),
        Ok(FollowUpDecision::AccessDenied)
    );
}
#[test]
fn conversation_follow_up_keeps_existing_origin_fallback_and_policy_retargeting() {
    let (store, id) = task_fixture();
    for explicit in [false, true] {
        let mut legacy = store.clone();
        let candidate = legacy.candidates.get_mut(&id).unwrap();
        candidate.work_ref = None;
        candidate.expires_at = None;
        let scope = candidate.scope.clone();
        let policy = legacy.member_policies.get_mut(&MEMBER).unwrap();
        policy.destinations.clear();
        let expected = if explicit {
            policy
                .destinations
                .insert(scope, DestinationPreference::Private);
            DestinationPreference::Private
        } else {
            DestinationPreference::Origin
        };
        let reservation = legacy.reserve(id, 1, NOW + 86_400).unwrap();
        assert_eq!(reservation.destination, expected);
        assert_eq!(legacy.candidates[&id].destination, expected);
        assert_eq!(legacy.validate_reserved(&reservation, NOW + 86_400), Ok(()));
    }
}

#[test]
fn task_follow_up_observed_send_and_uncertainty_clear_prior_cancellation_reason() {
    for outcome in [
        DeliveryOutcome::Sent { message_id: 12 },
        DeliveryOutcome::ReviewRequired,
    ] {
        let (mut store, id) = task_fixture();
        store.reserve(id, 1, NOW).unwrap();
        store
            .cancel_task_follow_up(id, 1, FollowUpDecision::Expired)
            .unwrap();
        assert_eq!(
            store.candidates[&id].follow_up_reason,
            Some(FollowUpDecision::Expired)
        );
        store.settle(id, outcome).unwrap();
        assert_eq!(store.candidates[&id].follow_up_reason, None);
        store.candidates[&id].validate_task_follow_up().unwrap();
        assert_eq!(store.charges.len(), 1);
    }
}
#[test]
fn task_follow_up_restart_keeps_reserved_charge_and_never_replays() {
    let (mut store, id) = task_fixture();
    store.reserve(id, 1, NOW).unwrap();
    store.validate().unwrap();
    let encoded = serde_json::to_vec(&store).unwrap();
    let mut loaded: EngagementStore = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(loaded.candidates[&id].state, CandidateState::ReviewRequired);
    assert_eq!(loaded.candidates[&id].follow_up_reason, None);
    assert_eq!(loaded.charges.len(), 1);
    assert_eq!(loaded.reserve(id, 1, NOW), Err(WorkError::Stale));
    assert_eq!(loaded.charges.len(), 1);
}

#[test]
fn task_follow_up_erased_settlement_requires_both_exact_replay_commitments() {
    let (mut store, id) = task_fixture();
    let original = store.candidates[&id].clone();
    assert!(!store.task_follow_up_erased(&original));
    store.erase_learning("discord:1", Some(MEMBER)).unwrap();
    assert!(!store.candidates.contains_key(&id));
    assert_eq!(store.erased_identities.len(), 2);
    assert!(store.task_follow_up_erased(&original));
    for one in store.erased_identities.iter() {
        let mut incomplete = store.clone();
        incomplete.erased_identities = BTreeSet::from([one.clone()]);
        assert!(!incomplete.task_follow_up_erased(&original));
    }
    for changed in 0..3 {
        let mut different = original.clone();
        match changed {
            0 => {
                if let Some(WorkContentRef::Task { revision, .. }) = &mut different.work_ref {
                    *revision += 1
                }
            }
            1 => different.source.as_mut().unwrap().message += 1,
            _ => different.work_ref = None,
        }
        assert!(!store.task_follow_up_erased(&different));
    }
}
