use super::*;

#[test]
fn pending_rewards_export_and_restore_across_a_restart() {
    let mut a = RewardCollector::new();
    a.register_reply(vec![0.0; 3], 1, "m1", "discord:g", 10);
    a.reaction("👍", "m1", true);
    let rows = a.export_pending();
    let json = serde_json::to_string(&rows).unwrap();
    let back: Vec<(String, Pending)> = serde_json::from_str(&json).unwrap();
    let mut b = RewardCollector::new();
    b.restore(back);
    assert_eq!(b.pending_len(), 1);
    let settled = b.settle_expired(10 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled.len(), 1);
    assert!(
        (settled[0].1.reward - 0.8).abs() < 1e-6,
        "the reaction survived the restart"
    );
}

const T0: u64 = 1_700_000_000;

fn collector_with_reply() -> RewardCollector {
    let mut c = RewardCollector::new();
    c.register_reply(
        vec![0.5, 0.25],
        BotAction::Reply.index(),
        "msg-1",
        "g-1",
        T0,
    );
    c
}

fn reward_of(c: &RewardCollector, id: &str) -> f32 {
    c.pending[id].reward
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn reply_starts_mildly_negative() {
    let c = collector_with_reply();
    assert_eq!(c.pending_len(), 1);
    assert!(approx(reward_of(&c, "msg-1"), -0.2));
    assert_eq!(c.pending["msg-1"].positive_reactions, 0);
    assert!(!c.pending["msg-1"].settle_immediately);
    assert_eq!(c.pending["msg-1"].created_at, T0);
}

#[test]
fn three_positives_cap_and_fourth_is_ignored() {
    let mut c = collector_with_reply();
    c.reaction("👍", "msg-1", true);
    c.reaction("❤️", "msg-1", true);
    c.reaction("🔥", "msg-1", true);
    assert!(approx(reward_of(&c, "msg-1"), 2.8));
    c.reaction("💯", "msg-1", true);
    assert!(
        approx(reward_of(&c, "msg-1"), 2.8),
        "fourth positive ignored"
    );
    assert_eq!(c.pending["msg-1"].positive_reactions, 3);
}

#[test]
fn negative_reaction_subtracts_one_without_cap() {
    let mut c = collector_with_reply();
    c.reaction("👎", "msg-1", true);
    assert!(approx(reward_of(&c, "msg-1"), -1.2));
    c.reaction("💀", "msg-1", true);
    c.reaction("😡", "msg-1", true);
    c.reaction("🤮", "msg-1", true);
    assert!(approx(reward_of(&c, "msg-1"), -4.2));
}

#[test]
fn removed_reactions_unknown_emoji_and_unknown_targets_are_ignored() {
    let mut c = collector_with_reply();
    c.reaction("👍", "msg-1", false);
    c.reaction("🙂", "msg-1", true);
    c.reaction("👍", "msg-other", true);
    assert!(approx(reward_of(&c, "msg-1"), -0.2));
    assert_eq!(c.pending_len(), 1);
}

#[test]
fn human_reply_adds_half() {
    let mut c = collector_with_reply();
    c.human_replied("msg-1");
    assert!(approx(reward_of(&c, "msg-1"), 0.3));
    c.human_replied("msg-other");
    assert_eq!(c.pending_len(), 1);
}

#[test]
fn deletion_settles_immediately_at_minus_two() {
    let mut c = collector_with_reply();
    c.reaction("👍", "msg-1", true);
    c.abbey_message_deleted("msg-1");
    let settled = c.settle_expired(T0 + 1);
    assert_eq!(settled.len(), 1);
    let (guild, exp) = &settled[0];
    assert_eq!(guild, "g-1");
    assert_eq!(exp.reward, -2.0);
    assert!(exp.done);
    assert_eq!(exp.state, exp.next_state);
    assert_eq!(exp.action, BotAction::Reply.index());
    assert_eq!(c.pending_len(), 0);
    c.abbey_message_deleted("msg-other");
    assert_eq!(c.pending_len(), 0);
}

#[test]
fn settles_only_strictly_after_the_window() {
    let mut c = collector_with_reply();
    assert!(c.settle_expired(T0).is_empty());
    assert!(
        c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS).is_empty(),
        "exactly at the window stays open"
    );
    assert_eq!(c.pending_len(), 1);
    let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled.len(), 1);
    assert!(approx(settled[0].1.reward, -0.2));
    assert!(settled[0].1.done);
    assert_eq!(settled[0].1.state, vec![0.5, 0.25]);
    assert_eq!(settled[0].1.next_state, vec![0.5, 0.25]);
    assert_eq!(c.pending_len(), 0);
    assert!(c.settle_expired(T0 + 10_000).is_empty());
}

#[test]
fn settled_reward_is_clamped_to_plus_minus_three() {
    let mut c = collector_with_reply();
    for _ in 0..10 {
        c.reaction("👎", "msg-1", true);
    }
    let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled[0].1.reward, -3.0);

    let mut c = collector_with_reply();
    for e in ["👍", "❤️", "🔥"] {
        c.reaction(e, "msg-1", true);
    }
    c.human_replied("msg-1");
    c.human_replied("msg-1");
    // −0.2 + 3 + 0.5 + 0.5 = 3.8 → clamped to 3.
    let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled[0].1.reward, 3.0);
}

#[test]
fn settle_drains_only_expired_entries() {
    let mut c = collector_with_reply();
    c.register_reply(vec![1.0], 1, "msg-2", "g-2", T0 + 100);
    let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].0, "g-1");
    assert_eq!(c.pending_len(), 1);
    assert!(c.pending.contains_key("msg-2"));
}

#[test]
fn silence_experience_is_done_with_zero_reward_and_stay() {
    let exp = RewardCollector::silence_experience(vec![0.1, 0.2, 0.3]);
    assert_eq!(exp.reward, 0.0);
    assert!(exp.done);
    assert_eq!(exp.action, BotAction::Stay.index());
    assert_eq!(exp.state, vec![0.1, 0.2, 0.3]);
    assert_eq!(exp.next_state, exp.state);
}

// ---- delayed outcome channel -------------------------------------------

const CHAN: &str = "discord:g-1:c-1";
/// The human whose ask the test turns answered.
const ASKER: &str = "discord:alice";
/// Someone else in the same channel.
const BYSTANDER: &str = "discord:bob";

fn turn(id: &str, scope: &str, ask: &str, now: u64) -> ReplyTurn {
    ReplyTurn {
        state: vec![0.5, 0.25],
        action: BotAction::Reply.index(),
        sent_native_message_id: id.to_owned(),
        scope: scope.to_owned(),
        scoped_guild_id: "g-1".to_owned(),
        ask: ask.to_owned(),
        asker: ASKER.to_owned(),
        now,
    }
}

fn collector_with_turn() -> RewardCollector {
    let mut c = RewardCollector::new();
    c.register_turn(turn(
        "abbey-1",
        CHAN,
        "how do I configure the voice gateway timeout?",
        T0,
    ));
    c
}

fn settled_reward(mut c: RewardCollector) -> f32 {
    let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled.len(), 1);
    settled[0].1.reward
}

#[test]
fn a_pending_row_written_before_delayed_outcomes_still_loads() {
    // Exactly the shape an older build wrote: no scope, ask, delayed_sum,
    // or delayed_count. If this ever fails, every existing state file on
    // disk fails to load — and it takes the whole `Stores` load with it,
    // not just the reward ledger.
    let legacy = r#"[["m1",{
            "state":[0.0,0.0,0.0],
            "action":1,
            "scoped_guild_id":"discord:g",
            "reward":0.8,
            "positive_reactions":1,
            "created_at":10,
            "settle_immediately":false
        }]]"#;
    let rows: Vec<(String, Pending)> =
        serde_json::from_str(legacy).expect("legacy pending rows must still deserialize");
    let mut c = RewardCollector::new();
    c.restore(rows);
    assert_eq!(c.pending_len(), 1);
    assert_eq!(c.pending["m1"].scope, "");
    assert_eq!(c.pending["m1"].ask, "");
    assert_eq!(c.pending["m1"].delayed_count, 0);
    // And it settles at the number the old build would have produced.
    let settled = c.settle_expired(10 + SETTLEMENT_WINDOW_SECS + 1);
    assert!(approx(settled[0].1.reward, 0.8));
}

#[test]
fn with_no_outcome_the_settled_reward_is_bit_identical_to_the_old_path() {
    // Same turn opened both ways; the delayed channel stays silent.
    let mut old = RewardCollector::new();
    old.register_reply(vec![0.5, 0.25], BotAction::Reply.index(), "m", "g-1", T0);
    old.reaction("👍", "m", true);
    old.human_replied("m");

    let mut new = collector_with_turn();
    new.reaction("👍", "abbey-1", true);
    new.human_replied("abbey-1");

    assert_eq!(
        settled_reward(new).to_bits(),
        settled_reward(old).to_bits(),
        "conversational context must not move the number by itself"
    );
}

#[test]
fn an_explicit_reply_to_credits_exactly_that_turn() {
    let mut c = collector_with_turn();
    c.register_turn(turn("abbey-2", CHAN, "and the retry budget?", T0 + 5));
    assert!(c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks));
    assert_eq!(c.pending["abbey-1"].delayed_count, 1);
    assert!(approx(c.pending["abbey-1"].delayed_sum, 1.0));
    assert_eq!(
        c.pending["abbey-2"].delayed_count, 0,
        "the newer turn earned nothing"
    );
    assert!(
        !c.observe_reply_to("no-such-turn", ReplyOutcome::ExplicitThanks),
        "an unknown target is ignored, not invented"
    );
}

#[test]
fn a_scoped_observation_lands_on_the_newest_turn_in_that_channel() {
    let mut c = collector_with_turn();
    c.register_turn(turn("abbey-2", CHAN, "and the retry budget?", T0 + 5));
    c.register_turn(turn("elsewhere", "discord:g-1:c-2", "unrelated", T0 + 9));

    let credited = c.observe_in_scope(CHAN, ASKER, ReplyOutcome::Correction, T0 + 10);
    assert_eq!(credited.as_deref(), Some("abbey-2"), "newest turn in scope");
    assert!(approx(c.pending["abbey-2"].delayed_sum, -1.0));
    assert_eq!(c.pending["abbey-1"].delayed_count, 0);
    assert_eq!(
        c.pending["elsewhere"].delayed_count, 0,
        "another channel's turn is out of scope"
    );
    assert_eq!(
        c.observe_in_scope("discord:g-9:c-9", ASKER, ReplyOutcome::Correction, T0 + 10),
        None,
        "a scope with no open turn credits nothing"
    );
}

#[test]
fn a_turn_opened_without_a_scope_is_unreachable_by_scope() {
    let mut c = RewardCollector::new();
    c.register_reply(vec![0.0], BotAction::React.index(), "react-1", "g-1", T0);
    assert_eq!(c.newest_open_turn_in_scope("", T0), None);
    assert_eq!(
        c.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0),
        None
    );
    assert_eq!(c.pending["react-1"].delayed_count, 0);
    // …but an explicit reply-to still reaches it.
    assert!(c.observe_reply_to("react-1", ReplyOutcome::ExplicitThanks));
}

#[test]
fn a_bystanders_thanks_does_not_land_on_abbeys_turn() {
    // "thanks Carol!" from someone Abbey never answered is the commonest
    // way scope attribution could lie, so a marker-only outcome needs the
    // original asker to corroborate it.
    let mut c = collector_with_turn();
    assert_eq!(
        c.observe_in_scope(CHAN, BYSTANDER, ReplyOutcome::ExplicitThanks, T0 + 1),
        None
    );
    assert_eq!(
        c.observe_in_scope(CHAN, BYSTANDER, ReplyOutcome::Correction, T0 + 1),
        None
    );
    assert_eq!(c.pending["abbey-1"].delayed_count, 0);
    assert_eq!(
        settled_reward(collector_with_turn()).to_bits(),
        settled_reward(c).to_bits()
    );
}

#[test]
fn a_bystanders_topical_follow_up_still_counts() {
    // Shared content words are their own corroboration, so this one does
    // not need to come from the asker.
    let mut c = collector_with_turn();
    assert_eq!(
        c.observe_in_scope(CHAN, BYSTANDER, ReplyOutcome::FollowUpQuestion, T0 + 1),
        Some("abbey-1".to_owned())
    );
    assert!(approx(settled_reward(c), 0.2));
}

#[test]
fn a_turn_with_no_recorded_asker_cannot_corroborate_a_marker() {
    // Rows restored from a state file written before `asker` existed.
    let mut c = RewardCollector::new();
    c.register_turn(ReplyTurn {
        asker: String::new(),
        ..turn("abbey-1", CHAN, "how do I build it?", T0)
    });
    assert_eq!(
        c.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0 + 1),
        None,
        "an empty asker matches nobody, not everybody"
    );
    // The topical path is unaffected.
    assert!(
        c.observe_in_scope(CHAN, ASKER, ReplyOutcome::RephrasedSameAsk, T0 + 1)
            .is_some()
    );
}

#[test]
fn an_explicit_reply_to_needs_no_asker_check() {
    // The pointer is the corroboration: Discord says which message this
    // answers, so a third party thanking Abbey for someone else's answer
    // is still real evidence about that answer.
    let mut c = collector_with_turn();
    assert!(c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks));
    assert!(approx(settled_reward(c), 0.8));
}

#[test]
fn attribution_expires_with_the_settlement_window() {
    let mut c = collector_with_turn();
    assert_eq!(
        c.newest_open_turn_in_scope(CHAN, T0 + ATTRIBUTION_TTL_SECS),
        Some("abbey-1"),
        "exactly at the TTL is still attributable"
    );
    assert_eq!(
        c.newest_open_turn_in_scope(CHAN, T0 + ATTRIBUTION_TTL_SECS + 1),
        None,
        "one second past and the turn is no longer a candidate"
    );
    assert_eq!(
        c.observe_in_scope(
            CHAN,
            ASKER,
            ReplyOutcome::ExplicitThanks,
            T0 + ATTRIBUTION_TTL_SECS + 1
        ),
        None
    );
    assert_eq!(c.pending["abbey-1"].delayed_count, 0);

    // Unattributed, it does not leak: the sweep drains it.
    let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
    assert_eq!(settled.len(), 1);
    assert!(approx(settled[0].1.reward, -0.2), "the untouched baseline");
    assert_eq!(c.pending_len(), 0);
}

#[test]
fn a_deleted_turn_cannot_absorb_a_later_observation() {
    let mut c = collector_with_turn();
    c.abbey_message_deleted("abbey-1");
    assert_eq!(c.newest_open_turn_in_scope(CHAN, T0 + 1), None);
    assert_eq!(
        c.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0 + 1),
        None
    );
    assert!(approx(settled_reward(c), -2.0), "deletion still wins");
}

#[test]
fn same_second_turns_break_their_tie_deterministically() {
    // Two turns registered in the same second: the choice must not depend
    // on HashMap iteration order, so run it repeatedly on fresh maps.
    for _ in 0..64 {
        let mut c = RewardCollector::new();
        c.register_turn(turn("aaa", CHAN, "first", T0));
        c.register_turn(turn("zzz", CHAN, "second", T0));
        assert_eq!(c.newest_open_turn_in_scope(CHAN, T0), Some("zzz"));
    }
}

#[test]
fn the_ask_travels_with_the_turn_for_topic_comparison() {
    let mut c = collector_with_turn();
    assert_eq!(
        c.open_ask("abbey-1"),
        Some("how do I configure the voice gateway timeout?")
    );
    assert_eq!(c.open_ask_in_scope(CHAN, T0 + 1), c.open_ask("abbey-1"));
    assert_eq!(c.open_ask("missing"), None);
    assert_eq!(
        c.open_ask_in_scope(CHAN, T0 + ATTRIBUTION_TTL_SECS + 1),
        None,
        "an expired turn offers no ask"
    );

    // The round trip the pipeline performs: ask → classify → credit.
    let ask = c.open_ask_in_scope(CHAN, T0 + 1).unwrap().to_owned();
    let o = crate::brain::outcome::classify(
        "how can the voice gateway timeout be configured?",
        Some(&ask),
    );
    assert_eq!(o, Some(ReplyOutcome::RephrasedSameAsk));
    c.observe_in_scope(CHAN, ASKER, o.unwrap(), T0 + 1);
    assert!(
        approx(settled_reward(c), -0.7),
        "-0.2 baseline + 1.0 × -0.5"
    );
}

#[test]
fn thanks_and_correction_settle_on_opposite_sides_of_silence() {
    let silent = settled_reward(collector_with_turn());

    let mut thanked = collector_with_turn();
    thanked.human_replied("abbey-1");
    thanked.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks);
    // −0.2 baseline + 0.5 engaged + 1.0 × 1.0 typed.
    assert!(approx(settled_reward(thanked), 1.3));

    let mut corrected = collector_with_turn();
    corrected.human_replied("abbey-1");
    corrected.observe_reply_to("abbey-1", ReplyOutcome::Correction);
    // −0.2 + 0.5 − 1.0: worse than saying nothing at all.
    let corrected = settled_reward(corrected);
    assert!(approx(corrected, -0.7));
    assert!(
        corrected < silent,
        "a reply that had to be corrected must cost more than silence"
    );
}

#[test]
fn no_engagement_neither_penalises_nor_dilutes() {
    let baseline = settled_reward(collector_with_turn());

    let mut c = collector_with_turn();
    for _ in 0..5 {
        assert!(c.observe_reply_to("abbey-1", ReplyOutcome::NoEngagement));
    }
    assert_eq!(c.pending["abbey-1"].delayed_count, 0, "recorded as nothing");
    assert_eq!(
        settled_reward(c).to_bits(),
        baseline.to_bits(),
        "weak evidence is not a negative"
    );

    // And it does not drag a real signal toward the middle.
    let mut c = collector_with_turn();
    c.observe_reply_to("abbey-1", ReplyOutcome::NoEngagement);
    c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks);
    assert!(approx(settled_reward(c), 0.8), "-0.2 + full 1.0, not 0.5");
}

#[test]
fn several_outcomes_average_rather_than_accumulate() {
    let mut c = collector_with_turn();
    for _ in 0..8 {
        c.observe_reply_to("abbey-1", ReplyOutcome::FollowUpQuestion);
    }
    assert_eq!(c.pending["abbey-1"].delayed_count, 8);
    // Eight follow-ups are still one follow-up's worth of verdict, so a
    // chatty thread cannot saturate the ±3 clamp on engagement alone.
    assert!(approx(settled_reward(c), 0.2));
}

#[test]
fn the_blended_reward_is_still_clamped() {
    let mut c = collector_with_turn();
    for e in ["👍", "❤️", "🔥"] {
        c.reaction(e, "abbey-1", true);
    }
    c.human_replied("abbey-1");
    c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks);
    // −0.2 + 3 + 0.5 + 1.0 = 4.3 → 3.
    assert_eq!(settled_reward(c), 3.0);
}

#[test]
fn the_delayed_channel_survives_a_restart() {
    let mut a = collector_with_turn();
    a.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0 + 1);
    let json = serde_json::to_string(&a.export_pending()).unwrap();
    let rows: Vec<(String, Pending)> = serde_json::from_str(&json).unwrap();
    let mut b = RewardCollector::new();
    b.restore(rows);
    assert_eq!(b.pending["abbey-1"].scope, CHAN);
    assert_eq!(b.pending["abbey-1"].delayed_count, 1);
    // Scope attribution keeps working against the restored row.
    assert_eq!(
        b.newest_open_turn_in_scope(CHAN, T0 + 2),
        Some("abbey-1"),
        "the ledger key survives, so a follow-up after a restart still lands"
    );
    assert!(approx(settled_reward(b), 0.8));
}
