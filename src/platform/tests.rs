use super::*;

fn event(network: SocialNetwork, guild: Option<&str>) -> SocialEvent {
    SocialEvent {
        network,
        kind: EventKind::Message {
            text: "hi".into(),
            attachments: vec![],
        },
        native_message_id: "m1".into(),
        native_channel_id: "c1".into(),
        native_guild_id: guild.map(str::to_string),
        native_user_id: "u1".into(),
        user_display_name: "Ada".into(),
        is_bot: false,
        timestamp: 0,
    }
}

// ---- common types ----

#[test]
fn network_names_match_the_serde_form() {
    for n in [
        SocialNetwork::Discord,
        SocialNetwork::Telegram,
        SocialNetwork::Slack,
    ] {
        let json = serde_json::to_string(&n).expect("serializes");
        assert_eq!(json, format!("\"{}\"", n.as_str()));
        let back: SocialNetwork = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, n);
    }
}

#[test]
fn scoped_ids_are_network_prefixed_and_dm_when_guildless() {
    let e = event(SocialNetwork::Telegram, Some("g9"));
    assert_eq!(e.scoped_guild_id(), "telegram:g9");
    assert_eq!(e.scoped_channel_id(), "telegram:c1");
    assert_eq!(e.scoped_user_id(), "telegram:u1");

    let dm = event(SocialNetwork::Discord, None);
    assert_eq!(
        dm.scoped_guild_id(),
        "discord:dm:u1",
        "a DM is a one-person guild"
    );
}

#[test]
fn attachment_kind_is_read_off_the_content_type_prefix() {
    let img = RemoteAttachment {
        url: "u".into(),
        filename: "a.png".into(),
        content_type: Some("image/png".into()),
    };
    assert!(img.is_image());
    let none = RemoteAttachment {
        content_type: None,
        ..img
    };
    assert!(!none.is_image());
}

// ---- triage ----

#[test]
fn triage_drops_bots_before_anything_else() {
    let mut e = event(SocialNetwork::Discord, Some("g"));
    e.is_bot = true;
    assert_eq!(
        triage(&e, true),
        RouteDecision::Ignore(IgnoreReason::FromBot)
    );
    // Even with the guild disabled, the bot reason wins — it is checked first.
    assert_eq!(
        triage(&e, false),
        RouteDecision::Ignore(IgnoreReason::FromBot)
    );
}

#[test]
fn triage_drops_events_from_disabled_guilds() {
    let e = event(SocialNetwork::Discord, Some("g"));
    assert_eq!(
        triage(&e, false),
        RouteDecision::Ignore(IgnoreReason::GuildDisabled)
    );
}

#[test]
fn triage_routes_each_kind() {
    let mut e = event(SocialNetwork::Discord, Some("g"));
    assert!(matches!(triage(&e, true), RouteDecision::Consider { text, .. } if text == "hi"));

    e.kind = EventKind::Reaction {
        emoji: "👍".into(),
        target_message_id: "m0".into(),
        added: true,
    };
    assert_eq!(
        triage(&e, true),
        RouteDecision::Reward {
            emoji: "👍".into(),
            target_message_id: "m0".into(),
            added: true
        }
    );

    e.kind = EventKind::MemberJoined;
    assert_eq!(
        triage(&e, true),
        RouteDecision::Welcome {
            display_name: "Ada".into()
        }
    );
}

// ---- telegram ----

const TG_GROUP: &str = r#"{
        "update_id": 41,
        "message": {
            "message_id": 7,
            "from": {"id": 100, "is_bot": false, "first_name": "Ada", "last_name": "Lovelace"},
            "chat": {"id": -2001, "type": "supergroup"},
            "date": 1700000000,
            "caption": "look",
            "photo": [
                {"file_id": "small", "width": 90, "height": 90},
                {"file_id": "big", "width": 1280, "height": 720},
                {"file_id": "mid", "width": 320, "height": 320}
            ]
        }
    }"#;

#[test]
fn telegram_group_message_translates_with_largest_photo() {
    let update: TgUpdate = serde_json::from_str(TG_GROUP).expect("parses");
    let e = translate_telegram(&update).expect("a message with a sender");
    assert_eq!(e.network, SocialNetwork::Telegram);
    assert_eq!(e.native_guild_id.as_deref(), Some("-2001"));
    assert_eq!(e.native_channel_id, "-2001");
    assert_eq!(e.native_message_id, "7");
    assert_eq!(e.native_user_id, "100");
    assert_eq!(e.user_display_name, "Ada Lovelace");
    assert!(!e.is_bot);
    assert_eq!(e.timestamp, 1_700_000_000);
    match &e.kind {
        EventKind::Message { text, attachments } => {
            assert_eq!(text, "look", "caption stands in for text");
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].url, "tgfile://big");
            assert!(attachments[0].is_image());
            assert_eq!(tgfile_id(&attachments[0].url), Some("big"));
        }
        other => panic!("expected a message, got {other:?}"),
    }
}

#[test]
fn telegram_private_chat_has_no_guild_and_single_name() {
    let raw = r#"{"update_id": 1, "message": {
            "message_id": 2,
            "from": {"id": 5, "is_bot": true, "first_name": "Bot"},
            "chat": {"id": 5, "type": "private"},
            "date": 10,
            "text": "hello"
        }}"#;
    let update: TgUpdate = serde_json::from_str(raw).expect("parses");
    let e = translate_telegram(&update).expect("translates");
    assert_eq!(e.native_guild_id, None);
    assert!(e.scoped_guild_id().starts_with("telegram:dm:"));
    assert_eq!(e.user_display_name, "Bot");
    assert!(e.is_bot);
    assert!(matches!(
        &e.kind,
        EventKind::Message { text, attachments } if text == "hello" && attachments.is_empty()
    ));
}

#[test]
fn telegram_update_without_message_or_sender_is_skipped() {
    let no_message: TgUpdate = serde_json::from_str(r#"{"update_id": 3}"#).expect("parses");
    assert!(translate_telegram(&no_message).is_none());
    let no_sender: TgUpdate = serde_json::from_str(
            r#"{"update_id": 4, "message": {"message_id": 1, "chat": {"id": 1, "type": "channel"}, "date": 0}}"#,
        )
        .expect("parses");
    assert!(translate_telegram(&no_sender).is_none());
}

#[test]
fn telegram_response_envelope_parses() {
    let raw = format!(r#"{{"ok": true, "result": [{TG_GROUP}]}}"#);
    let resp: TgResponse<Vec<TgUpdate>> = serde_json::from_str(&raw).expect("parses");
    assert!(resp.ok);
    assert_eq!(resp.result.as_ref().map(Vec::len), Some(1));
    let file: TgResponse<TgFile> =
        serde_json::from_str(r#"{"ok": true, "result": {"file_path": "photos/1.jpg"}}"#)
            .expect("parses");
    assert_eq!(
        file.result.and_then(|f| f.file_path).as_deref(),
        Some("photos/1.jpg")
    );
}

#[test]
fn telegram_poller_advances_past_the_highest_update_id() {
    let updates: Vec<TgUpdate> =
        serde_json::from_str(r#"[{"update_id": 10}, {"update_id": 12}, {"update_id": 11}]"#)
            .expect("parses");
    let mut poller = TelegramPoller::default();
    assert_eq!(poller.next_offset_after(&updates), 13);
    assert_eq!(poller.advance(&updates), 13);
    // An empty batch leaves the cursor alone; an older batch never rewinds it.
    assert_eq!(poller.next_offset_after(&[]), 13);
    assert_eq!(poller.advance(&updates), 13);
    assert_eq!(
        get_updates_url("https://api.telegram.org/botTOKEN/", poller.offset),
        "https://api.telegram.org/botTOKEN/getUpdates?timeout=50&offset=13"
    );
}

#[test]
fn telegram_urls_join_without_doubled_slashes() {
    assert_eq!(
        get_file_url("https://api.telegram.org/botT", "abc"),
        "https://api.telegram.org/botT/getFile?file_id=abc"
    );
    assert_eq!(
        resolve_file_url("https://api.telegram.org/file/botT/", "/photos/1.jpg"),
        "https://api.telegram.org/file/botT/photos/1.jpg"
    );
    assert_eq!(tgfile_id("https://x"), None);
    assert_eq!(tgfile_id("tgfile://"), None);
}

#[test]
fn telegram_send_payload_bolds_the_title_and_threads_numeric_replies() {
    let msg = OutboundMessage {
        text: "body".into(),
        reply_to_native_message_id: Some("42".into()),
        title: Some("Abbey".into()),
        accent_color: Some(0xff00ff),
    };
    assert_eq!(
        telegram_send_payload(&msg, "-2001"),
        json!({
            "chat_id": "-2001",
            "text": "*Abbey*\nbody",
            "parse_mode": "Markdown",
            "reply_to_message_id": 42,
        })
    );
    let plain = OutboundMessage {
        text: "just text".into(),
        reply_to_native_message_id: Some("not-a-number".into()),
        ..Default::default()
    };
    let payload = telegram_send_payload(&plain, "1");
    assert_eq!(payload["text"], "just text");
    assert!(payload.get("reply_to_message_id").is_none());
}

// ---- slack ----

#[test]
fn an_envelope_with_no_event_yields_nothing() {
    let env: SlackEnvelope =
        serde_json::from_str(r#"{"type": "url_verification", "challenge": "abc123"}"#)
            .expect("parses");
    assert!(translate_slack(&env).is_none());
}

#[test]
fn slack_user_message_translates_and_bot_message_is_ignored() {
    let raw = r#"{
            "team_id": "T1",
            "event": {
                "type": "message", "user": "U1", "text": "hey",
                "channel": "C1", "ts": "1700000000.000100", "event_ts": "1700000000.000100",
                "files": [{"name": "cat.png", "mimetype": "image/png", "url_private": "https://files.slack.com/x"}]
            }
        }"#;
    let env: SlackEnvelope = serde_json::from_str(raw).expect("parses");
    let e = translate_slack(&env).expect("a user message");
    assert_eq!(e.network, SocialNetwork::Slack);
    assert_eq!(e.native_guild_id.as_deref(), Some("T1"));
    assert_eq!(e.native_channel_id, "C1");
    assert_eq!(e.native_message_id, "1700000000.000100");
    assert_eq!(e.native_user_id, "U1");
    assert_eq!(e.timestamp, 1_700_000_000);
    match &e.kind {
        EventKind::Message { text, attachments } => {
            assert_eq!(text, "hey");
            assert_eq!(attachments.len(), 1);
            assert!(attachments[0].is_image());
            assert_eq!(attachments[0].filename, "cat.png");
        }
        other => panic!("expected a message, got {other:?}"),
    }

    let bot = r#"{"team_id": "T1", "event": {"type": "message", "user": "U2", "bot_id": "B1", "text": "beep", "channel": "C1", "ts": "1.2"}}"#;
    let env: SlackEnvelope = serde_json::from_str(bot).expect("parses");
    assert!(translate_slack(&env).is_none(), "bot messages are dropped");
}

#[test]
fn slack_reactions_become_reward_events() {
    let raw = r#"{"team_id": "T1", "event": {"type": "reaction_removed", "user": "U1", "reaction": "thumbsup", "item": {"channel": "C1", "ts": "9.9"}, "event_ts": "10.0"}}"#;
    let env: SlackEnvelope = serde_json::from_str(raw).expect("parses");
    let e = translate_slack(&env).expect("a reaction");
    assert_eq!(
        e.kind,
        EventKind::Reaction {
            emoji: "thumbsup".into(),
            target_message_id: "9.9".into(),
            added: false
        }
    );
    assert_eq!(e.native_channel_id, "C1");
    assert_eq!(e.timestamp, 10);
}

#[test]
fn slack_post_payload_is_mrkdwn_with_bold_title_and_thread() {
    let msg = OutboundMessage {
        text: "body".into(),
        reply_to_native_message_id: Some("1.2".into()),
        title: Some("Aviva".into()),
        accent_color: None,
    };
    assert_eq!(
        slack_post_message_payload(&msg, "C1"),
        json!({"channel": "C1", "text": "*Aviva*\nbody", "mrkdwn": true, "thread_ts": "1.2"})
    );
}

// ---- crypto ----
