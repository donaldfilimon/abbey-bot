//! Abbey Bot — Discord operational layer, Rust/serenity/poise.
//!
//! Configuration is entirely environment-driven, per the skill's standing rule
//! that tokens live in env vars and never in source:
//!
//! - `DISCORD_TOKEN` (required) — bot token.
//! - `ABBEY_GUILD_ID` (optional) — register commands to this guild only.
//!   Guild-scoped registration is instant; global registration can take up to an
//!   hour to propagate, which makes it useless during development. Unset means
//!   global, which is what you want once the command set has settled.
//! - `ANTHROPIC_API_KEY` (optional, secret) — makes `/persona ask` answer via
//!   the external Anthropic API. Same handling as `DISCORD_TOKEN`: env only.
//! - `ABBEY_BOT_LLM_ENDPOINT` + `ABBEY_BOT_LLM_MODEL` (optional) — answer via an
//!   OpenAI-compatible server, usually loopback. With neither this nor the key
//!   set, `/persona ask` replies that no generation backend is configured.
//! - `ABBEY_QUIET` (optional) — `1` forbids unsolicited replies everywhere.
//! - `ABBEY_DATA_DIR` (optional) — where learning, memory, and config persist.
//!   Unset means in-memory only.
//! - `ABBEY_MESSAGE_CONTENT` (optional) — `1` requests the privileged
//!   MESSAGE_CONTENT intent (must also be enabled in the Dev Portal).
//! - `ABBEY_VISION_*`, `TELEGRAM_BOT_TOKEN` (optional) — see `.env.example`.
//! - `ABBEY_VOICE_GUILD_ID` + `ABBEY_VOICE_CHANNEL_ID` (optional) — enable an
//!   admin-triggered, DAVE-capable Discord connection. `ABBEY_VOICE_AUTOJOIN=1`
//!   provides persistent muted/self-deafened no-audio presence. Conversational
//!   local or Realtime voice still requires `/voice join consent:true`.
//! - `--voice-self-test OUTPUT.wav` — run local TTS → STT → canonical Abbey
//!   reasoning → TTS without a Discord token, microphone, or call.
//! - `RUST_LOG` (optional) — tracing filter, defaults to `info`.
//!
//! Intents default to `non_privileged()` — which, since the adaptive loop
//! landed, includes the non-privileged message and reaction events the
//! pipeline listens to. Message *content* stays privileged: without
//! `ABBEY_MESSAGE_CONTENT=1` (and the Dev Portal toggle) Abbey sees the body
//! of mentions and DMs only, and learns from those alone. Presence and the
//! member list are never requested; commands that need guild data fetch it
//! over REST instead, which is why [`profile::summarize`] states that
//! presence is unavailable rather than guessing at it.

mod ask;
mod brain;
mod commands;
mod commands_brain;
mod commands_voice;
mod embedding;
mod engine;
mod gateway;
mod generation;
mod grounding;
mod guild;
mod http_body;
mod llm;
mod memory;
mod moderation;
mod offline_voice;
mod perms;
mod persist;
mod persona;
mod pipeline;
mod platform;
mod profile;
mod recall;
mod routing_signals;
mod runtime;
mod server;
mod tools;
mod vision;
mod voice;
mod voice_local;
mod voice_openai;
mod voice_self_test;
mod voice_session;
mod wdbx;
mod webhook;
mod wyhash;

use serenity::all::{GatewayIntents, GuildId};

/// Shared command state. Empty today; the type exists so adding state later does
/// not mean touching every command signature.
pub struct Data {
    pub state: std::sync::Arc<runtime::AppState>,
    pub voice: Option<std::sync::Arc<voice_session::VoiceRuntime>>,
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Some(output) = voice_self_test_output().map_err(runtime::StartupError)? {
        let report = voice_self_test::run(&output)
            .await
            .map_err(runtime::StartupError)?;
        println!(
            "local voice self-test passed\nstimulus transcript: {}\nreply: {}\nreply transcript: {}\nround-trip word recall: {:.0}%\naudio: {} ({} Hz, {} channel(s), {} ms)",
            report.transcript,
            report.spoken_answer,
            report.reply_transcript,
            report.round_trip_word_recall * 100.0,
            report.output.display(),
            report.sample_rate,
            report.channels,
            report.duration_millis,
        );
        return Ok(());
    }

    // Read before building anything else: a missing token should fail in the
    // first millisecond with a sentence you can act on, not inside a gateway
    // handshake error.
    let token = std::env::var("DISCORD_TOKEN")
        .map_err(|_| "DISCORD_TOKEN is not set. Export the bot token; never hardcode it.")?;
    if token.trim().is_empty() {
        return Err("DISCORD_TOKEN is blank. Export the bot token; never hardcode it.".into());
    }

    let guild_id = match std::env::var("ABBEY_GUILD_ID") {
        Ok(raw) => {
            let parsed = raw
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("ABBEY_GUILD_ID must be a numeric snowflake, got {raw:?}"))?;
            // GuildId::new PANICS on zero rather than returning an error, so a
            // literal "0" parses fine and then aborts the process -- the exact
            // opposite of failing with a sentence you can act on.
            if parsed == 0 {
                return Err("ABBEY_GUILD_ID must not be 0; that is not a valid snowflake".into());
            }
            Some(GuildId::new(parsed))
        }
        Err(_) => None,
    };

    let state = runtime::AppState::from_env()?;
    let voice_runtime = voice::VoiceConfig::from_env()
        .map_err(runtime::StartupError)?
        .map(voice_session::VoiceRuntime::new)
        .map(std::sync::Arc::new);
    match &state.data_dir {
        Some(dir) => tracing::info!(path = %dir.display(), "persisting to data dir"),
        None => tracing::warn!("ABBEY_DATA_DIR unset — learning and memory are in-memory only"),
    }
    match &state.backend {
        Some(b) => tracing::info!(backend = b.label(), "generation backend configured"),
        None => tracing::warn!("no generation backend — Abbey answers honestly that she cannot"),
    }
    if state.quiet {
        tracing::info!(
            "ABBEY_QUIET=1 — no unsolicited replies anywhere; mentions, DMs, and commands still answer"
        );
    }
    state.start_scheduler();
    gateway::maybe_start_telegram(&state);
    gateway::maybe_start_slack(&state);

    let intents = if std::env::var("ABBEY_MESSAGE_CONTENT")
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
    {
        tracing::info!(
            "requesting the privileged MESSAGE_CONTENT intent (must be enabled in the Dev Portal too)"
        );
        GatewayIntents::non_privileged()
            | GatewayIntents::GUILD_VOICE_STATES
            | GatewayIntents::MESSAGE_CONTENT
    } else {
        GatewayIntents::non_privileged() | GatewayIntents::GUILD_VOICE_STATES
    };

    let shell_state = std::sync::Arc::clone(&state);
    let setup_voice_runtime = voice_runtime.clone();
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands::persona(),
                commands::whois(),
                commands::perms(),
                commands::modcall(),
                commands::server(),
                commands::webhook(),
                commands_brain::remember(),
                commands_brain::forget(),
                commands_brain::recall(),
                commands_brain::reputation(),
                commands_brain::summarize(),
                commands_brain::see(),
                commands_brain::ocr(),
                commands_brain::stats(),
                commands_brain::admin(),
                commands_voice::voice(),
            ],
            event_handler: |ctx, event, _framework, data| {
                Box::pin(async move {
                    gateway::on_discord_event(ctx, event, &data.state, data.voice.as_deref()).await;
                    if let serenity::all::FullEvent::VoiceStateUpdate { old, new } = event {
                        commands_voice::on_voice_state_update(ctx, old, new, data).await;
                    }
                    if let serenity::all::FullEvent::ChannelUpdate { new, .. } = event {
                        commands_voice::on_voice_permissions_changed(
                            ctx,
                            new.guild_id,
                            Some(new.id),
                            data,
                        )
                        .await;
                    }
                    if let serenity::all::FullEvent::GuildRoleUpdate { new, .. } = event {
                        commands_voice::on_voice_permissions_changed(ctx, new.guild_id, None, data)
                            .await;
                    }
                    if let serenity::all::FullEvent::GuildRoleDelete { guild_id, .. } = event {
                        commands_voice::on_voice_permissions_changed(ctx, *guild_id, None, data)
                            .await;
                    }
                    if let serenity::all::FullEvent::GuildMemberUpdate { event, .. } = event
                        && event.user.id == ctx.cache.current_user().id
                    {
                        // Discord sends updates for the current bot member even
                        // without the privileged GUILD_MEMBERS intent. Role
                        // assignments can therefore revoke voice immediately.
                        commands_voice::on_voice_permissions_changed(
                            ctx,
                            event.guild_id,
                            None,
                            data,
                        )
                        .await;
                    }
                    Ok(())
                })
            },
            // Mentions are the pipeline's business, not a command prefix:
            // with the default on, `@Abbey hello` logs a poise "didn't
            // recognize command" warning for every mention she answers.
            prefix_options: poise::PrefixFrameworkOptions {
                mention_as_prefix: false,
                ..Default::default()
            },
            // Model and guild-derived text must never notify arbitrary users,
            // roles, or everyone. Replies also stay visually threaded without
            // pinging the author.
            allowed_mentions: Some(gateway::no_mentions()),
            post_command: |ctx| {
                Box::pin(async move {
                    record_interaction(ctx, true, None);
                })
            },
            on_error: |error| {
                Box::pin(async move {
                    if let poise::FrameworkError::Command { ctx, error, .. } = &error {
                        record_interaction(*ctx, false, Some(error.to_string()));
                    }
                    // Structured, not `println!` — and never swallowed: a command
                    // that fails silently is indistinguishable from Discord
                    // dropping the interaction.
                    if let Err(e) = poise::builtins::on_error(error).await {
                        tracing::error!(error = %e, "error handler itself failed");
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                match guild_id {
                    Some(id) => {
                        poise::builtins::register_in_guild(ctx, &framework.options().commands, id)
                            .await?;
                        tracing::info!(guild = %id, "registered guild-scoped commands (instant)");
                    }
                    None => {
                        register_globally_keeping_entry_point(ctx, &framework.options().commands)
                            .await?;
                        tracing::info!(
                            "registered global commands — propagation can take up to an hour"
                        );
                    }
                }
                let voice_autojoin = std::env::var("ABBEY_VOICE_AUTOJOIN")
                    .map(|value| value.trim() == "1")
                    .unwrap_or(false);
                if voice_autojoin {
                    match setup_voice_runtime.as_ref() {
                        Some(runtime) => {
                            commands_voice::autojoin_self_deafened(
                                ctx,
                                std::sync::Arc::clone(runtime),
                            )
                            .await
                            .map_err(runtime::StartupError)?;
                        }
                        None => {
                            return Err(runtime::StartupError(
                                "ABBEY_VOICE_AUTOJOIN=1 requires both voice destination IDs".into(),
                            )
                            .into());
                        }
                    }
                }
                tracing::info!(user = %ready.user.name, "connected");
                shell_state.register_self(format!("discord:{}", ready.user.id.get()));
                Ok(Data {
                    state: shell_state,
                    voice: setup_voice_runtime,
                })
            })
        })
        .build();

    let http = serenity::http::HttpBuilder::new(&token)
        .default_allowed_mentions(gateway::no_mentions())
        .build();
    use songbird::SerenityInit;
    let mut client = serenity::client::ClientBuilder::new_with_http(http, intents)
        .framework(framework)
        .register_songbird_from_config(
            songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Pass),
        )
        .await?;

    // Persist on interactive Ctrl-C and service-manager SIGTERM before taking
    // shards down. Otherwise a redeploy loses the current five-minute window.
    let shard_manager = client.shard_manager.clone();
    let shutdown_state = std::sync::Arc::clone(&state);
    let shutdown_voice = voice_runtime.clone();
    tokio::spawn(async move {
        if shutdown_signal().await.is_ok() {
            tracing::info!("shutting down");
            if let Some(voice) = shutdown_voice {
                voice.disconnect("process shutdown stopped voice").await;
            }
            gateway::shutdown(&shutdown_state);
            shard_manager.shutdown_all().await;
        }
    });

    // Persist whether the gateway ended cleanly or with an error — a bad
    // token after a long uptime must not also cost the last five minutes.
    let result = client.start().await;
    if let Some(voice) = voice_runtime {
        voice.disconnect("Discord gateway stopped voice").await;
    }
    gateway::shutdown(&state);
    result?;
    Ok(())
}

fn voice_self_test_output() -> Result<Option<std::path::PathBuf>, String> {
    parse_startup_arguments(std::env::args_os().skip(1))
}

fn parse_startup_arguments(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<Option<std::path::PathBuf>, String> {
    let Some(mode) = arguments.next() else {
        return Ok(None);
    };
    if mode != std::ffi::OsStr::new("--voice-self-test") {
        return Err(format!(
            "unknown argument {:?}; usage: abbey-bot [--voice-self-test OUTPUT.wav]",
            mode
        ));
    }
    let output = arguments.next().ok_or_else(|| {
        "usage: abbey-bot --voice-self-test OUTPUT.wav (the output must not already exist)"
            .to_string()
    })?;
    if arguments.next().is_some() {
        return Err(
            "usage: abbey-bot --voice-self-test OUTPUT.wav (exactly one output path is required)"
                .into(),
        );
    }
    Ok(Some(output.into()))
}

/// Global registration that survives Discord's Entry Point command.
///
/// Apps with Activities enabled get an auto-created command of type
/// `PrimaryEntryPoint`, and a bulk overwrite that omits it is rejected with
/// "You cannot remove this app's Entry Point command in a bulk update
/// operation" — which is exactly what `poise::builtins::register_globally`
/// sends, and what killed the first live connection in the ready callback.
/// Deleting the Entry Point would disable the app's Activity, which is not
/// this bot's call to make; instead it is read back and re-sent alongside
/// ours, unchanged.
async fn register_globally_keeping_entry_point(
    ctx: &serenity::all::Context,
    commands: &[poise::Command<Data, Error>],
) -> Result<(), Error> {
    use serenity::all::{Command, CommandType, CreateCommand};

    let mut create = poise::builtins::create_application_commands(commands);
    let existing = Command::get_global_commands(&ctx.http).await?;
    for cmd in existing
        .into_iter()
        .filter(|c| c.kind == CommandType::PrimaryEntryPoint)
    {
        let mut keep = CreateCommand::new(cmd.name.clone())
            .kind(CommandType::PrimaryEntryPoint)
            .description(cmd.description.clone())
            .integration_types(cmd.integration_types.clone());
        if let Some(contexts) = cmd.contexts.clone() {
            keep = keep.contexts(contexts);
        }
        if let Some(handler) = cmd.handler {
            keep = keep.handler(handler);
        }
        tracing::info!(name = %cmd.name, "preserving the app's Entry Point command");
        create.push(keep);
    }
    Command::set_global_commands(&ctx.http, create).await?;
    Ok(())
}

/// `InteractionLog` row per slash command (`docs/spec/botarchitecture.md`).
fn record_interaction(ctx: Context<'_>, succeeded: bool, error: Option<String>) {
    let started = ctx.created_at().unix_timestamp();
    let now = runtime::now();
    let duration_ms = u64::try_from(i64::try_from(now).unwrap_or(0) - started)
        .unwrap_or(0)
        .saturating_mul(1000);
    let entry = memory::InteractionEntry {
        command: ctx.command().qualified_name.clone(),
        user_id: guild::scoped_user_id("discord", &ctx.author().id.get().to_string()),
        guild_id: guild::scoped_guild_id(
            "discord",
            ctx.guild_id().map(|g| g.get().to_string()).as_deref(),
        ),
        channel_id: guild::scoped_channel_id("discord", &ctx.channel_id().get().to_string()),
        succeeded,
        error,
        duration_ms,
        at: now,
    };
    runtime::AppState::lock(&ctx.data().state.stores)
        .memory
        .interactions
        .record(entry);
}

#[cfg(test)]
mod startup_argument_tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Option<std::path::PathBuf>, String> {
        parse_startup_arguments(arguments.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn no_arguments_starts_the_discord_service() {
        assert_eq!(parse(&[]).unwrap(), None);
    }

    #[test]
    fn exact_voice_self_test_has_one_create_new_output() {
        assert_eq!(
            parse(&["--voice-self-test", "audition.wav"]).unwrap(),
            Some(std::path::PathBuf::from("audition.wav"))
        );
        assert!(parse(&["--voice-self-test"]).is_err());
        assert!(parse(&["--voice-self-test", "one.wav", "two.wav"]).is_err());
    }

    #[test]
    fn an_unknown_or_mistyped_mode_cannot_start_discord() {
        assert!(parse(&["--voice-self-tset", "audition.wav"]).is_err());
        assert!(parse(&["unexpected"]).is_err());
    }
}
