use super::*;
#[test]
fn introductions_component_invoker_revision_command_binding() {
    let s = Session {
        command: 1,
        owner: 2,
        guild: 3,
        channel: 4,
        introduction: 5,
        revision: 6,
    };
    assert_eq!(s.action(&s.custom_id("a"), 2), Some(true));
    assert_eq!(s.action(&s.custom_id("w"), 2), Some(false));
    assert_eq!(s.action(&s.custom_id("a"), 7), None);
    for id in [
        "i:9:2:5:6:a",
        "i:1:2:5:7:a",
        "i:1:2:8:6:a",
        "i:1:2:5:6:a:extra",
    ] {
        assert_eq!(s.action(id, 2), None);
    }
}
#[test]
fn introductions_private_listing_only_own_same_guild() {
    let mut s = EngagementStore::default();
    s.introductions.insert(
        1,
        Introduction {
            id: 1,
            revision: 1,
            scope: EngagementScope::Guild {
                guild: 3,
                channel: 4,
            },
            members: [10, 20],
            approved_self_descriptions: [Some("PRIVATE TEXT".into()), None],
            approvals: [None; 2],
            destination: 4,
            state: IntroductionState::Pending,
        },
    );
    assert!(own_record(&s, 1, 30, 3).is_err());
    assert!(own_record(&s, 1, 10, 4).is_err());
    assert!(!own_list(&s, 30, 3).contains("1: Pending"));
    assert!(own_list(&s, 10, 3).contains("1: Pending"));
    assert!(!own_list(&s, 10, 3).contains("PRIVATE TEXT"));
}

#[test]
fn introductions_components_reject_wrong_channel_guild_author_context_and_type() {
    let s = Session {
        command: 1,
        owner: 2,
        guild: 3,
        channel: 4,
        introduction: 5,
        revision: 6,
    };
    for changed in 0..8 {
        let mut e = Envelope {
            owner: 2,
            bot: false,
            guild: Some(3),
            channel: 4,
            guild_context: true,
            message_is_ours: true,
            button: true,
        };
        match changed {
            0 => {}
            1 => e.owner = 7,
            2 => e.bot = true,
            3 => e.guild = Some(9),
            4 => e.channel = 9,
            5 => e.guild_context = false,
            6 => e.message_is_ours = false,
            _ => e.button = false,
        }
        assert_eq!(
            s.authorize(&s.custom_id("a"), &e),
            if changed == 0 { Some(true) } else { None }
        );
    }
}

#[test]
fn introductions_private_review_distinguishes_delivery_outcomes_and_own_receipt() {
    let scope = EngagementScope::Guild {
        guild: 3,
        channel: 4,
    };
    let mut s = EngagementStore::default();
    s.introductions.insert(
        1,
        Introduction {
            id: 1,
            revision: 2,
            scope: scope.clone(),
            members: [10, 20],
            approved_self_descriptions: [
                Some("OWN DESCRIPTION".into()),
                Some("OTHER PRIVATE DESCRIPTION".into()),
            ],
            approvals: [Some(2); 2],
            destination: 4,
            state: IntroductionState::Consumed,
        },
    );
    s.candidates.insert(
        2,
        Candidate {
            id: 2,
            kind: EngagementKind::Introduction,
            source: None,
            member: None,
            scope,
            due_at: 1,
            revision: 2,
            state: CandidateState::Reserved,
            dedupe_key: "introduction:1".into(),
            policy_revision: 1,
            destination: DestinationPreference::Origin,
            message_id: None,
            introduction_id: Some(1),
            work_ref: None,
            expires_at: None,
            follow_up_reason: None,
        },
    );
    for state in [
        CandidateState::Reserved,
        CandidateState::Sent,
        CandidateState::Rejected,
        CandidateState::ReviewRequired,
        CandidateState::Cancelled,
    ] {
        let c = s.candidates.get_mut(&2).unwrap();
        c.state = state;
        c.message_id = if state == CandidateState::Sent {
            Some(123)
        } else {
            None
        };
        let copy = render_review(&s, 1, 10, 3).unwrap();
        assert!(copy.contains(&format!("Delivery: {state:?}.")));
        assert!(own_list(&s, 10, 3).contains(&format!("delivery {state:?}")));
        assert!(copy.contains("OWN DESCRIPTION"));
        assert!(!copy.contains("OTHER PRIVATE DESCRIPTION"));
        assert_eq!(
            copy.contains("https://discord.com/channels/3/4/123"),
            state == CandidateState::Sent
        );
        if state == CandidateState::ReviewRequired {
            assert!(copy.contains("uncertain"));
            assert!(copy.contains("will not be retried automatically"));
        }
        assert!(render_review(&s, 1, 30, 3).is_err());
        assert!(render_review(&s, 1, 10, 9).is_err());
    }
}
