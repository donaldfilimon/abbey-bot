use super::*;
const NOW: u64 = 1_790_683_200;
#[test]
fn community_public_reservation_needs_guild_authority_without_member_policy() {
    let mut s = EngagementStore::default();
    s.guild_features.insert(
        7,
        GuildFeaturePolicy {
            revision: 1,
            enabled: BTreeSet::from([CommunityFeature::Questions]),
            channels: BTreeMap::from([(CommunityFeature::Questions, BTreeSet::from([9]))]),
        },
    );
    let id = s
        .propose(
            schedule::CandidateProposal {
                kind: EngagementKind::UnansweredQuestion,
                source: Some(SourceRef {
                    scope: EngagementScope::Guild {
                        guild: 7,
                        channel: 9,
                    },
                    message: 10,
                    author: 2,
                    revision: 1,
                    at: NOW - 86_400,
                }),
                member: None,
                scope: EngagementScope::Guild {
                    guild: 7,
                    channel: 9,
                },
                due_at: NOW,
                introduction_id: None,
            },
            NOW,
        )
        .unwrap()
        .unwrap();
    s.community_receipts.insert(
        id,
        CommunityReceipt {
            policy_revision: 1,
            attempted_at: None,
            evidence: CommunityEvidence::Message,
        },
    );
    assert!(s.reserve(id, 1, NOW).is_ok());
}
fn scope() -> EngagementScope {
    EngagementScope::Guild {
        guild: 7,
        channel: 9,
    }
}
fn fact(kind: EngagementKind) -> CommunityFact {
    CommunityFact {
        kind,
        scope: scope(),
        source: Some(SourceRef {
            scope: scope(),
            message: 10,
            author: 2,
            revision: 1,
            at: NOW - 86_400,
        }),
        evidence: CommunityEvidence::Message,
        at: NOW - 86_400,
        useful: true,
        current: true,
    }
}
fn configured(kind: EngagementKind) -> EngagementStore {
    let f = feature(kind).unwrap();
    let mut s = EngagementStore::default();
    s.guild_features.insert(
        7,
        GuildFeaturePolicy {
            revision: 1,
            enabled: BTreeSet::from([f]),
            channels: BTreeMap::from([(f, BTreeSet::from([9]))]),
        },
    );
    s
}
fn facts(f: CommunityFact) -> CommunityFacts {
    CommunityFacts {
        rows: vec![f],
        join_events_available: true,
    }
}
#[test]
fn community_switches_are_independent_default_off_and_allowlisted() {
    for kind in [
        EngagementKind::ConversationStarter,
        EngagementKind::UnansweredQuestion,
        EngagementKind::Welcome,
        EngagementKind::ProjectCheckIn,
    ] {
        let mut row = fact(kind);
        if kind == EngagementKind::Welcome {
            row.source = None;
            row.evidence = CommunityEvidence::Join {
                member: 2,
                joined_at: NOW,
            };
            row.at = NOW;
        }
        if kind == EngagementKind::ProjectCheckIn {
            row.source = None;
            row.evidence = project_evidence();
            row.at = NOW;
        }
        let fs = facts(row);
        let mut s = configured(kind);
        s.member_policies.insert(
            2,
            MemberPolicy {
                daily_limit: Some(1),
                timezone: Some("UTC".into()),
                quiet_start: 0,
                quiet_end: 0,
                ..Default::default()
            },
        );
        assert!(community_candidates(&EngagementStore::default(), &fs, NOW).is_empty());
        assert_eq!(community_candidates(&s, &fs, NOW).len(), 1);
        for f in [
            CommunityFeature::Starters,
            CommunityFeature::Questions,
            CommunityFeature::Welcomes,
            CommunityFeature::Projects,
        ]
        .into_iter()
        .filter(|f| Some(*f) != feature(kind))
        {
            let mut other = s.clone();
            other.guild_features.get_mut(&7).unwrap().enabled = BTreeSet::from([f]);
            assert!(community_candidates(&other, &fs, NOW).is_empty());
        }
        s.guild_features
            .get_mut(&7)
            .unwrap()
            .channels
            .get_mut(&feature(kind).unwrap())
            .unwrap()
            .clear();
        assert!(community_candidates(&s, &fs, NOW).is_empty());
    }
}
fn project_evidence() -> CommunityEvidence {
    CommunityEvidence::Project {
        project: 5,
        revision: 1,
        actor: 2,
        audience: BTreeSet::from([2]),
        content: BTreeSet::from([crate::work::WorkContentRef::Task {
            project: 5,
            id: 6,
            revision: 1,
        }]),
    }
}
#[test]
fn community_requires_useful_current_old_human_source_and_exact_scope() {
    for alter in 0..5 {
        let mut row = fact(EngagementKind::UnansweredQuestion);
        match alter {
            0 => row.useful = false,
            1 => row.current = false,
            2 => {
                row.at = NOW - 86399;
                row.source.as_mut().unwrap().at = row.at;
            }
            3 => {
                row.source.as_mut().unwrap().scope = EngagementScope::Guild {
                    guild: 8,
                    channel: 9,
                }
            }
            _ => row.source.as_mut().unwrap().author = 0,
        }
        assert!(community_candidates(&configured(row.kind), &facts(row), NOW).is_empty());
    }
}
#[test]
fn community_duplicate_join_and_absent_capability_never_create_again() {
    let mut s = configured(EngagementKind::Welcome);
    let mut row = fact(EngagementKind::Welcome);
    row.source = None;
    row.evidence = CommunityEvidence::Join {
        member: 2,
        joined_at: NOW,
    };
    row.at = NOW;
    let mut fs = facts(row);
    fs.join_events_available = false;
    assert_eq!(s.propose_community(&fs, NOW).unwrap(), 0);
    fs.join_events_available = true;
    assert_eq!(s.propose_community(&fs, NOW).unwrap(), 1);
    assert_eq!(s.propose_community(&fs, NOW).unwrap(), 0);
    let r = s.reserve(1, 1, NOW).unwrap();
    s.validate_reserved(&r, NOW).unwrap();
    s.settle(1, lifecycle::DeliveryOutcome::ReviewRequired)
        .unwrap();
    let mut loaded: EngagementStore =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(loaded.propose_community(&fs, NOW).unwrap(), 0);
    assert!(loaded.charges.is_empty());
}
#[test]
fn community_starter_floor_and_question_source_dedupe_survive_cancel() {
    for kind in [
        EngagementKind::ConversationStarter,
        EngagementKind::UnansweredQuestion,
    ] {
        let mut s = configured(kind);
        let fs = facts(fact(kind));
        assert_eq!(s.propose_community(&fs, NOW).unwrap(), 1);
        s.cancel_public_origin(&scope(), NOW);
        assert_eq!(s.propose_community(&fs, NOW + 86_400).unwrap(), 0);
        let mut new = fs.clone();
        new.rows[0].source.as_mut().unwrap().message = 11;
        assert_eq!(
            s.propose_community(&new, NOW + 1).unwrap(),
            usize::from(kind == EngagementKind::UnansweredQuestion)
        );
    }
}
#[test]
fn community_public_policy_change_after_reserve_rejects_and_restart_consumes() {
    let mut s = configured(EngagementKind::UnansweredQuestion);
    s.propose_community(&facts(fact(EngagementKind::UnansweredQuestion)), NOW)
        .unwrap();
    let r = s.reserve(1, 1, NOW).unwrap();
    let loaded: EngagementStore =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(loaded.candidates[&1].state, CandidateState::ReviewRequired);
    s.guild_features.get_mut(&7).unwrap().revision += 1;
    assert_eq!(s.validate_reserved(&r, NOW), Err(WorkError::Stale));
    s.settle(1, lifecycle::DeliveryOutcome::Rejected).unwrap();
    assert!(s.reserve(1, 1, NOW).is_err());
    assert!(s.charges.is_empty());
}
#[test]
fn community_work_requires_member_controls_and_weekly_identity() {
    let mut s = configured(EngagementKind::ProjectCheckIn);
    let mut row = fact(EngagementKind::ProjectCheckIn);
    row.source = None;
    row.evidence = project_evidence();
    row.at = NOW;
    let fs = facts(row);
    assert_eq!(s.propose_community(&fs, NOW).unwrap(), 0);
    s.member_policies.insert(
        2,
        MemberPolicy {
            revision: 1,
            daily_limit: Some(1),
            timezone: Some("UTC".into()),
            quiet_start: 0,
            quiet_end: 0,
            ..Default::default()
        },
    );
    assert_eq!(s.propose_community(&fs, NOW).unwrap(), 1);
    let r = s.reserve(1, 1, NOW).unwrap();
    s.validate_reserved(&r, NOW).unwrap();
    assert_eq!(s.charges.len(), 1);
    s.settle(1, lifecycle::DeliveryOutcome::Sent { message_id: 12 })
        .unwrap();
    assert_eq!(s.propose_community(&fs, NOW + WEEK - 1).unwrap(), 0);
    assert_eq!(s.propose_community(&fs, NOW + WEEK).unwrap(), 1);
    s.member_policies.get_mut(&2).unwrap().global_stop = true;
    assert!(s.reserve(2, 1, NOW + WEEK).is_err());
}
#[test]
fn community_receipt_is_bounded_metadata_and_malformed_loading_refuses() {
    let mut s = configured(EngagementKind::Welcome);
    let mut row = fact(EngagementKind::Welcome);
    row.source = None;
    row.evidence = CommunityEvidence::Join {
        member: 2,
        joined_at: NOW,
    };
    row.at = NOW;
    s.propose_community(&facts(row), NOW).unwrap();
    let mut json = serde_json::to_value(&s).unwrap();
    json["community_receipts"]["1"]["evidence"]["Join"]["body"] = serde_json::json!("raw text");
    assert!(serde_json::from_value::<EngagementStore>(json).is_err());
    s.community_receipts.get_mut(&1).unwrap().evidence = CommunityEvidence::Join {
        member: 0,
        joined_at: NOW,
    };
    assert!(s.validate().is_err());
}
#[test]
fn community_delayed_starter_reservation_keeps_actual_attempt_floor() {
    let mut s = configured(EngagementKind::ConversationStarter);
    let mut fs = facts(fact(EngagementKind::ConversationStarter));
    s.propose_community(&fs, NOW).unwrap();
    let r = s.reserve(1, 1, NOW + 86_400).unwrap();
    s.settle(
        r.candidate_id,
        lifecycle::DeliveryOutcome::Sent { message_id: 12 },
    )
    .unwrap();
    fs.rows[0].source.as_mut().unwrap().message = 11;
    assert_eq!(s.propose_community(&fs, NOW + 86_401).unwrap(), 0);
    assert_eq!(s.propose_community(&fs, NOW + 172_800).unwrap(), 1);
}
#[test]
fn community_join_identity_survives_destination_change() {
    let mut s = configured(EngagementKind::Welcome);
    let mut row = fact(EngagementKind::Welcome);
    row.source = None;
    row.evidence = CommunityEvidence::Join {
        member: 2,
        joined_at: NOW,
    };
    row.at = NOW;
    s.propose_community(&facts(row.clone()), NOW).unwrap();
    s.guild_features
        .get_mut(&7)
        .unwrap()
        .channels
        .insert(CommunityFeature::Welcomes, BTreeSet::from([19]));
    row.scope = EngagementScope::Guild {
        guild: 7,
        channel: 19,
    };
    assert_eq!(s.propose_community(&facts(row), NOW).unwrap(), 0);
}
#[test]
fn community_pending_old_and_new_starters_cannot_reserve_within_actual_floor() {
    let mut s = configured(EngagementKind::ConversationStarter);
    let mut fs = facts(fact(EngagementKind::ConversationStarter));
    s.propose_community(&fs, NOW).unwrap();
    fs.rows[0].source.as_mut().unwrap().message = 11;
    s.propose_community(&fs, NOW + 86_400).unwrap();
    assert_eq!(s.candidates.len(), 2);
    s.reserve(1, 1, NOW + 86_400).unwrap();
    assert_eq!(s.reserve(2, 1, NOW + 86_401), Err(WorkError::Denied));
}
#[test]
fn community_public_project_does_not_inherit_private_followup_destination() {
    let mut s = configured(EngagementKind::ProjectCheckIn);
    let mut policy = MemberPolicy {
        daily_limit: Some(1),
        timezone: Some("UTC".into()),
        quiet_start: 0,
        quiet_end: 0,
        ..Default::default()
    };
    policy
        .destinations
        .insert(scope(), DestinationPreference::Private);
    s.member_policies.insert(2, policy);
    let mut row = fact(EngagementKind::ProjectCheckIn);
    row.source = None;
    row.evidence = project_evidence();
    row.at = NOW;
    assert_eq!(s.propose_community(&facts(row), NOW).unwrap(), 1);
    assert_eq!(s.candidates[&1].destination, DestinationPreference::Origin);
    assert_eq!(
        s.reserve(1, 1, NOW).unwrap().destination,
        DestinationPreference::Origin
    );
}
