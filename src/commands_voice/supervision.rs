//! Voice lifecycle supervision driven by Discord gateway evidence.

use std::collections::HashSet;
use std::sync::Arc;

use serenity::all::{ChannelId, ChannelType, GuildId, VoiceState};

use super::discord::{
    bot_has_required_voice_permissions, cached_bot_voice_state_from_serenity,
    cached_participants_from_serenity, no_audio_songbird_config, pause_call_for_consent,
    remove_call_for_consent, set_muted_self_deafened, wait_for_bot_voice_state,
    wait_for_voice_session_gone,
};
use crate::voice_session::{DiscordSessionEvent, VoicePhase, VoiceRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BotVoiceImpact {
    Healthy,
    Adverse(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct BotVoiceFacts {
    in_configured_channel: bool,
    mute: bool,
    deaf: bool,
    suppress: bool,
    self_mute: bool,
    self_deaf: bool,
    media_open: bool,
}

fn classify_bot_voice_payload(facts: BotVoiceFacts) -> BotVoiceImpact {
    if !facts.in_configured_channel {
        return BotVoiceImpact::Adverse("Discord moved or disconnected Abbey; audio stopped");
    }
    if facts.mute || facts.deaf || facts.suppress {
        return BotVoiceImpact::Adverse(
            "Discord muted, deafened, or suppressed Abbey; audio stopped",
        );
    }
    if facts.media_open && (facts.self_mute || facts.self_deaf) {
        return BotVoiceImpact::Adverse(
            "Discord reported Abbey self-muted or self-deafened during active voice; audio stopped",
        );
    }
    BotVoiceImpact::Healthy
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
    if let Some(old_session_id) = old_voice_session.as_ref() {
        runtime.remember_retired_discord_session(old_session_id.clone());
    }
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
    let session_id = match wait_for_bot_voice_state(ctx, guild_id, channel_id).await {
        Ok(session_id) => session_id,
        Err(error) => {
            let _ = manager.remove(guild_id).await;
            runtime
                .fail_safe("Discord did not confirm safe no-audio presence")
                .await;
            return Err(error);
        }
    };
    runtime
        .set_presence_with_discord_session(
            session_id,
            "connected muted/self-deafened; media decoding is disabled",
        )
        .await;
    tracing::info!(guild = %guild_id, channel = %channel_id, "joined Discord voice with decryption/decoding and transmission disabled");
    Ok(())
}

/// Tear down the exact epoch already revoked by an authenticated member choice.
/// A newer cache or replacement call cannot erase or expand that authority.
pub(super) async fn stop_voice_for_withdrawal(
    ctx: &serenity::all::Context,
    runtime: &VoiceRuntime,
    epoch: u64,
) -> bool {
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let participants = cached_participants_from_serenity(ctx, guild_id, channel_id);
    let snapshot = runtime.snapshot().await;
    if snapshot.epoch != epoch
        || !matches!(
            snapshot.phase,
            VoicePhase::Connecting
                | VoicePhase::Listening
                | VoicePhase::Thinking
                | VoicePhase::Speaking
        )
    {
        return true;
    }
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
        runtime
            .music_consent_teardown_complete(epoch.saturating_add(1))
            .await;
    }
    drop(transition);
    true
}

/// Preserve the actual VoiceStateUpdate payload as revocation evidence. Only
/// an event for a session already retired by a newer runtime epoch is ignored.
pub(super) async fn on_voice_state_update(
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
        let epoch = runtime.current_epoch();
        let impact = classify_bot_voice_payload(BotVoiceFacts {
            in_configured_channel: new.channel_id == Some(channel_id),
            mute: new.mute,
            deaf: new.deaf,
            suppress: new.suppress,
            self_mute: new.self_mute,
            self_deaf: new.self_deaf,
            media_open: runtime.media_enabled(epoch),
        });
        let BotVoiceImpact::Adverse(reason) = impact else {
            return;
        };

        // No await occurs before session classification and gate revocation.
        let event = runtime.revoke_for_discord_session(&new.session_id);
        let epoch = match event {
            DiscordSessionEvent::Retired => return,
            DiscordSessionEvent::Current { epoch, .. } => epoch,
            DiscordSessionEvent::Unknown { epoch, .. } => {
                tracing::warn!(session = %new.session_id, epoch, "adverse bot voice event had no current/retired binding; failing closed");
                epoch
            }
        };
        stop_for_bot_payload(ctx, runtime, guild_id, channel_id, epoch, reason).await;
        return;
    }

    let joined_target = new.channel_id == Some(channel_id)
        && old.as_ref().and_then(|state| state.channel_id) != Some(channel_id);
    if !joined_target {
        return;
    }
    // The join payload itself is evidence. A newer cache where the user has
    // already left must not erase a transient consent boundary. Attestation
    // and revocation share one critical section so an intervening replacement
    // cannot be revoked by a delayed event for one of its known participants.
    let Some(epoch) = runtime.revoke_for_unattested_participant(new.user_id.get()) else {
        return;
    };
    let snapshot = runtime.snapshot().await;
    if snapshot.epoch != epoch
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
    let mut participants =
        cached_participants_from_serenity(ctx, guild_id, channel_id).unwrap_or_default();
    participants.insert(new.user_id.get());
    stop_for_new_participant(ctx, runtime, guild_id, channel_id, epoch, participants).await;
}

async fn stop_for_bot_payload(
    ctx: &serenity::all::Context,
    runtime: Arc<VoiceRuntime>,
    guild_id: GuildId,
    channel_id: ChannelId,
    epoch: u64,
    reason: &'static str,
) {
    let snapshot = runtime.snapshot().await;
    if snapshot.epoch != epoch
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
    let manager = songbird::get(ctx).await;
    let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
    let Some(pause) = runtime
        .begin_pause_epoch_for_consent(epoch, HashSet::new(), reason)
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
    if current.epoch != epoch.saturating_add(1) || current.phase != VoicePhase::AwaitingConsent {
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
}

async fn stop_for_new_participant(
    ctx: &serenity::all::Context,
    runtime: Arc<VoiceRuntime>,
    guild_id: GuildId,
    channel_id: ChannelId,
    epoch: u64,
    participants: HashSet<u64>,
) {
    let manager = songbird::get(ctx).await;
    let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
    let Some(pause) = runtime
        .begin_participant_pause_epoch_for_consent(
            epoch,
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
    let current = runtime.snapshot().await;
    if current.epoch != epoch.saturating_add(1) || current.phase != VoicePhase::AwaitingConsent {
        drop(transition);
        return;
    }
    if let Some(manager) = manager {
        remove_call_for_consent(&manager, guild_id).await;
    }
    runtime
        .music_consent_teardown_complete(epoch.saturating_add(1))
        .await;
    drop(transition);
    let _ = channel_id
        .say(
            &ctx.http,
            "Abbey disconnected listening because someone new joined. Discord input and spoken replies are stopped; separately enabled music may continue. Each uncovered member can choose Agree in `/voice consent` or the pinned voice notice. Saved choices still count; a manager can then use `/voice resume consent:true`.",
        )
        .await;
}

/// Re-evaluate permissions, using `payload_revoked` when the gateway payload
/// itself proves a relevant loss. That proof closes gates before the first
/// await; a later cache recovery cannot erase it.
pub(super) async fn on_voice_permissions_changed(
    ctx: &serenity::all::Context,
    affected_guild_id: GuildId,
    affected_channel_id: Option<ChannelId>,
    payload_revoked: bool,
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

    let epoch = if payload_revoked {
        runtime.revoke_for_external_event()
    } else {
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
        runtime.revoke_for_external_event()
    };
    runtime.music.stop(
        "required Discord permissions changed",
        crate::voice_session::PlaybackTermination::Errored,
    );
    let snapshot = runtime.snapshot().await;
    if snapshot.epoch != epoch
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
    let manager = songbird::get(ctx).await;
    let exact_call = manager.as_ref().and_then(|manager| manager.get(guild_id));
    let reason = "required Discord voice permissions changed; audio stopped";
    let Some(pause) = runtime
        .begin_pause_epoch_for_consent(epoch, HashSet::new(), reason)
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
    if current.epoch != epoch.saturating_add(1) || current.phase != VoicePhase::AwaitingConsent {
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
            "Abbey stopped voice because she needs View Channel, Send Messages, Connect, Speak, Stream, and Use Embedded Activities for a public bidirectional session. Restore those permissions, then use `/voice join consent:true`.",
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> BotVoiceFacts {
        BotVoiceFacts {
            in_configured_channel: true,
            mute: false,
            deaf: false,
            suppress: false,
            self_mute: false,
            self_deaf: false,
            media_open: true,
        }
    }

    #[test]
    fn every_adverse_payload_fact_is_preserved_as_revocation_evidence() {
        assert_eq!(
            classify_bot_voice_payload(healthy()),
            BotVoiceImpact::Healthy
        );
        for mutate in [
            |facts: &mut BotVoiceFacts| facts.in_configured_channel = false,
            |facts: &mut BotVoiceFacts| facts.mute = true,
            |facts: &mut BotVoiceFacts| facts.deaf = true,
            |facts: &mut BotVoiceFacts| facts.suppress = true,
            |facts: &mut BotVoiceFacts| facts.self_mute = true,
            |facts: &mut BotVoiceFacts| facts.self_deaf = true,
        ] {
            let mut facts = healthy();
            mutate(&mut facts);
            assert!(matches!(
                classify_bot_voice_payload(facts),
                BotVoiceImpact::Adverse(_)
            ));
        }
    }

    #[test]
    fn expected_pre_activation_self_flags_do_not_cancel_a_closed_gate() {
        let mut facts = healthy();
        facts.media_open = false;
        facts.self_mute = true;
        facts.self_deaf = true;
        assert_eq!(classify_bot_voice_payload(facts), BotVoiceImpact::Healthy);
    }
}
