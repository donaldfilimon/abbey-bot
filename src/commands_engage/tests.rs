use super::controls::*;
use crate::engagement::*;
#[test]
fn engage_defaults_and_configuration_are_explicit() {
    let mut s = EngagementStore::default();
    let origin = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    assert!(!MemberPolicy::default().personalized_enabled());
    assert!(configure(&mut s, 7, &origin, 0, "UTC".into(), None, None, None, None).is_err());
    assert!(s.member_policies.is_empty());
    configure(&mut s, 7, &origin, 2, "UTC".into(), None, None, None, None).unwrap();
    assert!(s.member_policies[&7].personalized_enabled());
    assert!(!s.member_policies.contains_key(&8));
}
#[test]
fn engage_stops_span_guild_and_scoped_resume_keeps_global_stop() {
    let mut p = MemberPolicy::default();
    let a = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let b = EngagementScope::Guild {
        guild: 1,
        channel: 3,
    };
    let other = EngagementScope::Guild {
        guild: 9,
        channel: 3,
    };
    set_stop(&mut p, &a, StopScope::CurrentServer, true).unwrap();
    assert!(blocked_scope(&p, &a));
    assert!(blocked_scope(&p, &b));
    assert!(!blocked_scope(&p, &other));
    set_stop(&mut p, &a, StopScope::Global, true).unwrap();
    set_stop(&mut p, &a, StopScope::CurrentServer, false).unwrap();
    assert!(p.global_stop);
    assert!(
        set_stop(
            &mut p,
            &EngagementScope::Dm {
                member: 7,
                channel: 2
            },
            StopScope::CurrentServer,
            true
        )
        .is_err()
    );
}
#[test]
fn engage_community_requires_fresh_manager_and_explicit_channel() {
    let mut s = EngagementStore::default();
    assert!(set_community(&mut s, 1, false, CommunityFeature::Starters, true, Some(2)).is_err());
    assert!(set_community(&mut s, 1, true, CommunityFeature::Starters, true, None).is_err());
    assert!(s.guild_features.is_empty());
}
#[test]
fn engage_representative_private_status() {
    for origin in [
        EngagementScope::Guild {
            guild: 1,
            channel: 2,
        },
        EngagementScope::Dm {
            member: 7,
            channel: 2,
        },
    ] {
        let text = render_status(&EngagementStore::default(), 7, &origin, 10);
        println!("{text}");
        assert!(text.contains("disabled"));
        assert!(text.contains("daily limit"));
    }
}
fn receipt(state: CandidateState) -> EngagementStore {
    let mut s = EngagementStore {
        sequence: 1,
        ..Default::default()
    };
    s.candidates.insert(
        1,
        Candidate {
            id: 1,
            kind: EngagementKind::WeeklyCheckIn,
            source: None,
            member: Some(7),
            scope: EngagementScope::Dm {
                member: 7,
                channel: 2,
            },
            due_at: 10,
            revision: 1,
            state,
            dedupe_key: "weekly-1".into(),
            policy_revision: 0,
            destination: DestinationPreference::Origin,
            message_id: (state == CandidateState::Sent).then_some(88),
            introduction_id: None,
        },
    );
    if matches!(state, CandidateState::Reserved | CandidateState::Sent) {
        s.charges.push(ContactCharge {
            candidate_id: 1,
            member: 7,
            local_day: "2026-10-01".into(),
            local_week: "2026-09-28".into(),
            at: 10,
        });
    }
    s
}
#[test]
fn engage_feedback_is_explicit_own_sent_and_deduplicated() {
    let mut s = receipt(CandidateState::Sent);
    assert!(feedback(&mut s, 8, 1, FeedbackKind::Useful, 11).is_err());
    feedback(&mut s, 7, 1, FeedbackKind::Useful, 11).unwrap();
    assert!(feedback(&mut s, 7, 1, FeedbackKind::Dismissed, 12).is_err());
    assert_eq!(s.feedback[&1][&7].kind, FeedbackKind::Useful);
    assert!(s.validate().is_ok());
    let json = serde_json::to_vec(&s).unwrap();
    assert_eq!(serde_json::from_slice::<EngagementStore>(&json).unwrap(), s);
    s.feedback.get_mut(&1).unwrap().get_mut(&7).unwrap().actor = 8;
    assert!(serde_json::from_value::<EngagementStore>(serde_json::to_value(s).unwrap()).is_err());
    let mut pending = receipt(CandidateState::Pending);
    assert!(feedback(&mut pending, 7, 1, FeedbackKind::Useful, 11).is_err());
}
#[test]
fn engage_dismiss_reserved_keeps_charge_and_rejects_other_member() {
    let mut s = receipt(CandidateState::Reserved);
    assert!(dismiss(&mut s, 8, 1).is_err());
    dismiss(&mut s, 7, 1).unwrap();
    assert_eq!(s.candidates[&1].state, CandidateState::Cancelled);
    assert_eq!(s.candidates[&1].revision, 2);
    assert_eq!(s.charges.len(), 1);
    assert!(s.validate().is_ok());
}
#[test]
fn engage_weekly_alone_never_configures_contact() {
    let mut s = EngagementStore::default();
    let origin = EngagementScope::Dm {
        member: 7,
        channel: 2,
    };
    let p = MemberPolicy {
        weekly_subscription: Some(WeeklySubscription {
            weekday: 1,
            hour: 9,
            scope: origin.clone(),
            destination: DestinationPreference::Origin,
        }),
        ..Default::default()
    };
    save_policy(&mut s, 7, p).unwrap();
    assert!(!s.member_policies[&7].personalized_enabled());
    let text = render_status(&s, 7, &origin, 10);
    println!("{text}");
    assert!(text.contains("Eligibility: established"));
}
#[test]
fn engage_stopped_guild_zero_is_invalid_and_roundtrips() {
    let mut p = MemberPolicy::default();
    p.stopped_guilds.insert(0);
    assert!(p.validate().is_err());
    p.stopped_guilds.clear();
    p.stopped_guilds.insert(5);
    assert_eq!(
        serde_json::from_value::<MemberPolicy>(serde_json::to_value(&p).unwrap()).unwrap(),
        p
    );
}
#[test]
fn engage_weekly_requires_explicit_timing_and_captures_only_origin_destination() {
    let a = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let b = EngagementScope::Guild {
        guild: 1,
        channel: 3,
    };
    let mut p = MemberPolicy::default();
    p.destinations
        .insert(a.clone(), DestinationPreference::Private);
    assert!(set_weekly(&mut p, a.clone(), true, None, Some(9)).is_err());
    assert!(p.weekly_subscription.is_none());
    set_weekly(&mut p, a.clone(), true, Some(1), Some(9)).unwrap();
    assert_eq!(
        p.weekly_subscription.as_ref().unwrap().destination,
        DestinationPreference::Private
    );
    assert_eq!(p.weekly_subscription.as_ref().unwrap().scope, a);
    assert!(!p.destinations.contains_key(&b));
    assert!(!p.personalized_enabled());
}
#[test]
fn engage_configure_requires_limit_and_timezone_in_registered_adapter() {
    let c = super::configure();
    for name in ["daily_limit", "timezone"] {
        assert!(
            c.parameters
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .required
        );
    }
    assert!(
        c.parameters
            .iter()
            .all(|p| !["actor", "member", "user"].contains(&p.name.as_str()))
    );
}
#[test]
fn engage_stop_consumes_reserved_and_resume_never_rearms_it() {
    let mut s = receipt(CandidateState::Reserved);
    let origin = s.candidates[&1].scope.clone();
    stop_store(&mut s, 7, &origin, StopScope::CurrentConversation).unwrap();
    assert_eq!(s.candidates[&1].state, CandidateState::Cancelled);
    assert_eq!(s.candidates[&1].revision, 2);
    assert_eq!(s.charges.len(), 1);
    let dedupe = s.candidates[&1].dedupe_key.clone();
    let p = s.member_policies.get_mut(&7).unwrap();
    set_stop(p, &origin, StopScope::CurrentConversation, false).unwrap();
    assert_eq!(s.candidates[&1].state, CandidateState::Cancelled);
    assert_eq!(s.candidates[&1].dedupe_key, dedupe);
}
#[test]
fn engage_guild_stop_cancels_two_channels_preserves_other_guild_and_member() {
    let mut s = receipt(CandidateState::Pending);
    s.sequence = 4;
    s.candidates.get_mut(&1).unwrap().scope = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    for (id, guild, channel, member) in [(2, 1, 3, 7), (3, 9, 3, 7), (4, 1, 3, 8)] {
        let mut c = s.candidates[&1].clone();
        c.id = id;
        c.scope = EngagementScope::Guild { guild, channel };
        c.member = Some(member);
        c.dedupe_key = format!("weekly-{id}");
        s.candidates.insert(id, c);
    }
    stop_store(
        &mut s,
        7,
        &EngagementScope::Guild {
            guild: 1,
            channel: 2,
        },
        StopScope::CurrentServer,
    )
    .unwrap();
    assert_eq!(s.candidates[&1].state, CandidateState::Cancelled);
    assert_eq!(s.candidates[&2].state, CandidateState::Cancelled);
    assert_eq!(s.candidates[&3].state, CandidateState::Pending);
    assert_eq!(s.candidates[&4].state, CandidateState::Pending);
}
#[test]
fn engage_stop_invalidates_pending_introduction_approvals_and_reservation() {
    let origin = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let mut s = EngagementStore {
        sequence: 2,
        ..Default::default()
    };
    s.introductions.insert(
        1,
        Introduction {
            id: 1,
            revision: 1,
            scope: origin.clone(),
            members: [7, 8],
            approved_self_descriptions: [Some("a".into()), Some("b".into())],
            approvals: [Some(1); 2],
            destination: 2,
            state: IntroductionState::Ready,
        },
    );
    let mut c = receipt(CandidateState::Pending)
        .candidates
        .remove(&1)
        .unwrap();
    c.id = 2;
    c.kind = EngagementKind::Introduction;
    c.member = None;
    c.scope = origin.clone();
    c.introduction_id = Some(1);
    s.candidates.insert(2, c);
    stop_store(&mut s, 7, &origin, StopScope::Global).unwrap();
    assert_eq!(s.introductions[&1].state, IntroductionState::Cancelled);
    assert_eq!(s.introductions[&1].approvals, [None; 2]);
    assert_eq!(s.candidates[&2].state, CandidateState::Cancelled);
    assert!(s.validate().is_ok());
}

#[test]
fn engage_receipt_counts_are_private_closed_and_include_both_introduction_members() {
    let mut s = receipt(CandidateState::Sent);
    s.introductions.insert(
        99,
        Introduction {
            id: 99,
            revision: 1,
            scope: EngagementScope::Guild {
                guild: 3,
                channel: 4,
            },
            members: [7, 8],
            approved_self_descriptions: [
                Some("own approved description".into()),
                Some("PRIVATE_CANARY".into()),
            ],
            approvals: [Some(1), Some(1)],
            destination: 4,
            state: IntroductionState::Consumed,
        },
    );
    s.candidates.get_mut(&1).unwrap().introduction_id = Some(99);
    for actor in [7, 8] {
        let text = render_status(
            &s,
            actor,
            &EngagementScope::Guild {
                guild: 3,
                channel: 4,
            },
            10,
        );
        assert!(text.contains("sent (confirmed Discord receipt): 1"));
        for forbidden in ["PRIVATE_CANARY", "own approved description", "99", "88"] {
            assert!(!text.contains(forbidden));
        }
    }
    let outsider = render_status(
        &s,
        9,
        &EngagementScope::Guild {
            guild: 3,
            channel: 4,
        },
        10,
    );
    assert!(outsider.contains("sent (confirmed Discord receipt): 0"));
    for state in [
        CandidateState::Pending,
        CandidateState::Reserved,
        CandidateState::Cancelled,
        CandidateState::Rejected,
        CandidateState::ReviewRequired,
    ] {
        let row = receipt(state);
        let text = render_status(
            &row,
            7,
            &EngagementScope::Dm {
                member: 7,
                channel: 2,
            },
            10,
        );
        assert!(text.contains(": 1"));
        assert!(!text.contains("weekly-1"));
    }
    let policies = s.member_policies.clone();
    feedback(&mut s, 8, 1, FeedbackKind::Dismissed, 11).unwrap();
    feedback(&mut s, 7, 1, FeedbackKind::Useful, 11).unwrap();
    assert!(feedback(&mut s, 8, 1, FeedbackKind::Useful, 12).is_err());
    assert_eq!(s.member_policies, policies);
    assert_eq!(s.candidates[&1].state, CandidateState::Sent);
}
