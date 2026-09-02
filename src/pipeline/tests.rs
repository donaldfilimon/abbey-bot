use super::testing::FakeOut;
use super::*;
use crate::platform::{EventKind, SocialNetwork};

fn message(text: &str, guild: Option<&str>, from: &str) -> SocialEvent {
    SocialEvent {
        network: SocialNetwork::Discord,
        kind: EventKind::Message {
            text: text.into(),
            attachments: vec![],
        },
        native_message_id: "m1".into(),
        native_channel_id: "c1".into(),
        native_guild_id: guild.map(Into::into),
        native_user_id: from.into(),
        user_display_name: "Sam".into(),
        is_bot: false,
        timestamp: 0,
    }
}

fn network_message(
    network: SocialNetwork,
    text: &str,
    guild: Option<&str>,
    from: &str,
) -> SocialEvent {
    SocialEvent {
        network,
        ..message(text, guild, from)
    }
}

#[tokio::test]
async fn own_traffic_is_ignored() {
    let state = AppState::in_memory();
    state.register_self("discord:bot".into());
    let out = FakeOut::default();
    let outcome = handle(&state, &out, message("hi", Some("g"), "bot"), false, None).await;
    assert_eq!(outcome, Outcome::Ignored("own traffic"));
}

#[tokio::test]
async fn a_mention_with_no_backend_gets_the_honest_degraded_reply() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let outcome = handle(
        &state,
        &out,
        message("hey abbey?", Some("g"), "u1"),
        true,
        None,
    )
    .await;
    assert_eq!(outcome, Outcome::Replied);
    let sent = out.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.text.contains("no generation backend"),
        "{}",
        sent[0].1.text
    );
    assert_eq!(sent[0].1.reply_to_native_message_id.as_deref(), Some("m1"));
}

#[tokio::test]
async fn discord_telegram_and_slack_share_canonical_persona_tool_memory_and_vision_seams() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let networks = [
        SocialNetwork::Discord,
        SocialNetwork::Telegram,
        SocialNetwork::Slack,
    ];
    let text = "Aviva, remember this image-backed fact";
    let settings = GuildSettings::default();
    let mut behavior = Vec::new();
    let mut scopes = Vec::new();

    for network in networks {
        let event = network_message(network, text, Some("g"), "u");
        let scoped_guild = event.scoped_guild_id();
        let scoped_user = event.scoped_user_id();
        let scoped_channel = event.scoped_channel_id();
        assert_eq!(
            handle(&state, &out, event.clone(), true, None).await,
            Outcome::Replied
        );

        // With no configured backend, the fixed degraded reply still exposes
        // the persona selected by the real pipeline. The explicit cue must be
        // interpreted identically on every network.
        let reply = out
            .sent
            .lock()
            .unwrap()
            .last()
            .expect("the forced pipeline path replies")
            .1
            .text
            .clone();
        assert!(reply.starts_with("**Aviva** was routed"), "{reply}");

        // Exercise image understanding at the canonical seam without a real
        // endpoint. This is the same fold the pipeline applies before intent
        // and persona selection after a configured provider describes bytes.
        let vision =
            crate::vision::RecordingVision::returning("a blue square beside the text ABBEY 427");
        let description = vision
            .describe(b"synthetic-network-parity-image".to_vec())
            .await
            .expect("recording vision succeeds");
        assert_eq!(
            vision.calls(),
            [crate::vision::VisionTask::Describe],
            "one description, no OCR or second-provider retry"
        );
        let enriched =
            crate::vision::fold_descriptions(text, &[("fixture.png".into(), description)]);
        let selected_persona = persona_for(&enriched, &settings);
        assert_eq!(selected_persona, Persona::Aviva);

        // Dispatch all five model-callable tools through the production
        // ToolScope. Their results must be network-neutral while every read
        // and mutation stays inside this event's network-prefixed scope.
        let mut host = crate::runtime::ToolScope {
            state: &state,
            network,
            scoped_guild: scoped_guild.clone(),
            scoped_user: scoped_user.clone(),
            scoped_channel: scoped_channel.clone(),
            now: 42,
            persona: selected_persona,
        };
        let calls = [
            crate::tools::ToolCall {
                id: "remember".into(),
                name: "remember_fact".into(),
                arguments: serde_json::json!({"fact": "  Sam\nlikes Rust.  "}),
            },
            crate::tools::ToolCall {
                id: "recall".into(),
                name: "recall".into(),
                arguments: serde_json::json!({"query": "Sam likes Rust"}),
            },
            crate::tools::ToolCall {
                id: "reputation".into(),
                name: "lookup_reputation".into(),
                arguments: serde_json::json!({"user_id": "u"}),
            },
            crate::tools::ToolCall {
                id: "recent".into(),
                name: "recent_messages".into(),
                arguments: serde_json::json!({"limit": 1}),
            },
            crate::tools::ToolCall {
                id: "persona".into(),
                name: "switch_persona".into(),
                arguments: serde_json::json!({"persona": "abi"}),
            },
        ];
        let tool_results = calls
            .iter()
            .map(|call| crate::tools::dispatch(call, &mut host).content)
            .collect::<Vec<_>>();
        assert_eq!(tool_results[0], "Stored: Sam likes Rust.");
        assert_eq!(tool_results[1], "• Sam likes Rust.");
        assert_eq!(
            tool_results[2],
            "Reputation 0.50 (0 = poor, 1 = excellent)."
        );
        assert!(tool_results[3].contains(text), "{}", tool_results[3]);
        assert!(tool_results[4].contains("Abi"), "{}", tool_results[4]);
        assert_eq!(host.persona, Persona::Abi);
        assert_eq!(
            state.memory_service().facts(&scoped_guild, &scoped_user),
            ["Sam likes Rust."]
        );

        behavior.push((reply, enriched, selected_persona, tool_results));
        scopes.push((scoped_guild, scoped_user, scoped_channel));
    }

    assert!(
        behavior.windows(2).all(|pair| pair[0] == pair[1]),
        "canonical replies, enrichment, persona, and tool results must match"
    );

    let stores = AppState::lock(&state.stores);
    for (index, (guild, user, channel_id)) in scopes.iter().enumerate() {
        let channel = stores
            .memory
            .channels
            .get(channel_id)
            .expect("each network records its own channel context");
        assert_eq!(channel.guild.as_deref(), Some(guild.as_str()));
        assert_eq!(channel.recent.len(), 1);
        for (other_index, (_, other_user, _)) in scopes.iter().enumerate() {
            if index != other_index {
                assert!(
                    stores.memory.facts(guild, other_user).is_empty(),
                    "a native-id collision on another network must not share facts"
                );
            }
        }
        assert_eq!(stores.memory.facts(guild, user), ["Sam likes Rust."]);
    }
    assert_eq!(stores.memory.channels.len(), networks.len());
    assert_eq!(stores.memory.fact_records().len(), networks.len());
    drop(stores);

    assert_eq!(
        AppState::lock(&state.brains).loaded_guilds(),
        ["discord:g", "slack:g", "telegram:g"]
    );
}

#[tokio::test]
async fn a_reaction_feeds_the_reward_collector() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    AppState::lock(&state.rewards).register_reply(vec![0.0; 18], 1, "abbey-msg", "discord:g", 0);
    let event = SocialEvent {
        kind: EventKind::Reaction {
            emoji: "🔥".into(),
            target_message_id: "abbey-msg".into(),
            added: true,
        },
        ..message("", Some("g"), "u2")
    };
    assert_eq!(
        handle(&state, &out, event, false, None).await,
        Outcome::Rewarded
    );
    let settled = AppState::lock(&state.rewards).settle_expired(10_000);
    assert_eq!(settled.len(), 1);
    assert!((settled[0].1.reward - 0.8).abs() < 1e-6, "−0.2 + 1.0");
}

/// An open Abbey turn in `discord:c1`, the scope `message()` produces,
/// answering `discord:u1` — the user the tests below speak as.
fn open_turn(state: &AppState, now: u64) {
    AppState::lock(&state.rewards).register_turn(ReplyTurn {
        state: vec![0.0; 18],
        action: BotAction::Reply.index(),
        sent_native_message_id: "abbey-msg".into(),
        scope: "discord:c1".into(),
        scoped_guild_id: "discord:g".into(),
        ask: "how do I configure the voice gateway timeout?".into(),
        asker: "discord:u1".into(),
        now,
    });
}

fn only_settled_reward(state: &AppState, now: u64) -> f32 {
    let settled = AppState::lock(&state.rewards).settle_expired(now);
    assert_eq!(settled.len(), 1);
    settled[0].1.reward
}

#[tokio::test]
async fn a_thanks_in_reply_to_abbey_reaches_the_delayed_channel() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    open_turn(&state, 0);
    // The guild has not opted in, so Abbey stays silent — but reward
    // bookkeeping for a turn she already took runs before the gates.
    let outcome = handle(
        &state,
        &out,
        message("thanks, that worked", Some("g"), "u1"),
        false,
        Some("abbey-msg"),
    )
    .await;
    assert_eq!(outcome, Outcome::Ignored("act off"));
    // −0.2 baseline + 0.5 untyped engagement + 1.0 typed thanks.
    let reward = only_settled_reward(&state, 10_000);
    assert!((reward - 1.3).abs() < 1e-6, "{reward}");
}

#[tokio::test]
async fn a_correction_in_reply_to_abbey_costs_more_than_silence() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    open_turn(&state, 0);
    let _ = handle(
        &state,
        &out,
        message("no, that's not the right port", Some("g"), "u1"),
        false,
        Some("abbey-msg"),
    )
    .await;
    // −0.2 + 0.5 engaged − 1.0 typed correction: below the −0.2 a turn
    // nobody answered would have settled at.
    let reward = only_settled_reward(&state, 10_000);
    assert!((reward + 0.7).abs() < 1e-6, "{reward}");
    assert!(reward < -0.2);
}

#[tokio::test]
async fn a_same_channel_follow_up_needs_no_reply_pointer() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let now = runtime::now();
    open_turn(&state, now);
    let _ = handle(
        &state,
        &out,
        message("does the gateway retry after a timeout?", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    // −0.2 + 0.4 topical follow-up. No untyped +0.5: there was no reply-to,
    // which is exactly the gap scope attribution exists to cover.
    let reward = only_settled_reward(
        &state,
        now + crate::brain::reward::SETTLEMENT_WINDOW_SECS + 1,
    );
    assert!((reward - 0.2).abs() < 1e-6, "{reward}");
}

#[tokio::test]
async fn a_bystanders_thanks_in_the_same_channel_is_not_credited() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let now = runtime::now();
    open_turn(&state, now);
    // u2 was never answered by Abbey — this is "thanks Carol!", not
    // feedback, and it must not move the turn u1's ask earned.
    let _ = handle(
        &state,
        &out,
        message("thanks, that worked", Some("g"), "u2"),
        false,
        None,
    )
    .await;
    let reward = only_settled_reward(
        &state,
        now + crate::brain::reward::SETTLEMENT_WINDOW_SECS + 1,
    );
    assert_eq!(reward.to_bits(), (-0.2f32).to_bits(), "{reward}");
}

#[tokio::test]
async fn unrelated_chatter_leaves_the_turn_exactly_as_it_was() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let now = runtime::now();
    open_turn(&state, now);
    let _ = handle(
        &state,
        &out,
        message("anyone up for lunch?", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    let reward = only_settled_reward(
        &state,
        now + crate::brain::reward::SETTLEMENT_WINDOW_SECS + 1,
    );
    assert_eq!(
        reward.to_bits(),
        (-0.2f32).to_bits(),
        "an off-topic message is no engagement, and no engagement is free"
    );
}

#[tokio::test]
async fn blank_unsolicited_content_is_not_learned_from() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let outcome = handle(&state, &out, message("", Some("g"), "u1"), false, None).await;
    assert_eq!(outcome, Outcome::Ignored("no content available"));
    assert!(AppState::lock(&state.brains).loaded_guilds().is_empty());
}

#[tokio::test]
async fn unsolicited_text_consults_the_policy_and_never_speaks_without_a_backend() {
    let state = AppState::in_memory();
    opt_in(&state, "discord:g", 6);
    let out = FakeOut::default();
    for _ in 0..20 {
        let outcome = handle(
            &state,
            &out,
            message("lol nice one", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        assert!(
            matches!(
                outcome,
                Outcome::Stayed | Outcome::Reacted | Outcome::CooledDown | Outcome::OverBudget
            ),
            "{outcome:?}"
        );
    }
    assert!(
        out.sent.lock().unwrap().is_empty(),
        "no backend → no text ever"
    );
    assert_eq!(AppState::lock(&state.brains).loaded_guilds(), ["discord:g"]);
}

#[tokio::test]
async fn a_disabled_guild_is_ignored_even_when_mentioned() {
    let state = AppState::in_memory();
    {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update("discord:g", &mut *stores, |s| s.enabled = false);
    }
    let out = FakeOut::default();
    let outcome = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
    assert_eq!(outcome, Outcome::Ignored("triage"));
}

/// Live: a DM through the real pipeline against whatever
/// `ABBEY_BOT_LLM_ENDPOINT` / `ABBEY_BOT_LLM_MODEL` name. Ignored by
/// default so the gate stays offline; run with
/// `cargo test live_dm -- --ignored --nocapture` when a backend is up.
#[tokio::test]
#[ignore = "needs a running generation backend"]
async fn live_dm_round_trip_against_the_configured_backend() {
    let Some(backend) = crate::llm::Backend::from_env() else {
        panic!("set ABBEY_BOT_LLM_ENDPOINT (and ABBEY_BOT_LLM_MODEL) to run this");
    };
    let mut state = AppState::in_memory();
    std::sync::Arc::get_mut(&mut state).unwrap().backend = Some(backend);
    let out = FakeOut::default();
    let first = handle(
        &state,
        &out,
        message("hey abbey", None, "donald"),
        false,
        None,
    )
    .await;
    assert_eq!(first, Outcome::Replied);
    let mut second_event = message("remember that I build in nightly Rust", None, "donald");
    second_event.native_message_id = "m2".into();
    let second = handle(&state, &out, second_event, false, None).await;
    assert_eq!(second, Outcome::Replied);
    let mut third_event = message("so what toolchain am I on?", None, "donald");
    third_event.native_message_id = "m3".into();
    let third = handle(&state, &out, third_event, false, None).await;
    assert_eq!(third, Outcome::Replied);
    let sent = out.sent.lock().unwrap();
    for (i, (_, m)) in sent.iter().enumerate() {
        eprintln!(
            "--- reply {} ({} chars):\n{}",
            i + 1,
            m.text.chars().count(),
            m.text
        );
        assert!(!m.text.contains("no generation backend"));
        assert!(m.text.chars().count() <= 2000);
    }
    assert_eq!(sent.len(), 3);
    assert_eq!(
        AppState::lock(&state.engine).session_len("discord:c1"),
        6,
        "three exchanges committed to one transcript"
    );
    assert!(
        sent[2].1.text.to_lowercase().contains("nightly"),
        "the transcript should carry the toolchain fact: {}",
        sent[2].1.text
    );
}

#[tokio::test]
async fn quiet_and_learning_off_gate_unsolicited_speech_before_the_policy() {
    let mut state = AppState::in_memory();
    std::sync::Arc::get_mut(&mut state).unwrap().quiet = true;
    let out = FakeOut::default();
    let outcome = handle(
        &state,
        &out,
        message("lol nice", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    assert_eq!(outcome, Outcome::Ignored("quiet"));
    assert!(
        AppState::lock(&state.brains).loaded_guilds().is_empty(),
        "nothing learned"
    );
    // A mention still answers under quiet (degraded — no backend).
    let outcome = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
    assert_eq!(outcome, Outcome::Replied);

    let state = AppState::in_memory();
    {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update("discord:g", &mut *stores, |s| {
            s.unsolicited = true;
            s.learning_enabled = false;
        });
    }
    let outcome = handle(
        &state,
        &out,
        message("lol nice", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    assert_eq!(outcome, Outcome::Ignored("learning off"));
}

#[tokio::test]
async fn a_forced_reply_loads_the_brain_so_its_reward_is_not_dropped() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    // No backend → degraded reply, but the brain must already be loaded.
    let outcome = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
    assert_eq!(outcome, Outcome::Replied);
    assert_eq!(AppState::lock(&state.brains).loaded_guilds(), ["discord:g"]);
    // Simulate a settled reward for that guild: it lands in the buffer.
    AppState::lock(&state.rewards).register_reply(vec![0.0; 18], 1, "sent-1", "discord:g", 0);
    AppState::lock(&state.rewards).reaction("👍", "sent-1", true);
    let settled = AppState::lock(&state.rewards).settle_expired(1_000);
    let mut brains = AppState::lock(&state.brains);
    for (g, exp) in settled {
        brains.remember(&g, exp);
    }
    assert_eq!(brains.get("discord:g").map(|b| b.buffer_len()), Some(1));
}

fn opt_in(state: &AppState, guild: &str, per_hour: u32) {
    let mut stores = AppState::lock(&state.stores);
    AppState::lock(&state.guilds).update(guild, &mut *stores, |s| {
        s.unsolicited = true;
        s.unsolicited_per_hour = per_hour;
        s.reply_cooldown_seconds = 0;
        // Learning is opt-in and default-off, so a helper named `opt_in` has to
        // say so. It used to rely on the default being true, which is exactly
        // the implicit consent the default-off change removes.
        s.learning_enabled = true;
    });
}

#[tokio::test]
async fn a_guild_that_has_not_opted_in_is_ignored_before_the_policy() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let outcome = handle(
        &state,
        &out,
        message("lol nice", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    assert_eq!(outcome, Outcome::Ignored("act off"));
    assert!(
        AppState::lock(&state.brains).loaded_guilds().is_empty(),
        "no brain, no experience"
    );
}

#[tokio::test]
async fn an_opted_in_guild_consults_the_policy_and_records_the_decision() {
    let state = AppState::in_memory();
    opt_in(&state, "discord:g", 6);
    let out = FakeOut::default();
    let outcome = handle(
        &state,
        &out,
        message("lol nice", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    assert!(
        matches!(
            outcome,
            Outcome::Stayed | Outcome::Reacted | Outcome::OverBudget
        ),
        "{outcome:?} (no backend → reply degrades to Stayed)"
    );
    let brains = AppState::lock(&state.brains);
    let stats = brains
        .stats("discord:g")
        .expect("brain loaded by the decision");
    assert_eq!(
        stats.action_counts.iter().sum::<u64>(),
        1,
        "exactly one policy decision"
    );
    assert_eq!(stats.last_q.len(), 3);
    assert_eq!(stats.forced_replies, 0);
}

#[tokio::test]
async fn a_mention_counts_as_forced_not_as_a_decision() {
    let state = AppState::in_memory();
    let out = FakeOut::default();
    let _ = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
    let brains = AppState::lock(&state.brains);
    let stats = brains.stats("discord:g").unwrap();
    assert_eq!(stats.forced_replies, 1);
    assert_eq!(stats.action_counts, [0, 0, 0]);
}

#[tokio::test]
async fn over_budget_is_silent_and_unlearned() {
    let state = AppState::in_memory();
    opt_in(&state, "discord:g", 1);
    assert!(AppState::lock(&state.budget).try_take("discord:g", 1, runtime::now()));
    let out = FakeOut::default();
    let mut saw_over_budget = false;
    for i in 0..400 {
        let mut m = message("lol nice", Some("g"), "u1");
        m.native_message_id = format!("m{i}");
        match handle(&state, &out, m, false, None).await {
            Outcome::OverBudget => {
                saw_over_budget = true;
                break;
            }
            Outcome::Stayed => continue,
            other => panic!("acted past the budget: {other:?}"),
        }
    }
    assert!(
        saw_over_budget,
        "the policy never picked reply/react in 400 tries"
    );
    assert!(out.reacted.lock().unwrap().is_empty());
    assert!(out.sent.lock().unwrap().is_empty());
    let brains = AppState::lock(&state.brains);
    let stats = brains.stats("discord:g").unwrap();
    let stays = stats.action_counts[BotAction::Stay.index()];
    assert_eq!(brains.get("discord:g").unwrap().buffer_len() as u64, stays);
}

#[tokio::test]
async fn a_reply_the_bot_cannot_make_does_not_burn_budget() {
    let state = AppState::in_memory();
    opt_in(&state, "discord:g", 6);
    let out = FakeOut::default();
    // Walk the policy until it picks Reply at least a few times; with no
    // backend each one degrades to Stayed — and must leave the budget full
    // minus only the reacts that actually went out.
    let mut reacted = 0u32;
    for i in 0..60 {
        let mut m = message("lol nice", Some("g"), "u1");
        m.native_message_id = format!("m{i}");
        match handle(&state, &out, m, false, None).await {
            Outcome::Reacted => reacted += 1,
            Outcome::Stayed | Outcome::OverBudget => {}
            other => panic!("{other:?}"),
        }
    }
    let left = AppState::lock(&state.budget).tokens_left("discord:g", 6, runtime::now());
    assert!(
        (6.0 - left - reacted as f32).abs() < 0.05,
        "tokens spent ({}) should equal reacts sent ({reacted}); phantom replies must not cost quota",
        6.0 - left
    );
}

#[tokio::test]
async fn a_member_join_welcome_is_gated_like_any_unsolicited_speech() {
    let out = FakeOut::default();
    let join = |guild: &str| SocialEvent {
        kind: EventKind::MemberJoined,
        ..message("", Some(guild), "newbie")
    };
    // Quiet wins.
    let mut state = AppState::in_memory();
    std::sync::Arc::get_mut(&mut state).unwrap().quiet = true;
    assert_eq!(
        handle(&state, &out, join("g"), false, None).await,
        Outcome::Ignored("quiet")
    );
    // Not opted in.
    let state = AppState::in_memory();
    assert_eq!(
        handle(&state, &out, join("g"), false, None).await,
        Outcome::Ignored("act off")
    );
    // Opted in, no backend → honest silence, nothing sent.
    opt_in(&state, "discord:g", 6);
    assert_eq!(
        handle(&state, &out, join("g"), false, None).await,
        Outcome::Ignored("welcome needs a backend")
    );
    assert!(out.sent.lock().unwrap().is_empty());
}

#[test]
fn two_dm_users_never_share_recall_or_facts() {
    let state = AppState::in_memory();
    let alice = message("hi", None, "alice");
    let bob = message("hi", None, "bob");
    assert_ne!(alice.scoped_guild_id(), bob.scoped_guild_id());
    AppState::lock(&state.recall).remember(
        &alice.scoped_guild_id(),
        &alice.scoped_user_id(),
        "alice likes rust",
        1,
    );
    let for_bob = assemble_context(
        &state,
        &bob.scoped_guild_id(),
        &bob.scoped_user_id(),
        &bob.scoped_channel_id(),
        "rust",
        0.5,
    );
    assert!(for_bob.user_facts.is_empty(), "{:?}", for_bob.user_facts);
    let for_alice = assemble_context(
        &state,
        &alice.scoped_guild_id(),
        &alice.scoped_user_id(),
        &alice.scoped_channel_id(),
        "rust",
        0.5,
    );
    assert_eq!(for_alice.user_facts, ["alice likes rust"]);
}

#[test]
fn two_users_in_one_guild_never_share_semantic_recall() {
    let state = AppState::in_memory();
    AppState::lock(&state.recall).remember(
        "discord:g",
        "discord:alice",
        "alice's private editor preference is helix",
        1,
    );
    AppState::lock(&state.recall).remember(
        "discord:g",
        "discord:bob",
        "bob's private editor preference is vim",
        2,
    );
    let alice = assemble_context(
        &state,
        "discord:g",
        "discord:alice",
        "discord:c",
        "editor preference",
        0.5,
    );
    let bob = assemble_context(
        &state,
        "discord:g",
        "discord:bob",
        "discord:c",
        "editor preference",
        0.5,
    );
    assert_eq!(
        alice.user_facts,
        ["alice's private editor preference is helix"]
    );
    assert_eq!(bob.user_facts, ["bob's private editor preference is vim"]);
}

#[test]
fn assembled_context_uses_the_live_social_standing_snapshot() {
    let state = AppState::in_memory();
    AppState::lock(&state.stores)
        .memory
        .user_mut("discord:g", "discord:u")
        .reputation = 0.12;
    let live = {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).record_interaction(
            "discord:u",
            "discord:g",
            1.0,
            7,
            &mut *stores,
        )
    };

    let context = assemble_context(
        &state,
        "discord:g",
        "discord:u",
        "discord:c",
        "standing",
        live,
    );

    assert_eq!(context.reputation, live);
    assert_ne!(context.reputation, 0.12);
}

#[test]
fn persona_routing_follows_the_canonical_abi_router() {
    let s = GuildSettings::default();
    assert_eq!(persona_for("hello there", &s), Persona::Abbey);
    assert_eq!(
        persona_for("execute the deploy quickly", &s),
        Persona::Aviva
    );
    assert_eq!(persona_for("ABI: review governance risk", &s), Persona::Abi);
    let aviva = GuildSettings {
        default_persona: Persona::Aviva,
        ..GuildSettings::default()
    };
    assert_eq!(persona_for("hello there", &aviva), Persona::Aviva);
    assert_eq!(persona_for("Abbey, help me", &aviva), Persona::Abbey);
    assert_eq!(
        persona_for_session("and what about tomorrow?", &s, Some(Persona::Aviva)),
        Persona::Aviva,
        "a neutral follow-up keeps a tool-selected persona"
    );
    assert_eq!(
        persona_for_session("Abbey, take this one", &s, Some(Persona::Aviva)),
        Persona::Abbey,
        "an explicit canonical name still overrides sticky state"
    );
}

/// The signal layer is only worth having if it survives the `Default`
/// sentinel: before it, a distressed message carried no canonical evidence,
/// so its route was discarded and the guild (or sticky) persona answered —
/// which is the one case where the wrong register hurts most.
#[test]
fn distress_outranks_the_guild_default_and_sticky_state() {
    let aviva = GuildSettings {
        default_persona: Persona::Aviva,
        ..GuildSettings::default()
    };
    let distressed = "I'm completely stuck and losing my mind on this";
    assert_eq!(
        crate::persona::route(distressed, None).reason,
        crate::persona::Reason::Default,
        "precondition: the canonical keyword table sees nothing here"
    );
    assert_eq!(persona_for(distressed, &aviva), Persona::Abbey);
    assert_eq!(
        persona_for_session(distressed, &aviva, Some(Persona::Abi)),
        Persona::Abbey,
        "distress is a handoff, not a follow-up"
    );

    // Terse urgency goes the other way, and neutral text still sticks.
    let s = GuildSettings::default();
    assert_eq!(persona_for("restart it now", &s), Persona::Aviva);
    assert_eq!(
        persona_for_session("and what about tomorrow?", &aviva, Some(Persona::Abi)),
        Persona::Abi,
        "text neutral to both layers keeps the sticky persona"
    );
}
