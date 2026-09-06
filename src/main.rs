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
mod commands_help;
mod commands_memory_browser;
mod commands_voice;
#[cfg(test)]
mod contracts;
mod embedding;
mod engine;
mod episode_gate;
mod gateway;
mod generation;
mod grounding;
mod guild;
mod help_center;
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
    let voice = voice::VoiceConfig::from_env()
        .map_err(runtime::StartupError)?
        .map(|config| {
            let consent = std::sync::Arc::new(voice_consent_store::ConsentStore::load(
                state.data_dir.as_deref(),
                config.guild_id,
            ));
            voice_session::VoiceRuntime::new_with_inspect(
                config,
                state.voice_inspect.clone(),
                consent,
            )
        })
        .map(std::sync::Arc::new);
    Ok(Data { state, voice })
}

async fn run(
    startup: StartupAction,
    managed: Option<ManagedStartup>,
    terminal: &mut service::shutdown::TerminalBoundary,
) -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(if managed.is_some() {
            tracing_subscriber::EnvFilter::new("off")
        } else {
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        })
        .init();

    match startup {
        StartupAction::Discord | StartupAction::ManagedDiscord => {}
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
        StartupAction::ServerPlan(options) => {
            // Same fail-closed token selection as the service path; the plan
            // engine is REST-only and never opens the gateway.
            let credential = match read_discord_token(|source| std::env::var(source.env_name())) {
                Ok(credential) => credential,
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(2);
                }
            };
            let http = serenity::http::HttpBuilder::new(credential.secret()).build();
            let code = server::run::run(&options, &http).await;
            if code != 0 {
                std::process::exit(code);
            }
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

    let mut supervisor = service::ServiceSupervisor::new();
    let signal = shutdown_signal();
    tokio::pin!(signal);
    let connectors = gateway::ConnectorConfig::from_env()?;
    let managed_fatal = managed.as_ref().map(|prepared| prepared.fatal.clone());
    let managed_status = if let Some(prepared) = managed {
        let identity = prepared.publisher.identity().clone();
        let writer = service::telemetry::TelemetryWriter::start(
            prepared.log,
            prepared.publisher,
            prepared.fatal.clone(),
        );
        let status = std::sync::Arc::new(service::status::ManagedStatus::new(
            identity,
            writer.requests(),
            prepared.privacy_report,
            connectors.telegram_enabled(),
            connectors.slack_enabled(),
        ));
        terminal.telemetry = Some(writer);
        terminal.refresh = Some(service::refresh::RefreshOwner::start(
            status.clone(),
            prepared.fatal,
        ));
        let result = status
            .refresh()
            .map_err(|_| "managed starting publication failed")?;
        tokio::select! {
            result = result => result.map_err(|_| "managed starting publication interrupted")?.map_err(|_| "managed starting publication failed")?,
            _ = &mut signal => return Err(startup_stop(&mut supervisor, terminal, service::ShutdownReason::Signal)),
        }
        Some(status)
    } else {
        None
    };
    let fatal_wait = async {
        match &managed_fatal {
            Some(fatal) => fatal.notified().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(fatal_wait);

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
    tokio::select! {
        result = http.get_current_user() => if let Err(error) = result { return Err(map_discord_startup_error(error, credential_source)); },
        () = &mut fatal_wait => return Err(startup_stop(&mut supervisor, terminal, service::ShutdownReason::OperationalInvariantFailed)),
        _ = &mut signal => return Err(startup_stop(&mut supervisor, terminal, service::ShutdownReason::Signal)),
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

    let initialization = terminal
        .initialization
        .insert(tokio::task::spawn_blocking(initialize_state));
    let initialized = tokio::select! {
        result = initialization => result,
        _ = &mut signal => return Err(startup_stop(&mut supervisor, terminal, service::ShutdownReason::Signal)),
        () = &mut fatal_wait => return Err(startup_stop(&mut supervisor, terminal, service::ShutdownReason::OperationalInvariantFailed)),
    };
    terminal.initialization = None;
    let Data {
        state,
        voice: voice_runtime,
    } = initialized.map_err(|_| "state initialization panicked")??;
    if let (Some(status), Some(writer)) = (&managed_status, &terminal.telemetry) {
        state.attach_observability(writer.requests(), status.clone());
    }
    match &state.data_dir {
        Some(dir) => tracing::info!(path = %dir.display(), "persisting to data dir"),
        None => tracing::warn!("ABBEY_DATA_DIR unset — learning and memory are in-memory only"),
    }
    match state.generation_label() {
        Some(label) => tracing::info!(backend = label, "generation backend configured"),
        None => tracing::warn!("no generation backend — Abbey answers honestly that she cannot"),
    }
    let env_presence = voice::OperatorEnvPresence::from_env();
    // Field names must stay off the privacy denylist (`discord_token`,
    // `vision_endpoint`, …). Log only counts + a names-only summary string.
    let presence_pairs = [
        ("DISCORD_TOKEN", env_presence.discord_token),
        ("ABBEY_GUILD_ID", env_presence.abbey_guild_id),
        ("ABBEY_BOT_LLM_ENDPOINT", env_presence.llm_endpoint),
        ("ABBEY_BOT_LLM_MODEL", env_presence.llm_model),
        ("ABBEY_VISION_ENDPOINT", env_presence.vision_endpoint),
        ("ABBEY_VISION_MODEL", env_presence.vision_model),
        ("ABBEY_VOICE_GUILD_ID", env_presence.voice_guild_id),
        ("ABBEY_VOICE_CHANNEL_ID", env_presence.voice_channel_id),
        ("ABBEY_VOICE_MODE", env_presence.voice_mode),
        (
            "ABBEY_VOICE_LOCAL_ENDPOINT",
            env_presence.voice_local_endpoint,
        ),
    ];
    let present: Vec<&str> = presence_pairs
        .iter()
        .filter_map(|(name, set)| set.then_some(*name))
        .collect();
    let missing: Vec<&str> = presence_pairs
        .iter()
        .filter_map(|(name, set)| (!set).then_some(*name))
        .collect();
    tracing::info!(
        present_count = present.len(),
        missing_count = missing.len(),
        "operator env key presence (values withheld); present={present:?}; missing={missing:?}"
    );
    let voice_is_local = voice_runtime
        .as_ref()
        .is_some_and(|runtime| runtime.config.mode() == voice::VoiceMode::Local);
    let has_loopback_llm = state.providers.local_voice_route().is_some();
    if let Some(warning) = env_presence.local_voice_llm_gap(voice_is_local, has_loopback_llm) {
        tracing::warn!("{warning}");
    }
    if voice_is_local && !env_presence.voice_local_endpoint {
        tracing::info!(
            "ABBEY_VOICE_LOCAL_ENDPOINT unset — local speech defaults to http://127.0.0.1:8181"
        );
    }
    if let Some(fm) = state.providers.foundation_models() {
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
    let mut writer = state.attach_service(supervisor.operations());
    let mut provider_writer = state.providers.attach_block_writer();
    if let Some(voice) = &voice_runtime {
        voice.attach_service(supervisor.operations());
        if let Some(events) = state.operational_events() {
            voice.attach_telemetry(events.clone());
        }
    }

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
            commands: application_commands(),
            event_handler: |ctx, event, framework, data| {
                Box::pin(async move {
                    if let serenity::all::FullEvent::InteractionCreate {
                        interaction: serenity::all::Interaction::Component(interaction),
                    } = event
                        && (commands_brain::dispatch_admin_component(ctx, interaction, data).await
                            || commands_help::dispatch_component(
                                ctx,
                                interaction,
                                data,
                                framework.options().owners.contains(&interaction.user.id),
                            )
                            .await)
                    {
                        return Ok(());
                    }
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
                    if let poise::FrameworkError::Command { ctx, .. } = &error {
                        record_interaction(
                            *ctx,
                            false,
                            Some(memory::InteractionErrorCategory::Internal),
                        );
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
                if let Some(events) = shell_state.operational_events() {
                    let _ = events.record(
                        observability::EventComponent::Discord,
                        observability::EventCode::CommandsRegistered,
                        observability::EventOutcome::Succeeded,
                        None,
                    );
                }
                // Spec (botarchitecture / discordbmapi): Online + Listening "for questions".
                ctx.set_presence(
                    Some(serenity::gateway::ActivityData::listening("for questions")),
                    serenity::model::user::OnlineStatus::Online,
                );
                tracing::info!("presence set: Online, listening for questions");
                if let Some(events) = shell_state.operational_events() {
                    let _ = events.record(
                        observability::EventComponent::Discord,
                        observability::EventCode::PresenceApplied,
                        observability::EventOutcome::Succeeded,
                        None,
                    );
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
                if let Some(events) = shell_state.operational_events() {
                    let _ = events.record(
                        observability::EventComponent::Discord,
                        observability::EventCode::DiscordReady,
                        observability::EventOutcome::Ready,
                        None,
                    );
                }
                if let Some(status) = shell_state.managed_status() {
                    status.discord_ready();
                    let receipt = status
                        .refresh()
                        .map_err(|_| "managed ready publication failed")?;
                    receipt
                        .await
                        .map_err(|_| "managed ready publication interrupted")?
                        .map_err(|_| "managed ready publication failed")?;
                }
                Ok(Data {
                    state: shell_state,
                    voice: setup_voice_runtime,
                })
            })
        })
        .build();

    use songbird::SerenityInit;
    let persistence_failure = writer.failure();
    let provider_failure = provider_writer.failure();
    let construction = serenity::client::ClientBuilder::new_with_http(http, intents)
        .framework(service::framework::OwnedFramework::new(
            framework,
            supervisor.operations(),
        ))
        .register_songbird_from_config(
            songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Pass),
        );
    let built = tokio::select! {
        result = construction => result.map_err(|error| map_discord_startup_error(error, credential_source)),
        _ = &mut signal => Err(runtime::StartupError("service interrupted during Discord initialization".into()).into()),
        () = &mut fatal_wait => Err("managed output failed during initialization".into()),
        () = persistence_failure.notified() => Err("persistence owner failed during initialization".into()),
        () = provider_failure.notified() => Err("provider state owner failed during initialization".into()),
    };
    let mut client = match built {
        Ok(client) => client,
        Err(error) => {
            let start = supervisor.begin_draining(
                service::ShutdownReason::OperationalInvariantFailed,
                tokio::time::Instant::now(),
            );
            supervisor.finish_startup();
            writer.stop();
            provider_writer.stop();
            let stage = start.budget.stage(tokio::time::Instant::now());
            let joined = tokio::time::timeout_at(stage.deadline, async {
                writer.joined().await.is_ok() && provider_writer.joined().await.is_ok()
            })
            .await
            .unwrap_or(false);
            terminal.budget = Some(start.budget);
            terminal.incomplete = !joined;
            terminal.supervisor = Some(supervisor);
            terminal.writer = Some(writer);
            terminal.provider_writer = Some(provider_writer);
            return Err(error);
        }
    };

    supervisor.finish_startup();
    let scheduler_state = state.clone();
    supervisor
        .spawn_service(service::TaskName::Scheduler, move |cancel| {
            scheduler_state.run_scheduler(cancel)
        })
        .map_err(|_| runtime::StartupError("scheduler ownership failed".into()))?;
    gateway::start_connectors(&state, &mut supervisor, connectors)
        .map_err(|_| runtime::StartupError("connector ownership failed".into()))?;
    let shard_manager = client.shard_manager.clone();
    let songbird_manager = client
        .data
        .read()
        .await
        .get::<songbird::SongbirdKey>()
        .cloned();
    let reason = {
        let connection = client.start();
        tokio::pin!(connection);
        loop {
            tokio::select! {
                signal = &mut signal => break if signal.is_ok() { service::ShutdownReason::Signal } else { service::ShutdownReason::OperationalInvariantFailed },
                result = &mut connection => break if result.is_ok() { service::ShutdownReason::DiscordClientReturned } else { service::ShutdownReason::DiscordClientFailed },
                completion = supervisor.next_completion() => {
                    if let Some(events) = state.operational_events() {
                        let _ = events.record(observability::EventComponent::Process, observability::EventCode::TaskExit, match completion.exit { service::TaskExit::Returned => observability::EventOutcome::Succeeded, service::TaskExit::Cancelled => observability::EventOutcome::Cancelled, service::TaskExit::Panicked => observability::EventOutcome::Failed }, None);
                    }
                    if let Some(reason) = completion.fatal { break reason; }
                },
                () = &mut fatal_wait => break service::ShutdownReason::OperationalInvariantFailed,
                () = persistence_failure.notified() => break service::ShutdownReason::OperationalInvariantFailed,
                () = provider_failure.notified() => break service::ShutdownReason::OperationalInvariantFailed,
            }
        }
    };
    let started = supervisor.begin_draining(reason, tokio::time::Instant::now());
    assert!(started.first_trigger, "one root shutdown trigger");
    debug_assert_eq!(supervisor.phase(), service::ServicePhase::Draining);
    let mut stages = [service::shutdown::StageOutcome::Completed; 4];
    terminal.budget = Some(started.budget);
    if let Some(voice) = &voice_runtime {
        voice.begin_draining();
    }
    if let Some(status) = &managed_status {
        status.draining();
        let _ = status.refresh();
    }
    if let Some(events) = state.operational_events() {
        let _ = events.record(
            observability::EventComponent::Shutdown,
            observability::EventCode::ShutdownStarted,
            observability::EventOutcome::Draining,
            None,
        );
    }
    if let Some(voice) = voice_runtime {
        let guild_id = voice.config.guild_id;
        let mut cleanup: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), ()>> + Send>,
        > = Box::pin(async move {
            service::shutdown::close_voice(
                voice.disconnect("process shutdown stopped voice"),
                async {
                    let manager = songbird_manager.ok_or(())?;
                    manager
                        .remove(std::num::NonZeroU64::new(guild_id).ok_or(())?)
                        .await
                        .map_err(|_| ())
                },
            )
            .await
        });
        match tokio::time::timeout_at(
            started.budget.stage(tokio::time::Instant::now()).deadline,
            &mut cleanup,
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(())) => stages[0] = service::shutdown::StageOutcome::Failed,
            Err(_) => {
                stages[0] = service::shutdown::StageOutcome::TimedOut;
                terminal.voice_cleanup = Some(cleanup);
            }
        }
    }
    let mut shards: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        Box::pin(async move {
            shard_manager.shutdown_all().await;
        });
    if tokio::time::timeout_at(
        started.budget.stage(tokio::time::Instant::now()).deadline,
        &mut shards,
    )
    .await
    .is_err()
    {
        stages[1] = service::shutdown::StageOutcome::TimedOut;
        terminal.shard_cleanup = Some(shards);
    }
    let reap = supervisor
        .cancel_and_reap(started.budget.stage(tokio::time::Instant::now()))
        .await;
    stages[2] = match reap.outcome {
        service::ReapOutcome::Joined => service::shutdown::StageOutcome::Completed,
        service::ReapOutcome::TimedOut => service::shutdown::StageOutcome::TimedOut,
    };
    writer.close_admission();
    let quiescent = terminal.voice_cleanup.is_none()
        && terminal.shard_cleanup.is_none()
        && reap.outstanding.is_empty()
        && supervisor
            .try_freeze(writer.idle() && provider_writer.idle())
            .is_ok();
    let final_stage = started.budget.stage(tokio::time::Instant::now());
    let outcome = if quiescent && tokio::time::Instant::now() < final_stage.deadline {
        let snapshot_state = state.clone();
        let snapshot_task =
            terminal
                .final_snapshot
                .insert(tokio::task::spawn_blocking(move || {
                    snapshot_state.final_snapshot()
                }));
        match tokio::time::timeout_at(final_stage.deadline, snapshot_task).await {
            Ok(Ok(snapshot)) if tokio::time::Instant::now() < final_stage.deadline => {
                terminal.final_snapshot = None;
                match writer.final_snapshot(snapshot, final_stage.deadline) {
                    Ok(result) => match tokio::time::timeout_at(final_stage.deadline, result).await
                    {
                        Ok(Ok(Ok(report))) => {
                            service::shutdown::FinalPersistOutcome::Completed(report)
                        }
                        Ok(Ok(Err(service::persistence::RequestError::DeadlineExpired))) => {
                            service::shutdown::FinalPersistOutcome::NotStarted(
                                service::shutdown::NotStartedReason::DeadlineExpired,
                            )
                        }
                        _ => service::shutdown::FinalPersistOutcome::Incomplete {
                            progress: writer.final_progress(),
                        },
                    },
                    Err(_) => service::shutdown::FinalPersistOutcome::NotStarted(
                        service::shutdown::NotStartedReason::WriterUnavailable,
                    ),
                }
            }
            _ => service::shutdown::FinalPersistOutcome::NotStarted(
                service::shutdown::NotStartedReason::SnapshotIncomplete,
            ),
        }
    } else {
        service::shutdown::FinalPersistOutcome::NotStarted(if quiescent {
            service::shutdown::NotStartedReason::DeadlineExpired
        } else {
            service::shutdown::NotStartedReason::NotQuiescent
        })
    };
    provider_writer.stop();
    let provider_joined = tokio::time::timeout_at(final_stage.deadline, provider_writer.joined())
        .await
        .is_ok_and(|r| r.is_ok());
    writer.stop();
    let writer_joined = tokio::time::timeout_at(final_stage.deadline, writer.joined())
        .await
        .is_ok_and(|r| r.is_ok());
    if let Some(events) = state.operational_events() {
        let _ = events.record(
            observability::EventComponent::Persistence,
            observability::EventCode::PersistenceAttempt,
            if outcome.successful_completion() {
                observability::EventOutcome::Succeeded
            } else {
                observability::EventOutcome::Failed
            },
            None,
        );
    }
    match &outcome {
        service::shutdown::FinalPersistOutcome::Completed(report) => {
            crate::persist::log_report("shutdown", report)
        }
        service::shutdown::FinalPersistOutcome::Incomplete { progress } => tracing::error!(
            canonical = ?progress.canonical_state, projection = ?progress.wdbx_projection,
            "final persistence incomplete"
        ),
        service::shutdown::FinalPersistOutcome::NotStarted(reason) => {
            tracing::error!(reason = ?reason, "final persistence not started")
        }
    }
    terminal.incomplete = !quiescent
        || !provider_joined
        || !writer_joined
        || !matches!(
            outcome,
            service::shutdown::FinalPersistOutcome::Completed(_)
        );
    if let Some(refresh) = &mut terminal.refresh {
        refresh.stop();
        let result = tokio::time::timeout_at(final_stage.deadline, refresh.joined()).await;
        terminal.refresh_joined = result.is_ok();
        terminal.incomplete |= !result.is_ok_and(|r| r.is_ok());
    }
    if let Some(telemetry) = &mut terminal.telemetry {
        if let service::shutdown::FinalPersistOutcome::Completed(report) = outcome
            && let Some(status) = &managed_status
        {
            status.persisted(report);
            let _ = status.refresh();
        }
        let _ = telemetry.requests().record(
            observability::EventComponent::Shutdown,
            observability::EventCode::ShutdownFinalizing,
            observability::EventOutcome::Started,
            None,
        );
        let removed = match telemetry.requests().remove_final() {
            Ok(receipt) => tokio::time::timeout_at(final_stage.deadline, receipt)
                .await
                .is_ok_and(|r| r.is_ok_and(|r| r.is_ok())),
            Err(_) => false,
        };
        telemetry.stop();
        let join_result = tokio::time::timeout_at(final_stage.deadline, telemetry.joined()).await;
        terminal.telemetry_joined = join_result.is_ok();
        terminal.incomplete |= !removed || !join_result.is_ok_and(|r| r.is_ok());
    }
    stages[3] = if tokio::time::Instant::now() >= final_stage.deadline {
        service::shutdown::StageOutcome::TimedOut
    } else if terminal.incomplete || !outcome.successful_completion() {
        service::shutdown::StageOutcome::Failed
    } else {
        service::shutdown::StageOutcome::Completed
    };
    let mut outstanding_resources = Vec::new();
    if !writer_joined {
        outstanding_resources.push(service::shutdown::ResourceCategory::SnapshotWriter);
    }
    if !provider_joined {
        outstanding_resources.push(service::shutdown::ResourceCategory::ProviderBlockWriter);
    }
    if terminal
        .telemetry
        .as_ref()
        .is_some_and(|writer| !writer.idle())
        || !terminal.telemetry_joined && terminal.telemetry.is_some()
    {
        outstanding_resources.push(service::shutdown::ResourceCategory::TelemetryWriter);
    }
    if terminal.refresh.is_some() && !terminal.refresh_joined {
        outstanding_resources.push(service::shutdown::ResourceCategory::ReadinessRefresh);
    }
    if terminal.voice_cleanup.is_some() {
        outstanding_resources.push(service::shutdown::ResourceCategory::VoiceCleanup);
    }
    if terminal.shard_cleanup.is_some() {
        outstanding_resources.push(service::shutdown::ResourceCategory::ShardCleanup);
    }
    if terminal
        .final_snapshot
        .as_ref()
        .is_some_and(|task| !task.is_finished())
    {
        outstanding_resources.push(service::shutdown::ResourceCategory::SnapshotPreparation);
    }
    let report = service::shutdown::ShutdownReport {
        reason: started.reason,
        stages,
        aborted: reap
            .joined
            .iter()
            .filter(|task| task.abort_requested)
            .map(|task| task.kind)
            .collect(),
        reaped: reap.joined.iter().map(|task| task.kind).collect(),
        outstanding: reap.outstanding.iter().map(|task| task.kind).collect(),
        outstanding_resources,
        total_duration: started.budget.elapsed(tokio::time::Instant::now()),
        final_persist: outcome,
    };
    report.log();
    let clean_shutdown = report.clean();
    terminal.report = Some(report);
    terminal.supervisor = Some(supervisor);
    terminal.writer = Some(writer);
    terminal.provider_writer = Some(provider_writer);
    if reason == service::ShutdownReason::Signal && clean_shutdown {
        Ok(())
    } else {
        Err(runtime::StartupError("service ended without a complete clean shutdown".into()).into())
    }
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
            .integration_types(command.integration_types);
        if let Some(contexts) = command.contexts {
            preserved = preserved.contexts(contexts);
        }
        if let Some(handler) = command.handler {
            preserved = preserved.handler(handler);
        }
        generated.push(preserved);
    }
    generated
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
    let existing = Command::get_global_commands(&ctx.http).await?;
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
mod startup_argument_tests {
    #[test]
    fn managed_panic_hook_child() {
        if std::env::var_os("ABBEY_TEST_MANAGED_PANIC").is_some() {
            super::configure_managed_panic_hook(None);
            panic!("MANAGED-PANIC-PRIVATE-CANARY");
        }
    }
    #[test]
    fn managed_panic_payload_cannot_reach_process_output() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "startup_argument_tests::managed_panic_hook_child",
                "--nocapture",
            ])
            .env_clear()
            .env("ABBEY_TEST_MANAGED_PANIC", "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        for bytes in [&output.stdout, &output.stderr] {
            assert!(!String::from_utf8_lossy(bytes).contains("MANAGED-PANIC-PRIVATE-CANARY"));
        }
    }
    #[test]
    fn managed_service_mode_requires_exactly_one_argument() {
        assert_eq!(
            super::parse_startup_arguments(
                ["--managed-service"]
                    .into_iter()
                    .map(std::ffi::OsString::from)
            )
            .unwrap(),
            super::StartupAction::ManagedDiscord
        );
        for tail in [
            "--managed-service",
            "--provider-self-test",
            "--voice-self-test",
            "private-canary",
        ] {
            assert!(
                super::parse_startup_arguments(
                    ["--managed-service", tail]
                        .into_iter()
                        .map(std::ffi::OsString::from)
                )
                .is_err()
            );
        }
    }

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
    fn server_plan_hands_its_arguments_to_the_engine_parser() {
        let action = parse(&[
            "--server-plan",
            "blueprints/mlai-community.toml",
            "--guild",
            "42",
        ])
        .unwrap();
        assert_eq!(
            action,
            StartupAction::ServerPlan(server::run::Options {
                plan: std::path::PathBuf::from("blueprints/mlai-community.toml"),
                guild_id: 42,
                stage: server::diff::Stage::Additive,
                category: None,
                apply: false,
            })
        );
        assert!(parse(&["--server-plan"]).is_err());
        assert!(
            parse(&["--server-plan", "p.toml"]).is_err(),
            "--guild is required"
        );
        assert!(parse(&["--server-plan", "p.toml", "--guild", "0"]).is_err());
    }

    #[test]
    fn an_unknown_or_mistyped_mode_cannot_start_discord() {
        assert!(parse(&["--voice-self-tset", "audition.wav"]).is_err());
        assert!(parse(&["unexpected"]).is_err());
    }

    #[test]
    fn provider_self_test_requires_exact_target_and_json_mode() {
        assert_eq!(
            provider_self_test_usage(),
            "usage: abbey-bot --provider-self-test primary|fm|all --json"
        );
        assert_eq!(provider_self_test::SelfTestExit::Success.code(), 0);
        assert_eq!(provider_self_test::SelfTestExit::ProbeFailure.code(), 1);
        assert_eq!(provider_self_test::SelfTestExit::Configuration.code(), 2);
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
