use super::*;

fn reaction_event(added: bool, user: &str) -> SocialEvent {
    SocialEvent {
        kind: EventKind::Reaction {
            emoji: "👍".into(),
            target_message_id: "abbey-msg".into(),
            added,
        },
        ..message("", Some("g"), user)
    }
}

#[tokio::test]
async fn disabled_learning_unknown_and_bot_reactors_never_credit_or_track() {
    for case in ["disabled", "unknown", "bot", "cross-scope"] {
        let state = AppState::in_memory();
        let now = runtime::now();
        open_turn(&state, now);
        if case != "disabled" {
            enable_feedback(&state);
        }
        let before = AppState::lock(&state.rewards).clone();
        let mut event = reaction_event(true, if case == "unknown" { "" } else { "u1" });
        event.is_bot = case == "bot";
        if case == "cross-scope" {
            event.native_channel_id = "other".into();
        }
        handle(&state, &FakeOut::default(), event, false, None).await;
        assert_eq!(*AppState::lock(&state.rewards), before, "{case}");
        if case != "disabled" {
            let audit = AppState::lock(&state.brains).learning_audit("discord:g");
            if case == "expired" {
                assert_eq!(audit.expired, 1);
            } else {
                assert_eq!(audit.unsupported, 1);
            }
        }
    }
}

#[tokio::test]
async fn reaction_duplicate_add_and_remove_flow_through_real_pipeline() {
    let state = AppState::in_memory();
    let now = runtime::now();
    open_turn(&state, now);
    enable_feedback(&state);
    let out = FakeOut::default();
    for added in [true, true, false, false] {
        handle(&state, &out, reaction_event(added, "u1"), false, None).await;
    }
    assert!((only_settled_reward(&state, now + 151) + 0.2).abs() < 1e-6);
    assert!(out.sent.lock().unwrap().is_empty());
    let audit = AppState::lock(&state.brains).learning_audit("discord:g");
    assert_eq!(audit.exact, 2);
    assert_eq!(audit.duplicate, 2);
}

#[tokio::test]
async fn exact_reply_refuses_disabled_expired_wrong_scope_or_unidentified_observer() {
    for case in ["disabled", "expired", "wrong-scope", "unknown"] {
        let state = AppState::in_memory();
        let now = runtime::now();
        open_turn(&state, if case == "expired" { now - 151 } else { now });
        if case != "disabled" {
            enable_feedback(&state);
        }
        let before = AppState::lock(&state.rewards).clone();
        let mut event = message(
            "thanks, that worked",
            Some("g"),
            if case == "unknown" { "" } else { "u1" },
        );
        if case == "wrong-scope" {
            event.native_channel_id = "other".into();
        }
        handle(&state, &FakeOut::default(), event, false, Some("abbey-msg")).await;
        assert_eq!(*AppState::lock(&state.rewards), before, "{case}");
        if case != "disabled" {
            let audit = AppState::lock(&state.brains).learning_audit("discord:g");
            if case == "expired" {
                assert_eq!(audit.expired, 1);
            } else {
                assert_eq!(audit.unsupported, 1);
            }
        }
    }
}

#[tokio::test]
async fn competing_pipeline_turns_refuse_unpointed_feedback_then_accept_exact() {
    let state = AppState::in_memory();
    let now = runtime::now();
    open_turn(&state, now);
    enable_feedback(&state);
    AppState::lock(&state.rewards).register_turn(ReplyTurn {
        state: vec![0.0; 18],
        action: 1,
        sent_native_message_id: "other".into(),
        scope: "discord:c1".into(),
        scoped_guild_id: "discord:g".into(),
        ask: "another question".into(),
        asker: "discord:u1".into(),
        now,
    });
    let before = AppState::lock(&state.rewards).clone();
    let out = FakeOut::default();
    handle(
        &state,
        &out,
        message("thanks, that worked", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    assert_eq!(*AppState::lock(&state.rewards), before);
    handle(
        &state,
        &out,
        message("thanks, that worked", Some("g"), "u1"),
        false,
        Some("abbey-msg"),
    )
    .await;
    let audit = AppState::lock(&state.brains).learning_audit("discord:g");
    assert_eq!(audit.ambiguous, 1);
    assert_eq!(audit.exact, 1);
    let rows = AppState::lock(&state.rewards).export_pending();
    assert_eq!(
        rows.iter()
            .find(|(id, _)| id == "abbey-msg")
            .unwrap()
            .1
            .delayed_count,
        1
    );
    assert_eq!(
        rows.iter()
            .find(|(id, _)| id == "other")
            .unwrap()
            .1
            .delayed_count,
        0
    );
}

#[tokio::test]
async fn unique_feedback_audit_records_without_admitting_or_loading_a_policy() {
    let state = AppState::in_memory();
    open_turn(&state, runtime::now());
    enable_feedback(&state);
    handle(
        &state,
        &FakeOut::default(),
        message("does the gateway retry after a timeout?", Some("g"), "u1"),
        false,
        None,
    )
    .await;
    let brains = AppState::lock(&state.brains);
    assert_eq!(brains.learning_audit("discord:g").unique, 1);
    assert!(brains.loaded_guilds().is_empty());
    assert_eq!(brains.experience_count("discord:g"), None);
    drop(brains);
    assert_eq!(
        AppState::lock(&state.rewards).export_pending()[0]
            .1
            .delayed_count,
        1
    );
}
