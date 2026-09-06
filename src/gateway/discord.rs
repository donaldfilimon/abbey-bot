//! Discord adapter — `DiscordOutbound`, `on_discord_event`, `strip_bot_mention`.

use std::sync::Arc;

use serenity::all::{
    ChannelId, CreateAllowedMentions, CreateMessage, EditMessage, FullEvent, Http, Message,
    MessageId, MessageReference, Reaction, ReactionType,
};

use crate::gateway::shared::{DISCORD_MESSAGE_CAP, Snowflake, clamp, fetch_capped};
use crate::pipeline::{self, Outbound};
use crate::platform::{EventKind, OutboundMessage, RemoteAttachment, SocialEvent, SocialNetwork};
use crate::runtime::AppState;

/// No generated or guild-derived text may trigger a Discord notification.
pub(crate) fn no_mentions() -> CreateAllowedMentions {
    CreateAllowedMentions::new().replied_user(false)
}

/// Delivery through serenity's REST client.
pub struct DiscordOutbound {
    pub http: Arc<Http>,
    pub fetcher: reqwest::Client,
}

impl Outbound for DiscordOutbound {
    async fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> Result<String, String> {
        let channel = ChannelId::new(Snowflake::parse(native_channel_id)?.get());
        let mut builder = CreateMessage::new()
            .content(clamp(&message.text, DISCORD_MESSAGE_CAP))
            .allowed_mentions(no_mentions());
        if let Some(reply) = &message.reply_to_native_message_id {
            let mut reference: MessageReference =
                (channel, MessageId::new(Snowflake::parse(reply)?.get())).into();
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
        if let Ok(id) = Snowflake::parse(native_channel_id).map(|s| s.get()) {
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
                ChannelId::new(Snowflake::parse(native_channel_id)?.get()),
                MessageId::new(Snowflake::parse(native_message_id)?.get()),
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
                ChannelId::new(Snowflake::parse(native_channel_id)?.get()),
                MessageId::new(Snowflake::parse(native_message_id)?.get()),
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

/// Strip the bot's own mention from text using `memchr` to avoid a double
/// allocation. Finds each `<` with `memchr` and checks for the two mention
/// forms `<@id>` and `<@!id>`.
pub(crate) fn strip_bot_mention(text: &str, bot_id: u64) -> String {
    let pat1 = format!("<@{bot_id}>");
    let pat2 = format!("<@!{bot_id}>");
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut search_from = 0usize;
    while search_from < bytes.len() {
        let Some(rel) = memchr::memchr(b'<', &bytes[search_from..]) else {
            break;
        };
        let abs = search_from + rel;
        if text[abs..].starts_with(&pat2) {
            out.push_str(&text[last..abs]);
            last = abs + pat2.len();
            search_from = last;
        } else if text[abs..].starts_with(&pat1) {
            out.push_str(&text[last..abs]);
            last = abs + pat1.len();
            search_from = last;
        } else {
            search_from = abs + 1;
        }
    }
    out.push_str(&text[last..]);
    out.trim().to_string()
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
            discord_connection_observed(state, true);
        }
        FullEvent::Resume { .. } => {
            discord_connection_observed(state, true);
        }
        FullEvent::ShardStageUpdate { event } => {
            if event.new != serenity::gateway::ConnectionStage::Connected {
                discord_connection_observed(state, false);
            }
        }
        FullEvent::Message { new_message } => {
            let me = ctx.cache.current_user().id;
            let mentions_bot = new_message.mentions_user_id(me);
            let translated_text = if mentions_bot {
                strip_bot_mention(&new_message.content, me.get())
            } else {
                new_message.content.clone()
            };
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
            if mentions_bot && let EventKind::Message { text, .. } = &mut event.kind {
                *text = translated_text;
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
            let scoped = format!("discord:{}", guild.id.get());
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.guilds).config(&scoped, &mut *stores);
        }
        FullEvent::GuildDelete { incomplete, .. } => {
            let scoped = format!("discord:{}", incomplete.id.get());
            AppState::lock(&state.guilds).evict(&scoped);
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.brains).persist_and_evict(&scoped, &mut *stores);
        }
        FullEvent::GuildMemberAddition { new_member } => {
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

fn discord_connection_observed(state: &AppState, ready: bool) {
    if let Some(status) = state.managed_status() {
        if ready {
            status.discord_resumed();
        } else {
            status.discord_connecting();
        }
        // Do not leave the preceding ready document eligible for an installer's
        // entire stability window while waiting for the periodic heartbeat.
        let _ = status.refresh();
    }
    if let Some(events) = state.operational_events() {
        use crate::observability::{EventCode, EventComponent, EventOutcome};
        let _ = events.record(
            EventComponent::Discord,
            EventCode::ConnectorState,
            if ready {
                EventOutcome::Ready
            } else {
                EventOutcome::Degraded
            },
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::shared;
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn observed_disconnect_publishes_non_ready_without_waiting_for_heartbeat() {
        let home = crate::readiness::tests::temporary_home();
        let environment =
            crate::readiness::private::PrivateDirectory::open(&home, &[".config", "abbey-bot"])
                .unwrap();
        environment
            .publish("env", b"DISCORD_TOKEN=fixture-only\n")
            .unwrap();
        let managed = crate::managed_service::begin(&home)
            .unwrap_or_else(|_| panic!("fixture preflight failed"));
        let identity = managed.publisher.identity().clone();
        let fatal = managed.fatal.clone();
        let mut writer = crate::service::telemetry::TelemetryWriter::start(
            managed.log,
            managed.publisher,
            managed.fatal,
        );
        let status = Arc::new(crate::service::status::ManagedStatus::new(
            identity,
            writer.requests(),
            managed.privacy_report,
            false,
            false,
        ));
        let state = AppState::in_memory();
        state.attach_observability(writer.requests(), status.clone());
        status.scheduler_running();
        status.discord_ready();
        status.refresh().unwrap().await.unwrap().unwrap();
        let path = home.join(".local/share/abbey-bot/readiness.json");
        assert!(
            crate::readiness::ReadinessDocument::decode(&std::fs::read(&path).unwrap())
                .unwrap()
                .is_ready()
        );
        // This is the same helper used by the actual shard-stage event arm.
        // Stopping drains only already enqueued work; it schedules no refresh.
        discord_connection_observed(&state, false);
        writer.stop();
        writer.joined().await.unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["phase"], "starting");
        assert_eq!(document["discord"], "connecting");
        let events =
            std::fs::read_to_string(home.join("Library/Logs/abbey-bot/abbey-bot.events.jsonl"))
                .unwrap();
        let publications: Vec<serde_json::Value> = events
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|row: &serde_json::Value| row["code"] == "readiness_published")
            .collect();
        assert_eq!(
            publications.len(),
            2,
            "only the initial ready and observed disconnect were published"
        );
        assert!(publications.iter().all(|row| row["outcome"] == "succeeded"));
        assert!(!events.contains("fixture-only"));
        assert!(
            publications
                .iter()
                .all(|row| row.get("pid").is_none() && row.get("run_nonce").is_none())
        );
        assert!(!fatal.pending());
        drop(environment);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn discord_mentions_are_fully_disabled() {
        let value = serde_json::to_value(no_mentions()).unwrap();
        assert_eq!(value["parse"], serde_json::json!([]));
        assert_eq!(value["users"], serde_json::json!([]));
        assert_eq!(value["roles"], serde_json::json!([]));
        assert_eq!(value["replied_user"], false);
    }

    #[test]
    fn raw_discord_bot_mentions_are_removed_before_core_voice_matching() {
        assert_eq!(strip_bot_mention("<@123> I consent", 123), "I consent");
        assert_eq!(
            strip_bot_mention("<@!123> we all consent", 123),
            "we all consent"
        );
        assert_eq!(
            strip_bot_mention("please ask <@456> too", 123),
            "please ask <@456> too"
        );
        assert_eq!(
            strip_bot_mention("<@123> hi <@!123> again", 123),
            "hi  again"
        );
    }

    #[test]
    fn mentioned_written_stop_reaches_the_core_withdrawal_classifier() {
        let translated = strip_bot_mention("<@123> stop listening", 123);

        assert!(crate::voice_session::withdrawal_requested(&translated));
    }

    #[test]
    fn shared_clamp_still_used_for_discord() {
        let long = "é".repeat(2500);
        let clamped = shared::clamp(&long, shared::DISCORD_MESSAGE_CAP);
        assert_eq!(clamped.chars().count(), shared::DISCORD_MESSAGE_CAP);
    }
}
