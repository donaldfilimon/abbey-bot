use super::*;
use crate::work::WorkError;
#[test]
fn disabled_defaults_and_limits() {
    assert!(!MemberPolicy::default().personalized_enabled());
    for daily in [0, 5] {
        let invalid = MemberPolicy {
            daily_limit: Some(daily),
            ..Default::default()
        };
        assert_eq!(invalid.validate(), Err(WorkError::Invalid));
    }
}
#[test]
fn invalid_timezone_and_weekly_limits() {
    for policy in [
        MemberPolicy {
            timezone: Some("not/a-zone".into()),
            ..Default::default()
        },
        MemberPolicy {
            daily_limit: Some(1),
            weekly_limit: Some(8),
            ..Default::default()
        },
        MemberPolicy {
            daily_limit: Some(4),
            weekly_limit: Some(29),
            ..Default::default()
        },
    ] {
        assert_eq!(policy.validate(), Err(WorkError::Invalid));
    }
}
#[test]
fn subscription_and_quiet_boundaries() {
    let scope = EngagementScope::Dm {
        member: 9,
        channel: 10,
    };
    for (weekday, hour) in [(7, 9), (0, 24)] {
        let invalid = MemberPolicy {
            weekly_subscription: Some(WeeklySubscription {
                weekday,
                hour,
                scope: scope.clone(),
                destination: DestinationPreference::Origin,
            }),
            ..Default::default()
        };
        assert_eq!(invalid.validate(), Err(WorkError::Invalid));
    }
    for hour in [24, 255] {
        assert_eq!(
            MemberPolicy {
                quiet_start: hour,
                ..Default::default()
            }
            .validate(),
            Err(WorkError::Invalid)
        );
    }
    assert!(
        MemberPolicy {
            quiet_start: 8,
            quiet_end: 8,
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}
fn candidate(id: u64) -> Candidate {
    Candidate {
        id,
        kind: EngagementKind::FollowUp,
        source: Some(SourceRef {
            scope: EngagementScope::Guild {
                guild: 1,
                channel: 2,
            },
            message: 3,
            author: 9,
            revision: 1,
            at: 5,
        }),
        member: Some(9),
        scope: EngagementScope::Guild {
            guild: 1,
            channel: 2,
        },
        due_at: 86405,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: format!("source-{id}"),
        policy_revision: 0,
        destination: DestinationPreference::Origin,
        message_id: None,
        introduction_id: None,
    }
}
#[test]
fn invalid_ids_scope_and_duplicate_identity() {
    let mut store = EngagementStore {
        sequence: 2,
        ..Default::default()
    };
    let mut c = candidate(1);
    c.scope = EngagementScope::Guild {
        guild: 0,
        channel: 2,
    };
    store.candidates.insert(1, c);
    assert_eq!(store.validate(), Err(WorkError::Invalid));
    let mut c = candidate(1);
    c.scope = EngagementScope::Guild {
        guild: 1,
        channel: 4,
    };
    store.candidates.insert(1, c);
    assert_eq!(store.validate(), Err(WorkError::Invalid));
    store.candidates.insert(1, candidate(1));
    let mut c = candidate(2);
    c.dedupe_key = "source-1".into();
    store.candidates.insert(2, c);
    assert_eq!(store.validate(), Err(WorkError::Invalid));
}
#[test]
fn bounded_records() {
    let mut store = EngagementStore {
        sequence: 10001,
        ..Default::default()
    };
    for id in 1..=1001 {
        let mut c = candidate(id);
        c.source.as_mut().unwrap().message = id;
        store.candidates.insert(id, c);
    }
    assert_eq!(store.validate(), Err(WorkError::Full));
    for c in store.candidates.values_mut() {
        c.state = CandidateState::Cancelled;
    }
    assert!(store.validate().is_ok());
    for id in 1002..=10001 {
        let mut c = candidate(id);
        c.source.as_mut().unwrap().message = id;
        c.state = CandidateState::Cancelled;
        store.candidates.insert(id, c);
    }
    assert_eq!(store.validate(), Err(WorkError::Full));
}
#[test]
fn scope_map_roundtrip_and_reservation_recovery() {
    let scope = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let mut store = EngagementStore {
        sequence: 1,
        ..Default::default()
    };
    let mut policy = MemberPolicy::default();
    policy
        .destinations
        .insert(scope, DestinationPreference::Private);
    store.member_policies.insert(9, policy);
    let mut c = candidate(1);
    c.state = CandidateState::Reserved;
    store.candidates.insert(1, c);
    store.charges.push(ContactCharge {
        candidate_id: 1,
        member: 9,
        local_day: "2026-10-01".into(),
        local_week: "2026-09-28".into(),
        at: 5,
    });
    let encoded = serde_json::to_string(&store).unwrap();
    let loaded: EngagementStore = serde_json::from_str(&encoded).unwrap();
    assert_eq!(loaded.member_policies, store.member_policies);
    assert_eq!(loaded.candidates[&1].state, CandidateState::ReviewRequired);
    assert_eq!(loaded.candidates[&1].dedupe_key, "source-1");
}
#[test]
fn duplicate_charges_rejected_and_retained() {
    let mut store = EngagementStore {
        sequence: 1,
        ..Default::default()
    };
    let mut c = candidate(1);
    c.state = CandidateState::ReviewRequired;
    store.candidates.insert(1, c);
    let charge = ContactCharge {
        candidate_id: 1,
        member: 9,
        local_day: "2026-10-01".into(),
        local_week: "2026-09-28".into(),
        at: 5,
    };
    store.charges.push(charge.clone());
    assert!(store.validate().is_ok());
    store.charges.push(charge);
    assert_eq!(store.validate(), Err(WorkError::Invalid));
}

#[test]
fn disabled_defaults_and_policy_stops_survive_load() {
    let mut store = EngagementStore::default();
    store.member_policies.insert(
        9,
        MemberPolicy {
            daily_limit: Some(4),
            timezone: Some("UTC".into()),
            global_stop: true,
            ..Default::default()
        },
    );
    let loaded: EngagementStore =
        serde_json::from_value(serde_json::to_value(&store).unwrap()).unwrap();
    assert!(!loaded.member_policies[&9].personalized_enabled());
    assert_eq!(loaded, store);
    assert!(loaded.eligibility.is_empty());
    assert!(loaded.guild_features.is_empty());
}

#[test]
fn source_identity_does_not_depend_on_arbitrary_dedupe_string() {
    let mut store = EngagementStore {
        sequence: 2,
        ..Default::default()
    };
    store.candidates.insert(1, candidate(1));
    store.candidates.insert(2, candidate(2));
    assert_eq!(store.validate(), Err(WorkError::Invalid));
}

#[test]
fn malformed_scope_wire_keys_fail_closed() {
    for key in [
        "guild:1:2",
        "guild\u{1f}01\u{1f}2",
        "dm\u{1f}0\u{1f}2",
        "guild\u{1f}1\u{1f}2\u{1f}3",
    ] {
        assert!(serde_json::from_value::<EngagementScope>(serde_json::json!(key)).is_err());
    }
}

#[test]
fn malformed_reservations_and_unknown_fields_fail_loading() {
    let mut store = EngagementStore {
        sequence: 1,
        ..Default::default()
    };
    let mut c = candidate(1);
    c.state = CandidateState::Reserved;
    store.candidates.insert(1, c);
    assert!(
        serde_json::from_value::<EngagementStore>(serde_json::to_value(&store).unwrap()).is_err()
    );
    assert!(
        serde_json::from_value::<EngagementStore>(serde_json::json!({"transcript":"forbidden"}))
            .is_err()
    );
}

#[test]
fn empty_eligibility_buckets_are_bounded() {
    let mut store = EngagementStore::default();
    for member in 1..=10_001 {
        store.eligibility.insert(member, Default::default());
    }
    assert_eq!(store.validate(), Err(WorkError::Full));
    assert!(
        serde_json::from_value::<EngagementStore>(serde_json::to_value(store).unwrap()).is_err()
    );
}
