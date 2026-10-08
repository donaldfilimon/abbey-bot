use super::*;

fn canonical_restart(c: &RewardCollector) -> RewardCollector {
    let stores = crate::persist::Stores {
        pending_rewards: c.export_pending(),
        reward_recovery: c.export_recovery(),
        ..Default::default()
    };
    let encoded = serde_json::to_vec(&stores).unwrap();
    let loaded: crate::persist::Stores = serde_json::from_slice(&encoded).unwrap();
    let mut restored = RewardCollector::new();
    restored
        .restore_recovered(loaded.pending_rewards, loaded.reward_recovery)
        .unwrap();
    restored
}

#[test]
fn reaction_add_remove_restart_is_idempotent() {
    let mut c = collector_with_turn();
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 1), true, T0),
        FeedbackAttribution::ExactReply
    );
    let mut c = canonical_restart(&c);
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 1), true, T0 + 1),
        FeedbackAttribution::Duplicate
    );
    assert!(approx(reward_of(&c, "abbey-1"), -1.2));
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 1), false, T0 + 2),
        FeedbackAttribution::ExactReply
    );
    let mut c = canonical_restart(&c);
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 1), false, T0 + 3),
        FeedbackAttribution::Duplicate
    );
    assert!(approx(reward_of(&c, "abbey-1"), -0.2));
    // Without a native event sequence number this is a new observed active
    // state, not a claim to distinguish an arbitrarily reordered old add.
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 1), true, T0 + 4),
        FeedbackAttribution::ExactReply
    );
    assert!(approx(reward_of(&c, "abbey-1"), -1.2));
}

#[test]
fn competing_scope_turns_are_ambiguous_but_exact_has_priority() {
    let mut c = collector_with_turn();
    c.register_turn(turn("abbey-2", CHAN, "second", T0 + 1));
    assert_eq!(
        c.feedback(CHAN, ASKER, None, "thanks", T0 + 2),
        FeedbackAttribution::Ambiguous
    );
    assert_eq!(
        c.feedback(CHAN, ASKER, Some("abbey-1"), "thanks", T0 + 2),
        FeedbackAttribution::ExactReply
    );
    assert_eq!(c.pending["abbey-1"].delayed_count, 1);
    assert_eq!(c.pending["abbey-2"].delayed_count, 0);
}

#[test]
fn exact_feedback_checks_scope_ttl_deletion_and_clock_regression() {
    let mut c = collector_with_turn();
    for (scope, time, expected) in [
        ("slack:c1", T0, FeedbackAttribution::Unsupported),
        (CHAN, T0 + 151, FeedbackAttribution::Expired),
        (CHAN, T0 - 1, FeedbackAttribution::Expired),
    ] {
        assert_eq!(
            c.feedback(scope, ASKER, Some("abbey-1"), "thanks", time),
            expected
        );
        assert_eq!(
            c.reaction(
                ReactionKey {
                    scope: scope.into(),
                    ..key("abbey-1", "👍", 1)
                },
                true,
                time
            ),
            expected
        );
    }
    assert_eq!(
        c.feedback(CHAN, "", Some("abbey-1"), "thanks", T0),
        FeedbackAttribution::Unsupported
    );
    assert!(approx(reward_of(&c, "abbey-1"), -0.2));
    c.abbey_message_deleted("abbey-1");
    assert_eq!(
        c.feedback(CHAN, ASKER, Some("abbey-1"), "thanks", T0),
        FeedbackAttribution::Expired
    );
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 1), true, T0),
        FeedbackAttribution::Expired
    );
    assert_eq!(c.settle_expired(T0)[0].1.reward, -2.0);
}

#[test]
fn capped_positive_removal_reverses_only_its_recorded_contribution() {
    let mut c = collector_with_turn();
    for member in 1..=4 {
        c.reaction(key("abbey-1", "👍", member), true, T0);
    }
    let mut c = canonical_restart(&c);
    c.reaction(key("abbey-1", "👍", 4), false, T0 + 1);
    assert!(approx(reward_of(&c, "abbey-1"), 2.8));
    c.reaction(key("abbey-1", "👍", 1), false, T0 + 1);
    assert!(approx(reward_of(&c, "abbey-1"), 1.8));
    c.reaction(key("abbey-1", "👍", 4), true, T0 + 1);
    assert!(approx(reward_of(&c, "abbey-1"), 2.8));
    assert_eq!(c.pending["abbey-1"].positive_reactions, 3);
}

#[test]
fn saturation_cannot_forget_a_credited_key() {
    let mut c = collector_with_turn();
    for member in 0..4096 {
        c.reaction(key("abbey-1", "👎", member), true, T0);
    }
    assert_eq!(c.export_recovery().reactions.len(), 4096);
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 5000), true, T0),
        FeedbackAttribution::Unsupported
    );
    let mut c = canonical_restart(&c);
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 0), true, T0),
        FeedbackAttribution::Duplicate
    );
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 0), false, T0),
        FeedbackAttribution::ExactReply
    );
    assert_eq!(c.export_recovery().reactions.len(), 4095);
    assert_eq!(
        c.reaction(key("abbey-1", "👎", 5000), true, T0),
        FeedbackAttribution::ExactReply
    );
    assert_eq!(c.settle_expired(T0 + 151)[0].1.reward, -3.0);
}

#[test]
fn expired_turn_cannot_be_reopened_by_eviction() {
    let mut c = collector_with_turn();
    c.reaction(key("abbey-1", "👍", 1), true, T0);
    c.settle_expired(T0 + 151);
    let mut c = canonical_restart(&c);
    c.register_turn(turn("abbey-1", CHAN, "replay", T0));
    assert_eq!(c.pending_len(), 0);
    assert_eq!(
        c.reaction(key("abbey-1", "👍", 1), true, T0 + 152),
        FeedbackAttribution::Expired
    );
    assert!(c.export_recovery().reactions.is_empty());
    assert_eq!(c.export_recovery().settled.len(), 1);
    c.settle_expired(T0 + 451);
    let mut c = canonical_restart(&c);
    assert!(c.export_recovery().settled.is_empty());
    c.register_turn(turn("abbey-1", CHAN, "old creation", T0));
    assert_eq!(c.pending_len(), 0);
    c.register_turn(turn("new", "slack:other", "new delivery", T0 + 452));
    assert_eq!(c.pending_len(), 1);
    assert_eq!(
        c.reaction(key("abbey-1", "👍", 1), true, T0 + 452),
        FeedbackAttribution::Expired
    );
}

#[test]
fn duplicate_registration_cannot_erase_pending_or_cross_scope_credit() {
    let mut c = collector_with_turn();
    c.reaction(key("abbey-1", "👍", 1), true, T0);
    c.register_turn(turn("abbey-1", "slack:other", "collision", T0 + 1));
    assert_eq!(c.pending["abbey-1"].scope, CHAN);
    assert!(approx(reward_of(&c, "abbey-1"), 0.8));
    assert_eq!(
        c.reaction(
            ReactionKey {
                scope: "slack:other".into(),
                ..key("abbey-1", "👍", 1)
            },
            true,
            T0 + 1
        ),
        FeedbackAttribution::Unsupported
    );
    assert_eq!(
        c.reaction(key("abbey-1", "👍", 1), true, T0 + 1),
        FeedbackAttribution::Duplicate
    );
}

#[test]
fn legacy_reaction_credit_stays_inert_until_settlement() {
    let mut c = collector_with_turn();
    c.pending.get_mut("abbey-1").unwrap().reaction_tracking = false;
    c.pending.get_mut("abbey-1").unwrap().reward = 0.8;
    c.pending.get_mut("abbey-1").unwrap().positive_reactions = 1;
    let mut c = canonical_restart(&c);
    assert_eq!(
        c.reaction(key("abbey-1", "👍", 1), true, T0),
        FeedbackAttribution::Unsupported
    );
    assert_eq!(
        c.reaction(key("abbey-1", "👍", 1), false, T0),
        FeedbackAttribution::Unsupported
    );
    assert!(approx(c.settle_expired(T0 + 151)[0].1.reward, 0.8));
}

#[test]
fn a_bare_reaction_action_is_not_a_conversational_followup_candidate() {
    let mut c = RewardCollector::new();
    c.register_reply(
        vec![0.0; 18],
        BotAction::React.index(),
        "reaction-target",
        "g-1",
        CHAN,
        T0,
    );
    assert_eq!(
        c.feedback(CHAN, ASKER, None, "does the gateway retry?", T0),
        FeedbackAttribution::Expired
    );
    assert_eq!(
        c.reaction(key("reaction-target", "👍", 1), true, T0),
        FeedbackAttribution::ExactReply
    );
}

#[test]
fn pending_and_settled_capacity_refuses_new_turns_until_safe_retirement() {
    let mut c = RewardCollector::new();
    for id in 0..4096 {
        c.register_turn(turn(&id.to_string(), CHAN, "fixture", T0));
    }
    c.register_turn(turn("overflow", CHAN, "refuse", T0));
    assert_eq!(c.pending_len(), 4096);
    assert_eq!(c.settle_expired(T0 + 151).len(), 4096);
    let mut c = canonical_restart(&c);
    c.register_turn(turn("still-full", CHAN, "refuse", T0 + 152));
    assert_eq!(c.pending_len(), 0);
    assert_eq!(c.export_recovery().settled.len(), 4096);
    c.register_turn(turn("new", "slack:other", "admit", T0 + 451));
    assert_eq!(c.pending_len(), 1);
    assert!(c.export_recovery().settled.is_empty());
    // Old creation times stay refused after serialization even in another
    // scope. This conservative floor protects only registration; it does not
    // invalidate an existing open turn or extend attribution TTL.
    let mut c = canonical_restart(&c);
    c.register_turn(turn("old-other", "slack:other", "old replay", T0));
    assert_eq!(c.pending_len(), 1);
}

#[test]
fn immediate_deletion_recovery_cannot_reopen_with_a_regressed_clock() {
    let mut c = collector_with_turn();
    c.abbey_message_deleted("abbey-1");
    assert_eq!(c.settle_expired(T0 - 1).len(), 1);
    let mut c = canonical_restart(&c);
    c.register_turn(turn("abbey-1", CHAN, "replay", T0 - 10));
    assert_eq!(c.pending_len(), 0);
    assert_eq!(c.export_recovery().settled.len(), 1);
    c.settle_expired(T0 + 300);
    let mut c = canonical_restart(&c);
    assert!(c.export_recovery().settled.is_empty());
    c.register_turn(turn("abbey-1", CHAN, "replay", T0));
    assert_eq!(c.pending_len(), 0);
}
