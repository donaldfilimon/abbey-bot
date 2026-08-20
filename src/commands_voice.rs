//! Thin Discord/Songbird shell for Abbey voice.
//!
//! Commands validate runtime permission, exact-channel membership, explicit
//! participant attestation, and provider readiness while the call is muted and
//! self-deafened. Only after a public disclosure succeeds do they enable
//! decoding. The provider actors live in `voice_local` and `voice_openai`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use serenity::all::{ChannelId, ChannelType, GuildChannel, GuildId, Permissions, VoiceState};
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use tokio::sync::{Mutex, mpsc, oneshot, watch};

use crate::offline_voice::{FRAME_SAMPLES, MlxAudioClient, VoiceFrame, frame_is_voice};
use crate::voice::{VoiceBackendConfig, VoiceMode};
use crate::voice_local::LocalSession;
use crate::voice_openai::OpenAiSession;
use crate::voice_session::{SessionControl, SharedPlayback, VoicePhase, VoiceRuntime};
use crate::{Context, Error};

const INPUT_QUEUE_FRAMES: usize = 64;
const OPENAI_READY_TIMEOUT: Duration = Duration::from_secs(20);
const LOCAL_HEALTH_TIMEOUT: Duration = Duration::from_secs(600);

/// Clear this exact slow-start reservation on every return path. A newer
/// request is unaffected because `finish_start_attempt` compares generations.
struct StartAttempt {
    runtime: Arc<VoiceRuntime>,
    generation: u64,
}

impl Drop for StartAttempt {
    fn drop(&mut self) {
        self.runtime.finish_start_attempt(self.generation);
    }
}

/// `/voice join`, `/voice resume`, `/voice leave`, and `/voice status`.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    subcommands("voice_join", "voice_resume", "voice_leave", "voice_status")
)]
pub async fn voice(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Start the configured voice backend after everyone present was notified.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "join"
)]
pub async fn voice_join(
    ctx: Context<'_>,
    #[description = "Confirm everyone present was notified and consented"] consent: bool,
) -> Result<(), Error> {
    start_voice(ctx, consent, false).await
}

/// Resume after a new participant was notified and consented.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "resume"
)]
pub async fn voice_resume(
    ctx: Context<'_>,
    #[description = "Confirm everyone now present was notified and consented"] consent: bool,
) -> Result<(), Error> {
    start_voice(ctx, consent, true).await
}

async fn start_voice(ctx: Context<'_>, consent: bool, resumed: bool) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if !consent {
        ctx.say("Voice stayed off. Set `consent:true` only after everyone currently in the configured channel was notified and agreed.")
            .await?;
        return Ok(());
    }
    let Some(runtime) = ctx.data().voice.as_ref().cloned() else {
        ctx.say("Abbey voice is not configured. Set both destination IDs and ABBEY_VOICE_MODE, then restart Abbey.")
            .await?;
        return Ok(());
    };
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This command only works inside a server.").await?;
        return Ok(());
    };
    if guild_id.get() != runtime.config.guild_id {
        ctx.say("Abbey voice is locked to a different server by deployment configuration.")
            .await?;
        return Ok(());
    }
    let start_generation = runtime.reserve_start();
    let _start_attempt = StartAttempt {
        runtime: Arc::clone(&runtime),
        generation: start_generation,
    };
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let (caller_present, participants) = match cached_participants(ctx, guild_id, channel_id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            ctx.say(error).await?;
            return Ok(());
        }
    };
    if !caller_present {
        ctx.say(format!(
            "Join <#{channel_id}> yourself before starting Abbey voice; remote activation is not allowed."
        ))
        .await?;
        return Ok(());
    }

    let channel = channel_id.to_channel(ctx.http()).await?;
    let Some(channel) = channel.guild() else {
        ctx.say("The configured voice destination is not a server channel.")
            .await?;
        return Ok(());
    };
    if channel.guild_id != guild_id || channel.kind != ChannelType::Voice {
        ctx.say("The configured destination must be a voice channel in this server; Stage channels are not supported.")
            .await?;
        return Ok(());
    }
    if !bot_has_required_voice_permissions(ctx.serenity_context(), &channel) {
        ctx.say("Abbey needs View Channel, Send Messages, Connect, and Speak in the configured voice channel; voice stayed off.")
            .await?;
        return Ok(());
    }

    let local_runtime = match runtime.config.mode() {
        VoiceMode::Local => {
            let Some(config) = runtime.config.local().cloned() else {
                ctx.say("Local speech configuration is incomplete.").await?;
                return Ok(());
            };
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
            match tokio::time::timeout(LOCAL_HEALTH_TIMEOUT, client.prepare()).await {
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
                    ctx.say("Local speech models did not become ready within ten minutes; no audio was captured.")
                        .await?;
                    return Ok(());
                }
            }
            match select_local_backend(&ctx.data().state) {
                Ok(backend) => Some((client, backend)),
                Err(error) => {
                    ctx.say(error).await?;
                    return Ok(());
                }
            }
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
            .fail_safe("old Discord voice session did not finish leaving")
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
    manager.set_config(initial_songbird_config(runtime.config.mode()));
    let prepared_call = manager.get_or_insert(guild_id);
    if let Err(error) = set_muted_self_deafened(&prepared_call).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe("could not prepare the required muted/self-deafened state")
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
                .fail_safe("Discord refused the configured voice join")
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
            .fail_safe("could not establish the required muted/self-deafened state")
            .await;
        drop(transition);
        ctx.say(format!(
            "Joined Discord but could not establish the required safety state: {error}"
        ))
        .await?;
        return Ok(());
    }
    let joined_session_id = match wait_for_bot_voice_state(ctx, guild_id, channel_id).await {
        Ok(session_id) => session_id,
        Err(error) => {
            let _ = manager.remove(guild_id).await;
            runtime
                .fail_safe("Discord did not confirm a speak-capable bot voice state")
                .await;
            drop(transition);
            ctx.say(format!("Voice stayed off: {error}")).await?;
            return Ok(());
        }
    };

    if runtime.config.mode() == VoiceMode::Disabled {
        runtime
            .set_presence("connected muted/self-deafened; ABBEY_VOICE_MODE=disabled")
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
    let notice = consent_notice(&runtime, channel_id, resumed);
    if let Err(error) = channel_id.say(ctx.http(), notice).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe("public consent disclosure could not be posted")
            .await;
        drop(transition);
        ctx.say(format!(
            "Could not post the required public consent notice, so voice stayed off: {error}"
        ))
        .await?;
        return Ok(());
    }
    let pre_enable_participants = match cached_participants(ctx, guild_id, channel_id) {
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
            .fail_safe("Discord voice transport disconnected during startup")
            .await;
        drop(transition);
        ctx.say("Discord voice transport disconnected during startup; no audio was captured.")
            .await?;
        return Ok(());
    }
    let playback: SharedPlayback = Arc::new(Mutex::new(None));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let mut cloud_ready = None;
    let task = match (&runtime.config.backend, local_runtime) {
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
            tokio::spawn(crate::voice_local::run(session))
        }
        (VoiceBackendConfig::OpenAi(_), None) => {
            let (ready_tx, ready_rx) = oneshot::channel();
            cloud_ready = Some(ready_rx);
            let session = OpenAiSession {
                runtime: Arc::clone(&runtime),
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
            tokio::spawn(crate::voice_openai::run(session))
        }
        _ => {
            let _ = manager.remove(guild_id).await;
            runtime
                .fail_safe("selected voice backend was unavailable")
                .await;
            drop(transition);
            ctx.say("The selected voice backend was unavailable; no audio was captured.")
                .await?;
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
                    .fail_safe("OpenAI Realtime setup was rejected")
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
                    .fail_safe("OpenAI Realtime readiness timed out")
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
            .fail_safe("Discord moved or disconnected Abbey during startup")
            .await;
        drop(transition);
        ctx.say("Discord moved or disconnected Abbey during startup; no audio was captured.")
            .await?;
        return Ok(());
    }

    let latest_participants = match cached_participants(ctx, guild_id, channel_id) {
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
            .fail_safe("required Discord voice permissions could not be verified before activation")
            .await;
        drop(transition);
        ctx.say(error).await?;
        return Ok(());
    }

    if let Err(error) = enable_conversation(&call).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe("could not leave the muted/self-deafened startup state")
            .await;
        drop(transition);
        ctx.say(format!(
            "The public notice was posted, but Discord could not enable the consented session: {error}"
        ))
        .await?;
        return Ok(());
    }

    if let Err(error) =
        wait_for_enabled_bot_voice_state(ctx, guild_id, channel_id, joined_session_id.as_str())
            .await
    {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe("Discord did not confirm Abbey was unmuted and undeafened")
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
            .fail_safe("required Discord voice permissions changed during activation")
            .await;
        drop(transition);
        ctx.say(error).await?;
        return Ok(());
    }

    let post_enable_participants = match cached_participants(ctx, guild_id, channel_id) {
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
    let bot_voice_state_ok = cached_bot_voice_state(ctx, guild_id).is_some_and(|state| {
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
        .activate(
            epoch,
            start_generation,
            match runtime.config.mode() {
                VoiceMode::Local => "local inference ready; listening for Abbey",
                VoiceMode::OpenAi => "direct OpenAI backup ready; buffered output; listening",
                VoiceMode::Disabled => unreachable!(),
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
        runtime.config.mode().label(),
    ))
    .await?;
    Ok(())
}

/// Stop processing synchronously and leave Discord voice.
#[poise::command(slash_command, guild_only, ephemeral, rename = "leave")]
pub async fn voice_leave(ctx: Context<'_>) -> Result<(), Error> {
    let Some(runtime) = ctx.data().voice.as_ref().cloned() else {
        ctx.say("Abbey voice is not configured.").await?;
        return Ok(());
    };
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This command only works inside a server.").await?;
        return Ok(());
    };
    if guild_id.get() != runtime.config.guild_id {
        ctx.say("Abbey voice is locked to a different server by deployment configuration.")
            .await?;
        return Ok(());
    }
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let caller = ctx.author().id;
    let present = ctx.guild().is_some_and(|guild| {
        guild
            .voice_states
            .get(&caller)
            .and_then(|state| state.channel_id)
            == Some(channel_id)
    });
    // Slash-command interactions carry the caller's computed permissions in
    // their Member payload even when the guild member cache is incomplete.
    // Using guild.members here falsely denied out-of-channel managers.
    let interaction_permissions = if present {
        None
    } else {
        ctx.author_member()
            .await
            .and_then(|member| member.permissions)
    };
    if !can_stop_voice(present, interaction_permissions) {
        ctx.say("Only someone currently in the configured voice channel or a member with Manage Server can stop Abbey voice.")
            .await?;
        return Ok(());
    }
    // An authorized stop closes the software media gate before any further
    // await, including Songbird lookup and Discord acknowledgement.
    runtime.cancel_pending_start();
    let manager = match songbird::get(ctx.serenity_context()).await {
        Some(manager) => manager,
        None => {
            runtime
                .disconnect("voice stopped; Songbird runtime was unavailable")
                .await;
            return Err("Songbird was not registered in the Discord client".into());
        }
    };
    let exact_call = manager.get(guild_id);
    // The slow-start token and media gate were cancelled above, before lookup.
    // Enqueue the transition lock immediately so any later `/voice join`
    // waits behind this stop, while leaving the exact current Decode Call in
    // parallel instead of waiting behind an older start's network work. The
    // Discord acknowledgement is polled concurrently and cannot delay the
    // physical leave.
    let leave_exact = async {
        if let Some(call) = exact_call {
            pause_call_for_consent(&call).await;
        }
    };
    let (transition, (), _) = tokio::join!(
        runtime.transition.lock(),
        leave_exact,
        ctx.defer_ephemeral()
    );
    if let Some(call) = manager.get(guild_id) {
        // The software gate was closed before `transition`; stop the Decode
        // driver of any call that replaced the captured handle before the
        // transition became ours.
        pause_call_for_consent(&call).await;
    }
    runtime
        .disconnect("configured; disconnected by /voice leave")
        .await;
    let removed = manager.remove(guild_id).await;
    drop(transition);
    match removed {
        Ok(()) | Err(songbird::error::JoinError::NoCall) => {
            ctx.say("Left voice. Capture, provider work, queued audio, and playback are stopped.")
                .await?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Show mode, consent epoch, phase, models, and bounded-queue counters.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "status"
)]
pub async fn voice_status(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx.data().voice.as_ref() else {
        ctx.say("Abbey voice is off because no complete destination was configured at startup.")
            .await?;
        return Ok(());
    };
    if ctx
        .guild_id()
        .is_none_or(|guild| guild.get() != runtime.config.guild_id)
    {
        ctx.say("Abbey voice is locked to a different server by deployment configuration.")
            .await?;
        return Ok(());
    }
    let snapshot = runtime.snapshot().await;
    let current = match songbird::get(ctx.serenity_context()).await {
        Some(manager) => match manager.get(GuildId::new(runtime.config.guild_id)) {
            Some(call) => call.lock().await.current_channel().map_or_else(
                || "not connected".into(),
                |id| format!("connected to <#{id}>"),
            ),
            None => "not connected".into(),
        },
        None => "voice manager unavailable".into(),
    };
    let speech_models = match &runtime.config.backend {
        VoiceBackendConfig::Local(config) => format!(
            "STT: `{}`\nTTS: `{}` · voice: `{}`",
            config.stt_model, config.tts_model, config.voice
        ),
        VoiceBackendConfig::OpenAi(config) => {
            format!(
                "Realtime model: `{}` · voice: `{}`",
                config.model, config.voice
            )
        }
        VoiceBackendConfig::Disabled => "Speech models: none".into(),
    };
    ctx.say(format!(
        "Abbey voice: {current}\nMode: {}\nPhase: {}\nMedia gate: {}\nPending start: {}\nStatus: {}\nConsent epoch: {} · participants attested: {}\n{}\nQueue drops: {} · overrun-aborted turns: {} · barge-ins: {} · completed turns: {}\nSession epoch: {}",
        runtime.config.mode().label(),
        snapshot.phase.label(),
        if snapshot.media_enabled { "open" } else { "closed" },
        if snapshot.start_pending { "yes" } else { "no" },
        snapshot.status,
        snapshot.consent_epoch,
        snapshot.participant_count,
        speech_models,
        snapshot.dropped_input,
        snapshot.aborted_overruns,
        snapshot.barge_ins,
        snapshot.completed_turns,
        snapshot.epoch,
    ))
    .await?;
    Ok(())
}

/// Safe deployment-time presence. This never decodes, receives, or transmits
/// call audio, irrespective of the configured conversational backend.
pub async fn autojoin_self_deafened(
    ctx: &serenity::all::Context,
    runtime: Arc<VoiceRuntime>,
) -> Result<(), String> {
    let _transition = runtime.transition.lock().await;
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let channel = channel_id
        .to_channel(&ctx.http)
        .await
        .map_err(|error| format!("fetching the configured channel failed: {error}"))?;
    let Some(channel) = channel.guild() else {
        return Err("the configured voice destination is not a server channel".into());
    };
    if channel.guild_id != guild_id || channel.kind != ChannelType::Voice {
        return Err(
            "the configured destination is not a voice channel in its configured server".into(),
        );
    }
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| "Songbird was not registered in the Discord client".to_string())?;
    runtime
        .disconnect("replacing voice with safe no-audio presence")
        .await;
    let old_voice_session =
        cached_bot_voice_state_from_serenity(ctx, guild_id).map(|state| state.session_id);
    if let Some(existing) = manager.get(guild_id) {
        let _ = set_muted_self_deafened(&existing).await;
        manager
            .remove(guild_id)
            .await
            .map_err(|error| format!("replacing the existing voice session failed: {error}"))?;
    }
    if let Some(old_session_id) = old_voice_session.as_deref() {
        wait_for_voice_session_gone(ctx, guild_id, old_session_id).await?;
    }
    manager.set_config(no_audio_songbird_config());
    let prepared_call = manager.get_or_insert(guild_id);
    if let Err(error) = set_muted_self_deafened(&prepared_call).await {
        let _ = manager.remove(guild_id).await;
        return Err(format!(
            "preparing the muted/self-deafened state failed: {error}"
        ));
    }
    let call = match manager.join(guild_id, channel_id).await {
        Ok(call) => call,
        Err(error) => {
            let _ = set_muted_self_deafened(&prepared_call).await;
            let _ = manager.remove(guild_id).await;
            return Err(format!("Discord refused the voice join: {error}"));
        }
    };
    if let Err(error) = set_muted_self_deafened(&call).await {
        let _ = manager.remove(guild_id).await;
        return Err(format!(
            "entering the required muted/self-deafened state failed: {error}"
        ));
    }
    runtime
        .set_presence("connected muted/self-deafened; media decoding is disabled")
        .await;
    tracing::info!(guild = %guild_id, channel = %channel_id, "joined Discord voice with decryption/decoding and transmission disabled");
    Ok(())
}

/// Honor an unambiguous natural-language withdrawal only for a human who is
/// currently present in the configured voice channel. Positive prose never
/// enters this path and can never start or resume voice.
pub(crate) async fn withdraw_voice_from_text(
    ctx: &serenity::all::Context,
    runtime: &VoiceRuntime,
    author_id: u64,
) -> bool {
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let participants = cached_participants_from_serenity(ctx, guild_id, channel_id);
    // A populated cache authorizes only a currently present human. If the
    // guild cache is temporarily unavailable, fail closed for consent: an
    // explicit withdrawal in the exact configured voice-chat channel may stop
    // voice but still can never start it.
    if participants
        .as_ref()
        .is_some_and(|participants| !participants.contains(&author_id))
    {
        return false;
    }

    // Cancel slow preflight and close any current media gate before the first
    // await. Re-snapshot afterwards so a stale text event also stops a newer
    // active replacement instead of merely targeting its old epoch.
    runtime.cancel_pending_start();
    let snapshot = runtime.snapshot().await;
    if !matches!(
        snapshot.phase,
        VoicePhase::Connecting
            | VoicePhase::Listening
            | VoicePhase::Thinking
            | VoicePhase::Speaking
    ) {
        return true;
    }
    let epoch = snapshot.epoch;
    // Capture the Call while the retiring epoch is still current. A start
    // advances the runtime epoch before replacing Songbird's Call: if it wins
    // this race, the exact-epoch pause below fails and this Arc is untouched;
    // if the pause wins, this Arc can never become a later replacement.
    let manager = songbird::get(ctx).await;
    let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
    let Some(pause) = runtime
        .begin_pause_epoch_for_consent(
            epoch,
            participants.unwrap_or_default(),
            "audio stopped because a participant withdrew consent in voice chat",
        )
        .await
    else {
        return true;
    };
    if let Some(call) = exact_call {
        // Stop the exact Decode driver before waiting for provider-task
        // cancellation. The software gate is already closed above.
        pause_call_for_consent(&call).await;
    }
    pause.finish().await;
    let transition = runtime.transition.lock().await;
    let current = runtime.snapshot().await;
    if current.epoch == epoch.saturating_add(1)
        && current.phase == VoicePhase::AwaitingConsent
        && let Some(manager) = manager
    {
        remove_call_for_consent(&manager, guild_id).await;
    }
    drop(transition);
    true
}

/// Fail closed when Discord moves/disconnects Abbey, or when someone joins an
/// active consent epoch.
pub async fn on_voice_state_update(
    ctx: &serenity::all::Context,
    old: &Option<VoiceState>,
    new: &VoiceState,
    data: &crate::Data,
) {
    let Some(runtime) = data.voice.as_ref().cloned() else {
        return;
    };
    let Some(guild_id) = new.guild_id else {
        return;
    };
    if guild_id.get() != runtime.config.guild_id {
        return;
    }
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let bot_id = ctx.cache.current_user().id;
    if new.user_id == bot_id {
        let snapshot = runtime.snapshot().await;
        if matches!(
            snapshot.phase,
            VoicePhase::Disconnected | VoicePhase::AwaitingConsent | VoicePhase::Failed
        ) {
            // AwaitingConsent intentionally disconnects the Decode call. The
            // resulting gateway update is not a new failure, and `/resume`
            // will create a fresh DAVE session after renewed consent.
            return;
        }
        // Serenity spawns gateway callbacks independently after updating its
        // cache. A delayed disconnect from an old Call must not tear down a
        // replacement, so the current cache (not only this event payload) is
        // the authority and the epoch is checked again under `transition`.
        let cached = cached_bot_voice_state_from_serenity(ctx, guild_id);
        let moved = cached
            .as_ref()
            .is_none_or(|state| state.channel_id != Some(channel_id));
        let blocked = cached
            .as_ref()
            .is_some_and(|state| bot_voice_state_is_blocked(state, channel_id, snapshot.phase));
        if !moved && !blocked {
            return;
        }
        let reason = if moved {
            "Discord moved or disconnected Abbey; audio stopped"
        } else {
            "Discord muted, deafened, or suppressed Abbey; audio stopped"
        };
        if !runtime.media_enabled(snapshot.epoch) {
            // Connecting sessions have no readable/playable media and their
            // command owns `transition`. Wait behind it before acting so a
            // delayed `channel=None` callback from the retired Call cannot
            // invalidate the replacement epoch. Startup's final cache/driver
            // checks remain the safety authority during this window.
            let transition = runtime.transition.lock().await;
            let current = runtime.snapshot().await;
            if current.epoch != snapshot.epoch {
                drop(transition);
                return;
            }
            let cached = cached_bot_voice_state_from_serenity(ctx, guild_id);
            let still_moved = cached
                .as_ref()
                .is_none_or(|state| state.channel_id != Some(channel_id));
            let still_blocked = cached
                .as_ref()
                .is_some_and(|state| bot_voice_state_is_blocked(state, channel_id, current.phase));
            if !still_moved && !still_blocked {
                drop(transition);
                return;
            }
            let manager = songbird::get(ctx).await;
            let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
            let Some(pause) = runtime
                .begin_pause_epoch_for_consent(current.epoch, HashSet::new(), reason)
                .await
            else {
                drop(transition);
                return;
            };
            if let Some(call) = exact_call {
                pause_call_for_consent(&call).await;
            }
            pause.finish().await;
            runtime.fail_safe(reason).await;
            if let Some(manager) = manager {
                remove_call_for_consent(&manager, guild_id).await;
            }
            drop(transition);
            let _ = channel_id
                .say(
                    &ctx.http,
                    "Abbey stopped voice because Discord moved, disconnected, muted, deafened, or suppressed her. Use `/voice join consent:true` after restoring Connect and Speak and checking everyone present.",
                )
                .await;
            return;
        }
        if !runtime.revoke_media(snapshot.epoch) {
            return;
        }
        let manager = songbird::get(ctx).await;
        let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
        let Some(pause) = runtime
            .begin_pause_epoch_for_consent(snapshot.epoch, HashSet::new(), reason)
            .await
        else {
            return;
        };
        if let Some(call) = exact_call {
            pause_call_for_consent(&call).await;
        }
        pause.finish().await;
        let transition = runtime.transition.lock().await;
        let expected_paused_epoch = snapshot.epoch.saturating_add(1);
        let current = runtime.snapshot().await;
        // The consent epoch was already closed before waiting for transition.
        // A transient Discord recovery cannot reopen it, so teardown is
        // unconditional while this remains the same paused epoch.
        if current.epoch != expected_paused_epoch || current.phase != VoicePhase::AwaitingConsent {
            drop(transition);
            return;
        }
        runtime.fail_safe(reason).await;
        if let Some(manager) = manager {
            remove_call_for_consent(&manager, guild_id).await;
        }
        drop(transition);
        let _ = channel_id
            .say(
                &ctx.http,
                "Abbey stopped voice because Discord moved, disconnected, muted, deafened, or suppressed her. Use `/voice join consent:true` after restoring Connect and Speak and checking everyone present.",
            )
            .await;
        return;
    }
    let joined_target = new.channel_id == Some(channel_id)
        && old.as_ref().and_then(|state| state.channel_id) != Some(channel_id);
    let snapshot = runtime.snapshot().await;
    if !joined_target
        || !matches!(
            snapshot.phase,
            VoicePhase::Connecting
                | VoicePhase::Listening
                | VoicePhase::Thinking
                | VoicePhase::Speaking
        )
    {
        return;
    }
    if runtime
        .epoch_attests(snapshot.epoch, new.user_id.get())
        .await
    {
        return;
    }
    let participants = match cached_participants_from_serenity(ctx, guild_id, channel_id) {
        Some(participants) if !participants.contains(&new.user_id.get()) => {
            // This independently spawned join callback arrived after the user
            // had already left; the current cache is authoritative.
            return;
        }
        Some(participants) => participants,
        None => HashSet::from([new.user_id.get()]),
    };
    if snapshot.phase.processes_audio() && !runtime.revoke_media(snapshot.epoch) {
        return;
    }
    let manager = songbird::get(ctx).await;
    let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
    let Some(pause) = runtime
        .begin_pause_epoch_for_consent(
            snapshot.epoch,
            participants,
            "audio stopped; a new participant requires renewed consent",
        )
        .await
    else {
        return;
    };
    if let Some(call) = exact_call {
        pause_call_for_consent(&call).await;
    }
    pause.finish().await;
    let transition = runtime.transition.lock().await;
    let expected_paused_epoch = snapshot.epoch.saturating_add(1);
    let current = runtime.snapshot().await;
    if current.epoch != expected_paused_epoch || current.phase != VoicePhase::AwaitingConsent {
        drop(transition);
        return;
    }
    if let Some(manager) = manager {
        // Songbird 0.6 cannot change an active UDP receiver from Decode to
        // Pass. Remove it so no packets are decrypted/decoded while consent
        // is closed; `/resume` always performs a fresh remove/rejoin anyway.
        remove_call_for_consent(&manager, guild_id).await;
    }
    drop(transition);
    let _ = channel_id
        .say(
            &ctx.http,
            "Abbey disconnected voice because someone new joined. Capture and playback are stopped. Notify everyone now present, then use `/voice resume consent:true`.",
        )
        .await;
}

/// Channel overwrites, role permissions, role deletion, and Abbey's own role
/// assignments can turn an apparently healthy call into a silent receive-only
/// recorder. Serenity updates its cache before dispatching these callbacks, so
/// re-evaluate the complete effective permission set and fail closed.
pub async fn on_voice_permissions_changed(
    ctx: &serenity::all::Context,
    affected_guild_id: GuildId,
    affected_channel_id: Option<ChannelId>,
    data: &crate::Data,
) {
    let Some(runtime) = data.voice.as_ref().cloned() else {
        return;
    };
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);
    if affected_guild_id != guild_id
        || affected_channel_id.is_some_and(|affected| affected != channel_id)
    {
        return;
    }
    let snapshot = runtime.snapshot().await;
    if !matches!(
        snapshot.phase,
        VoicePhase::Connecting
            | VoicePhase::Listening
            | VoicePhase::Thinking
            | VoicePhase::Speaking
    ) {
        return;
    }
    let current_channel = ctx
        .cache
        .guild(guild_id)
        .and_then(|guild| guild.channels.get(&channel_id).cloned());
    if current_channel
        .as_ref()
        .is_some_and(|channel| bot_has_required_voice_permissions(ctx, channel))
    {
        return;
    }
    if snapshot.phase.processes_audio() && !runtime.revoke_media(snapshot.epoch) {
        return;
    }
    let manager = songbird::get(ctx).await;
    let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
    let Some(pause) = runtime
        .begin_pause_epoch_for_consent(
            snapshot.epoch,
            HashSet::new(),
            "required Discord voice permissions changed; audio stopped",
        )
        .await
    else {
        return;
    };
    if let Some(call) = exact_call {
        pause_call_for_consent(&call).await;
    }
    pause.finish().await;
    let transition = runtime.transition.lock().await;
    let current = runtime.snapshot().await;
    let expected_paused_epoch = snapshot.epoch.saturating_add(1);
    // Once the epoch is closed, even a fast permission recovery needs a fresh
    // consented join; never leave the old Decode transport connected.
    if current.epoch != expected_paused_epoch || current.phase != VoicePhase::AwaitingConsent {
        drop(transition);
        return;
    }
    runtime
        .fail_safe("required Discord voice permissions changed; audio stopped")
        .await;
    if let Some(manager) = manager {
        remove_call_for_consent(&manager, guild_id).await;
    }
    drop(transition);
    let _ = channel_id
        .say(
            &ctx.http,
            "Abbey stopped voice because she needs View Channel, Send Messages, Connect, and Speak for a public bidirectional session. Restore those permissions, then use `/voice join consent:true`.",
        )
        .await;
}

fn select_local_backend(state: &crate::runtime::AppState) -> Result<crate::llm::Backend, String> {
    state
        .backend
        .as_ref()
        .into_iter()
        .chain(state.fallback.as_ref())
        .find(|backend| backend.is_loopback_openai_compatible())
        .cloned()
        .ok_or_else(|| {
            "Local voice requires a loopback ABBEY_BOT_LLM_ENDPOINT; it will not send transcripts to a remote text provider.".into()
        })
}

fn required_voice_permissions() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::CONNECT
        | Permissions::SPEAK
}

fn bot_has_required_voice_permissions(
    ctx: &serenity::all::Context,
    channel: &GuildChannel,
) -> bool {
    let bot_id = ctx.cache.current_user().id;
    ctx.cache.guild(channel.guild_id).is_some_and(|guild| {
        guild.members.get(&bot_id).is_some_and(|member| {
            guild
                .user_permissions_in(channel, member)
                .contains(required_voice_permissions())
        })
    })
}

/// Fetch the channel, Abbey's member record, and guild roles directly from
/// Discord. This closes the long model-preflight and activation TOCTOU windows
/// even if a gateway callback is delayed. Passing `&ctx.http` deliberately
/// bypasses Serenity's cache for the guild request.
async fn verify_required_voice_permissions_live(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<(), String> {
    let bot_id = ctx.cache.current_user().id;
    let fetched = tokio::try_join!(
        channel_id.to_channel(&ctx.http),
        guild_id.member(&ctx.http, bot_id),
        guild_id.to_partial_guild(&ctx.http),
    );
    let (channel, member, guild) = fetched.map_err(|error| {
        tracing::warn!(%error, %guild_id, %channel_id, "could not revalidate Discord voice permissions");
        "Discord could not revalidate Abbey's View Channel, Send Messages, Connect, and Speak permissions; voice stayed off."
            .to_string()
    })?;
    let Some(channel) = channel.guild() else {
        return Err(
            "The configured voice destination is no longer a server channel; voice stayed off."
                .into(),
        );
    };
    if channel.guild_id != guild_id || channel.id != channel_id {
        return Err(
            "The configured voice destination changed unexpectedly; voice stayed off.".into(),
        );
    }
    if !guild
        .user_permissions_in(&channel, &member)
        .contains(required_voice_permissions())
    {
        return Err("Abbey needs View Channel, Send Messages, Connect, and Speak in the configured voice channel; voice stayed off."
            .into());
    }
    Ok(())
}

fn cached_participants(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<(bool, HashSet<u64>), String> {
    let guild = ctx.guild().ok_or_else(|| {
        "Discord's voice-state cache is not ready; no audio was enabled.".to_string()
    })?;
    let caller_present = guild
        .voice_states
        .get(&ctx.author().id)
        .and_then(|state| state.channel_id)
        == Some(channel_id);
    let bot_id = ctx.serenity_context().cache.current_user().id;
    let participants = guild
        .voice_states
        .iter()
        .filter(|(user_id, state)| {
            **user_id != bot_id
                && state.channel_id == Some(channel_id)
                && guild
                    .members
                    .get(user_id)
                    .is_none_or(|member| !member.user.bot)
        })
        .map(|(user_id, _)| user_id.get())
        .collect();
    let _ = guild_id;
    Ok((caller_present, participants))
}

fn cached_participants_from_serenity(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Option<HashSet<u64>> {
    let guild = ctx.cache.guild(guild_id)?;
    let bot_id = ctx.cache.current_user().id;
    Some(
        guild
            .voice_states
            .iter()
            .filter(|(user_id, state)| {
                **user_id != bot_id
                    && state.channel_id == Some(channel_id)
                    && guild
                        .members
                        .get(user_id)
                        .is_none_or(|member| !member.user.bot)
            })
            .map(|(user_id, _)| user_id.get())
            .collect(),
    )
}

fn cached_bot_voice_state(ctx: Context<'_>, guild_id: GuildId) -> Option<VoiceState> {
    let bot_id = ctx.serenity_context().cache.current_user().id;
    ctx.guild()
        .filter(|guild| guild.id == guild_id)
        .and_then(|guild| guild.voice_states.get(&bot_id).cloned())
}

fn cached_bot_voice_state_from_serenity(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
) -> Option<VoiceState> {
    let bot_id = ctx.cache.current_user().id;
    ctx.cache
        .guild(guild_id)
        .and_then(|guild| guild.voice_states.get(&bot_id).cloned())
}

async fn wait_for_voice_session_gone(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
    old_session_id: &str,
) -> Result<(), String> {
    for _ in 0..20 {
        let old_is_gone = cached_bot_voice_state_from_serenity(ctx, guild_id)
            .is_none_or(|state| state.channel_id.is_none() || state.session_id != old_session_id);
        if old_is_gone {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(
        "The previous Discord voice session did not finish leaving within five seconds; the replacement stayed off to avoid a stale disconnect race."
            .into(),
    )
}

async fn wait_for_bot_voice_state(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<String, String> {
    for _ in 0..20 {
        match cached_bot_voice_state(ctx, guild_id) {
            Some(state) if state.channel_id == Some(channel_id) => {
                if state.mute || state.deaf || state.suppress {
                    return Err(
                        "Discord reports Abbey server-muted, server-deafened, or suppressed; a receive-only or transmit-only session is forbidden."
                            .into(),
                    );
                }
                if state.self_mute && state.self_deaf {
                    return Ok(state.session_id);
                }
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(
        "Discord did not confirm Abbey's exact muted/self-deafened voice-channel safety state within five seconds."
            .into(),
    )
}

fn bot_voice_state_allows_conversation(
    state: &VoiceState,
    channel_id: ChannelId,
    session_id: &str,
) -> bool {
    state.channel_id == Some(channel_id)
        && state.session_id == session_id
        && conversation_flags_are_clear(
            state.mute,
            state.deaf,
            state.suppress,
            state.self_mute,
            state.self_deaf,
        )
}

fn conversation_flags_are_clear(
    mute: bool,
    deaf: bool,
    suppress: bool,
    self_mute: bool,
    self_deaf: bool,
) -> bool {
    !mute && !deaf && !suppress && !self_mute && !self_deaf
}

fn bot_voice_state_is_blocked(
    state: &VoiceState,
    channel_id: ChannelId,
    phase: VoicePhase,
) -> bool {
    if state.channel_id != Some(channel_id) {
        return false;
    }
    let server_blocked = state.mute || state.deaf || state.suppress;
    let self_blocked_while_active = phase.processes_audio() && (state.self_mute || state.self_deaf);
    server_blocked || self_blocked_while_active
}

async fn wait_for_enabled_bot_voice_state(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel_id: ChannelId,
    session_id: &str,
) -> Result<(), String> {
    for _ in 0..20 {
        if cached_bot_voice_state(ctx, guild_id).is_some_and(|state| {
            bot_voice_state_allows_conversation(&state, channel_id, session_id)
        }) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(
        "Discord did not confirm Abbey was unmuted, undeafened, unsuppressed, and still in the exact joined session within five seconds."
            .into(),
    )
}

fn no_audio_songbird_config() -> songbird::Config {
    songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Pass)
}

fn audio_songbird_config() -> songbird::Config {
    songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Decode(
        songbird::driver::DecodeConfig::new(
            songbird::driver::Channels::Mono,
            songbird::driver::SampleRate::Hz24000,
        ),
    ))
}

fn initial_songbird_config(mode: VoiceMode) -> songbird::Config {
    match mode {
        VoiceMode::Disabled => no_audio_songbird_config(),
        VoiceMode::Local | VoiceMode::OpenAi => audio_songbird_config(),
    }
}

async fn set_muted_self_deafened(
    call: &Arc<Mutex<songbird::Call>>,
) -> Result<(), songbird::error::JoinError> {
    let mut call = call.lock().await;
    call.deafen(true).await?;
    call.mute(true).await
}

async fn enable_conversation(
    call: &Arc<Mutex<songbird::Call>>,
) -> Result<(), songbird::error::JoinError> {
    let mut call = call.lock().await;
    // Unmute while still self-deafened, then make reception the final enabling
    // transition. A failed unmute therefore never exposes participant audio.
    call.mute(false).await?;
    call.deafen(false).await
}

async fn pause_call_for_consent(call: &Arc<Mutex<songbird::Call>>) {
    let mut call = call.lock().await;
    // `leave()` stops the driver locally before it sends the gateway update.
    // Muting/deafening first would leave Decode active during two extra
    // gateway round trips underneath the already-closed media gate.
    let _ = call.leave().await;
}

async fn remove_call_for_consent(manager: &songbird::Songbird, guild_id: GuildId) {
    // `Songbird::remove` calls `Call::leave`, which stops the driver locally
    // before awaiting Discord's gateway update, then drops the retained Call.
    let _ = manager.remove(guild_id).await;
}

fn can_stop_voice(
    present_in_configured_channel: bool,
    interaction_permissions: Option<serenity::all::Permissions>,
) -> bool {
    present_in_configured_channel
        || interaction_permissions.is_some_and(|permissions| permissions.manage_guild())
}

#[derive(Clone)]
struct DiscordAudioForwarder {
    tx: mpsc::Sender<VoiceFrame>,
    driver_disconnect: watch::Sender<bool>,
    call: Weak<Mutex<songbird::Call>>,
    manager: Weak<songbird::Songbird>,
    guild_id: GuildId,
    attested: Arc<HashSet<u64>>,
    mappings: Arc<RwLock<HashMap<u32, u64>>>,
    sequence: Arc<AtomicU64>,
    runtime: Arc<VoiceRuntime>,
    epoch: u64,
}

#[serenity::async_trait]
impl EventHandler for DiscordAudioForwarder {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        if !self.runtime.is_current(self.epoch) {
            return Some(Event::Cancel);
        }
        match context {
            EventContext::SpeakingStateUpdate(update) => {
                if let Some(user_id) = update.user_id {
                    self.mappings
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(update.ssrc, user_id.0);
                }
                None
            }
            EventContext::ClientDisconnect(disconnect) => {
                self.mappings
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|_, user_id| *user_id != disconnect.user_id.0);
                None
            }
            EventContext::VoiceTick(tick) => {
                if !self.runtime.media_enabled(self.epoch) {
                    return None;
                }
                let mappings = self
                    .mappings
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let mut voiced: Vec<(u32, &[i16], u64, u64)> = Vec::new();
                let mut unattested = None;
                for (ssrc, data) in &tick.speaking {
                    let Some(user_id) = mappings.get(ssrc).copied() else {
                        unattested = Some(None);
                        break;
                    };
                    if !self.attested.contains(&user_id) {
                        unattested = Some(Some(user_id));
                        break;
                    }
                    let Some(samples) = data.decoded_voice.as_deref() else {
                        continue;
                    };
                    if !frame_is_voice(samples) {
                        continue;
                    }
                    let energy = samples.iter().fold(0_u64, |sum, sample| {
                        let value = i64::from(*sample);
                        sum.saturating_add(
                            value.unsigned_abs().saturating_mul(value.unsigned_abs()),
                        )
                    });
                    voiced.push((*ssrc, samples, energy, user_id));
                }
                if let Some(user_id) = unattested {
                    let mut participants = self.attested.as_ref().clone();
                    if let Some(user_id) = user_id {
                        participants.insert(user_id);
                    }
                    if self.runtime.revoke_media(self.epoch) {
                        let runtime = Arc::clone(&self.runtime);
                        let call = self.call.clone();
                        let manager = self.manager.clone();
                        let guild_id = self.guild_id;
                        let epoch = self.epoch;
                        tokio::spawn(async move {
                            let Some(pause) = runtime
                                .begin_pause_epoch_for_consent(
                                    epoch,
                                    participants,
                                    "audio stopped before an unknown or unattested speaker frame was forwarded",
                                )
                                .await
                            else {
                                return;
                            };
                            let manager = manager.upgrade();
                            let exact_call = call.upgrade();
                            if let Some(call) = exact_call {
                                // This is the exact Call that emitted the
                                // unattested frame. Stop its Decode driver
                                // before awaiting actor cancellation.
                                pause_call_for_consent(&call).await;
                            }
                            pause.finish().await;
                            let transition = runtime.transition.lock().await;
                            let snapshot = runtime.snapshot().await;
                            if snapshot.epoch != epoch.saturating_add(1)
                                || snapshot.phase != VoicePhase::AwaitingConsent
                            {
                                drop(transition);
                                return;
                            }
                            if let Some(manager) = manager {
                                remove_call_for_consent(&manager, guild_id).await;
                            }
                            drop(transition);
                        });
                    }
                    return Some(Event::Cancel);
                }
                voiced.sort_unstable_by_key(|(_, _, energy, _)| std::cmp::Reverse(*energy));
                let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
                let frame = if let Some((_ssrc, samples, _, user_id)) = voiced.first() {
                    let mut samples = samples.to_vec();
                    samples.resize(FRAME_SAMPLES, 0);
                    samples.truncate(FRAME_SAMPLES);
                    VoiceFrame {
                        sequence,
                        speaker_id: Some(*user_id),
                        samples,
                        overlap: voiced.len() > 1,
                    }
                } else {
                    VoiceFrame::silence(sequence)
                };
                let sent = self
                    .runtime
                    .with_media_enabled(self.epoch, || self.tx.try_send(frame))?;
                match sent {
                    Ok(()) => None,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.runtime.note_dropped_input();
                        None
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => Some(Event::Cancel),
                }
            }
            EventContext::DriverDisconnect(_) => {
                let _ = self.driver_disconnect.send(true);
                Some(Event::Cancel)
            }
            _ => None,
        }
    }
}

struct ReceiveHandlerInstall<'a> {
    call: &'a Arc<Mutex<songbird::Call>>,
    manager: Weak<songbird::Songbird>,
    guild_id: GuildId,
    runtime: &'a Arc<VoiceRuntime>,
    epoch: u64,
    attested: HashSet<u64>,
    tx: mpsc::Sender<VoiceFrame>,
    driver_disconnect: watch::Sender<bool>,
}

async fn install_receive_handlers(input: ReceiveHandlerInstall<'_>) {
    let handler = DiscordAudioForwarder {
        tx: input.tx,
        driver_disconnect: input.driver_disconnect,
        // The Call owns these global handlers. Holding a strong Arc here would
        // create Call -> handler -> Call and keep the driver alive after leave.
        call: Arc::downgrade(input.call),
        manager: input.manager,
        guild_id: input.guild_id,
        attested: Arc::new(input.attested),
        mappings: Arc::new(RwLock::new(HashMap::new())),
        sequence: Arc::new(AtomicU64::new(0)),
        runtime: Arc::clone(input.runtime),
        epoch: input.epoch,
    };
    let mut call = input.call.lock().await;
    call.add_global_event(Event::Core(CoreEvent::SpeakingStateUpdate), handler.clone());
    call.add_global_event(Event::Core(CoreEvent::ClientDisconnect), handler.clone());
    call.add_global_event(Event::Core(CoreEvent::VoiceTick), handler.clone());
    call.add_global_event(Event::Core(CoreEvent::DriverDisconnect), handler);
}

fn consent_notice(runtime: &VoiceRuntime, channel_id: ChannelId, resumed: bool) -> String {
    let action = if resumed { "resuming" } else { "starting" };
    match runtime.config.mode() {
        VoiceMode::Local => format!(
            "🔒 Abbey is {action} consented voice in <#{channel_id}>. Discord still transports the call, but speech recognition, Abbey/Abi/Aviva reasoning, WDBX-scoped context, and speech synthesis run locally on Donald's Mac. Abbey does not retain raw audio. Person-specific WDBX context is read-only and is used only for one uniquely attributed speaker; overlap disables it. Say Abbey, Aviva, or ABI to start. A clearly attributed spoken withdrawal is honored locally; `/voice leave` or writing `stop listening` in this voice chat is the authoritative stop. A new participant pauses and disconnects the session until renewed consent."
        ),
        VoiceMode::OpenAi => format!(
            "☁️ Abbey is {action} consented voice in <#{channel_id}> using the explicitly configured direct OpenAI Realtime backup. Participant audio is sent to that provider and complete responses are buffered before Discord playback. This degraded backup does not use local ABI persona routing or WDBX context; use local mode for canonical Abbey. Abbey does not retain raw audio locally. Spoken control is not authoritative in this degraded mode: use `/voice leave` or write `stop listening` in this voice chat to stop immediately. A new participant pauses and disconnects the session until renewed consent."
        ),
        VoiceMode::Disabled => unreachable!(),
    }
}

fn public_error(error: &str) -> String {
    let flattened = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut value: String = flattened.chars().take(240).collect();
    if flattened.chars().count() > 240 {
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_conversation_flag_independently_blocks_audio() {
        // The consent contract is "all five clear", not "most are clear".
        // Each flag is asserted alone so a future edit that drops one from the
        // conjunction fails here rather than in a live call.
        assert!(conversation_flags_are_clear(
            false, false, false, false, false
        ));
        for (index, blocked) in [
            conversation_flags_are_clear(true, false, false, false, false),
            conversation_flags_are_clear(false, true, false, false, false),
            conversation_flags_are_clear(false, false, true, false, false),
            conversation_flags_are_clear(false, false, false, true, false),
            conversation_flags_are_clear(false, false, false, false, true),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(!blocked, "flag {index} alone must block conversation");
        }
    }

    #[test]
    fn stopping_voice_needs_presence_or_manage_guild() {
        // Presence in the configured channel is sufficient on its own.
        assert!(can_stop_voice(true, None));
        assert!(can_stop_voice(true, Some(Permissions::empty())));
        // Otherwise it takes Manage Guild specifically — not merely holding
        // some permission, and not the voice permissions Abbey itself needs.
        assert!(can_stop_voice(false, Some(Permissions::MANAGE_GUILD)));
        assert!(!can_stop_voice(false, Some(required_voice_permissions())));
        assert!(!can_stop_voice(false, Some(Permissions::empty())));
        assert!(!can_stop_voice(false, None));
    }

    #[test]
    fn required_voice_permissions_stay_least_privilege() {
        let required = required_voice_permissions();
        for expected in [
            Permissions::VIEW_CHANNEL,
            Permissions::SEND_MESSAGES,
            Permissions::CONNECT,
            Permissions::SPEAK,
        ] {
            assert!(required.contains(expected), "missing {expected:?}");
        }
        // Joining a voice channel must never quietly require moderator-grade
        // permissions; those gate the commands, not the connection.
        for forbidden in [
            Permissions::ADMINISTRATOR,
            Permissions::MANAGE_GUILD,
            Permissions::MANAGE_MESSAGES,
            Permissions::MUTE_MEMBERS,
            Permissions::MOVE_MEMBERS,
        ] {
            assert!(
                !required.contains(forbidden),
                "unexpectedly requires {forbidden:?}"
            );
        }
    }

    #[test]
    fn no_audio_config_disables_decryption_and_decoding() {
        let config = no_audio_songbird_config();
        assert_eq!(config.decode_mode, songbird::driver::DecodeMode::Pass);
    }

    #[test]
    fn conversational_decode_is_exactly_mono_24khz() {
        let config = audio_songbird_config();
        assert_eq!(
            config.decode_mode,
            songbird::driver::DecodeMode::Decode(songbird::driver::DecodeConfig::new(
                songbird::driver::Channels::Mono,
                songbird::driver::SampleRate::Hz24000,
            ))
        );
    }

    #[test]
    fn conversational_calls_are_constructed_decoding_before_join() {
        assert!(matches!(
            initial_songbird_config(VoiceMode::Disabled).decode_mode,
            songbird::driver::DecodeMode::Pass
        ));
        for mode in [VoiceMode::Local, VoiceMode::OpenAi] {
            assert!(matches!(
                initial_songbird_config(mode).decode_mode,
                songbird::driver::DecodeMode::Decode(_)
            ));
        }
    }

    #[test]
    fn public_errors_are_flat_and_bounded() {
        let error = public_error(&format!("bad\n{}", "x".repeat(500)));
        assert!(!error.contains('\n'));
        assert!(error.chars().count() <= 241);
    }

    #[test]
    fn out_of_channel_manager_can_stop_voice_without_member_cache_entry() {
        assert!(can_stop_voice(
            false,
            Some(serenity::all::Permissions::MANAGE_GUILD)
        ));
        assert!(can_stop_voice(true, None));
        assert!(!can_stop_voice(false, None));
    }

    #[test]
    fn activation_requires_discord_to_acknowledge_all_mute_flags_cleared() {
        assert!(conversation_flags_are_clear(
            false, false, false, false, false
        ));
        for flag in 0..5 {
            assert!(!conversation_flags_are_clear(
                flag == 0,
                flag == 1,
                flag == 2,
                flag == 3,
                flag == 4,
            ));
        }
    }
}
