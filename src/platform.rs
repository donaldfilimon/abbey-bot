//! The platform layer: one event model for every network Abbey speaks on.
//!
//! Port of `docs/spec/platforms.md`, pure parts only. Adapters own transport
//! (gateway, long-poll, HTTP); *this* module owns what an adapter translates
//! into and out of — [`SocialEvent`] inbound, [`OutboundMessage`] outbound —
//! plus the per-network wire types, the pure translate functions, the Slack
//! request-signature check, and the first two guards of the router.
//!
//! Nothing here imports serenity, poise, or reqwest. The Discord translation
//! is deliberately absent: building a [`SocialEvent`] from serenity types is
//! the gateway/commands code's job, because serenity is forbidden in the pure
//! modules (see `CLAUDE.md`). Likewise, no type in this module holds a bot
//! token or signing secret — callers pass `base: &str` / `signing_secret: &str`
//! per call, so there is no struct whose `Debug` could print a credential.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

/// Which network an event came from. The serialized form is the lowercase
/// name, which is also the prefix of every scoped id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SocialNetwork {
    Discord,
    Telegram,
    Slack,
}

impl SocialNetwork {
    /// The wire/prefix name: `"discord"`, `"telegram"`, `"slack"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::Slack => "slack",
        }
    }
}

/// A file attached to an inbound message, by reference. Fetching is the
/// vision layer's job; this only says where it is and what it claims to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAttachment {
    pub url: String,
    pub filename: String,
    pub content_type: Option<String>,
}

impl RemoteAttachment {
    pub fn is_image(&self) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|c| c.starts_with("image/"))
    }
}

/// What kind of thing happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Message {
        text: String,
        attachments: Vec<RemoteAttachment>,
    },
    Reaction {
        emoji: String,
        target_message_id: String,
        added: bool,
    },
    MemberJoined,
}

/// One inbound thing that happened on any network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialEvent {
    pub network: SocialNetwork,
    pub kind: EventKind,
    pub native_message_id: String,
    pub native_channel_id: String,
    /// `None` for DMs and for networks without a guild concept.
    pub native_guild_id: Option<String>,
    pub native_user_id: String,
    pub user_display_name: String,
    pub is_bot: bool,
    /// Unix seconds.
    pub timestamp: u64,
}

impl SocialEvent {
    /// `"{network}:{guild}"`, or `"{network}:dm:{user}"` when there is no
    /// guild. Everything downstream stores and looks up by these, so two
    /// networks with colliding native ids never share a row — and two people
    /// DMing the bot never share a namespace either: a DM is its own
    /// one-person guild for config, reputation, facts, and recall. A shared
    /// `"{network}:dm"` would let semantic recall surface one user's facts to
    /// another, which is the one thing the isolation invariant must never do.
    pub fn scoped_guild_id(&self) -> String {
        match self.native_guild_id.as_deref() {
            Some(guild) => format!("{}:{guild}", self.network.as_str()),
            None => format!("{}:dm:{}", self.network.as_str(), self.native_user_id),
        }
    }

    pub fn scoped_channel_id(&self) -> String {
        format!("{}:{}", self.network.as_str(), self.native_channel_id)
    }

    pub fn scoped_user_id(&self) -> String {
        format!("{}:{}", self.network.as_str(), self.native_user_id)
    }
}

/// What Abbey sends back. Rich fields degrade per network: Discord renders an
/// embed, Telegram gets Markdown, Slack gets mrkdwn — the payload builders
/// below own that degradation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutboundMessage {
    pub text: String,
    pub reply_to_native_message_id: Option<String>,
    pub title: Option<String>,
    pub accent_color: Option<u32>,
}

// ---------------------------------------------------------------------------
// Router: the guards that need no other subsystem
// ---------------------------------------------------------------------------

/// Why the router dropped an event before doing anything with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// Bots never talk to bots — the loop-breaker.
    FromBot,
    /// The guild's config says Abbey is off here.
    GuildDisabled,
}

/// The router's verdict on an event, up to the point where other subsystems
/// (intent, state, the per-guild brain) take over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    Ignore(IgnoreReason),
    /// A reaction is the reward channel: hand it to the reward collector.
    Reward {
        emoji: String,
        target_message_id: String,
        added: bool,
    },
    /// A member joined: always a warm welcome, no brain consulted.
    Welcome {
        display_name: String,
    },
    /// A message worth running through the pipeline.
    Consider {
        text: String,
        attachments: Vec<RemoteAttachment>,
    },
}

/// The first two guards of `SocialRouter.handle` plus the kind switch. Pure:
/// the caller has already looked up whether the guild is enabled.
pub fn triage(event: &SocialEvent, guild_enabled: bool) -> RouteDecision {
    if event.is_bot {
        return RouteDecision::Ignore(IgnoreReason::FromBot);
    }
    if !guild_enabled {
        return RouteDecision::Ignore(IgnoreReason::GuildDisabled);
    }
    match &event.kind {
        EventKind::Message { text, attachments } => RouteDecision::Consider {
            text: text.clone(),
            attachments: attachments.clone(),
        },
        EventKind::Reaction {
            emoji,
            target_message_id,
            added,
        } => RouteDecision::Reward {
            emoji: emoji.clone(),
            target_message_id: target_message_id.clone(),
            added: *added,
        },
        EventKind::MemberJoined => RouteDecision::Welcome {
            display_name: event.user_display_name.clone(),
        },
    }
}

// ---------------------------------------------------------------------------
// Telegram — Bot API wire types and pure translation
// ---------------------------------------------------------------------------

/// Every Bot API reply: `{ "ok": bool, "result": T }`.
#[derive(Debug, Clone, Deserialize)]
pub struct TgResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgUpdate {
    pub update_id: i64,
    pub message: Option<TgMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgMessage {
    pub message_id: i64,
    pub from: Option<TgUser>,
    pub chat: TgChat,
    pub date: i64,
    pub text: Option<String>,
    pub caption: Option<String>,
    pub photo: Option<Vec<TgPhotoSize>>,
    /// The message this one replies to, if any — the human-reply reward signal.
    #[serde(default)]
    pub reply_to_message: Option<Box<TgReplyRef>>,
}

/// The subset of a replied-to message the reward path needs.
#[derive(Debug, Clone, Deserialize)]
pub struct TgReplyRef {
    pub message_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgUser {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgPhotoSize {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgFile {
    pub file_path: Option<String>,
}

/// The pseudo-URL scheme a Telegram photo carries until `getFile` resolves it.
pub const TGFILE_SCHEME: &str = "tgfile://";

/// Translate one update into an event, or `None` when it carries nothing the
/// pipeline handles (no message, or a message with no sender).
///
/// A photo message carries several sizes; the largest by pixel area becomes a
/// single `tgfile://{file_id}` attachment, because turning a `file_id` into a
/// URL costs a `getFile` round trip that belongs at fetch time, not here.
pub fn translate_telegram(update: &TgUpdate) -> Option<SocialEvent> {
    let m = update.message.as_ref()?;
    let from = m.from.as_ref()?;

    let attachments = m
        .photo
        .as_deref()
        .and_then(|sizes| {
            sizes
                .iter()
                .max_by_key(|p| u64::from(p.width) * u64::from(p.height))
        })
        .map(|largest| RemoteAttachment {
            url: format!("{TGFILE_SCHEME}{}", largest.file_id),
            filename: "photo.jpg".to_string(),
            content_type: Some("image/jpeg".to_string()),
        })
        .into_iter()
        .collect();

    let text = m
        .text
        .clone()
        .or_else(|| m.caption.clone())
        .unwrap_or_default();

    let display_name = match &from.last_name {
        Some(last) => format!("{} {}", from.first_name, last),
        None => from.first_name.clone(),
    };

    Some(SocialEvent {
        network: SocialNetwork::Telegram,
        kind: EventKind::Message { text, attachments },
        native_message_id: m.message_id.to_string(),
        native_channel_id: m.chat.id.to_string(),
        native_guild_id: (m.chat.kind != "private").then(|| m.chat.id.to_string()),
        native_user_id: from.id.to_string(),
        user_display_name: display_name,
        is_bot: from.is_bot,
        timestamp: u64::try_from(m.date).unwrap_or(0),
    })
}

/// The JSON body for `sendMessage`. Markdown parse mode; a title becomes a
/// bold first line.
pub fn telegram_send_payload(message: &OutboundMessage, chat_id: &str) -> Value {
    let text = match &message.title {
        Some(title) => format!("*{title}*\n{}", message.text),
        None => message.text.clone(),
    };
    let mut payload = json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "Markdown",
    });
    // Telegram wants a numeric message id; a non-numeric one is silently not
    // a reply rather than a rejected request.
    if let Some(id) = message
        .reply_to_native_message_id
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
    {
        payload["reply_to_message_id"] = json!(id);
    }
    payload
}

/// The long-poll cursor. The loop itself (HTTP, backoff) lives with the
/// orchestrator; this holds only the arithmetic so it is testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TelegramPoller {
    pub offset: i64,
}

impl TelegramPoller {
    /// The offset to poll with after seeing `updates`: one past the highest
    /// `update_id`, or unchanged when the batch was empty.
    pub fn next_offset_after(&self, updates: &[TgUpdate]) -> i64 {
        updates
            .iter()
            .map(|u| u.update_id + 1)
            .max()
            .map_or(self.offset, |next| next.max(self.offset))
    }

    /// Advance the cursor in place; returns the new offset.
    pub fn advance(&mut self, updates: &[TgUpdate]) -> i64 {
        self.offset = self.next_offset_after(updates);
        self.offset
    }
}

/// `getUpdates` URL for a long poll. `base` is `https://api.telegram.org/bot{token}`
/// — the caller holds the token; it never enters a type here.
pub fn get_updates_url(base: &str, offset: i64) -> String {
    format!(
        "{}/getUpdates?timeout=50&offset={offset}",
        base.trim_end_matches('/')
    )
}

/// `getFile` URL for a `file_id`.
pub fn get_file_url(base: &str, file_id: &str) -> String {
    format!("{}/getFile?file_id={file_id}", base.trim_end_matches('/'))
}

/// The download URL for a resolved `file_path`. `token_base` is
/// `https://api.telegram.org/file/bot{token}`.
pub fn resolve_file_url(token_base: &str, file_path: &str) -> String {
    format!(
        "{}/{}",
        token_base.trim_end_matches('/'),
        file_path.trim_start_matches('/')
    )
}

/// The `file_id` inside a `tgfile://` pseudo-URL, if it is one.
pub fn tgfile_id(url: &str) -> Option<&str> {
    url.strip_prefix(TGFILE_SCHEME).filter(|id| !id.is_empty())
}

// ---------------------------------------------------------------------------
// Slack — Events API envelope (delivered over Socket Mode), payload
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SlackEnvelope {
    pub team_id: Option<String>,
    pub event: Option<SlackEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub user: Option<String>,
    pub bot_id: Option<String>,
    pub text: Option<String>,
    pub channel: Option<String>,
    pub team: Option<String>,
    pub ts: Option<String>,
    pub event_ts: Option<String>,
    pub reaction: Option<String>,
    pub item: Option<SlackItem>,
    pub files: Option<Vec<SlackFile>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackItem {
    pub channel: String,
    pub ts: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackFile {
    pub name: String,
    pub mimetype: Option<String>,
    pub url_private: String,
}

/// Translate an Events API envelope — the same shape arrives over Socket
/// Mode as `payload`, which is how [`crate::gateway`] receives it. Messages
/// from bots (any `bot_id`) are dropped, as are event types the pipeline
/// does not handle.
pub fn translate_slack(envelope: &SlackEnvelope) -> Option<SocialEvent> {
    let event = envelope.event.as_ref()?;
    let user = event.user.clone()?;
    let guild = envelope.team_id.clone().or_else(|| event.team.clone());
    match event.kind.as_str() {
        "message" if event.bot_id.is_none() => Some(SocialEvent {
            network: SocialNetwork::Slack,
            kind: EventKind::Message {
                text: event.text.clone().unwrap_or_default(),
                attachments: event
                    .files
                    .iter()
                    .flatten()
                    .map(|f| RemoteAttachment {
                        url: f.url_private.clone(),
                        filename: f.name.clone(),
                        content_type: f.mimetype.clone(),
                    })
                    .collect(),
            },
            native_message_id: event.ts.clone().unwrap_or_default(),
            native_channel_id: event.channel.clone().unwrap_or_default(),
            native_guild_id: guild,
            native_user_id: user.clone(),
            // Resolved lazily via users.info if ever needed.
            user_display_name: user,
            is_bot: false,
            timestamp: slack_ts_seconds(event.event_ts.as_deref().or(event.ts.as_deref())),
        }),
        "reaction_added" | "reaction_removed" => {
            let item = event.item.as_ref()?;
            Some(SocialEvent {
                network: SocialNetwork::Slack,
                kind: EventKind::Reaction {
                    emoji: event.reaction.clone().unwrap_or_default(),
                    target_message_id: item.ts.clone(),
                    added: event.kind == "reaction_added",
                },
                native_message_id: item.ts.clone(),
                native_channel_id: item.channel.clone(),
                native_guild_id: guild,
                native_user_id: user.clone(),
                user_display_name: user,
                is_bot: false,
                timestamp: slack_ts_seconds(event.event_ts.as_deref()),
            })
        }
        _ => None,
    }
}

/// Slack timestamps are `"1700000000.123456"`; the integer part is seconds.
fn slack_ts_seconds(ts: Option<&str>) -> u64 {
    ts.and_then(|t| t.split('.').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// The JSON body for `chat.postMessage`. mrkdwn; a title becomes a bold first
/// line; a reply threads under the original.
pub fn slack_post_message_payload(message: &OutboundMessage, channel: &str) -> Value {
    let text = match &message.title {
        Some(title) => format!("*{title}*\n{}", message.text),
        None => message.text.clone(),
    };
    let mut payload = json!({
        "channel": channel,
        "text": text,
        "mrkdwn": true,
    });
    if let Some(ts) = &message.reply_to_native_message_id {
        payload["thread_ts"] = json!(ts);
    }
    payload
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
