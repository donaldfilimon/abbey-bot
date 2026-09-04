//! Thin Discord/Songbird shell for Abbey voice.
//!
//! Commands validate runtime permission, exact-channel membership, explicit
//! participant attestation, and provider readiness while the call is muted and
//! self-deafened. Only after a public disclosure succeeds do they enable
//! decoding. The provider actors live in `voice_local` and `voice_openai`.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{ChannelId, ChannelType, GuildId};
use tokio::sync::{Mutex, mpsc, oneshot, watch};

use crate::gateway::shared::clamp_message;
use crate::offline_voice::MlxAudioClient;
use crate::voice::{VoiceBackendConfig, VoiceMode};
use crate::voice_local::LocalSession;
use crate::voice_openai::OpenAiSession;
use crate::voice_session::{SessionControl, SharedPlayback, VerificationActivation, VoiceRuntime};
use crate::{Context, Error};

mod consent;
mod discord;
mod events;
mod receive;
mod supervision;
mod verification;

use discord::*;
use receive::{ReceiveHandlerInstall, install_receive_handlers};

pub use consent::{voice_consent, voice_notice};
pub use events::on_gateway_event;
pub use supervision::autojoin_self_deafened;
pub use verification::voice_verify;

const INPUT_QUEUE_FRAMES: usize = 64;
const OPENAI_READY_TIMEOUT: Duration = Duration::from_secs(20);
const LOCAL_HEALTH_TIMEOUT: Duration = Duration::from_secs(600);
const SIDECAR_STATUS_TIMEOUT: Duration = Duration::from_secs(2);

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

/// Consent-gated Discord voice and redacted operator verification.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    subcommands(
        "voice_join",
        "voice_resume",
        "voice_leave",
        "voice_status",
        "voice_verify",
        "voice_mode",
        "voice_consent",
        "voice_notice"
    )
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

    // Make this issued start visible to later stop/withdrawal operations
    // before Discord's first await, without yet superseding a legitimate
    // pending preflight. Publication happens only after remote channel and
    // permission validation, if this lifecycle generation is still current.
    let start_operation = runtime.start_operation_token();
    ctx.defer_ephemeral().await?;
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
        ctx.say("Abbey needs View Channel, Send Messages, Connect, Speak, Stream, and Use Embedded Activities in the configured voice channel; voice stayed off.")
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
    manager.set_config(initial_songbird_config(effective_mode));
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
    let joined_session_id =
        match wait_for_bot_voice_state(ctx.serenity_context(), guild_id, channel_id).await {
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
            tokio::spawn(crate::voice_local::run(session))
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
    // Bind completion evidence to the run that was armed when this authorized
    // leave began. A later verifier must not inherit this leave's result.
    let verification_run = runtime.verification_run_token();
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
            if let Some(run) = verification_run {
                let _ = runtime.note_verification_final_leave(run);
            }
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
    let effective_mode = runtime.effective_mode();
    // Status describes the backend a join would connect now, not the startup
    // selection; otherwise "Mode: OpenAI" could sit above local speech models.
    let effective_backend = runtime.effective_backend();
    let speech_models = match &effective_backend {
        Some(VoiceBackendConfig::Local(config)) => format!(
            "STT: `{}`\nTTS: `{}` · voice: `{}`",
            config.stt_model, config.tts_model, config.voice
        ),
        Some(VoiceBackendConfig::OpenAi(config)) => {
            format!(
                "Realtime model: `{}` · voice: `{}`",
                config.model, config.voice
            )
        }
        Some(VoiceBackendConfig::Disabled) | None => "Speech models: none".into(),
    };
    let sidecar = match &effective_backend {
        Some(VoiceBackendConfig::Local(config)) => match MlxAudioClient::new(config.clone()) {
            Ok(client) => match tokio::time::timeout(SIDECAR_STATUS_TIMEOUT, client.health()).await
            {
                Ok(Ok(())) => format!(
                    "Local speech sidecar: listening at `{}`",
                    config.endpoint_display()
                ),
                Ok(Err(error)) => format!("Local speech sidecar: {}", public_error(&error)),
                Err(_) => format!(
                    "Local speech sidecar: not responding at `{}` within 2s (down or still loading Whisper/Kokoro)",
                    config.endpoint_display()
                ),
            },
            Err(error) => format!("Local speech sidecar: {}", public_error(&error)),
        },
        _ => "Local speech sidecar: not used in this mode".into(),
    };
    let loopback_llm = if effective_mode == VoiceMode::Local {
        match select_local_backend(&ctx.data().state) {
            Ok(_) => "Loopback LLM: configured".into(),
            Err(error) => format!("Loopback LLM: missing — {error}"),
        }
    } else {
        "Loopback LLM: not required for this voice mode".into()
    };
    let choices = cached_participants_from_serenity(
        ctx.serenity_context(),
        GuildId::new(runtime.config.guild_id),
        ChannelId::new(runtime.config.channel_id),
    )
    .map_or_else(
        || "Saved voice choices: current voice roster unavailable; activation is blocked.".into(),
        |users| consent::coverage_text(runtime, &users, effective_mode),
    );
    ctx.say(clamp_message(format!(
        "Abbey voice: {current}\nMode: {}\nPhase: {}\nMedia gate: {}\nPending start: {}\nStatus: {}\nConsent epoch: {} · participants attested: {}\n{choices}\n{}\n{}\n{}\nQueue drops: {} · overrun-aborted turns: {} · barge-ins: {} · completed turns: {}\nSession epoch: {}",
        effective_mode.label(),
        snapshot.phase.label(),
        if snapshot.media_enabled { "open" } else { "closed" },
        if snapshot.start_pending { "yes" } else { "no" },
        snapshot.status,
        snapshot.consent_epoch,
        snapshot.participant_count,
        speech_models,
        sidecar,
        loopback_llm,
        snapshot.dropped_input,
        snapshot.aborted_overruns,
        snapshot.barge_ins,
        snapshot.completed_turns,
        snapshot.epoch,
    )))
    .await?;
    Ok(())
}

/// Show or change the voice backend in force. Requires MANAGE_GUILD.
///
/// A backend can be selected only if its environment was complete at startup —
/// retained backends are inert until chosen here, and a provider key alone
/// still never selects cloud audio. Switching is refused while a call is
/// running or starting, because the public consent notice names the backend and
/// must not be overtaken by a change made mid-join.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "mode"
)]
pub async fn voice_mode(
    ctx: Context<'_>,
    #[description = "disabled, local, or openai. Omit to show the current mode."] mode: Option<
        String,
    >,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
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

    let current = runtime.effective_mode();
    let Some(requested) = mode else {
        ctx.say(clamp_message(format!(
            "Mode in force: `{}`\nSelected at startup: `{}`\nSelectable now: {}",
            current.label(),
            runtime.config.mode().label(),
            selectable_modes(&runtime),
        )))
        .await?;
        return Ok(());
    };

    // Reuse the environment parser so `/voice mode` accepts exactly what
    // ABBEY_VOICE_MODE accepts; a second, narrower parser here silently
    // rejected `off` and `offline`.
    let requested = match VoiceMode::parse(Some(requested)) {
        Ok(mode) => mode,
        Err(_) => {
            ctx.say("Mode must be `disabled`, `local`, or `openai`.")
                .await?;
            return Ok(());
        }
    };

    if requested == current {
        ctx.say(format!("Already in `{}`.", current.label()))
            .await?;
        return Ok(());
    }
    if runtime.config.backend_for(requested).is_none() {
        ctx.say(clamp_message(format!(
            "`{}` was not configured at startup, so Abbey holds no settings for it. Set its environment variables and restart. Selectable now: {}",
            requested.label(),
            selectable_modes(&runtime),
        )))
        .await?;
        return Ok(());
    }

    // Hold the transition lock across the check and the write. Reading a
    // snapshot without it would let a join that is already past its own mode
    // snapshot finish under the old backend while this reports the new one.
    let transition = runtime.transition.lock().await;
    let snapshot = runtime.snapshot().await;
    if let Some(blocker) = crate::voice_session::mode_switch_blocker(
        &snapshot,
        runtime.verification_active(),
        requested,
    ) {
        drop(transition);
        ctx.say(clamp_message(blocker)).await?;
        return Ok(());
    }
    // The blocker's `start_pending` read is advisory; this write is the
    // authoritative one, atomic with a join's reservation-plus-snapshot.
    if !runtime.switch_effective_mode_if_idle(requested) {
        drop(transition);
        ctx.say(
            "Voice is starting right now. Stop it with `/voice leave` before changing the backend.",
        )
        .await?;
        return Ok(());
    }
    drop(transition);

    ctx.say(format!(
        "Voice backend is now `{}`. It takes effect on the next `/voice join`.",
        requested.label(),
    ))
    .await?;
    Ok(())
}

/// The modes `/voice mode` would accept right now, for error and status text.
fn selectable_modes(runtime: &VoiceRuntime) -> String {
    let mut names = vec!["`disabled`"];
    if runtime.config.available_local().is_some() {
        names.push("`local`");
    }
    if runtime.config.available_openai().is_some() {
        names.push("`openai`");
    }
    names.join(", ")
}
