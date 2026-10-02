use super::*;
fn configured() -> EngagementStore {
    let mut s = EngagementStore::default();
    let scope = EngagementScope::Guild {
        guild: 10,
        channel: 20,
    };
    s.guild_features.insert(
        10,
        GuildFeaturePolicy {
            revision: 1,
            enabled: [CommunityFeature::Introductions].into(),
            channels: [(CommunityFeature::Introductions, [20].into())].into(),
        },
    );
    for member in [1, 2] {
        s.member_policies.insert(
            member,
            MemberPolicy {
                daily_limit: Some(1),
                timezone: Some("UTC".into()),
                quiet_start: 0,
                quiet_end: 0,
                ..Default::default()
            },
        );
        s.eligibility.insert(
            member,
            [SourceRef {
                scope: scope.clone(),
                message: member,
                author: member,
                revision: 1,
                at: 1,
            }]
            .into(),
        );
    }
    s
}
fn ready(s: &mut EngagementStore) -> (u64, u64) {
    let id = s
        .create_introduction(
            [1, 2],
            EngagementScope::Guild {
                guild: 10,
                channel: 20,
            },
            "I build compilers.".into(),
            100,
        )
        .unwrap();
    assert!(!s.approve_introduction(id, 1, 1).unwrap());
    s.edit_introduction(id, 2, 1, "I build runtimes.".into(), None)
        .unwrap();
    assert!(!s.approve_introduction(id, 1, 2).unwrap());
    assert!(s.approve_introduction(id, 2, 2).unwrap());
    (id, *s.candidates.keys().next().unwrap())
}
#[test]
fn introductions_single_wrong_stale_edit_and_private_preview() {
    let mut s = configured();
    let id = s
        .create_introduction(
            [1, 2],
            EngagementScope::Guild {
                guild: 10,
                channel: 20,
            },
            "OWN SECRET".into(),
            100,
        )
        .unwrap();
    let c = *s.candidates.keys().next().unwrap();
    assert!(s.reserve(c, 1, 100).is_err());
    assert!(s.approve_introduction(id, 3, 1).is_err());
    assert!(s.approve_introduction(id, 2, 1).is_err());
    assert!(
        !private_preview(&s.introductions[&id], 2)
            .unwrap()
            .contains("OWN SECRET")
    );
    assert!(private_preview(&s.introductions[&id], 3).is_err());
    s.approve_introduction(id, 1, 1).unwrap();
    s.edit_introduction(id, 2, 1, "Own other description".into(), None)
        .unwrap();
    assert_eq!(s.introductions[&id].approvals, [None; 2]);
    assert!(s.approve_introduction(id, 1, 1).is_err());
    assert!(publication(&s.introductions[&id]).is_err());
    assert!(s.reserve(c, 2, 100).is_err());
}
#[test]
fn introductions_double_capacity_atomic_and_forced_origin() {
    let mut s = configured();
    let (_, c) = ready(&mut s);
    s.member_policies.get_mut(&1).unwrap().destinations.insert(
        EngagementScope::Guild {
            guild: 10,
            channel: 20,
        },
        DestinationPreference::Private,
    );
    let mut exhausted = s.clone();
    exhausted.member_policies.get_mut(&2).unwrap().global_stop = true;
    assert!(exhausted.reserve(c, 2, 100).is_err());
    assert!(exhausted.charges.is_empty());
    assert_eq!(exhausted.candidates[&c].state, CandidateState::Pending);
    let r = s.reserve(c, 2, 100).unwrap();
    assert_eq!(r.destination, DestinationPreference::Origin);
    assert_eq!(s.charges.len(), 2);
    assert!(s.validate_reserved(&r, 100).is_ok());
    s.validate().unwrap();
    assert!(s.reserve(c, 2, 100).is_err());
    let mut blocked = configured();
    let (_, first) = ready(&mut blocked);
    blocked.reserve(first, 2, 100).unwrap();
    // Member 1 is exhausted while the new second member still has capacity.
    blocked
        .member_policies
        .insert(3, blocked.member_policies[&2].clone());
    let id = blocked
        .create_introduction(
            [1, 3],
            EngagementScope::Guild {
                guild: 10,
                channel: 20,
            },
            "one".into(),
            100,
        )
        .unwrap();
    blocked
        .edit_introduction(id, 3, 1, "three".into(), None)
        .unwrap();
    blocked.approve_introduction(id, 1, 2).unwrap();
    blocked.approve_introduction(id, 3, 2).unwrap();
    let c = blocked
        .candidates
        .values()
        .find(|c| c.introduction_id == Some(id))
        .unwrap()
        .id;
    let n = blocked.charges.len();
    assert!(blocked.reserve(c, 2, 100).is_err());
    assert_eq!(blocked.charges.len(), n);
    assert!(!blocked.charges.iter().any(|c| c.member == 3));
}
#[test]
fn introductions_withdraw_reserved_policy_change_restart_uncertainty() {
    let mut s = configured();
    let (id, c) = ready(&mut s);
    let r = s.reserve(c, 2, 100).unwrap();
    let bytes = serde_json::to_vec(&s).unwrap();
    let recovered: EngagementStore = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        recovered.candidates[&c].state,
        CandidateState::ReviewRequired
    );
    s.member_policies.get_mut(&2).unwrap().revision += 1;
    assert!(s.validate_reserved(&r, 100).is_err());
    s.withdraw_introduction(id, 2, 2).unwrap();
    assert_eq!(s.candidates[&c].state, CandidateState::Cancelled);
    assert!(s.validate_reserved(&r, 100).is_err());
    s.validate().unwrap();
    s.settle(c, lifecycle::DeliveryOutcome::ReviewRequired)
        .unwrap();
    assert_eq!(s.candidates[&c].state, CandidateState::ReviewRequired);
    s.validate().unwrap();
    assert!(
        s.create_introduction(
            [1, 2],
            EngagementScope::Guild {
                guild: 10,
                channel: 20
            },
            "repeat".into(),
            101
        )
        .is_err()
    );
}
#[test]
fn introductions_bounds_cross_guild_feature_and_double_charge_loading() {
    let mut s = configured();
    assert!(
        s.create_introduction(
            [1, 2],
            EngagementScope::Dm {
                member: 1,
                channel: 20
            },
            "own".into(),
            100
        )
        .is_err()
    );
    assert!(description_valid(&"🦀".repeat(300)));
    assert!(!description_valid(&"🦀".repeat(301)));
    s.member_policies.get_mut(&2).unwrap().daily_limit = None;
    assert!(
        s.create_introduction(
            [1, 2],
            EngagementScope::Guild {
                guild: 10,
                channel: 20
            },
            "own".into(),
            100
        )
        .is_err()
    );
    let mut s = configured();
    let (_, c) = ready(&mut s);
    let r = s.reserve(c, 2, 100).unwrap();
    s.guild_features.get_mut(&10).unwrap().revision += 1;
    assert!(s.validate_reserved(&r, 100).is_err());
    s.charges.pop();
    assert!(s.validate().is_err());
}
#[test]
fn introductions_publication_exact_approved_copy_and_duplicate_approval() {
    let mut s = configured();
    let (id, c) = ready(&mut s);
    assert!(s.approve_introduction(id, 2, 2).unwrap());
    assert_eq!(s.candidates.len(), 1);
    s.reserve(c, 2, 100).unwrap();
    let copy = publication(&s.introductions[&id]).unwrap();
    assert!(copy.contains("I build compilers."));
    assert!(copy.contains("I build runtimes."));
    assert!(!copy.contains("<@"));
    assert!(
        s.edit_introduction(id, 1, 2, "changed".into(), None)
            .is_err()
    );
}

#[test]
fn introductions_destination_change_clears_both_approvals_and_disabled_default_refuses() {
    let mut s = configured();
    let (id, c) = ready(&mut s);
    s.guild_features
        .get_mut(&10)
        .unwrap()
        .channels
        .get_mut(&CommunityFeature::Introductions)
        .unwrap()
        .insert(21);
    s.edit_introduction(id, 1, 2, "I build compilers.".into(), Some(21))
        .unwrap();
    assert_eq!(s.introductions[&id].approvals, [None; 2]);
    assert_eq!(s.introductions[&id].destination, 21);
    assert_eq!(
        s.candidates[&c].scope,
        EngagementScope::Guild {
            guild: 10,
            channel: 21
        }
    );
    assert!(s.approve_introduction(id, 2, 2).is_err());
    assert!(s.reserve(c, 3, 100).is_err());
    s.guild_features.clear();
    assert!(s.approve_introduction(id, 1, 3).is_err());
    let mut default = EngagementStore::default();
    assert!(
        default
            .create_introduction(
                [1, 2],
                EngagementScope::Guild {
                    guild: 10,
                    channel: 20
                },
                "own".into(),
                100
            )
            .is_err()
    );
}
