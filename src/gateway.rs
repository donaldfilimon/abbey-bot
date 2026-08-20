//! The shells: Discord gateway events and the Telegram long-poll adapter, both
//! translated into [`SocialEvent`]s and fed to [`crate::pipeline`].
//!
//! This file and `commands.rs` / `main.rs` are the only ones that import
//! serenity. Its job is translation in both directions — native event in,
//! [`OutboundMessage`] out — and nothing here decides anything.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{
    ChannelId, CreateAllowedMentions, CreateMessage, EditMessage, FullEvent, Http, Message,
    MessageId, MessageReference, Reaction, ReactionType,
};

use crate::persist::Stores;
use crate::pipeline::{self, Outbound};
use crate::platform::{
    self, EventKind, OutboundMessage, RemoteAttachment, SlackEnvelope, SocialEvent, SocialNetwork,
    TelegramPoller, TgFile, TgResponse, TgUpdate,
};
use crate::runtime::AppState;

/// Discord's message cap, in codepoints. Same value `commands::clamp_message`
/// enforces; repeated here because the pipeline's output also crosses it.
pub const DISCORD_MESSAGE_CAP: usize = 2_000;
/// Telegram's `sendMessage` cap.
pub const TELEGRAM_MESSAGE_CAP: usize = 4_096;

/// No generated or guild-derived text may trigger a Discord notification.
pub(crate) fn no_mentions() -> CreateAllowedMentions {
    CreateAllowedMentions::new().replied_user(false)
}

fn clamp(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut out: String = text.chars().take(cap - 1).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Discord
// ---------------------------------------------------------------------------

/// Delivery through serenity's REST client.
pub struct DiscordOutbound {
    pub http: Arc<Http>,
    pub fetcher: reqwest::Client,
}

fn parse_id(s: &str) -> Result<u64, String> {
    s.parse::<u64>()
        .map_err(|_| format!("{s:?} is not a snowflake"))
}

impl Outbound for DiscordOutbound {
    async fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> Result<String, String> {
        let channel = ChannelId::new(parse_id(native_channel_id)?);
        let mut builder = CreateMessage::new()
            .content(clamp(&message.text, DISCORD_MESSAGE_CAP))
            .allowed_mentions(no_mentions());
        if let Some(reply) = &message.reply_to_native_message_id {
            // A message deleted mid-generation must not turn a good reply into
            // a 400: reference it if it still exists, post plainly otherwise.
            let mut reference: MessageReference =
                (channel, MessageId::new(parse_id(reply)?)).into();
            reference.fail_if_not_exists = Some(false);
            builder = builder.reference_message(reference);
        }
        channel
            .send_message(&self.http, builder)
            .await
            .map(|m| m.id.get().to_string())
            .map_err(|e| e.to_string())
    }

    async fn typing(&self, native_channel_id: &str) {
        if let Ok(id) = parse_id(native_channel_id) {
            let _ = ChannelId::new(id).broadcast_typing(&self.http).await;
        }
    }

    async fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        self.http
            .create_reaction(
                ChannelId::new(parse_id(native_channel_id)?),
                MessageId::new(parse_id(native_message_id)?),
                &ReactionType::Unicode(emoji.to_string()),
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn fetch(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        fetch_capped(&self.fetcher, url, max, None).await
    }

    async fn edit(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        text: &str,
    ) -> Result<(), String> {
        self.http
            .edit_message(
                ChannelId::new(parse_id(native_channel_id)?),
                MessageId::new(parse_id(native_message_id)?),
                &EditMessage::new()
                    .content(clamp(text, DISCORD_MESSAGE_CAP))
                    .allowed_mentions(no_mentions()),
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// GET `url` and refuse bodies over `max` bytes — attachments are
/// attacker-controlled (`docs/spec/vision.md`).
pub(crate) async fn fetch_capped(
    client: &reqwest::Client,
    url: &str,
    max: usize,
    bearer: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut req = client.get(url);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    let response = req.send().await.map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("attachment fetch failed: HTTP {status}"));
    }
    crate::http_body::read_capped(response, max)
        .await
        .map_err(|error| {
            if error.is_too_large() {
                format!("attachment exceeds the {max}-byte cap")
            } else {
                "attachment download failed while reading the response".to_string()
            }
        })
}

fn discord_message_event(msg: &Message) -> SocialEvent {
    SocialEvent {
        network: SocialNetwork::Discord,
        kind: EventKind::Message {
            text: msg.content.clone(),
            attachments: msg
                .attachments
                .iter()
                .map(|a| RemoteAttachment {
                    url: a.url.clone(),
                    filename: a.filename.clone(),
                    content_type: a.content_type.clone(),
                })
                .collect(),
        },
        native_message_id: msg.id.get().to_string(),
        native_channel_id: msg.channel_id.get().to_string(),
        native_guild_id: msg.guild_id.map(|g| g.get().to_string()),
        native_user_id: msg.author.id.get().to_string(),
        user_display_name: msg
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| msg.author.name.clone()),
        is_bot: msg.author.bot,
        timestamp: u64::try_from(msg.timestamp.unix_timestamp()).unwrap_or(0),
    }
}

fn discord_reaction_event(reaction: &Reaction, added: bool) -> SocialEvent {
    let emoji = match &reaction.emoji {
        ReactionType::Unicode(s) => s.clone(),
        ReactionType::Custom { name, .. } => name.clone().unwrap_or_default(),
        _ => String::new(),
    };
    SocialEvent {
        network: SocialNetwork::Discord,
        kind: EventKind::Reaction {
            emoji,
            target_message_id: reaction.message_id.get().to_string(),
            added,
        },
        native_message_id: reaction.message_id.get().to_string(),
        native_channel_id: reaction.channel_id.get().to_string(),
        native_guild_id: reaction.guild_id.map(|g| g.get().to_string()),
        native_user_id: reaction
            .user_id
            .map(|u| u.get().to_string())
            .unwrap_or_default(),
        user_display_name: String::new(),
        is_bot: false,
        timestamp: crate::runtime::now(),
    }
}

/// The poise `event_handler` body. Every arm is translate-then-hand-off.
pub async fn on_discord_event(
    ctx: &serenity::all::Context,
    event: &FullEvent,
    state: &Arc<AppState>,
) {
    let out = DiscordOutbound {
        http: Arc::clone(&ctx.http),
        fetcher: state.attachments.clone(),
    };
    match event {
        FullEvent::Ready { data_about_bot } => {
            state.register_self(format!("discord:{}", data_about_bot.user.id.get()));
        }
        FullEvent::Message { new_message } => {
            let me = ctx.cache.current_user().id;
            let mentions_bot = new_message.mentions_user_id(me);
            let reply_to = new_message
                .referenced_message
                .as_ref()
                .map(|m| m.id.get().to_string())
                .or_else(|| {
                    new_message
                        .message_reference
                        .as_ref()
                        .and_then(|r| r.message_id)
                        .map(|m| m.get().to_string())
                });
            let mut event = discord_message_event(new_message);
            // The model should see the words, not `<@1234>`; strip our own
            // mention (shell knowledge — the pipeline never learns our id).
            if mentions_bot && let EventKind::Message { text, .. } = &mut event.kind {
                let stripped = text
                    .replace(&format!("<@{me}>"), "")
                    .replace(&format!("<@!{me}>"), "")
                    .trim()
                    .to_string();
                *text = stripped;
            }
            let channel = new_message.channel_id.get();
            let guild = new_message.guild_id.map(|g| g.get());
            let outcome =
                pipeline::handle(state, &out, event, mentions_bot, reply_to.as_deref()).await;
            tracing::info!(channel, ?guild, mentions_bot, ?outcome, "message handled");
        }
        FullEvent::ReactionAdd { add_reaction } => {
            let event = discord_reaction_event(add_reaction, true);
            let outcome = pipeline::handle(state, &out, event, false, None).await;
            tracing::info!(
                message = add_reaction.message_id.get(),
                ?outcome,
                "reaction handled"
            );
        }
        FullEvent::ReactionRemove { removed_reaction } => {
            let event = discord_reaction_event(removed_reaction, false);
            pipeline::handle(state, &out, event, false, None).await;
        }
        FullEvent::MessageDelete {
            deleted_message_id, ..
        } => {
            AppState::lock(&state.rewards)
                .abbey_message_deleted(&deleted_message_id.get().to_string());
        }
        FullEvent::GuildCreate { guild, .. } => {
            // Fires on connect for every existing guild and on join: provision
            // defaults so the first message does not pay for it.
            let scoped = format!("discord:{}", guild.id.get());
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.guilds).config(&scoped, &mut *stores);
        }
        FullEvent::GuildDelete { incomplete, .. } => {
            let scoped = format!("discord:{}", incomplete.id.get());
            AppState::lock(&state.guilds).evict(&scoped);
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.brains).persist_and_evict(&scoped, &mut *stores);
            // Rows stay on disk — a kicked-and-reinvited guild resumes.
        }
        FullEvent::GuildMemberAddition { new_member } => {
            // Needs the privileged GUILD_MEMBERS intent to ever fire; when it
            // does, the welcome goes to the guild's system channel if there is
            // one — the spec leaves the channel choice to the adapter.
            let Some(channel) = ctx
                .cache
                .guild(new_member.guild_id)
                .and_then(|g| g.system_channel_id)
            else {
                return;
            };
            let event = SocialEvent {
                network: SocialNetwork::Discord,
                kind: EventKind::MemberJoined,
                native_message_id: String::new(),
                native_channel_id: channel.get().to_string(),
                native_guild_id: Some(new_member.guild_id.get().to_string()),
                native_user_id: new_member.user.id.get().to_string(),
                user_display_name: new_member.display_name().to_string(),
                is_bot: new_member.user.bot,
                timestamp: crate::runtime::now(),
            };
            pipeline::handle(state, &out, event, false, None).await;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Telegram
// ---------------------------------------------------------------------------

/// Telegram Bot API delivery. Holds the token; no `Debug` on purpose.
pub struct TelegramOutbound {
    base: String,
    file_base: String,
    client: reqwest::Client,
}

impl TelegramOutbound {
    pub fn new(token: &str) -> Self {
        Self {
            base: format!("https://api.telegram.org/bot{token}"),
            file_base: format!("https://api.telegram.org/file/bot{token}"),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(65))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn post_json(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let response = self
            .client
            .post(format!("{}/{method}", self.base))
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        if !status.is_success()
            || value.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        {
            return Err(format!("telegram {method} failed: HTTP {status}"));
        }
        Ok(value)
    }
}

impl Outbound for TelegramOutbound {
    async fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> Result<String, String> {
        let clamped = OutboundMessage {
            text: clamp(&message.text, TELEGRAM_MESSAGE_CAP),
            ..message.clone()
        };
        let payload = platform::telegram_send_payload(&clamped, native_channel_id);
        let value = self.post_json("sendMessage", &payload).await?;
        Ok(value
            .pointer("/result/message_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_default())
    }

    async fn typing(&self, native_channel_id: &str) {
        let _ = self
            .post_json(
                "sendChatAction",
                &serde_json::json!({ "chat_id": native_channel_id, "action": "typing" }),
            )
            .await;
    }

    async fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        self.post_json(
            "setMessageReaction",
            &serde_json::json!({
                "chat_id": native_channel_id,
                "message_id": native_message_id.parse::<i64>().map_err(|e| e.to_string())?,
                "reaction": [{ "type": "emoji", "emoji": emoji }],
            }),
        )
        .await
        .map(|_| ())
    }

    async fn fetch(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        let resolved = match platform::tgfile_id(url) {
            Some(file_id) => {
                let body: TgResponse<TgFile> = self
                    .client
                    .get(platform::get_file_url(&self.base, file_id))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .json()
                    .await
                    .map_err(|e| e.to_string())?;
                let path = body
                    .result
                    .filter(|_| body.ok)
                    .and_then(|f| f.file_path)
                    .ok_or("getFile returned no file_path")?;
                platform::resolve_file_url(&self.file_base, path.as_str())
            }
            None => url.to_string(),
        };
        fetch_capped(&self.client, &resolved, max, None).await
    }

    async fn edit(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        text: &str,
    ) -> Result<(), String> {
        self.post_json(
            "editMessageText",
            &serde_json::json!({
                "chat_id": native_channel_id,
                "message_id": native_message_id.parse::<i64>().map_err(|e| e.to_string())?,
                "text": clamp(text, TELEGRAM_MESSAGE_CAP),
            }),
        )
        .await
        .map(|_| ())
    }
}

/// Long-poll `getUpdates` forever, feeding the pipeline. Errors back off five
/// seconds and re-poll, per the spec.
pub async fn run_telegram(state: Arc<AppState>, token: String) {
    let out = TelegramOutbound::new(&token);
    let mut poller = TelegramPoller::default();
    // Learn our own id so the pipeline can skip our traffic.
    if let Ok(me) = out.post_json("getMe", &serde_json::json!({})).await
        && let Some(id) = me.pointer("/result/id").and_then(serde_json::Value::as_i64)
    {
        state.register_self(format!("telegram:{id}"));
    }
    tracing::info!("telegram adapter polling");
    loop {
        let url = platform::get_updates_url(&out.base, poller.offset);
        let updates = match out.client.get(&url).send().await {
            Ok(resp) => match resp.json::<TgResponse<Vec<TgUpdate>>>().await {
                Ok(body) if body.ok => body.result.unwrap_or_default(),
                Ok(_) => {
                    tracing::warn!("telegram getUpdates returned ok=false");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "telegram getUpdates decode failed");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "telegram getUpdates failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        poller.advance(&updates);
        for update in &updates {
            if let Some(event) = platform::translate_telegram(update) {
                let reply_to = update
                    .message
                    .as_ref()
                    .and_then(|m| m.reply_to_message.as_ref())
                    .map(|r| r.message_id.to_string());
                // Telegram has no mention flag in the wire subset we read; a
                // private chat counts as addressed (guild is None → forced).
                pipeline::handle(&state, &out, event, false, reply_to.as_deref()).await;
            }
        }
    }
}

/// Spawn the Telegram adapter if `TELEGRAM_BOT_TOKEN` is set.
pub fn maybe_start_telegram(state: &Arc<AppState>) {
    let Some(token) = std::env::var("TELEGRAM_BOT_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(run_telegram(state, token));
}

// ---------------------------------------------------------------------------
// Slack — Socket Mode
// ---------------------------------------------------------------------------
//
// The spec's Slack adapter receives Events API envelopes over an inbound HTTP
// route. This crate has no HTTP server, so it takes Slack's other transport:
// Socket Mode, where the app opens a WebSocket to Slack
// (`apps.connections.open` with an app-level `xapp-` token) and the same
// envelopes arrive as `events_api` frames, each acknowledged by envelope id.
// The event payload is byte-for-byte the shape `platform::translate_slack`
// already parses, so the pure layer is unchanged.

/// Slack Web API delivery. Holds the bot token; no `Debug` on purpose.
pub struct SlackOutbound {
    bot_token: String,
    client: reqwest::Client,
}

impl SlackOutbound {
    pub fn new(bot_token: &str) -> Self {
        Self {
            bot_token: bot_token.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn call(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let response = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(&self.bot_token)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            let err = value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            return Err(format!("slack {method} failed: {err}"));
        }
        Ok(value)
    }
}

impl Outbound for SlackOutbound {
    async fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> Result<String, String> {
        let payload = platform::slack_post_message_payload(message, native_channel_id);
        let value = self.call("chat.postMessage", &payload).await?;
        Ok(value
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    async fn typing(&self, _native_channel_id: &str) {
        // Slack has no typing indicator for bots over the Web API.
    }

    async fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        // reactions.add takes a short name, not the glyph.
        let name = match emoji {
            "👍" => "+1",
            "❤️" | "❤" => "heart",
            "🔥" => "fire",
            _ => "eyes",
        };
        self.call(
            "reactions.add",
            &serde_json::json!({ "channel": native_channel_id, "timestamp": native_message_id, "name": name }),
        )
        .await
        .map(|_| ())
    }

    async fn fetch(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        // Private files need the bot token (docs/spec/vision.md ImageFetcher).
        let bearer = url
            .contains("files.slack.com")
            .then_some(self.bot_token.as_str());
        fetch_capped(&self.client, url, max, bearer).await
    }

    async fn edit(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        text: &str,
    ) -> Result<(), String> {
        self.call(
            "chat.update",
            &serde_json::json!({ "channel": native_channel_id, "ts": native_message_id, "text": text }),
        )
        .await
        .map(|_| ())
    }
}

/// One Socket Mode frame. Only the fields the loop reads.
#[derive(Debug, serde::Deserialize)]
struct SocketFrame {
    #[serde(rename = "type")]
    kind: String,
    envelope_id: Option<String>,
    payload: Option<SlackEnvelope>,
}

/// Run Socket Mode forever: open, pump, ack, reconnect on close.
pub async fn run_slack(state: Arc<AppState>, bot_token: String, app_token: String) {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let out = SlackOutbound::new(&bot_token);
    if let Ok(me) = out.call("auth.test", &serde_json::json!({})).await
        && let Some(id) = me.get("user_id").and_then(serde_json::Value::as_str)
    {
        state.register_self(format!("slack:{id}"));
    }
    let opener = reqwest::Client::new();
    loop {
        let url = match opener
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(&app_token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) if v.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => v
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                Ok(v) => {
                    tracing::warn!(body = %v, "apps.connections.open refused");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "apps.connections.open decode failed");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "apps.connections.open failed");
                None
            }
        };
        let Some(url) = url else {
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        };
        let (mut socket, _) = match tokio_tungstenite::connect_async(&url).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "slack socket connect failed");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        tracing::info!("slack socket mode connected");
        while let Some(frame) = socket.next().await {
            let text = match frame {
                Ok(WsMessage::Text(t)) => t,
                Ok(WsMessage::Ping(p)) => {
                    let _ = socket.send(WsMessage::Pong(p)).await;
                    continue;
                }
                Ok(WsMessage::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            let Ok(parsed) = serde_json::from_str::<SocketFrame>(&text) else {
                continue;
            };
            if let Some(id) = &parsed.envelope_id {
                let ack = serde_json::json!({ "envelope_id": id }).to_string();
                if socket.send(WsMessage::Text(ack)).await.is_err() {
                    break;
                }
            }
            match parsed.kind.as_str() {
                "disconnect" => break,
                "events_api" => {
                    if let Some(event) = parsed.payload.as_ref().and_then(platform::translate_slack)
                    {
                        pipeline::handle(&state, &out, event, false, None).await;
                    }
                }
                _ => {}
            }
        }
        tracing::info!("slack socket closed; reconnecting");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Spawn the Slack adapter if both `SLACK_BOT_TOKEN` and `SLACK_APP_TOKEN`
/// are set (the `xoxb-` Web API token and the `xapp-` Socket Mode token).
pub fn maybe_start_slack(state: &Arc<AppState>) {
    let read = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let (Some(bot), Some(app)) = (read("SLACK_BOT_TOKEN"), read("SLACK_APP_TOKEN")) else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(run_slack(state, bot, app));
}

/// Persist on the way out. Called from the ctrl-c handler in `main`.
pub fn shutdown(state: &AppState) {
    state.persist_all();
    if let Some(dir) = &state.data_dir {
        tracing::info!(path = %Stores::state_path(dir).display(), "state persisted on shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_respects_the_cap_in_codepoints() {
        let long = "é".repeat(2_500);
        let clamped = clamp(&long, DISCORD_MESSAGE_CAP);
        assert_eq!(clamped.chars().count(), DISCORD_MESSAGE_CAP);
        assert!(clamped.ends_with('…'));
        assert_eq!(clamp("short", DISCORD_MESSAGE_CAP), "short");
    }

    #[test]
    fn snowflakes_parse_and_garbage_does_not() {
        assert_eq!(parse_id("123"), Ok(123));
        assert!(parse_id("abc").is_err());
    }

    #[test]
    fn discord_mentions_are_fully_disabled() {
        let value = serde_json::to_value(no_mentions()).unwrap();
        assert_eq!(value["parse"], serde_json::json!([]));
        assert_eq!(value["users"], serde_json::json!([]));
        assert_eq!(value["roles"], serde_json::json!([]));
        assert_eq!(value["replied_user"], false);
    }
}
