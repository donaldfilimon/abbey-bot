//! Discord facts and Songbird call operations shared by voice commands and
//! gateway supervision.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serenity::all::{ChannelId, GuildChannel, GuildId, Member, Permissions, VoiceState};
use tokio::sync::Mutex;

use crate::Context;
use crate::voice::VoiceMode;

pub(super) fn select_local_backend(
    state: &crate::runtime::AppState,
) -> Result<crate::provider::ProviderId, String> {
    state.providers.local_voice_route()
        .ok_or_else(|| {
            "Local voice requires a loopback ABBEY_BOT_LLM_ENDPOINT; it will not send transcripts to a remote text provider.".into()
        })
}

pub(super) fn required_voice_permissions() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::STREAM
        | Permissions::USE_EMBEDDED_ACTIVITIES
}

pub(super) fn bot_has_required_voice_permissions(
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

pub(super) fn member_has_required_voice_permissions(
    ctx: &serenity::all::Context,
    channel: &GuildChannel,
    member: &Member,
) -> bool {
    ctx.cache.guild(channel.guild_id).is_some_and(|guild| {
        guild
            .user_permissions_in(channel, member)
            .contains(required_voice_permissions())
    })
}

/// Why a live permission revalidation failed.
///
/// The four failure paths are operationally different and only one of them is
/// an authorization fact. Returning a bare string forced every caller to guess,
/// and they all guessed `Authorization` — so a rate-limited REST call reported
/// itself as a permission denial, sending an operator to the Discord role
/// editor for a network problem.
#[derive(Debug)]
pub(super) struct VoicePermissionError {
    message: String,
    category: crate::observability::OperationalErrorCategory,
}

impl VoicePermissionError {
    fn new(
        message: impl Into<String>,
        category: crate::observability::OperationalErrorCategory,
    ) -> Self {
        Self {
            message: message.into(),
            category,
        }
    }

    /// Closed-vocabulary cause, for the operational event.
    pub(super) fn category(&self) -> crate::observability::OperationalErrorCategory {
        self.category
    }
}

impl std::fmt::Display for VoicePermissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VoicePermissionError {}

/// Existing callers surface the message to the operator and do not care about
/// the category; this keeps their `?` and `return Err(..)` unchanged.
impl From<VoicePermissionError> for String {
    fn from(error: VoicePermissionError) -> Self {
        error.message
    }
}

/// Fetch channel, member, and roles directly to close preflight/activation
/// TOCTOU windows. Passing the HTTP client deliberately bypasses the cache.
pub(super) async fn verify_required_voice_permissions_live(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<(), VoicePermissionError> {
    let bot_id = ctx.cache.current_user().id;
    let fetched = tokio::try_join!(
        channel_id.to_channel(&ctx.http),
        guild_id.member(&ctx.http, bot_id),
        guild_id.to_partial_guild(&ctx.http),
    );
    let (channel, member, guild) = fetched.map_err(|error| {
        tracing::warn!(%error, %guild_id, %channel_id, "could not revalidate Discord voice permissions");
        // The read did not happen. Nothing here says anything about the bot's
        // permissions, so this must not be reported as a denial.
        VoicePermissionError::new(
            "Discord could not revalidate Abbey's View Channel, Send Messages, Connect, Speak, Stream, and Use Embedded Activities permissions; voice stayed off.",
            crate::observability::OperationalErrorCategory::Unavailable,
        )
    })?;
    let Some(channel) = channel.guild() else {
        // The configured destination is wrong, not forbidden.
        return Err(VoicePermissionError::new(
            "The configured voice destination is no longer a server channel; voice stayed off.",
            crate::observability::OperationalErrorCategory::Configuration,
        ));
    };
    if channel.guild_id != guild_id || channel.id != channel_id {
        return Err(VoicePermissionError::new(
            "The configured voice destination changed unexpectedly; voice stayed off.",
            crate::observability::OperationalErrorCategory::Configuration,
        ));
    }
    if !guild
        .user_permissions_in(&channel, &member)
        .contains(required_voice_permissions())
    {
        // The only genuine authorization failure of the four.
        return Err(VoicePermissionError::new(
            "Abbey needs View Channel, Send Messages, Connect, Speak, Stream, and Use Embedded Activities in the configured voice channel; voice stayed off.",
            crate::observability::OperationalErrorCategory::Authorization,
        ));
    }
    Ok(())
}

pub(super) fn cached_participants(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<(bool, HashSet<u64>), String> {
    let guild = ctx.guild().ok_or_else(|| {
        "Discord's voice-state cache is not ready; no audio was enabled.".to_string()
    })?;
    if guild.id != guild_id {
        return Err("Discord's configured guild cache is not ready; no audio was enabled.".into());
    }
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
    Ok((caller_present, participants))
}

pub(super) fn cached_participants_from_serenity(
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

pub(super) fn cached_bot_voice_state(ctx: Context<'_>, guild_id: GuildId) -> Option<VoiceState> {
    let bot_id = ctx.serenity_context().cache.current_user().id;
    ctx.guild()
        .filter(|guild| guild.id == guild_id)
        .and_then(|guild| guild.voice_states.get(&bot_id).cloned())
}

pub(super) fn cached_bot_voice_state_from_serenity(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
) -> Option<VoiceState> {
    let bot_id = ctx.cache.current_user().id;
    ctx.cache
        .guild(guild_id)
        .and_then(|guild| guild.voice_states.get(&bot_id).cloned())
}

pub(super) async fn wait_for_voice_session_gone(
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

pub(super) async fn wait_for_bot_voice_state(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<String, String> {
    for _ in 0..20 {
        match cached_bot_voice_state_from_serenity(ctx, guild_id) {
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

pub(super) fn bot_voice_state_allows_conversation(
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

pub(super) async fn wait_for_enabled_bot_voice_state(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel_id: ChannelId,
    session_id: &str,
) -> Result<(), String> {
    wait_for_enabled_bot_voice_state_from_serenity(
        ctx.serenity_context(),
        guild_id,
        channel_id,
        session_id,
    )
    .await
}

pub(super) async fn wait_for_enabled_bot_voice_state_from_serenity(
    ctx: &serenity::all::Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    session_id: &str,
) -> Result<(), String> {
    for _ in 0..20 {
        if cached_bot_voice_state_from_serenity(ctx, guild_id).is_some_and(|state| {
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

pub(super) fn no_audio_songbird_config() -> songbird::Config {
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

pub(super) fn initial_songbird_config(mode: VoiceMode) -> songbird::Config {
    match mode {
        VoiceMode::Disabled => no_audio_songbird_config(),
        VoiceMode::Local | VoiceMode::OpenAi => audio_songbird_config(),
    }
}

pub(super) async fn set_muted_self_deafened(
    call: &Arc<Mutex<songbird::Call>>,
) -> Result<(), songbird::error::JoinError> {
    let mut call = call.lock().await;
    call.deafen(true).await?;
    call.mute(true).await
}

pub(super) async fn enable_conversation(
    call: &Arc<Mutex<songbird::Call>>,
) -> Result<(), songbird::error::JoinError> {
    let mut call = call.lock().await;
    // Unmute while still self-deafened; reception is the final transition.
    call.mute(false).await?;
    call.deafen(false).await
}

pub(super) async fn pause_call_for_consent(call: &Arc<Mutex<songbird::Call>>) {
    // `leave` stops the driver locally before its gateway round trip.
    let _ = call.lock().await.leave().await;
}

pub(super) async fn remove_call_for_consent(manager: &songbird::Songbird, guild_id: GuildId) {
    let _ = manager.remove(guild_id).await;
}

pub(super) fn can_stop_voice(
    present_in_configured_channel: bool,
    interaction_permissions: Option<Permissions>,
) -> bool {
    crate::commands_help::leave_allowed(present_in_configured_channel, interaction_permissions)
}

/// The public disclosure participants see. Takes the mode explicitly rather
/// than reading the runtime: this text is a promise about where audio goes, so
/// it must describe the same backend the caller is about to connect, not
/// whatever the runtime says by the time this renders.
pub(super) fn consent_notice(mode: VoiceMode, channel_id: ChannelId, resumed: bool) -> String {
    let action = if resumed { "resuming" } else { "starting" };
    match mode {
        VoiceMode::Local => format!(
            "🔒 Abbey is {action} consented voice in <#{channel_id}>. Discord still transports the call, but speech recognition, Abbey/Abi/Aviva reasoning, WDBX-scoped context, and speech synthesis run locally on Donald's Mac. Abbey does not retain raw audio. Person-specific WDBX context is read-only and is used only for one uniquely attributed speaker; overlap disables it. Say Abbey, Aviva, or ABI to start. A clearly attributed spoken withdrawal is honored locally; for an authoritative stop, use `/voice leave` or mention Abbey and write `stop listening` in this voice chat. A new participant pauses and disconnects the session until renewed consent."
        ),
        VoiceMode::OpenAi => format!(
            "☁️ Abbey is {action} consented voice in <#{channel_id}> using the explicitly configured direct OpenAI Realtime backup. Participant audio is sent to that provider and complete responses are buffered before Discord playback. This degraded backup does not use local ABI persona routing or WDBX context; use local mode for canonical Abbey. Abbey does not retain raw audio locally. Spoken control is not authoritative in this degraded mode: use `/voice leave` or mention Abbey and write `stop listening` in this voice chat to stop immediately. A new participant pauses and disconnects the session until renewed consent."
        ),
        VoiceMode::Disabled => unreachable!(),
    }
}

pub(super) fn public_error(error: &str) -> String {
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

    #[test]
    fn stopping_voice_needs_presence_or_manage_guild() {
        assert!(can_stop_voice(true, None));
        assert!(can_stop_voice(true, Some(Permissions::empty())));
        assert!(can_stop_voice(false, Some(Permissions::MANAGE_GUILD)));
        assert!(!can_stop_voice(false, Some(required_voice_permissions())));
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
            Permissions::STREAM,
            Permissions::USE_EMBEDDED_ACTIVITIES,
        ] {
            assert!(required.contains(expected), "missing {expected:?}");
        }
        for forbidden in [
            Permissions::ADMINISTRATOR,
            Permissions::MANAGE_GUILD,
            Permissions::MANAGE_MESSAGES,
            Permissions::MUTE_MEMBERS,
            Permissions::MOVE_MEMBERS,
        ] {
            assert!(!required.contains(forbidden), "unexpected {forbidden:?}");
        }
    }

    #[test]
    fn songbird_modes_do_not_decode_presence_and_decode_conversation() {
        assert_eq!(
            no_audio_songbird_config().decode_mode,
            songbird::driver::DecodeMode::Pass
        );
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
    fn consent_notices_name_guaranteed_written_stop_route_and_fit_discord() {
        for mode in [VoiceMode::Local, VoiceMode::OpenAi] {
            let notice = consent_notice(mode, ChannelId::new(u64::MAX), false);
            assert!(notice.contains("mention Abbey and write `stop listening`"));
            assert!(notice.chars().count() < 2000);
        }
    }

    #[test]
    fn a_permission_error_keeps_its_cause_and_still_reads_as_its_message() {
        use crate::observability::OperationalErrorCategory as Category;

        // The four failure paths in verify_required_voice_permissions_live are
        // not reachable without a live Discord context, so what is pinned here
        // is the carrier: the category survives, and the message is what an
        // operator sees. Every caller previously hardcoded Authorization, which
        // reported a rate-limited REST call as a permission denial.
        let unavailable = VoicePermissionError::new("read failed", Category::Unavailable);
        assert_eq!(unavailable.category(), Category::Unavailable);
        assert_eq!(unavailable.to_string(), "read failed");
        assert_eq!(String::from(unavailable), "read failed");

        let denied = VoicePermissionError::new("missing Connect", Category::Authorization);
        assert_eq!(denied.category(), Category::Authorization);

        // The distinction is the whole point: these must not collapse.
        assert_ne!(
            VoicePermissionError::new("x", Category::Unavailable).category(),
            VoicePermissionError::new("x", Category::Authorization).category()
        );
        assert_ne!(
            VoicePermissionError::new("x", Category::Configuration).category(),
            VoicePermissionError::new("x", Category::Authorization).category()
        );
    }
}
