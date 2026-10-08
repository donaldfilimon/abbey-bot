use super::*;
const NOW: u64 = 1_790_683_200;
fn scope(guild: u64) -> EngagementScope {
    EngagementScope::Guild { guild, channel: 2 }
}
fn policy() -> MemberPolicy {
    MemberPolicy {
        revision: 1,
        daily_limit: Some(1),
        weekly_limit: Some(1),
        timezone: Some("UTC".into()),
        quiet_start: 0,
        quiet_end: 0,
        ..Default::default()
    }
}
fn source(guild: u64, member: u64, message: u64) -> SourceRef {
    SourceRef {
        scope: scope(guild),
        message,
        author: member,
        revision: 1,
        at: NOW - 10,
    }
}
fn seed() -> EngagementStore {
    let mut s = EngagementStore::default();
    for member in [7, 8] {
        s.member_policies.insert(member, policy());
    }
    for (g, m, id) in [(1, 7, 70), (1, 8, 80), (3, 7, 71)] {
        let source = source(g, m, id);
        s.observations
            .entry(source.scope.clone())
            .or_default()
            .insert(m, source.clone());
        s.eligibility.entry(m).or_default().insert(source);
    }
    s
}
fn proposal() -> schedule::CandidateProposal {
    schedule::CandidateProposal {
        kind: EngagementKind::FollowUp,
        source: Some(source(1, 7, 70)),
        member: Some(7),
        scope: scope(1),
        due_at: NOW,
        introduction_id: None,
    }
}
#[test]
fn erasure_uncertain_candidate_reopen_preserves_ceiling_and_never_replays() {
    let mut s = seed();
    let id = s.propose(proposal(), NOW).unwrap().unwrap();
    s.reserve(id, 1, NOW).unwrap();
    let preserved = s.eligibility[&8].clone();
    let other = s.eligibility[&7]
        .iter()
        .find(|r| r.scope == scope(3))
        .unwrap()
        .clone();
    let charge = s.charges[0].clone();
    s.erase_learning("discord:1", Some(7)).unwrap();
    assert!(!s.candidates.contains_key(&id));
    assert!(s.charges.is_empty());
    assert_eq!(s.erased_contact_charges.len(), 1);
    assert_eq!(s.eligibility[&8], preserved);
    assert!(s.eligibility[&7].contains(&other));
    assert_eq!(
        s.capacity(
            7,
            NOW,
            &(charge.local_day.clone(), charge.local_week.clone())
        ),
        Err(WorkError::Denied)
    );
    s.validate().unwrap();
    let mut restored: EngagementStore =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    restored
        .eligibility
        .entry(7)
        .or_default()
        .insert(source(1, 7, 70));
    assert_eq!(
        restored.propose(proposal(), NOW + 301),
        Err(WorkError::Stale)
    );
    assert_eq!(
        restored.propose(proposal(), NOW + 10 * 86400),
        Err(WorkError::Stale)
    );
    restored.erase_learning("discord:1", Some(7)).unwrap();
    assert_eq!(restored.erased_contact_charges.len(), 1);
    restored.member_policies.get_mut(&7).unwrap().timezone = Some("America/New_York".into());
    assert_eq!(
        restored.capacity(7, NOW, &(charge.local_day, charge.local_week)),
        Err(WorkError::Denied)
    );
}
#[test]
fn erasure_invitation_and_introduction_suppression_survive_quota_rollover() {
    let mut s = seed();
    let req = InvitationRequest {
        activity: None,
        interaction: 999,
        member: 7,
        scope: scope(1),
        at: NOW,
    };
    let id = s
        .request_invitation(EngagementKind::VoiceInvite, req.clone())
        .unwrap()
        .unwrap();
    s.reserve(id, 1, NOW).unwrap();
    s.erase_learning("discord:1", Some(7)).unwrap();
    let mut s: EngagementStore = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    let mut late = req;
    late.at = NOW + 10 * 86400;
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, late),
        Err(WorkError::Stale)
    );
    s.guild_features.insert(
        1,
        GuildFeaturePolicy {
            revision: 1,
            enabled: [CommunityFeature::Introductions].into_iter().collect(),
            channels: [(CommunityFeature::Introductions, [2].into_iter().collect())]
                .into_iter()
                .collect(),
        },
    );
    let id = s
        .create_introduction([7, 8], scope(1), "My own project".into(), NOW + 10 * 86400)
        .unwrap();
    assert!(s.introductions.contains_key(&id));
    s.erase_learning("discord:1", Some(7)).unwrap();
    let mut s: EngagementStore = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(
        s.create_introduction([8, 7], scope(1), "new text".into(), NOW + 20 * 86400),
        Err(WorkError::Stale)
    );
}
#[test]
fn erasure_marker_capacity_refuses_atomically() {
    let mut s = seed();
    s.propose(proposal(), NOW).unwrap();
    s.erased_identities = (0..10_000).map(|n| format!("{n:064x}")).collect();
    let before = s.clone();
    assert_eq!(s.erase_learning("discord:1", Some(7)), Err(WorkError::Full));
    assert_eq!(s, before);
}
#[test]
fn erasure_safety_pruning_is_dst_boundary_and_clock_rollback_safe() {
    let mut s = seed();
    let id = s.propose(proposal(), NOW).unwrap().unwrap();
    s.reserve(id, 1, NOW).unwrap();
    s.erase_learning("discord:1", Some(7)).unwrap();
    for at in [NOW - 1, NOW + 7 * 86400, NOW + 9 * 86400] {
        s.prune_erasure_safety(at);
        assert_eq!(s.erased_contact_charges.len(), 1);
    }
    s.prune_erasure_safety(NOW + 10 * 86400);
    assert!(s.erased_contact_charges.is_empty());
    assert_eq!(
        s.capacity(7, NOW, &("2026-09-29".into(), "2026-09-28".into())),
        Err(WorkError::Denied)
    );
    // Saved labels across DST/timezone changes can outlive elapsed accounting.
    s.erased_contact_charges.push(ErasedContactCharge {
        member: 7,
        local_day: "2030-11-03".into(),
        local_week: "2030-10-28".into(),
        at: NOW,
    });
    s.prune_erasure_safety(NOW + 20 * 86400);
    assert_eq!(s.erased_contact_charges.len(), 1);
}

#[test]
fn erasure_dst_week_and_community_attempt_windows_remain_protected() {
    use chrono::TimeZone;
    let tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
    let at = tz
        .with_ymd_and_hms(2026, 11, 1, 1, 30, 0)
        .earliest()
        .unwrap()
        .timestamp() as u64;
    let mut s = seed();
    s.member_policies.get_mut(&7).unwrap().timezone = Some("America/New_York".into());
    s.erased_contact_charges.push(ErasedContactCharge {
        member: 7,
        local_day: "2026-11-01".into(),
        local_week: "2026-10-26".into(),
        at,
    });
    assert_eq!(
        s.capacity(7, at + 3600, &("2026-11-01".into(), "2026-10-26".into())),
        Err(WorkError::Denied)
    );
    s.prune_erasure_safety(at + 7 * 86400);
    assert_eq!(s.erased_contact_charges.len(), 1);
    let mut c = Candidate {
        id: 99,
        kind: EngagementKind::ConversationStarter,
        source: None,
        member: None,
        scope: scope(1),
        due_at: at,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: "test".into(),
        policy_revision: 1,
        destination: DestinationPreference::Origin,
        message_id: None,
        introduction_id: None,
        work_ref: None,
        expires_at: None,
        follow_up_reason: None,
    };
    s.erased_community_charges.push(ErasedCommunityCharge {
        project: None,
        scope: scope(1),
        kind: c.kind,
        at,
    });
    assert!(!s.community_capacity(&c, at + 86399));
    assert!(s.community_capacity(&c, at + 86400));
    c.scope = scope(3);
    assert!(s.community_capacity(&c, at));
}

#[test]
fn erasure_original_public_actor_audience_source_and_cursor_are_fenced() {
    let mut ledger = crate::brain::erasure::ErasureLedger::default();
    ledger.insert("discord:1", Some("discord:7"), NOW).unwrap();
    for evidence in [
        community::CommunityEvidence::Message,
        community::CommunityEvidence::Join {
            member: 7,
            joined_at: NOW - 1,
        },
        community::CommunityEvidence::Project {
            project: 1,
            revision: 1,
            actor: 8,
            audience: BTreeSet::from([7, 8]),
            content: BTreeSet::new(),
        },
    ] {
        let before = EngagementStore::default();
        let mut after = before.clone();
        after.candidates.insert(
            1,
            Candidate {
                id: 1,
                kind: EngagementKind::Welcome,
                scope: scope(1),
                member: None,
                source: matches!(evidence, community::CommunityEvidence::Message)
                    .then(|| source(1, 7, 70)),
                due_at: NOW,
                revision: 1,
                state: CandidateState::Pending,
                dedupe_key: String::new(),
                policy_revision: 1,
                destination: DestinationPreference::Origin,
                message_id: None,
                introduction_id: None,
                work_ref: None,
                expires_at: None,
                follow_up_reason: None,
            },
        );
        after.community_receipts.insert(
            1,
            community::CommunityReceipt {
                policy_revision: 1,
                attempted_at: None,
                evidence,
            },
        );
        assert!(!after.erasure_admitted(&ledger, &before, NOW - 1));
        let unrelated = crate::brain::erasure::ErasureLedger::default();
        assert!(after.erasure_admitted(&unrelated, &before, NOW - 1));
    }
    let before = EngagementStore::default();
    let mut after = before.clone();
    after.community_cursor = Some(source(1, 7, 70));
    assert!(!after.erasure_admitted(&ledger, &before, NOW + 301));
    after.community_cursor = Some(source(1, 8, 80));
    assert!(after.erasure_admitted(&ledger, &before, NOW + 301));
}

#[test]
fn erasure_project_budget_digest_preserves_only_same_project_window_and_reopens() {
    let mut s = EngagementStore::default();
    s.erased_community_charges.push(ErasedCommunityCharge {
        scope: scope(1),
        kind: EngagementKind::ProjectCheckIn,
        at: NOW,
        project: Some(erasure_identity::project(10)),
    });
    let mut s: EngagementStore = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    let c = Candidate {
        id: 99,
        kind: EngagementKind::ProjectCheckIn,
        source: None,
        member: None,
        scope: scope(1),
        due_at: NOW,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: "test".into(),
        policy_revision: 1,
        destination: DestinationPreference::Origin,
        message_id: None,
        introduction_id: None,
        work_ref: None,
        expires_at: None,
        follow_up_reason: None,
    };
    for project in [10, 11] {
        s.community_receipts.insert(
            99,
            community::CommunityReceipt {
                policy_revision: 1,
                attempted_at: None,
                evidence: community::CommunityEvidence::Project {
                    project,
                    revision: 2,
                    actor: 8,
                    audience: BTreeSet::from([8]),
                    content: BTreeSet::new(),
                },
            },
        );
        assert_eq!(
            s.community_capacity(&c, NOW + community::WEEK - 1),
            project != 10
        );
        assert!(s.community_capacity(&c, NOW + community::WEEK));
    }
    for digest in [None, Some("invalid".into())] {
        s.erased_community_charges[0].project = digest;
        assert_eq!(s.validate_erased_charges(), Err(WorkError::Invalid));
        assert!(
            serde_json::from_str::<EngagementStore>(&serde_json::to_string(&s).unwrap()).is_err()
        );
    }
}
