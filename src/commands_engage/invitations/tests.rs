use super::*;
fn fixture() -> (EngagementStore, InvitationRequest) {
    let scope = EngagementScope::Dm {
        member: 2,
        channel: 3,
    };
    let mut store = EngagementStore::default();
    store.member_policies.insert(
        2,
        MemberPolicy {
            daily_limit: Some(4),
            timezone: Some("UTC".into()),
            quiet_start: 0,
            quiet_end: 0,
            ..Default::default()
        },
    );
    store.eligibility.entry(2).or_default().insert(SourceRef {
        scope: scope.clone(),
        message: 4,
        author: 2,
        revision: 1,
        at: 1,
    });
    (
        store,
        InvitationRequest {
            activity: None,
            interaction: 5,
            member: 2,
            scope,
            at: 100,
        },
    )
}
#[test]
fn invitation_missing_member_policy_disabled_and_stopped_are_denied() {
    for mode in 0..4 {
        let (mut s, r) = fixture();
        match mode {
            0 => {
                s.member_policies.clear();
            }
            1 => s.member_policies.get_mut(&2).unwrap().daily_limit = None,
            2 => s.member_policies.get_mut(&2).unwrap().global_stop = true,
            _ => {
                s.eligibility.clear();
            }
        }
        assert!(
            s.request_invitation(EngagementKind::VoiceInvite, r)
                .is_err()
        );
        assert!(s.candidates.is_empty());
    }
}
#[test]
fn invitation_dedupe_restart_shared_budget_and_explicit_repeat() {
    let (mut s, r) = fixture();
    let id = s
        .request_invitation(EngagementKind::VoiceInvite, r.clone())
        .unwrap()
        .unwrap();
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, r.clone())
            .unwrap(),
        None
    );
    let mut fresh = r.clone();
    fresh.interaction = 6;
    fresh.at = 101;
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, fresh.clone())
            .unwrap(),
        None
    );
    let reservation = s.reserve(id, 1, 100).unwrap();
    assert!(s.validate_reserved(&reservation, 100).is_ok());
    assert_eq!(s.charges.len(), 1);
    s.settle(
        id,
        crate::engagement::lifecycle::DeliveryOutcome::Sent { message_id: 7 },
    )
    .unwrap();
    let bytes = serde_json::to_vec(&s).unwrap();
    let mut s: EngagementStore = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, r)
            .unwrap(),
        None
    );
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, fresh.clone())
            .unwrap(),
        None
    );
    fresh.interaction = 9;
    let next = s
        .request_invitation(EngagementKind::VoiceInvite, fresh)
        .unwrap()
        .unwrap();
    assert_ne!(next, id);
    assert!(s.reserve(next, 1, 101).is_ok());
    assert_eq!(s.charges.len(), 2);
}
#[test]
fn invitation_receipt_tampering_and_voice_copy_do_not_create_consent() {
    let (mut s, r) = fixture();
    let id = s
        .request_invitation(EngagementKind::VoiceInvite, r)
        .unwrap()
        .unwrap();
    let before = serde_json::to_value(&s).unwrap();
    let copy = crate::engagement::invitations::voice_invitation();
    assert!(copy.contains("/voice consent"));
    assert!(copy.contains("/voice status"));
    assert!(copy.contains("/voice join consent:true"));
    assert!(copy.contains("/voice resume consent:true"));
    assert!(copy.contains("starts no audio"));
    assert_eq!(before, serde_json::to_value(&s).unwrap());
    s.invitation_requests.get_mut(&id).unwrap().member = 99;
    assert!(s.validate().is_err());
    assert!(s.reserve(id, 1, 100).is_err());
}

#[test]
fn invitation_subscription_only_eligibility_keeps_explicit_limits_and_reservation() {
    let (mut s, r) = fixture();
    s.eligibility.clear();
    s.member_policies.get_mut(&2).unwrap().weekly_subscription = Some(WeeklySubscription {
        weekday: 0,
        hour: 12,
        scope: r.scope.clone(),
        destination: DestinationPreference::Origin,
    });
    let id = s
        .request_invitation(EngagementKind::VoiceInvite, r.clone())
        .expect("Explicit subscription establishes eligibility without inventing a human source")
        .unwrap();
    assert!(s.candidates[&id].source.is_none());
    let reservation = s.reserve(id, 1, 100).unwrap();
    assert!(s.validate_reserved(&reservation, 100).is_ok());
    let (mut disabled, r) = fixture();
    disabled.eligibility.clear();
    disabled
        .member_policies
        .get_mut(&2)
        .unwrap()
        .weekly_subscription = s.member_policies[&2].weekly_subscription.clone();
    disabled.member_policies.get_mut(&2).unwrap().daily_limit = None;
    assert!(
        disabled
            .request_invitation(EngagementKind::VoiceInvite, r)
            .is_err()
    );
    assert!(disabled.charges.is_empty());
}

#[test]
fn invitation_suppressed_interaction_stays_deduped_after_terminal_and_restart() {
    let (mut s, r) = fixture();
    let id = s
        .request_invitation(EngagementKind::VoiceInvite, r.clone())
        .unwrap()
        .unwrap();
    let mut suppressed = r;
    suppressed.interaction = 6;
    suppressed.at = 101;
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, suppressed.clone())
            .unwrap(),
        None
    );
    s.reserve(id, 1, 100).unwrap();
    s.settle(
        id,
        crate::engagement::lifecycle::DeliveryOutcome::Sent { message_id: 7 },
    )
    .unwrap();
    let mut s: EngagementStore = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(
        s.request_invitation(EngagementKind::VoiceInvite, suppressed.clone())
            .unwrap(),
        None,
        "An ignored slash event must not create a second candidate when replayed after the blocking invitation was sent"
    );
    assert_eq!(s.candidates.len(), 1);
    assert_eq!(s.charges.len(), 1);
    suppressed.interaction = 8;
    assert!(
        s.request_invitation(EngagementKind::VoiceInvite, suppressed)
            .unwrap()
            .is_some()
    );
}
