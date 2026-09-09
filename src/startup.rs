//! Root-owned application startup and shutdown orchestration.
use super::*;
pub(crate) mod command_errors;

pub(super) async fn run(
    startup: StartupAction,
    managed: Option<ManagedStartup>,
    terminal: &mut service::shutdown::TerminalBoundary,
) -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(if managed.is_some() {
            // Managed (launchd) runs emit no tracing at all: the closed
            // operational events in `observability` are the sole record, and
            // the agent routes stdout/stderr to /dev/null anyway. `RUST_LOG`
            // in deploy/com.donaldfilimon.abbey-bot.plist is therefore inert
            // for this path — raising it will not produce diagnostics, so a
            // managed failure must carry its cause as an
            // `OperationalErrorCategory` on the event instead.
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
    let voice_is_local = state
        .voice_registry
        .template()
        .is_some_and(|template| template.mode() == voice::VoiceMode::Local);
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
    state.voice_registry.attach_service(supervisor.operations());
    if let Some(events) = state.operational_events() {
        state.voice_registry.attach_telemetry(events.clone());
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
                        interaction: serenity::all::Interaction::Modal(interaction),
                    } = event
                        && commands_help::workflows::dispatch_modal(ctx, interaction, data).await
                    {
                        return Ok(());
                    }
                    if let serenity::all::FullEvent::InteractionCreate {
                        interaction: serenity::all::Interaction::Component(interaction),
                    } = event
                        && (commands_brain::dispatch_admin_component(ctx, interaction, data).await
                            || commands_voice::dispatch_ux_component(ctx, interaction, data).await
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
            on_error: |error| Box::pin(command_errors::handle(error)),
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                register_command_scopes(
                    guild_id,
                    register_globally_keeping_entry_point(ctx, &framework.options().commands),
                    |id| async move {
                        poise::builtins::register_in_guild(ctx, &framework.options().commands, id)
                            .await?;
                        Ok::<(), Error>(())
                    },
                )
                .await?;
                tracing::info!("registered global commands and optional home guild copy");
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
    let voice_runtimes = state.voice_registry.begin_draining();
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
    if !voice_runtimes.is_empty() {
        let mut cleanup: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), ()>> + Send>,
        > = Box::pin(async move {
            let cleanups = voice_runtimes.into_iter().map(|voice| {
                let manager = songbird_manager.clone();
                async move {
                    let guild_id = voice.config.guild_id;
                    service::shutdown::close_voice(
                        voice.disconnect("process shutdown stopped voice"),
                        async {
                            manager
                                .ok_or(())?
                                .remove(std::num::NonZeroU64::new(guild_id).ok_or(())?)
                                .await
                                .map_err(|_| ())
                        },
                    )
                    .await
                }
            });
            let results = futures_util::future::join_all(cleanups).await;
            if results.iter().all(Result::is_ok) {
                Ok(())
            } else {
                Err(())
            }
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
