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
mod tests;
