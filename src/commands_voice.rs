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

mod acknowledgement;
mod play;
use play::{voice_pause, voice_play, voice_resume_music, voice_stop_music, voice_volume};
mod auto_listen;
mod consent;
mod discord;
mod events;
mod receive;
mod start;
mod supervision;
use start::start_voice;
mod ux;
mod verification;

#[cfg(test)]
mod acknowledgement_tests;

use acknowledgement::{
    AcknowledgedContext, acknowledge_with_transition, authorize_and_close_media,
    with_acknowledged_context,
};
use discord::*;
use receive::{ReceiveHandlerInstall, install_receive_handlers};

#[cfg(test)]
pub(crate) struct VoiceLeaveTransitionProbe {
    pub(crate) entered: tokio::sync::Semaphore,
    pub(crate) release: tokio::sync::Semaphore,
}

#[cfg(test)]
impl VoiceLeaveTransitionProbe {
    pub(crate) fn new() -> Self {
        Self {
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        }
    }
}

#[cfg(test)]
pub(crate) struct VoiceLeaveTransitionProbeKey;

#[cfg(test)]
impl serenity::prelude::TypeMapKey for VoiceLeaveTransitionProbeKey {
    type Value = Arc<VoiceLeaveTransitionProbe>;
}

#[cfg(test)]
async fn hold_voice_leave_transition_for_test(ctx: &serenity::all::Context) {
    let probe = ctx
        .data
        .read()
        .await
        .get::<VoiceLeaveTransitionProbeKey>()
        .cloned();
    if let Some(probe) = probe {
        probe.entered.add_permits(1);
        probe.release.acquire().await.unwrap().forget();
    }
}

/// Select the exact disconnected guild call's decoder before joining. Changing
/// Songbird's shared defaults here would race another guild's call creation.
async fn configure_disconnected_call(call: &Arc<Mutex<songbird::Call>>, mode: VoiceMode) {
    call.lock().await.set_config(initial_songbird_config(mode));
}

pub use auto_listen::{AutoListenStartup, try_auto_listen_at_startup};
pub use consent::{voice_consent, voice_notice};
pub use events::on_gateway_event;
pub use supervision::autojoin_self_deafened;
pub use ux::dispatch_ux_component;
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
        "voice_play",
        "voice_pause",
        "voice_resume_music",
        "voice_stop_music",
        "voice_volume",
        "voice_join",
        "voice_resume",
        "voice_leave",
        "voice_status",
        "voice_diagnostics",
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
    with_acknowledged_context(
        ctx,
        |ctx| async move { start_voice(ctx, consent, false).await },
    )
    .await
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
    with_acknowledged_context(
        ctx,
        |ctx| async move { start_voice(ctx, consent, true).await },
    )
    .await
}

/// Stop processing synchronously and leave Discord voice.
#[poise::command(slash_command, guild_only, ephemeral, rename = "leave")]
pub async fn voice_leave(ctx: Context<'_>) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This command only works inside a server.").await?;
        return Ok(());
    };
    let Some(runtime) = ctx.data().voice_for(guild_id.get()) else {
        let permissions = match ctx {
            poise::Context::Application(application) => application
                .interaction
                .member
                .as_deref()
                .and_then(|member| member.permissions),
            poise::Context::Prefix(_) => None,
        };
        if !can_stop_voice(false, permissions) {
            ctx.say(
                "Only a server manager can cancel a voice start before its channel is prepared.",
            )
            .await?;
            return Ok(());
        }
        // A first join may still be validating Discord before any runtime is
        // published. Revoke its registry generation before this first await.
        ctx.data().cancel_voice_join(guild_id.get());
        ctx.say("Any pending voice start in this server was cancelled; no prepared voice session remains.").await?;
        return Ok(());
    };
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
        match ctx {
            poise::Context::Application(application) => application
                .interaction
                .member
                .as_deref()
                .and_then(|member| member.permissions),
            poise::Context::Prefix(_) => None,
        }
    };
    let Some(closed_media) = authorize_and_close_media(
        || can_stop_voice(present, interaction_permissions),
        || {
            ctx.data().cancel_voice_join(guild_id.get());
            runtime.music.stop(
                "voice leave",
                crate::voice_session::PlaybackTermination::Stopped,
            );
            runtime.cancel_pending_start();
        },
    ) else {
        ctx.say("Only someone currently in the configured voice channel or a member with Manage Server can stop Abbey voice.")
            .await?;
        return Ok(());
    };
    // Bind completion evidence to the run that was armed when this authorized
    // leave began. A later verifier must not inherit this leave's result.
    let verification_run = runtime.verification_run_token();
    // The slow-start token and software gate are already closed. Start the
    // acknowledgement while Songbird lookup, transition-lock acquisition and
    // physical teardown run, so neither side delays the other.
    let transition_work = async {
        #[cfg(test)]
        hold_voice_leave_transition_for_test(ctx.serenity_context()).await;
        let manager = match songbird::get(ctx.serenity_context()).await {
            Some(manager) => manager,
            None => {
                runtime
                    .disconnect("voice stopped; Songbird runtime was unavailable")
                    .await;
                return Err::<(), Error>(
                    "Songbird was not registered in the Discord client".into(),
                );
            }
        };
        let exact_call = manager.get(guild_id);
        // Enqueue the transition lock immediately so any later `/voice join`
        // waits behind this stop, while leaving the exact current Decode Call
        // in parallel instead of waiting behind an older start's network work.
        let leave_exact = async {
            if let Some(call) = exact_call {
                pause_call_for_consent(&call).await;
            }
        };
        let (transition, ()) = tokio::join!(runtime.transition.lock(), leave_exact);
        if let Some(call) = manager.get(guild_id) {
            // Stop the Decode driver of any call that replaced the captured
            // handle before the transition became ours.
            pause_call_for_consent(&call).await;
        }
        runtime
            .disconnect("configured; disconnected by /voice leave")
            .await;
        let removed = manager.remove(guild_id).await;
        let result = match removed {
            Ok(()) | Err(songbird::error::JoinError::NoCall) => {
                if let Some(run) = verification_run {
                    let _ = runtime.note_verification_final_leave(run);
                }
                ctx.data()
                    .retire_voice_after_leave(guild_id.get(), &runtime);
                Ok(())
            }
            Err(error) => Err(error.into()),
        };
        drop(transition);
        result
    };
    let (deferred, transition_result) =
        acknowledge_with_transition(closed_media, ctx.defer_ephemeral(), transition_work).await;
    transition_result?;
    deferred?;
    ctx.say("Left voice. Capture, provider work, queued audio, and playback are stopped.")
        .await?;
    Ok(())
}

/// Show the member-safe voice state without operational diagnostics.
#[poise::command(slash_command, guild_only, ephemeral, rename = "status")]
pub async fn voice_status(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx
        .guild_id()
        .and_then(|guild| ctx.data().voice_for(guild.get()))
    else {
        ctx.say("No voice session is prepared in this server. A manager in a voice channel can use /voice join first.")
            .await?;
        return Ok(());
    };
    let effective_mode = runtime.effective_mode();
    let snapshot = runtime.snapshot().await;
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let permissions = crate::commands_help::current_permissions(
        ctx.serenity_context(),
        GuildId::new(runtime.config.guild_id),
        channel_id,
        ctx.author().id,
    )
    .await
    .unwrap_or_default();
    let caller_present = cached_participants_from_serenity(
        ctx.serenity_context(),
        GuildId::new(runtime.config.guild_id),
        channel_id,
    )
    .is_some_and(|users| users.contains(&ctx.author().id.get()));
    let view = crate::voice_views::MemberVoiceView::project(crate::voice_views::MemberVoiceInput {
        configured: true,
        phase: Some(snapshot.phase),
        mode: effective_mode,
        caller_agrees: runtime
            .consent
            .agrees(ctx.author().id.get(), effective_mode),
        channel_id: Some(runtime.config.channel_id),
        caller_can_view_channel: permissions.contains(serenity::all::Permissions::VIEW_CHANNEL),
        caller_present,
        caller_can_manage: permissions.contains(serenity::all::Permissions::MANAGE_GUILD),
    });
    ctx.say(clamp_message(view.render())).await?;
    Ok(())
}

/// Show content-free operational detail to current server managers.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "diagnostics"
)]
pub async fn voice_diagnostics(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx
        .guild_id()
        .and_then(|guild| ctx.data().voice_for(guild.get()))
    else {
        ctx.say("No voice session is prepared in this server. A manager in a voice channel can use /voice join first.")
            .await?;
        return Ok(());
    };
    let snapshot = runtime.snapshot().await;
    let effective_mode = runtime.effective_mode();
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
    let verifier = runtime.verification_snapshot().map_or_else(
        || "not armed".into(),
        |state| {
            format!(
                "{}; {} of 8 checks",
                state.status.label(),
                state.observed_checks()
            )
        },
    );
    let view = crate::voice_views::AdminVoiceView::project(crate::voice_views::AdminVoiceInput {
        music_status: runtime.music.status(),
        phase: format!("{} ({})", snapshot.phase.label(), snapshot.status),
        media_gate_open: snapshot.media_enabled,
        pending_start: snapshot.start_pending,
        selected_mode: effective_mode.label().into(),
        configured_modes: selectable_modes_raw(&runtime),
        consent_epoch: snapshot.consent_epoch,
        session_epoch: snapshot.epoch,
        participant_count: snapshot.participant_count,
        dropped_input: snapshot.dropped_input,
        aborted_overruns: snapshot.aborted_overruns,
        barge_ins: snapshot.barge_ins,
        completed_turns: snapshot.completed_turns,
        speech_models,
        sidecar_readiness: sidecar,
        text_backend_readiness: loopback_llm,
        verifier,
    });
    ctx.say(clamp_message(view.0)).await?;
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
    #[description = "Off, Local, or OpenAI. Omit to show the current mode."] mode: Option<
        VoiceModeChoice,
    >,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx
        .guild_id()
        .and_then(|guild| ctx.data().voice_for(guild.get()))
    else {
        ctx.say("No voice session is prepared in this server. A manager in a voice channel can use /voice join first.")
            .await?;
        return Ok(());
    };

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

    let requested: VoiceMode = requested.into();

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
    if let Err(refusal) = runtime.switch_effective_mode(requested, snapshot.phase) {
        drop(transition);
        ctx.say(clamp_message(refusal.message())).await?;
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

fn selectable_modes_raw(runtime: &VoiceRuntime) -> Vec<String> {
    let mut modes = vec!["Off".into()];
    if runtime.config.available_local().is_some() {
        modes.push("Local".into());
    }
    if runtime.config.available_openai().is_some() {
        modes.push("OpenAI".into());
    }
    modes
}

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum VoiceModeChoice {
    #[name = "Off"]
    Off,
    #[name = "Local"]
    Local,
    #[name = "OpenAI"]
    OpenAi,
}

impl From<VoiceModeChoice> for VoiceMode {
    fn from(value: VoiceModeChoice) -> Self {
        match value {
            VoiceModeChoice::Off => Self::Disabled,
            VoiceModeChoice::Local => Self::Local,
            VoiceModeChoice::OpenAi => Self::OpenAi,
        }
    }
}
