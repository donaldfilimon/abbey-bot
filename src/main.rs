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
//! - `ABBEY_FM_MODE`, `ABBEY_FM_ENDPOINT`, `ABBEY_FM_CLI`, and
//!   `ABBEY_FM_FALLBACK` (optional) — explicit Apple Foundation Models
//!   secondary routing; off by default. Enabled routes also require the
//!   exact-bound owner-only capability manifest described in `.env.example`.
//! - `ABBEY_QUIET` (optional) — `1` forbids unsolicited replies everywhere.
//! - `ABBEY_DATA_DIR` (optional) — where learning, memory, and config persist.
//!   Unset means in-memory only.
//! - `ABBEY_MESSAGE_CONTENT` (optional) — `1` requests the privileged
//!   MESSAGE_CONTENT intent (must also be enabled in the Dev Portal).
//! - `ABBEY_VISION_PROVIDER=remote|fm|off`, other `ABBEY_VISION_*`, and
//!   `TELEGRAM_BOT_TOKEN` (optional) — see `.env.example`.
//! - `ABBEY_VOICE_GUILD_ID` + `ABBEY_VOICE_CHANNEL_ID` (optional) — enable an
//!   admin-triggered, DAVE-capable Discord connection. `ABBEY_VOICE_AUTOJOIN=1`
//!   provides persistent muted/self-deafened no-audio presence. Conversational
//!   local or Realtime voice still requires `/voice join consent:true`.
//! - `--voice-self-test OUTPUT.wav` — run local TTS → STT → canonical Abbey
//!   reasoning → TTS without a Discord token, microphone, or call.
//! - `--provider-self-test primary|fm|all --json` — qualify configured routes
//!   with synthetic, non-persistent fixtures before reading Discord or state.
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
#[cfg(test)]
mod contracts;
mod embedding;
mod engine;
mod gateway;
mod generation;
mod grounding;
mod guild;
mod http_body;
mod inspect;
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
mod provider;
mod provider_self_test;
mod recall;
mod routing_signals;
mod runtime;
mod server;
mod text;
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

    let startup = match startup_action() {
        Ok(action) => action,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    match startup {
        StartupAction::Discord => {}
        StartupAction::VoiceSelfTest(output) => {
            let report = voice_self_test::run(&output)
                .await
                .map_err(runtime::StartupError)?;
            println!(
                "local voice self-test passed\nround-trip word recall: {:.0}%\naudio: {} ({} Hz, {} channel(s), {} ms)",
                report.round_trip_word_recall * 100.0,
                report.output.display(),
                report.sample_rate,
                report.channels,
                report.duration_millis,
            );
            return Ok(());
        }
        StartupAction::ProviderSelfTest(target) => {
            let outcome = provider_self_test::run(target).await;
            println!(
                "{}",
                serde_json::to_string_pretty(&outcome.report)
                    .map_err(|error| runtime::StartupError(error.to_string()))?
            );
            if outcome.exit != provider_self_test::SelfTestExit::Success {
                std::process::exit(outcome.exit.code());
            }
            return Ok(());
        }
    }

    // Read before building anything else: a missing token should fail in the
    // first millisecond with a sentence you can act on, not inside a gateway
    // handshake error.
    let (http, credential_source) = {
        let credential = read_discord_token(|source| std::env::var(source.env_name()))?;
        let source = credential.source();
        let http = serenity::http::HttpBuilder::new(credential.secret())
            .default_allowed_mentions(gateway::no_mentions())
            .build();
        (http, source)
    };
    if let Err(error) = http.get_current_user().await {
        return Err(map_discord_startup_error(error, credential_source));
    }
    tracing::info!("{}", credential_source.accepted_diagnostic());

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
        .map(|config| {
            voice_session::VoiceRuntime::new_with_inspect(
                config,
                std::sync::Arc::clone(&state.voice_inspect),
            )
        })
        .map(std::sync::Arc::new);
    match &state.data_dir {
        Some(dir) => tracing::info!(path = %dir.display(), "persisting to data dir"),
        None => tracing::warn!("ABBEY_DATA_DIR unset — learning and memory are in-memory only"),
    }
    match state.generation_label() {
        Some(label) => tracing::info!(backend = label, "generation backend configured"),
        None => tracing::warn!("no generation backend — Abbey answers honestly that she cannot"),
    }
    if let Some(fm) = &state.foundation_models {
        tracing::info!(
            mode = fm.config.mode.as_str(),
            fallback = fm.config.fallback,
            server = fm.config.endpoint.is_some(),
            "Apple Foundation Models secondary configured"
        );
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
                commands_brain::pending(),
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
                    let handled = commands_voice::on_gateway_event(ctx, event, data).await;
                    if !handled {
                        gateway::on_discord_event(ctx, event, &data.state).await;
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

    use songbird::SerenityInit;
    let mut client = serenity::client::ClientBuilder::new_with_http(http, intents)
        .framework(framework)
        .register_songbird_from_config(
            songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Pass),
        )
        .await
        .map_err(|error| map_discord_startup_error(error, credential_source))?;

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
    result.map_err(|error| map_discord_startup_error(error, credential_source))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscordTokenSource {
    Primary,
    Fallback,
}

impl DiscordTokenSource {
    const fn env_name(self) -> &'static str {
        match self {
            Self::Primary => "DISCORD_TOKEN",
            Self::Fallback => "DISCORD_BOT_TOKEN",
        }
    }

    const fn blank_error(self) -> &'static str {
        match self {
            Self::Primary => {
                "DISCORD_TOKEN is present but blank; refusing to consult DISCORD_BOT_TOKEN."
            }
            Self::Fallback => "DISCORD_BOT_TOKEN is present but blank.",
        }
    }

    const fn non_unicode_error(self) -> &'static str {
        match self {
            Self::Primary => {
                "DISCORD_TOKEN is not valid Unicode; refusing to consult DISCORD_BOT_TOKEN."
            }
            Self::Fallback => "DISCORD_BOT_TOKEN is not valid Unicode.",
        }
    }

    const fn accepted_diagnostic(self) -> &'static str {
        match self {
            Self::Primary => {
                "Discord authentication preflight accepted the credential from DISCORD_TOKEN."
            }
            Self::Fallback => {
                "Discord authentication preflight accepted the credential from DISCORD_BOT_TOKEN."
            }
        }
    }

    const fn rejected_diagnostic(self) -> &'static str {
        match self {
            Self::Primary => {
                "DISCORD_TOKEN was rejected by Discord during authentication. Reset the bot token in the Developer Portal, export the new value as DISCORD_TOKEN, and never hardcode it."
            }
            Self::Fallback => {
                "DISCORD_BOT_TOKEN was rejected by Discord during authentication. Reset the bot token in the Developer Portal, export the new value as DISCORD_BOT_TOKEN, and never hardcode it."
            }
        }
    }
}

struct DiscordToken(Box<str>);

struct SelectedDiscordToken {
    secret: DiscordToken,
    source: DiscordTokenSource,
}

impl SelectedDiscordToken {
    fn secret(&self) -> &str {
        &self.secret.0
    }

    const fn source(&self) -> DiscordTokenSource {
        self.source
    }
}

fn read_discord_token(
    mut read: impl FnMut(DiscordTokenSource) -> Result<String, std::env::VarError>,
) -> Result<SelectedDiscordToken, String> {
    match read(DiscordTokenSource::Primary) {
        Ok(value) => select_present_discord_token(DiscordTokenSource::Primary, value),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(DiscordTokenSource::Primary.non_unicode_error().into())
        }
        Err(std::env::VarError::NotPresent) => {
            match read(DiscordTokenSource::Fallback) {
                Ok(value) => select_present_discord_token(DiscordTokenSource::Fallback, value),
                Err(std::env::VarError::NotUnicode(_)) => {
                    Err(DiscordTokenSource::Fallback.non_unicode_error().into())
                }
                Err(std::env::VarError::NotPresent) => Err(
                    "Neither DISCORD_TOKEN nor DISCORD_BOT_TOKEN is set. Export one bot token; never hardcode it."
                        .into(),
                ),
            }
        }
    }
}

fn select_present_discord_token(
    source: DiscordTokenSource,
    value: String,
) -> Result<SelectedDiscordToken, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(source.blank_error().into());
    }
    Ok(SelectedDiscordToken {
        secret: DiscordToken(value.into()),
        source,
    })
}

fn explain_discord_http_status(source: DiscordTokenSource, status: u16) -> Option<&'static str> {
    (status == 401).then_some(source.rejected_diagnostic())
}

fn explain_discord_gateway_error(
    source: DiscordTokenSource,
    error: &serenity::gateway::GatewayError,
) -> Option<&'static str> {
    matches!(
        error,
        serenity::gateway::GatewayError::InvalidAuthentication
    )
    .then_some(source.rejected_diagnostic())
}

fn explain_discord_startup_error(
    source: DiscordTokenSource,
    error: &serenity::Error,
) -> Option<&'static str> {
    match error {
        serenity::Error::Http(http) => http
            .status_code()
            .map(|code| code.as_u16())
            .and_then(|status| explain_discord_http_status(source, status)),
        serenity::Error::Gateway(error) => explain_discord_gateway_error(source, error),
        _ => None,
    }
}

fn map_discord_startup_error(error: serenity::Error, source: DiscordTokenSource) -> Error {
    match explain_discord_startup_error(source, &error) {
        Some(message) => message.into(),
        None => error.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupAction {
    Discord,
    VoiceSelfTest(std::path::PathBuf),
    ProviderSelfTest(provider::QualificationTarget),
}

fn startup_action() -> Result<StartupAction, String> {
    parse_startup_arguments(std::env::args_os().skip(1))
}

fn parse_startup_arguments(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<StartupAction, String> {
    let Some(mode) = arguments.next() else {
        return Ok(StartupAction::Discord);
    };
    if mode == std::ffi::OsStr::new("--voice-self-test") {
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
        return Ok(StartupAction::VoiceSelfTest(output.into()));
    }
    if mode == std::ffi::OsStr::new("--provider-self-test") {
        let target = arguments.next().ok_or_else(provider_self_test_usage)?;
        let target = match target.to_str() {
            Some("primary") => provider::QualificationTarget::Primary,
            Some("fm") => provider::QualificationTarget::Fm,
            Some("all") => provider::QualificationTarget::All,
            _ => return Err(provider_self_test_usage()),
        };
        if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--json"))
            || arguments.next().is_some()
        {
            return Err(provider_self_test_usage());
        }
        return Ok(StartupAction::ProviderSelfTest(target));
    }
    Err(format!(
        "unknown argument {mode:?}; usage: abbey-bot [--voice-self-test OUTPUT.wav | --provider-self-test primary|fm|all --json]"
    ))
}

fn provider_self_test_usage() -> String {
    "usage: abbey-bot --provider-self-test primary|fm|all --json".into()
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

    fn parse(arguments: &[&str]) -> Result<StartupAction, String> {
        parse_startup_arguments(arguments.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn no_arguments_starts_the_discord_service() {
        assert_eq!(parse(&[]).unwrap(), StartupAction::Discord);
    }

    #[test]
    fn exact_voice_self_test_has_one_create_new_output() {
        assert_eq!(
            parse(&["--voice-self-test", "audition.wav"]).unwrap(),
            StartupAction::VoiceSelfTest(std::path::PathBuf::from("audition.wav"))
        );
        assert!(parse(&["--voice-self-test"]).is_err());
        assert!(parse(&["--voice-self-test", "one.wav", "two.wav"]).is_err());
    }

    #[test]
    fn an_unknown_or_mistyped_mode_cannot_start_discord() {
        assert!(parse(&["--voice-self-tset", "audition.wav"]).is_err());
        assert!(parse(&["unexpected"]).is_err());
    }

    #[test]
    fn provider_self_test_requires_exact_target_and_json_mode() {
        assert_eq!(
            parse(&["--provider-self-test", "primary", "--json"]).unwrap(),
            StartupAction::ProviderSelfTest(provider::QualificationTarget::Primary)
        );
        assert_eq!(
            parse(&["--provider-self-test", "fm", "--json"]).unwrap(),
            StartupAction::ProviderSelfTest(provider::QualificationTarget::Fm)
        );
        assert_eq!(
            parse(&["--provider-self-test", "all", "--json"]).unwrap(),
            StartupAction::ProviderSelfTest(provider::QualificationTarget::All)
        );
        for invalid in [
            &["--provider-self-test"][..],
            &["--provider-self-test", "pcc", "--json"],
            &["--provider-self-test", "fm"],
            &["--provider-self-test", "fm", "--json", "extra"],
        ] {
            assert!(parse(invalid).is_err(), "accepted {invalid:?}");
        }
    }
}

#[cfg(test)]
mod discord_token_tests {
    use super::*;
    use std::cell::Cell;

    fn missing() -> Result<String, std::env::VarError> {
        Err(std::env::VarError::NotPresent)
    }

    fn non_unicode() -> Result<String, std::env::VarError> {
        Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
            "private-byte-canary",
        )))
    }

    fn select(
        primary: Result<String, std::env::VarError>,
        fallback: Result<String, std::env::VarError>,
    ) -> Result<SelectedDiscordToken, String> {
        let mut primary = Some(primary);
        let mut fallback = Some(fallback);
        read_discord_token(|source| match source {
            DiscordTokenSource::Primary => primary.take().expect("primary read once"),
            DiscordTokenSource::Fallback => fallback.take().expect("fallback read once"),
        })
    }

    #[test]
    fn missing_both_sources_fails_with_a_sentence() {
        assert_eq!(
            select(missing(), missing()).err().expect("must fail"),
            "Neither DISCORD_TOKEN nor DISCORD_BOT_TOKEN is set. Export one bot token; never hardcode it."
        );
    }

    #[test]
    fn nonblank_primary_wins_without_reading_fallback() {
        let fallback_reads = Cell::new(0);
        let selected = read_discord_token(|source| match source {
            DiscordTokenSource::Primary => Ok("  primary-token  ".into()),
            DiscordTokenSource::Fallback => {
                fallback_reads.set(fallback_reads.get() + 1);
                Ok("fallback-token".into())
            }
        })
        .expect("primary selected");
        assert_eq!(selected.source(), DiscordTokenSource::Primary);
        assert_eq!(selected.secret(), "primary-token");
        assert_eq!(fallback_reads.get(), 0);
    }

    #[test]
    fn blank_primary_fails_without_reading_fallback() {
        let fallback_reads = Cell::new(0);
        let error = read_discord_token(|source| match source {
            DiscordTokenSource::Primary => Ok("  ".into()),
            DiscordTokenSource::Fallback => {
                fallback_reads.set(fallback_reads.get() + 1);
                Ok("fallback-token".into())
            }
        })
        .err()
        .expect("blank primary must fail");
        assert_eq!(
            error,
            "DISCORD_TOKEN is present but blank; refusing to consult DISCORD_BOT_TOKEN."
        );
        assert_eq!(fallback_reads.get(), 0);
    }

    #[test]
    fn absent_primary_selects_nonblank_fallback() {
        let selected = select(missing(), Ok(" fallback-token ".into())).expect("fallback selected");
        assert_eq!(selected.source(), DiscordTokenSource::Fallback);
        assert_eq!(selected.secret(), "fallback-token");
    }

    #[test]
    fn blank_fallback_has_a_source_specific_error() {
        assert_eq!(
            select(missing(), Ok(" \t ".into()))
                .err()
                .expect("blank fallback must fail"),
            "DISCORD_BOT_TOKEN is present but blank."
        );
    }

    #[test]
    fn non_unicode_primary_fails_without_reading_fallback_or_bytes() {
        let fallback_reads = Cell::new(0);
        let error = read_discord_token(|source| match source {
            DiscordTokenSource::Primary => non_unicode(),
            DiscordTokenSource::Fallback => {
                fallback_reads.set(fallback_reads.get() + 1);
                Ok("fallback-token".into())
            }
        })
        .err()
        .expect("non-Unicode primary must fail");
        assert_eq!(
            error,
            "DISCORD_TOKEN is not valid Unicode; refusing to consult DISCORD_BOT_TOKEN."
        );
        assert!(!error.contains("private-byte-canary"));
        assert_eq!(fallback_reads.get(), 0);
    }

    #[test]
    fn non_unicode_fallback_fails_without_reproducing_bytes() {
        let error = select(missing(), non_unicode())
            .err()
            .expect("non-Unicode fallback must fail");
        assert_eq!(error, "DISCORD_BOT_TOKEN is not valid Unicode.");
        assert!(!error.contains("private-byte-canary"));
    }

    #[test]
    fn accepted_and_rejected_diagnostics_name_only_the_selected_source() {
        for (source, selected_name, other_name) in [
            (
                DiscordTokenSource::Primary,
                "DISCORD_TOKEN",
                "DISCORD_BOT_TOKEN",
            ),
            (
                DiscordTokenSource::Fallback,
                "DISCORD_BOT_TOKEN",
                "DISCORD_TOKEN",
            ),
        ] {
            let accepted = source.accepted_diagnostic();
            let rejected = source.rejected_diagnostic();
            assert!(accepted.contains(selected_name));
            assert!(rejected.contains(selected_name));
            assert!(!accepted.contains(other_name));
            assert!(!rejected.contains(other_name));
            assert!(!accepted.contains("secret-canary"));
            assert!(!rejected.contains("secret-canary"));
        }
    }

    #[test]
    fn auth_rejections_are_mapped_but_other_failures_are_preserved() {
        for source in [DiscordTokenSource::Primary, DiscordTokenSource::Fallback] {
            assert_eq!(
                explain_discord_http_status(source, 401),
                Some(source.rejected_diagnostic())
            );
            assert_eq!(explain_discord_http_status(source, 403), None);
            assert_eq!(explain_discord_http_status(source, 500), None);
            assert_eq!(
                explain_discord_gateway_error(
                    source,
                    &serenity::gateway::GatewayError::InvalidAuthentication,
                ),
                Some(source.rejected_diagnostic())
            );
            assert_eq!(
                explain_discord_gateway_error(
                    source,
                    &serenity::gateway::GatewayError::InvalidGatewayIntents,
                ),
                None
            );
            let original = serenity::gateway::GatewayError::InvalidGatewayIntents;
            let expected = original.to_string();
            let mapped = map_discord_startup_error(serenity::Error::Gateway(original), source);
            assert_eq!(mapped.to_string(), expected);
        }
    }
}
