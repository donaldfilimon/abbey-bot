//! Auto-listen upgrade after muted autojoin when consent already covers the room.
//!
//! This reuses the same Songbird/local-session helpers as `/voice join consent:true`
//! (`configure_disconnected_call`, receive handlers, `enable_conversation`, local
//! actor). It is not a second voice stack.

use std::collections::HashSet;
use std::sync::Arc;

use serenity::all::{ChannelId, GuildId};
use tokio::sync::{Mutex, mpsc, watch};

use super::discord::*;
use super::receive::{ReceiveHandlerInstall, install_receive_handlers};
use super::{INPUT_QUEUE_FRAMES, LOCAL_HEALTH_TIMEOUT, StartAttempt, configure_disconnected_call};
use crate::offline_voice::MlxAudioClient;
use crate::voice::{VoiceBackendConfig, VoiceMode};
use crate::voice_local::LocalSession;
use crate::voice_session::{SessionControl, SharedPlayback, VerificationActivation, VoiceRuntime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoListenDecision {
    Disabled,
    WrongMode(VoiceMode),
    EmptyChannel,
    ConsentUnavailable(&'static str),
    ConsentIncomplete(Vec<u64>),
    Ready,
}

pub(crate) fn decide_auto_listen(
    enabled: bool,
    mode: VoiceMode,
    participants: &HashSet<u64>,
    coverage: Result<Vec<u64>, &'static str>,
) -> AutoListenDecision {
    if !enabled {
        return AutoListenDecision::Disabled;
    }
    if mode != VoiceMode::Local {
        return AutoListenDecision::WrongMode(mode);
    }
    if participants.is_empty() {
        return AutoListenDecision::EmptyChannel;
    }
    match coverage {
        Err(message) => AutoListenDecision::ConsentUnavailable(message),
        Ok(missing) if missing.is_empty() => AutoListenDecision::Ready,
        Ok(missing) => AutoListenDecision::ConsentIncomplete(missing),
    }
}

fn auto_listen_enabled() -> bool {
    std::env::var("ABBEY_VOICE_AUTO_LISTEN")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoListenStartup {
    /// Consented Local listening is active.
    Listening,
    /// Caller should keep/establish muted autojoin presence.
    MutedPresence,
}

/// Startup entry for `ABBEY_VOICE_AUTOJOIN` + optional `ABBEY_VOICE_AUTO_LISTEN`.
/// When auto-listen is enabled and Local consent already covers everyone present,
/// start the same listening path as `/voice join consent:true` without a prior
/// muted Pass-mode bounce. Otherwise ask the caller to take muted autojoin.
pub async fn try_auto_listen_at_startup(
    ctx: &serenity::all::Context,
    runtime: Arc<VoiceRuntime>,
    state: Arc<crate::runtime::AppState>,
) -> Result<AutoListenStartup, String> {
    let enabled = auto_listen_enabled();
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let participants =
        cached_participants_from_serenity(ctx, guild_id, channel_id).unwrap_or_default();
    let mode = runtime.effective_mode();
    let coverage = runtime.consent.coverage(&participants, VoiceMode::Local);
    let decision = decide_auto_listen(enabled, mode, &participants, coverage);
    match decision {
        AutoListenDecision::Disabled => Ok(AutoListenStartup::MutedPresence),
        AutoListenDecision::WrongMode(mode) => {
            tracing::info!(
                %guild_id,
                %channel_id,
                mode = mode.label(),
                "ABBEY_VOICE_AUTO_LISTEN=1 ignored because effective mode is not Local; using muted/self-deafened autojoin"
            );
            Ok(AutoListenStartup::MutedPresence)
        }
        AutoListenDecision::EmptyChannel => {
            tracing::info!(
                %guild_id,
                %channel_id,
                "ABBEY_VOICE_AUTO_LISTEN=1: no non-bot members in the voice channel; using muted/self-deafened autojoin"
            );
            Ok(AutoListenStartup::MutedPresence)
        }
        AutoListenDecision::ConsentUnavailable(message) => {
            tracing::warn!(
                %guild_id,
                %channel_id,
                message,
                "ABBEY_VOICE_AUTO_LISTEN=1: consent store unavailable; using muted/self-deafened autojoin"
            );
            Ok(AutoListenStartup::MutedPresence)
        }
        AutoListenDecision::ConsentIncomplete(missing) => {
            tracing::info!(
                %guild_id,
                %channel_id,
                missing = ?missing,
                present = participants.len(),
                "ABBEY_VOICE_AUTO_LISTEN=1: Local consent incomplete; using muted/self-deafened autojoin until unanimous coverage"
            );
            Ok(AutoListenStartup::MutedPresence)
        }
        AutoListenDecision::Ready => {
            tracing::info!(
                %guild_id,
                %channel_id,
                present = participants.len(),
                "ABBEY_VOICE_AUTO_LISTEN=1: unanimous Local consent present; starting consented listening"
            );
            activate_local_auto_listen(ctx, runtime, state, participants).await?;
            Ok(AutoListenStartup::Listening)
        }
    }
}

async fn activate_local_auto_listen(
    ctx: &serenity::all::Context,
    runtime: Arc<VoiceRuntime>,
    state: Arc<crate::runtime::AppState>,
    participants: HashSet<u64>,
) -> Result<(), String> {
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);

    verify_required_voice_permissions_live(ctx, guild_id, channel_id).await?;

    let operation = runtime.start_operation_token();
    let Some((start_generation, effective_backend)) = runtime.reserve_start_with_backend(operation)
    else {
        return Err(
            "auto-listen start reservation was cancelled before activation; muted presence kept"
                .into(),
        );
    };
    let _start_attempt = StartAttempt {
        runtime: Arc::clone(&runtime),
        generation: start_generation,
    };
    let Some(effective_backend) = effective_backend else {
        return Err("Local voice backend is not configured; auto-listen aborted".into());
    };
    if effective_backend.mode() != VoiceMode::Local {
        return Err(format!(
            "auto-listen requires Local mode; effective backend was {}",
            effective_backend.mode().label()
        ));
    }
    let VoiceBackendConfig::Local(config) = &effective_backend else {
        return Err("Local speech configuration is incomplete for auto-listen".into());
    };

    if !runtime
        .consent
        .coverage(&participants, VoiceMode::Local)
        .is_ok_and(|missing| missing.is_empty())
    {
        return Err("Local consent coverage changed before auto-listen activation".into());
    }

    let backend = select_local_backend(&state)?;
    let client = MlxAudioClient::new(config.clone())
        .map_err(|error| format!("Local speech is not ready: {}", public_error(&error)))?;
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
        return Err("auto-listen cancelled while local models were preparing".into());
    };
    match prepared {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(format!(
                "Local speech is not ready: {}",
                public_error(&error)
            ));
        }
        Err(_) => {
            return Err(
                "Local speech models did not become ready within ten minutes during auto-listen"
                    .into(),
            );
        }
    }

    if !runtime.start_is_current(start_generation) {
        return Err("auto-listen cancelled before Discord join".into());
    }
    let transition = runtime.transition.lock().await;
    if !runtime.start_is_current(start_generation) {
        drop(transition);
        return Err("auto-listen superseded before Discord join".into());
    }
    if let Err(error) = verify_required_voice_permissions_live(ctx, guild_id, channel_id).await {
        drop(transition);
        return Err(error);
    }
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| "Songbird was not registered in the Discord client".to_string())?;
    runtime
        .disconnect_for_replace("auto-listen replacing muted autojoin with consented listening")
        .await;
    let old_voice_session =
        cached_bot_voice_state_from_serenity(ctx, guild_id).map(|state| state.session_id);
    if let Some(old_session_id) = old_voice_session.as_ref() {
        runtime.remember_retired_discord_session(old_session_id.clone());
    }
    if let Some(existing) = manager.get(guild_id) {
        let _ = set_muted_self_deafened(&existing).await;
        manager
            .remove(guild_id)
            .await
            .map_err(|error| format!("replacing muted autojoin failed: {error}"))?;
    }
    if let Some(old_session_id) = old_voice_session.as_deref() {
        wait_for_voice_session_gone(ctx, guild_id, old_session_id).await?;
    }
    if !runtime.start_is_current(start_generation) {
        drop(transition);
        return Err("auto-listen cancelled while leaving muted presence".into());
    }

    let prepared_call = manager.get_or_insert(guild_id);
    configure_disconnected_call(&prepared_call, VoiceMode::Local).await;
    if let Err(error) = set_muted_self_deafened(&prepared_call).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "could not prepare the required muted/self-deafened state",
                crate::observability::OperationalErrorCategory::Protocol,
            )
            .await;
        drop(transition);
        return Err(format!(
            "Could not prepare the required voice safety state: {error}"
        ));
    }
    let epoch = runtime.begin(participants.clone()).await;
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
            return Err(format!("Discord refused the voice join: {error}"));
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
        return Err(format!(
            "Joined Discord but could not establish the required safety state: {error}"
        ));
    }
    let joined_session_id = match wait_for_bot_voice_state(ctx, guild_id, channel_id).await {
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
            return Err(error);
        }
    };
    if !runtime.start_is_current(start_generation)
        || !runtime
            .bind_discord_session(epoch, joined_session_id.clone())
            .await
    {
        let _ = manager.remove(guild_id).await;
        runtime
            .disconnect_for_replace("auto-listen cancelled before Discord session binding")
            .await;
        drop(transition);
        return Err("auto-listen cancelled before Discord session binding".into());
    }

    let notice = consent_notice(VoiceMode::Local, channel_id, false);
    if let Err(error) = channel_id.say(&ctx.http, notice).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "public consent disclosure could not be posted",
                crate::observability::OperationalErrorCategory::Protocol,
            )
            .await;
        drop(transition);
        return Err(format!(
            "Could not post the required public consent notice, so voice stayed off: {error}"
        ));
    }

    let pre_enable_participants = cached_participants_from_serenity(ctx, guild_id, channel_id)
        .ok_or_else(|| {
            "Discord's voice-state cache is not ready during auto-listen; no audio was enabled."
                .to_string()
        })?;
    if pre_enable_participants != participants {
        remove_call_for_consent(&manager, guild_id).await;
        runtime.pause_for_consent(pre_enable_participants).await;
        drop(transition);
        let _ = channel_id
            .say(
                &ctx.http,
                "Abbey disconnected because channel membership changed during auto-listen startup. Notify everyone now present, then use `/voice resume consent:true`.",
            )
            .await;
        return Err("channel membership changed before auto-listen activation".into());
    }

    let (events, lifecycle) = mpsc::unbounded_channel();
    if *driver_disconnect.borrow() {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "Discord voice transport disconnected during startup",
                crate::observability::OperationalErrorCategory::Unavailable,
            )
            .await;
        drop(transition);
        return Err("Discord voice transport disconnected during auto-listen startup".into());
    }
    let playback: SharedPlayback = Arc::new(Mutex::new(None));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let session = LocalSession {
        runtime: Arc::clone(&runtime),
        state,
        call: Arc::clone(&call),
        client,
        epoch,
        input,
        lifecycle,
        events,
        driver_disconnect,
        cancel: cancel_rx,
        playback: Arc::clone(&playback),
        backend,
    };
    let task = runtime
        .spawn_actor(cancel_tx.clone(), crate::voice_local::run(session))
        .map_err(|message| message.to_string())?;
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
        return Err("voice session changed while auto-listen was starting".into());
    }

    if !runtime.start_is_current(start_generation) || !runtime.is_current(epoch) {
        let _ = manager.remove(guild_id).await;
        runtime
            .disconnect_for_replace("auto-listen superseded before activation")
            .await;
        drop(transition);
        return Err("auto-listen superseded before activation".into());
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
        return Err("Discord moved or disconnected Abbey during auto-listen startup".into());
    }

    let latest_participants = cached_participants_from_serenity(ctx, guild_id, channel_id)
        .ok_or_else(|| {
            "Discord's voice-state cache is not ready before auto-listen activation".to_string()
        })?;
    if latest_participants != participants {
        remove_call_for_consent(&manager, guild_id).await;
        runtime.pause_for_consent(latest_participants).await;
        drop(transition);
        let _ = channel_id
            .say(
                &ctx.http,
                "Abbey disconnected because channel membership changed during auto-listen startup. Notify everyone now present, then use `/voice resume consent:true`.",
            )
            .await;
        return Err("channel membership changed before auto-listen enable_conversation".into());
    }

    if let Err(error) = verify_required_voice_permissions_live(ctx, guild_id, channel_id).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "required Discord voice permissions could not be verified before activation",
                crate::observability::OperationalErrorCategory::Authorization,
            )
            .await;
        drop(transition);
        return Err(error);
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
        return Err(format!(
            "public notice posted, but Discord could not enable the consented session: {error}"
        ));
    }

    if let Err(error) = wait_for_enabled_bot_voice_state_from_serenity(
        ctx,
        guild_id,
        channel_id,
        joined_session_id.as_str(),
    )
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
        return Err(error);
    }

    runtime.arm_unmute_grace(std::time::Duration::from_secs(3));

    if let Err(error) = verify_required_voice_permissions_live(ctx, guild_id, channel_id).await {
        let _ = manager.remove(guild_id).await;
        runtime
            .fail_safe(
                "required Discord voice permissions changed during activation",
                crate::observability::OperationalErrorCategory::Authorization,
            )
            .await;
        drop(transition);
        return Err(error);
    }

    let post_enable_participants = cached_participants_from_serenity(ctx, guild_id, channel_id)
        .ok_or_else(|| {
            "Discord's voice-state cache is not ready after auto-listen activation".to_string()
        })?;
    if post_enable_participants != participants {
        remove_call_for_consent(&manager, guild_id).await;
        runtime.pause_for_consent(post_enable_participants).await;
        drop(transition);
        let _ = channel_id
            .say(
                &ctx.http,
                "Abbey disconnected immediately because channel membership changed during auto-listen startup. Notify everyone now present, then use `/voice resume consent:true`.",
            )
            .await;
        return Err("channel membership changed during auto-listen activation".into());
    }

    if !runtime
        .activate_verified(
            epoch,
            start_generation,
            "local inference ready; listening for Abbey",
            VerificationActivation {
                manager_authorized: true,
                caller_present: true,
                participant_count: participants.len(),
                resumed: false,
            },
        )
        .await
    {
        remove_call_for_consent(&manager, guild_id).await;
        if runtime.is_current(epoch) {
            runtime.pause_for_consent(participants).await;
        }
        drop(transition);
        return Err("voice session changed at auto-listen activation".into());
    }
    drop(transition);
    tracing::info!(
        %guild_id,
        %channel_id,
        "ABBEY_VOICE_AUTO_LISTEN upgraded muted autojoin to Local listening"
    );
    let _ = channel_id
        .say(
            &ctx.http,
            format!(
                "Joined <#{channel_id}> with {} via ABBEY_VOICE_AUTO_LISTEN. The public consent notice is posted; `/voice status` shows health and `/voice leave` stops processing.",
                VoiceMode::Local.label()
            ),
        )
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_listen_requires_env_local_mode_members_and_complete_consent() {
        let members = HashSet::from([1122140354737623110]);
        assert_eq!(
            decide_auto_listen(false, VoiceMode::Local, &members, Ok(vec![])),
            AutoListenDecision::Disabled
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::OpenAi, &members, Ok(vec![])),
            AutoListenDecision::WrongMode(VoiceMode::OpenAi)
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &HashSet::new(), Ok(vec![])),
            AutoListenDecision::EmptyChannel
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &members, Err("unavailable")),
            AutoListenDecision::ConsentUnavailable("unavailable")
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &members, Ok(vec![99])),
            AutoListenDecision::ConsentIncomplete(vec![99])
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &members, Ok(vec![])),
            AutoListenDecision::Ready
        );
    }
}
