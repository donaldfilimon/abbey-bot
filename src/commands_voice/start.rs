//! Muted voice startup, consent disclosure and epoch-bound activation.
use super::*;

pub(super) async fn start_voice(
    ctx: AcknowledgedContext<Context<'_>>,
    consent: bool,
    resumed: bool,
) -> Result<(), Error> {
    if !consent {
        ctx.say("Voice stayed off. Set `consent:true` only after everyone currently in the configured channel was notified and agreed.")
            .await?;
        return Ok(());
    }
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This command only works inside a server.").await?;
        return Ok(());
    };
    let Some(channel_id) = ctx.guild().and_then(|guild| {
        guild
            .voice_states
            .get(&ctx.author().id)
            .and_then(|voice| voice.channel_id)
    }) else {
        ctx.say("Join a voice channel yourself before starting Abbey voice; remote activation is not allowed.")
            .await?;
        return Ok(());
    };
    let existing = ctx.data().voice_for(guild_id.get());
    if existing
        .as_ref()
        .is_some_and(|runtime| runtime.config.channel_id != channel_id.get())
    {
        ctx.say("Abbey already has a voice session prepared in another channel in this server. Use /voice leave before choosing another channel.")
            .await?;
        return Ok(());
    }
    // Capture the bound runtime's cancellation generation before any REST
    // work. A registry reservation also protects a first join with no runtime.
    let previous_operation = existing
        .as_ref()
        .map(|runtime| runtime.start_operation_token());
    let reservation = match ctx.data().reserve_voice_join(guild_id.get()) {
        Ok(reservation) => reservation,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };
    let channel = channel_id.to_channel(ctx.http()).await?;
    let Some(channel) = channel.guild() else {
        ctx.say("The voice destination is not a server channel.")
            .await?;
        return Ok(());
    };
    if channel.guild_id != guild_id || channel.kind != ChannelType::Voice {
        ctx.say("The destination must be a voice channel in this server; Stage channels are not supported.")
            .await?;
        return Ok(());
    }
    let permissions = crate::commands_help::current_permissions(
        ctx.serenity_context(),
        guild_id,
        channel_id,
        ctx.author().id,
    )
    .await?;
    if !permissions.intersects(
        serenity::all::Permissions::MANAGE_GUILD | serenity::all::Permissions::ADMINISTRATOR,
    ) {
        ctx.say("Starting Abbey voice requires current Manage Server permission.")
            .await?;
        return Ok(());
    }
    if let Err(message) =
        verify_required_voice_permissions_live(ctx.serenity_context(), guild_id, channel_id).await
    {
        ctx.say(message).await?;
        return Ok(());
    }
    if !cached_participants(*ctx, guild_id, channel_id).is_ok_and(|(present, _)| present) {
        ctx.say(
            "Your voice channel changed while Discord validated the request; voice stayed off.",
        )
        .await?;
        return Ok(());
    }
    // Provision only after current caller, channel and bot authorization. The
    // new runtime stays media-closed while members save their consent choices.
    let runtime = match ctx
        .data()
        .voice_for_join(guild_id.get(), channel_id.get(), &reservation)
        .await
    {
        Ok(runtime) => runtime,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };
    let Some(start_operation) = previous_operation.or_else(|| reservation.operation_token()) else {
        ctx.say("This voice start was cancelled before its session was prepared; no audio was captured.").await?;
        return Ok(());
    };
    let (caller_present, participants) = match cached_participants(*ctx, guild_id, channel_id) {
        Ok(snapshot) => snapshot,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };
    if !caller_present {
        ctx.say("Your voice channel changed while the session was prepared; voice stayed off.")
            .await?;
        return Ok(());
    }

    // Invalid callers/channels must not cancel or advertise a slow start. At
    // the same time, an authorized stop that crossed either Discord await must
    // prevent this older request from publishing a fresh reservation.
    // Decide this join's backend exactly once, in the same critical section
    // as the start reservation. Everything downstream — the Songbird decode
    // mode, the consent disclosure, the actor that connects, and the
    // confirmation reply — reads this snapshot rather than the shared
    // runtime, so a concurrent `/voice mode` cannot leave participants told
    // "local, stays on this Mac" while a cloud actor connects; and because
    // the switch refuses under the same lock while this reservation is
    // pending, it cannot report a backend this join is not using either.
    let Some((start_generation, effective_backend)) =
        runtime.reserve_start_with_backend(start_operation)
    else {
        ctx.say("This voice start was cancelled while Discord validated the channel; no audio was captured.")
            .await?;
        return Ok(());
    };
    let _start_attempt = StartAttempt {
        runtime: Arc::clone(&runtime),
        generation: start_generation,
    };
    let Some(effective_backend) = effective_backend else {
        ctx.say("The voice backend selected for this server is not configured; voice stayed off.")
            .await?;
        return Ok(());
    };
    let effective_mode = effective_backend.mode();

    if !runtime
        .consent
        .coverage(&participants, effective_mode)
        .is_ok_and(|missing| missing.is_empty())
    {
        ctx.say(clamp_message(consent::coverage_text(
            &runtime,
            &participants,
            effective_mode,
        )))
        .await?;
        return Ok(());
    }

    let local_runtime = match effective_mode {
        VoiceMode::Local => {
            // Fail closed on the loopback LLM *before* the 10-minute sidecar
            // prepare. A missing ABBEY_BOT_LLM_ENDPOINT must not look like a
            // hung MLX-Audio install.
            let backend = match select_local_backend(&ctx.data().state) {
                Ok(backend) => backend,
                Err(error) => {
                    ctx.say(error).await?;
                    return Ok(());
                }
            };
            let VoiceBackendConfig::Local(config) = &effective_backend else {
                ctx.say("Local speech configuration is incomplete.").await?;
                return Ok(());
            };
            let config = config.clone();
            let client = match MlxAudioClient::new(config) {
                Ok(client) => client,
                Err(error) => {
                    ctx.say(format!(
                        "Local speech is not ready: {}",
                        public_error(&error)
                    ))
                    .await?;
                    return Ok(());
                }
            };
            let prepared = {
                let prepare = tokio::time::timeout(LOCAL_HEALTH_TIMEOUT, client.prepare());
                tokio::pin!(prepare);
                tokio::select! {
                    biased;
                    () = runtime.wait_for_start_cancellation(start_generation) => None,
                    result = &mut prepare => Some(result),
                }
            };
            let Some(prepared) = prepared else {
                ctx.say("This voice start was cancelled while local models were preparing; no audio was captured.")
                    .await?;
                return Ok(());
            };
            match prepared {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    ctx.say(format!(
                        "Local speech is not ready: {}",
                        public_error(&error)
                    ))
                    .await?;
                    return Ok(());
                }
                Err(_) => {
                    ctx.say("Local speech models did not become ready within ten minutes. If 127.0.0.1:8181 is down, run deploy/install-mlx-audio-launchd.sh and retry /voice status; no audio was captured.")
                        .await?;
                    return Ok(());
                }
            }
            Some((client, backend))
        }
        _ => None,
    };

    if !runtime.start_is_current(start_generation) {
        ctx.say("This voice start was cancelled before model preflight finished; no audio was captured.")
            .await?;
        return Ok(());
    }
    let transition = runtime.transition.lock().await;
    if !runtime.start_is_current(start_generation) {
        drop(transition);
        ctx.say("This voice start was superseded or cancelled; no audio was captured.")
            .await?;
        return Ok(());
    }
    if let Err(error) =
        verify_required_voice_permissions_live(ctx.serenity_context(), guild_id, channel_id).await
    {
        drop(transition);
        ctx.say(error).await?;
        return Ok(());
    }
    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("Songbird was not registered in the Discord client")?;
    runtime
        .disconnect_for_replace("replacing any previous voice session")
        .await;
    let old_voice_session = cached_bot_voice_state_from_serenity(ctx.serenity_context(), guild_id)
        .map(|state| state.session_id);
    if let Some(old_session_id) = old_voice_session.as_ref() {
        runtime.remember_retired_discord_session(old_session_id.clone());
    }
    if let Some(existing) = manager.get(guild_id) {
        // Stop the old call at Discord as well as stopping its actor. If
        // removal fails, it must not leave an unobserved live connection able
        // to receive or transmit audio.
        let _ = set_muted_self_deafened(&existing).await;
        if let Err(error) = manager.remove(guild_id).await {
            drop(transition);
            ctx.say(format!(
                "Could not replace the existing voice session cleanly: {error}"
            ))
            .await?;
            return Ok(());
        }
    }
    if let Some(old_session_id) = old_voice_session.as_deref()
        && let Err(error) =
            wait_for_voice_session_gone(ctx.serenity_context(), guild_id, old_session_id).await
    {
        runtime
            .fail_safe(
                "old Discord voice session did not finish leaving",
                crate::observability::OperationalErrorCategory::Timeout,
            )
            .await;
        drop(transition);
        ctx.say(error).await?;
        return Ok(());
    }
    if !runtime.start_is_current(start_generation) {
        drop(transition);
        ctx.say(
            "This voice start was cancelled while the previous call left; no audio was captured.",
        )
        .await?;
        return Ok(());
    }

    // Songbird 0.6 can reconfigure an existing decoder, but its UDP receiver
    // cannot change from DecodeMode::Pass to DecodeMode::Decode after it has
    // started. Construct conversational calls in Decode mode from the outset,
    // while setting self-mute and self-deafen before the gateway join so no
    // participant audio is delivered before consent and the public notice.
    let prepared_call = manager.get_or_insert(guild_id);
    configure_disconnected_call(&prepared_call, effective_mode).await;
    if let Err(error) = set_muted_self_deafened(&prepared_call).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "could not prepare the required muted/self-deafened state",
                crate::observability::OperationalErrorCategory::Protocol,
            )
            .await;
        drop(transition);
        ctx.say(format!(
            "Could not prepare the required voice safety state: {error}"
        ))
        .await?;
        return Ok(());
    }
    let epoch = runtime.begin(participants.clone()).await;
    // Core events are not replayed by Songbird. Install the shared mapping,
    // liveness, and VoiceTick handlers before joining so an already-speaking
    // participant and an early driver disconnect cannot be missed. The media
    // epoch is still closed: VoiceTick returns before reading or forwarding a
    // decoded frame until disclosure and all final checks succeed.
    let (frames_tx, input) = mpsc::channel(INPUT_QUEUE_FRAMES);
    let (driver_disconnect_tx, driver_disconnect) = watch::channel(false);
    install_receive_handlers(ReceiveHandlerInstall {
        call: &prepared_call,
        manager: Arc::downgrade(&manager),
        guild_id,
        runtime: &runtime,
        epoch,
        attested: participants.clone(),
        tx: frames_tx,
        driver_disconnect: driver_disconnect_tx,
    })
    .await;
    let call = match manager.join(guild_id, channel_id).await {
        Ok(call) => call,
        Err(error) => {
            let _ = manager.remove(guild_id).await;
            runtime
                .fail_safe(
                    "Discord refused the configured voice join",
                    crate::observability::OperationalErrorCategory::Protocol,
                )
                .await;
            drop(transition);
            ctx.say(format!("Discord refused the voice join: {error}"))
                .await?;
            return Ok(());
        }
    };
    if let Err(error) = set_muted_self_deafened(&call).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "could not establish the required muted/self-deafened state",
                crate::observability::OperationalErrorCategory::Protocol,
            )
            .await;
        drop(transition);
        ctx.say(format!(
            "Joined Discord but could not establish the required safety state: {error}"
        ))
        .await?;
        return Ok(());
    }
    let joined_session_id =
        match wait_for_bot_voice_state(ctx.serenity_context(), guild_id, channel_id).await {
            Ok(session_id) => session_id,
            Err(error) => {
                let _ = manager.remove(guild_id).await;
                runtime
                    .fail_safe(
                        "Discord did not confirm a speak-capable bot voice state",
                        crate::observability::OperationalErrorCategory::Timeout,
                    )
                    .await;
                drop(transition);
                ctx.say(format!("Voice stayed off: {error}")).await?;
                return Ok(());
            }
        };
    if !runtime.start_is_current(start_generation)
        || !runtime
            .bind_discord_session(epoch, joined_session_id.clone())
            .await
    {
        let _ = manager.remove(guild_id).await;
        runtime
            .disconnect_for_replace("voice start was cancelled before Discord session binding")
            .await;
        drop(transition);
        ctx.say("This voice start was superseded or cancelled before Discord confirmed its session; no audio was captured.")
            .await?;
        return Ok(());
    }

    if effective_mode == VoiceMode::Disabled {
        runtime
            .set_presence_with_discord_session(
                joined_session_id,
                "connected muted/self-deafened; ABBEY_VOICE_MODE=disabled",
            )
            .await;
        drop(transition);
        ctx.say(format!(
            "Joined <#{channel_id}> in no-audio presence mode. Abbey cannot receive or transmit call audio; `/voice leave` disconnects her."
        ))
        .await?;
        return Ok(());
    }

    // The provider actor does not exist yet, and the closed software media gate
    // makes VoiceTick return without inspecting participant samples. Only
    // SSRC mapping and transport-liveness metadata are tracked before this
    // required public disclosure succeeds.
    let notice = consent_notice(effective_mode, channel_id, resumed);
    if let Err(error) = channel_id.say(ctx.http(), notice).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "public consent disclosure could not be posted",
                crate::observability::OperationalErrorCategory::Protocol,
            )
            .await;
        drop(transition);
        ctx.say(format!(
            "Could not post the required public consent notice, so voice stayed off: {error}"
        ))
        .await?;
        return Ok(());
    }
    let pre_enable_participants = match cached_participants(*ctx, guild_id, channel_id) {
        Ok((_, participants)) => participants,
        Err(error) => {
            remove_call_for_consent(&manager, guild_id).await;
            runtime.pause_for_consent(participants.clone()).await;
            drop(transition);
            ctx.say(format!(
                "Voice disconnected because the participant list could not be verified: {error}"
            ))
            .await?;
            return Ok(());
        }
    };
    if pre_enable_participants != participants {
        remove_call_for_consent(&manager, guild_id).await;
        runtime.pause_for_consent(pre_enable_participants).await;
        drop(transition);
        channel_id
            .say(
                ctx.http(),
                "Abbey disconnected because channel membership changed during startup. Notify everyone now present, then use `/voice resume consent:true`.",
            )
            .await?;
        ctx.say(
            "Channel membership changed before activation, so no participant audio was processed.",
        )
        .await?;
        return Ok(());
    }

    let (events, lifecycle) = mpsc::unbounded_channel();
    let driver_disconnected = *driver_disconnect.borrow();
    if driver_disconnected {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "Discord voice transport disconnected during startup",
                crate::observability::OperationalErrorCategory::Unavailable,
            )
            .await;
        drop(transition);
        ctx.say("Discord voice transport disconnected during startup; no audio was captured.")
            .await?;
        return Ok(());
    }
    let playback: SharedPlayback = Arc::new(Mutex::new(None));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let mut cloud_ready = None;
    let task = match (&effective_backend, local_runtime) {
        (VoiceBackendConfig::Local(_), Some((client, backend))) => {
            let session = LocalSession {
                runtime: Arc::clone(&runtime),
                state: Arc::clone(&ctx.data().state),
                call: Arc::clone(&call),
                client,
                epoch,
                input,
                lifecycle,
                events: events.clone(),
                driver_disconnect,
                cancel: cancel_rx,
                playback: Arc::clone(&playback),
                backend,
            };
            runtime.spawn_actor(cancel_tx.clone(), crate::voice_local::run(session))
        }
        (VoiceBackendConfig::OpenAi(config), None) => {
            let (ready_tx, ready_rx) = oneshot::channel();
            cloud_ready = Some(ready_rx);
            let session = OpenAiSession {
                runtime: Arc::clone(&runtime),
                config: config.clone(),
                call: Arc::clone(&call),
                epoch,
                input,
                lifecycle,
                events: events.clone(),
                driver_disconnect,
                cancel: cancel_rx,
                playback: Arc::clone(&playback),
                ready: Some(ready_tx),
            };
            runtime.spawn_actor(cancel_tx.clone(), crate::voice_openai::run(session))
        }
        _ => {
            let _ = manager.remove(guild_id).await;
            runtime
                .fail_safe(
                    "selected voice backend was unavailable",
                    crate::observability::OperationalErrorCategory::Configuration,
                )
                .await;
            drop(transition);
            ctx.say("The selected voice backend was unavailable; no audio was captured.")
                .await?;
            return Ok(());
        }
    };
    let task = match task {
        Ok(task) => task,
        Err(message) => {
            let _ = manager.remove(guild_id).await;
            runtime
                .fail_safe(
                    "service stopped during voice startup",
                    crate::observability::OperationalErrorCategory::Internal,
                )
                .await;
            drop(transition);
            ctx.say(message).await?;
            return Ok(());
        }
    };
    if !runtime
        .install_control(
            epoch,
            SessionControl {
                cancel: cancel_tx,
                task,
                playback,
            },
        )
        .await
    {
        let _ = manager.remove(guild_id).await;
        drop(transition);
        ctx.say("The voice session changed while it was starting; no audio was captured.")
            .await?;
        return Ok(());
    }

    if let Some(ready) = cloud_ready {
        match tokio::time::timeout(OPENAI_READY_TIMEOUT, ready).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                let _ = manager.remove(guild_id).await;
                runtime
                    .fail_safe(
                        "OpenAI Realtime setup was rejected",
                        crate::observability::OperationalErrorCategory::Protocol,
                    )
                    .await;
                drop(transition);
                ctx.say(format!(
                    "OpenAI Realtime did not start: {}",
                    public_error(&error)
                ))
                .await?;
                return Ok(());
            }
            Ok(Err(_)) | Err(_) => {
                let _ = manager.remove(guild_id).await;
                runtime
                    .fail_safe(
                        "OpenAI Realtime readiness timed out",
                        crate::observability::OperationalErrorCategory::Timeout,
                    )
                    .await;
                drop(transition);
                ctx.say("OpenAI Realtime did not become ready within 20 seconds; no participant audio was captured.")
                    .await?;
                return Ok(());
            }
        }
    }

    if !runtime.start_is_current(start_generation) || !runtime.is_current(epoch) {
        let _ = manager.remove(guild_id).await;
        runtime
            .disconnect_for_replace("voice start was superseded before activation")
            .await;
        drop(transition);
        ctx.say("This voice start was superseded or cancelled; no audio was captured.")
            .await?;
        return Ok(());
    }

    let bot_is_exact = call.lock().await.current_channel() == Some(channel_id.into());
    if !bot_is_exact {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "Discord moved or disconnected Abbey during startup",
                crate::observability::OperationalErrorCategory::Unavailable,
            )
            .await;
        drop(transition);
        ctx.say("Discord moved or disconnected Abbey during startup; no audio was captured.")
            .await?;
        return Ok(());
    }

    let latest_participants = match cached_participants(*ctx, guild_id, channel_id) {
        Ok((_, participants)) => participants,
        Err(error) => {
            remove_call_for_consent(&manager, guild_id).await;
            runtime.pause_for_consent(participants.clone()).await;
            drop(transition);
            ctx.say(format!(
                "Abbey stayed paused because the participant list could not be verified: {error}"
            ))
            .await?;
            return Ok(());
        }
    };
    if latest_participants != participants {
        remove_call_for_consent(&manager, guild_id).await;
        runtime.pause_for_consent(latest_participants).await;
        drop(transition);
        let _ = channel_id
            .say(
                ctx.http(),
                "Abbey disconnected because channel membership changed during startup. Notify everyone now present, then use `/voice resume consent:true`.",
            )
            .await;
        ctx.say(
            "Channel membership changed before activation, so no participant audio was processed.",
        )
        .await?;
        return Ok(());
    }

    if let Err(error) =
        verify_required_voice_permissions_live(ctx.serenity_context(), guild_id, channel_id).await
    {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "required Discord voice permissions could not be verified before activation",
                crate::observability::OperationalErrorCategory::Authorization,
            )
            .await;
        drop(transition);
        ctx.say(error).await?;
        return Ok(());
    }

    if let Err(error) = enable_conversation(&call).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "could not leave the muted/self-deafened startup state",
                crate::observability::OperationalErrorCategory::Protocol,
            )
            .await;
        drop(transition);
        ctx.say(format!(
            "The public notice was posted, but Discord could not enable the consented session: {error}"
        ))
        .await?;
        return Ok(());
    }

    if let Err(error) =
        wait_for_enabled_bot_voice_state(*ctx, guild_id, channel_id, joined_session_id.as_str())
            .await
    {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "Discord did not confirm Abbey was unmuted and undeafened",
                crate::observability::OperationalErrorCategory::Timeout,
            )
            .await;
        drop(transition);
        ctx.say(format!("Voice stayed off: {error}")).await?;
        return Ok(());
    }

    if let Err(error) =
        verify_required_voice_permissions_live(ctx.serenity_context(), guild_id, channel_id).await
    {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "required Discord voice permissions changed during activation",
                crate::observability::OperationalErrorCategory::Authorization,
            )
            .await;
        drop(transition);
        ctx.say(error).await?;
        return Ok(());
    }

    let post_enable_participants = match cached_participants(*ctx, guild_id, channel_id) {
        Ok((_, participants)) => participants,
        Err(error) => {
            remove_call_for_consent(&manager, guild_id).await;
            runtime.pause_for_consent(participants.clone()).await;
            drop(transition);
            ctx.say(format!(
                "Abbey paused because the participant list could not be verified after activation: {error}"
            ))
            .await?;
            return Ok(());
        }
    };
    let bot_voice_state_ok = cached_bot_voice_state(*ctx, guild_id).is_some_and(|state| {
        bot_voice_state_allows_conversation(&state, channel_id, joined_session_id.as_str())
    });
    if post_enable_participants != participants
        || !runtime.start_is_current(start_generation)
        || !runtime.is_current(epoch)
        || call.lock().await.current_channel() != Some(channel_id.into())
        || !bot_voice_state_ok
    {
        remove_call_for_consent(&manager, guild_id).await;
        runtime.pause_for_consent(post_enable_participants).await;
        drop(transition);
        channel_id
            .say(
                ctx.http(),
                "Abbey disconnected immediately because channel membership changed during startup. Notify everyone now present, then use `/voice resume consent:true`.",
            )
            .await?;
        ctx.say(
            "Channel membership changed during startup, so Abbey paused before processing audio.",
        )
        .await?;
        return Ok(());
    }

    if !runtime
        .activate_verified(
            epoch,
            start_generation,
            match effective_mode {
                VoiceMode::Local => "local inference ready; listening for Abbey",
                VoiceMode::OpenAi => "direct OpenAI backup ready; buffered output; listening",
                VoiceMode::Disabled => unreachable!(),
            },
            VerificationActivation {
                manager_authorized: true,
                caller_present: true,
                participant_count: participants.len(),
                resumed,
            },
        )
        .await
    {
        remove_call_for_consent(&manager, guild_id).await;
        if runtime.is_current(epoch) {
            runtime.pause_for_consent(participants).await;
        }
        drop(transition);
        ctx.say("The voice session changed at activation, so no participant audio was processed.")
            .await?;
        return Ok(());
    }
    drop(transition);
    ctx.say(format!(
        "{} <#{channel_id}> with {}. The public consent notice is posted; `/voice status` shows health and `/voice leave` stops processing.",
        if resumed { "Resumed" } else { "Joined" },
        effective_mode.label(),
    ))
    .await?;
    // Phase A classic Action Row — never starts STT/consent from buttons.
    if let Err(error) = super::ux::send_post_join_panel(*ctx, &runtime, channel_id, resumed).await {
        tracing::warn!(%error, "voice UX status panel follow-up failed");
    }
    Ok(())
}
