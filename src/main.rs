//! Abbey Bot — Discord operational layer, Rust/serenity/poise.
//!
//! Configuration is entirely environment-driven, per the skill's standing rule
//! that tokens live in env vars and never in source:
//!
//! - `DISCORD_TOKEN` (required) — bot token.
//! - `ABBEY_GUILD_ID` (optional) — also register commands immediately in this
//!   home guild. Commands always register globally for every installation;
//!   global propagation can take up to an hour. This does not restrict routing.
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
//! - `ABBEY_EPISODE_GATE_CONFIG` (optional) — absolute path to a JSON file
//!   that turns on the constitutional episode gate client (`episode_gate.rs`):
//!   `/admin learning on|off` is then mirrored into the WDBX ledger through
//!   the `abi` binary as a content-free `proposal`. Unset means no ledger
//!   write is ever attempted. See `.env.example`.
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
//! - `--server-plan PLAN.toml --guild ID [--stage …] [--category …] [--apply]` —
//!   diff a server plan against a live guild; dry run unless `--apply`
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

mod admin_dashboard;
mod ask;
mod audio_tap;
mod bootstrap;
mod brain;
mod checkpoint_gate;
mod command_catalog;
#[cfg(test)]
mod command_registration_tests;
mod commands;
mod commands_brain;
mod commands_context;
mod commands_forum;
mod commands_help;
mod commands_memory_browser;
mod commands_voice;
#[cfg(test)]
mod contracts;
mod embedding;
mod engine;
mod episode_gate;
mod forum;
mod gateway;
mod generation;
mod grounding;
mod guild;
mod help_center;
mod host_music;
mod http_body;
mod image_attachment;
mod inspect;
mod llm;
mod managed_env;
mod managed_log;
mod managed_service;
mod memory;
mod memory_browser;
mod memory_card;
mod memory_gate;
mod moderation;
mod music;
mod observability;
mod offline_voice;
mod operator_guidance;
mod perms;
mod persist;
mod persona;
mod pipeline;
mod platform;
mod player_control;
mod profile;
mod provider;
mod provider_self_test;
mod readiness;
mod recall;
mod routing_signals;
mod runtime;
mod scoped_stats;
mod server;
mod service;
mod text;
mod tools;
mod vad;
mod vision;
mod voice;
mod voice_consent;
mod voice_consent_store;
mod voice_local;
mod voice_openai;
mod voice_registry;
mod voice_self_test;
mod voice_session;
mod voice_views;
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

impl Data {
    /// Registry lookup never falls back to another guild's default session.
    pub(crate) fn voice_for(
        &self,
        guild: u64,
    ) -> Option<std::sync::Arc<voice_session::VoiceRuntime>> {
        if self.state.voice_registry.is_configured() {
            self.state.voice_registry.get(guild)
        } else {
            self.voice
                .as_ref()
                .filter(|v| v.config.guild_id == guild)
                .cloned()
        }
    }

    pub(crate) fn reserve_voice_join(
        &self,
        guild: u64,
    ) -> Result<voice_registry::JoinReservation, &'static str> {
        if !self.state.voice_registry.is_configured() {
            let runtime = self.voice.as_ref().filter(|v| v.config.guild_id == guild)
                .ok_or("Voice is not configured. A service operator can configure a speech backend, then restart Abbey.")?;
            self.state.voice_registry.configure(
                runtime.config.template(),
                Some(runtime.clone()),
                self.state.data_dir.clone(),
                self.state.voice_inspect.clone(),
            )?;
        }
        self.state.voice_registry.reserve_join(guild)
    }

    pub(crate) fn cancel_voice_join(&self, guild: u64) {
        self.state.voice_registry.cancel_join(guild);
    }

    pub(crate) async fn voice_for_join(
        &self,
        guild: u64,
        channel: u64,
        reservation: &voice_registry::JoinReservation,
    ) -> Result<std::sync::Arc<voice_session::VoiceRuntime>, &'static str> {
        self.state
            .voice_registry
            .get_or_create(guild, channel, reservation)
            .await
    }

    pub(crate) fn retire_voice_after_leave(
        &self,
        guild: u64,
        runtime: &std::sync::Arc<voice_session::VoiceRuntime>,
    ) -> bool {
        self.state.voice_registry.retire(guild, runtime)
    }
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

struct ManagedStartup {
    publisher: readiness::ReadinessPublisher,
    log: std::sync::Arc<managed_log::ManagedLog>,
    privacy_report: persist::PersistReport,
    fatal: std::sync::Arc<managed_service::ManagedFatalSignal>,
}

fn configure_managed_panic_hook(
    fatal: Option<std::sync::Arc<managed_service::ManagedFatalSignal>>,
) {
    std::panic::set_hook(Box::new(move |_| {
        if let Some(fatal) = &fatal {
            fatal.trigger();
        }
    }));
}

mod startup;
use startup::run;

fn main() -> Result<(), Error> {
    // Argument validation and managed credential loading happen before any
    // runtime/log worker thread exists. Self-tests never enter managed preflight.
    let managed_requested = std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--managed-service");
    let startup = match startup_action() {
        Ok(action) => action,
        Err(error) => {
            if managed_requested {
                eprintln!("invalid managed service arguments");
            } else {
                eprintln!("{error}");
            }
            std::process::exit(2);
        }
    };
    let managed = if startup == StartupAction::ManagedDiscord {
        configure_managed_panic_hook(None);
        let Some(home) = std::env::var_os("HOME") else {
            std::process::exit(78);
        };
        let prepared = match managed_service::begin(std::path::Path::new(&home)) {
            Ok(prepared) => prepared,
            Err(_) => std::process::exit(78),
        };
        let fatal = prepared.fatal.clone();
        configure_managed_panic_hook(Some(fatal));
        for (name, value) in prepared.environment.into_values() {
            // SAFETY: main has not constructed a runtime, logging worker or any
            // other application thread. Every environment reader starts later.
            unsafe {
                std::env::set_var(name, value);
            }
        }
        Some(ManagedStartup {
            publisher: prepared.publisher,
            log: prepared.log,
            privacy_report: prepared.privacy_report,
            fatal: prepared.fatal,
        })
    } else {
        None
    };
    let is_managed = managed.is_some();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut terminal = service::shutdown::TerminalBoundary::default();
    let result = runtime.block_on(run(startup, managed, &mut terminal));
    if let Some(initialization) = &mut terminal.initialization {
        let remaining = terminal.budget.map_or(service::SHUTDOWN_BUDGET, |budget| {
            budget.remaining(tokio::time::Instant::now())
        });
        let joined = runtime.block_on(async {
            tokio::time::timeout(remaining, initialization)
                .await
                .is_ok()
        });
        terminal.incomplete |= !joined;
    }
    if !terminal.refresh_joined
        && let Some(refresh) = &mut terminal.refresh
    {
        refresh.stop();
        let remaining = terminal.budget.map_or(service::SHUTDOWN_BUDGET, |budget| {
            budget.remaining(tokio::time::Instant::now())
        });
        let result =
            runtime.block_on(async { tokio::time::timeout(remaining, refresh.joined()).await });
        terminal.refresh_joined = result.is_ok();
        terminal.incomplete |= !result.is_ok_and(|r| r.is_ok());
    }
    if !terminal.telemetry_joined
        && let Some(telemetry) = &mut terminal.telemetry
    {
        telemetry.stop();
        let remaining = terminal.budget.map_or(service::SHUTDOWN_BUDGET, |budget| {
            budget.remaining(tokio::time::Instant::now())
        });
        terminal.telemetry_joined = runtime.block_on(async {
            tokio::time::timeout(remaining, telemetry.joined())
                .await
                .is_ok_and(|r| r.is_ok())
        });
        terminal.incomplete |= !terminal.telemetry_joined;
    }
    let remaining = terminal.budget.map_or(service::SHUTDOWN_BUDGET, |budget| {
        budget.remaining(tokio::time::Instant::now())
    });
    runtime.shutdown_timeout(remaining);
    // Incomplete blocking work is contained by process termination, never
    // reported as joined or allowed into unbounded implicit Runtime::drop.
    if terminal.incomplete {
        std::process::exit(70);
    }
    if is_managed && result.is_err() {
        std::process::exit(if terminal.budget.is_none() { 78 } else { 1 });
    }
    result
}

fn startup_stop(
    supervisor: &mut service::ServiceSupervisor,
    terminal: &mut service::shutdown::TerminalBoundary,
    reason: service::ShutdownReason,
) -> Error {
    let start = supervisor.begin_draining(reason, tokio::time::Instant::now());
    terminal.budget = Some(start.budget);
    runtime::StartupError("service interrupted during startup".into()).into()
}

fn initialize_state() -> Result<Data, Error> {
    let state = runtime::AppState::from_env()?;
    let environment = voice::VoiceEnvironment::from_env().map_err(runtime::StartupError)?;
    let voice = environment
        .as_ref()
        .and_then(|env| env.default.clone())
        .map(|config| {
            let consent = std::sync::Arc::new(voice_consent_store::ConsentStore::load(
                state.data_dir.as_deref(),
                config.guild_id,
            ));
            std::sync::Arc::new(voice_session::VoiceRuntime::new_with_inspect(
                config,
                state.voice_inspect.clone(),
                consent,
            ))
        });
    if let Some(environment) = environment {
        state
            .voice_registry
            .configure(
                environment.template,
                voice.clone(),
                state.data_dir.clone(),
                state.voice_inspect.clone(),
            )
            .map_err(|error| runtime::StartupError(error.into()))?;
    }
    Ok(Data { state, voice })
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
    ManagedDiscord,
    VoiceSelfTest(std::path::PathBuf),
    ProviderSelfTest(provider::QualificationTarget),
    ServerPlan(server::run::Options),
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
    if mode == std::ffi::OsStr::new("--managed-service") {
        if arguments.next().is_some() {
            return Err("--managed-service must be the sole argument".into());
        }
        return Ok(StartupAction::ManagedDiscord);
    }
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
    if mode == std::ffi::OsStr::new("--server-plan") {
        return server::run::parse_options(arguments).map(StartupAction::ServerPlan);
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
        "unknown argument {mode:?}; usage: abbey-bot [--voice-self-test OUTPUT.wav | --provider-self-test primary|fm|all --json | --server-plan PLAN.toml --guild ID [--stage additive|reveal|overwrites] [--category NAME] [--apply]]"
    ))
}

fn provider_self_test_usage() -> String {
    "usage: abbey-bot --provider-self-test primary|fm|all --json".into()
}

/// Abbey-owned commands only. Discord's application-owned Entry Point is
/// deliberately absent and is merged only for global registration.
fn application_commands() -> Vec<poise::Command<Data, Error>> {
    let mut commands = vec![
        commands_help::help(),
        commands::persona(),
        commands::whois(),
        commands::profile_context_menu(),
        commands::ask_context_menu(),
        commands_brain::memory_context_menu(),
        commands_context::describe_image(),
        commands_context::read_image_text(),
        commands::perms(),
        commands::modcall(),
        commands::server(),
        commands::webhook(),
        commands_forum::forum(),
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
    ];
    commands_help::bind_commands(&mut commands);
    commands
}

/// Purely merge fetched application-owned Entry Points into Abbey's freshly
/// generated global command payload. Guild registration never calls this.
fn merge_entry_point_commands(
    mut generated: Vec<serenity::all::CreateCommand>,
    existing: impl IntoIterator<Item = serenity::all::Command>,
) -> Vec<serenity::all::CreateCommand> {
    use serenity::all::{CommandType, CreateCommand};

    for command in existing
        .into_iter()
        .filter(|command| command.kind == CommandType::PrimaryEntryPoint)
    {
        let mut preserved = CreateCommand::new(command.name)
            .kind(CommandType::PrimaryEntryPoint)
            .description(command.description)
            .integration_types(command.integration_types)
            .nsfw(command.nsfw);
        for (locale, name) in command.name_localizations.unwrap_or_default() {
            preserved = preserved.name_localized(locale, name);
        }
        for (locale, description) in command.description_localizations.unwrap_or_default() {
            preserved = preserved.description_localized(locale, description);
        }
        if let Some(permissions) = command.default_member_permissions {
            preserved = preserved.default_member_permissions(permissions);
        }
        if let Some(contexts) = command.contexts {
            preserved = preserved.contexts(contexts);
        } else if command.dm_permission == Some(false) {
            preserved = preserved.contexts(vec![serenity::all::InteractionContext::Guild]);
        }
        if let Some(handler) = command.handler {
            preserved = preserved.handler(handler);
        }
        generated.push(preserved);
    }
    generated
}

/// Register every installation first, then the optional immediate home copy.
/// The caller can publish registration readiness only after both have succeeded.
async fn register_command_scopes<G, F, H, E>(
    home: Option<serenity::all::GuildId>,
    global: G,
    register_home: F,
) -> Result<(), E>
where
    G: std::future::Future<Output = Result<(), E>>,
    F: FnOnce(serenity::all::GuildId) -> H,
    H: std::future::Future<Output = Result<(), E>>,
{
    global.await?;
    if let Some(home) = home {
        register_home(home).await?;
    }
    Ok(())
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
    use serenity::all::{Command, CommandType};

    let create = poise::builtins::create_application_commands(commands);
    let existing = Command::get_global_commands_with_localizations(&ctx.http).await?;
    for command in &existing {
        if command.kind == CommandType::PrimaryEntryPoint {
            tracing::info!(name = %command.name, "preserving the app's Entry Point command");
        }
    }
    let create = merge_entry_point_commands(create, existing);
    Command::set_global_commands(&ctx.http, create).await?;
    Ok(())
}

/// `InteractionLog` row per slash command (`docs/spec/botarchitecture.md`).
fn record_interaction(
    ctx: Context<'_>,
    succeeded: bool,
    error: Option<memory::InteractionErrorCategory>,
) {
    let started = u64::try_from(ctx.created_at().timestamp_millis()).unwrap_or(0);
    let now = runtime::now_millis();
    let duration_ms = memory::InteractionEntry::total_latency_ms(started, now);
    let entry = memory::InteractionEntry::new(
        &ctx.command().qualified_name,
        succeeded,
        error,
        duration_ms,
        now,
    );
    runtime::AppState::lock(&ctx.data().state.stores)
        .memory
        .interactions
        .record(entry);
}

#[cfg(test)]
mod startup_argument_tests;

#[cfg(test)]
mod discord_token_tests;
