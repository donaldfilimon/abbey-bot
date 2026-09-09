//! Classic voice Action Row adapter (Phase A status → B leave confirm → C play).
//!
//! Buttons never start STT or consent. Custom ids stay `abbey:v:{sid}:{act}`.

use std::sync::Arc;

use serenity::all::{
    ButtonStyle, ChannelId, ComponentInteraction, ComponentInteractionDataKind, CreateActionRow,
    CreateButton, CreateInteractionResponse, CreateInteractionResponseMessage, GuildId,
    InteractionResponseFlags, Permissions,
};

use crate::{
    Data, Error,
    gateway::shared::clamp_message,
    voice_session::{PlaybackTermination, VoicePhase, VoiceRuntime},
    voice_ux::{self, Act, Phase, Rejection, Session},
};

use super::discord::{cached_participants_from_serenity, can_stop_voice, pause_call_for_consent};

/// Post the Phase A status panel after a successful consented join/resume.
pub(super) async fn send_post_join_panel(
    ctx: crate::Context<'_>,
    runtime: &VoiceRuntime,
    channel_id: ChannelId,
    resumed: bool,
) -> Result<(), Error> {
    let sid = voice_ux::mint_sid();
    let now = crate::runtime::now();
    let session = Session {
        sid: sid.clone(),
        guild: runtime.config.guild_id,
        user: ctx.author().id.get(),
        channel: channel_id.get(),
        expiry: now.saturating_add(voice_ux::SESSION_SECONDS),
        phase: Phase::Status,
    };
    ctx.data()
        .state
        .voice_ux
        .insert(session.clone())
        .map_err(Error::from)?;
    let is_playable = live_playable(ctx.data(), runtime).await;
    let status = status_body(runtime, resumed, is_playable).await;
    let content = voice_ux::panel_content(Phase::Status, &status, is_playable);
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(content))
            .components(rows_for(&session, is_playable))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

/// Central `abbey:v:` component dispatch. Returns false when the custom id is
/// not a voice UX control so other dispatchers can continue.
pub async fn dispatch_ux_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
) -> bool {
    if !interaction
        .data
        .custom_id
        .starts_with(voice_ux::CUSTOM_ID_PREFIX)
    {
        return false;
    }
    if let Err(rejection) = prepare_and_handle(ctx, interaction, data).await {
        deny(ctx, interaction, data, rejection).await;
    }
    true
}

async fn prepare_and_handle(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
) -> Result<(), Rejection> {
    if interaction.user.bot || interaction.message.author.id != ctx.cache.current_user().id {
        return Err(Rejection::Malformed);
    }
    if !matches!(interaction.data.kind, ComponentInteractionDataKind::Button) {
        return Err(Rejection::Malformed);
    }
    let (sid, act) = voice_ux::parse_custom_id(&interaction.data.custom_id)?;
    let session = data
        .state
        .voice_ux
        .get(&sid)
        .ok()
        .flatten()
        .ok_or(Rejection::Missing)?;
    voice_ux::authorize(
        &session,
        interaction.user.id.get(),
        interaction.guild_id.map(|g| g.get()),
        crate::runtime::now(),
    )?;
    let next_phase = voice_ux::reduce(session.phase, act)?;
    match act {
        Act::Ref => refresh(ctx, interaction, data, session).await,
        Act::Leave => swap_phase(ctx, interaction, data, session, Phase::ConfirmLeave).await,
        Act::Cancel => swap_phase(ctx, interaction, data, session, Phase::Status).await,
        Act::Ok => confirm_leave(ctx, interaction, data, session).await,
        Act::Play | Act::Stop | Act::Skip => {
            music_act(ctx, interaction, data, session, act, next_phase).await
        }
    }
}

async fn refresh(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    session: Session,
) -> Result<(), Rejection> {
    let runtime = data.voice_for(session.guild);
    let is_playable = match runtime.as_ref() {
        Some(runtime) => live_playable_http(ctx, data, runtime).await,
        None => false,
    };
    let status = match runtime.as_ref() {
        Some(runtime) => status_body_http(ctx, runtime, false, is_playable).await,
        None => "No voice session is prepared in this server.".to_owned(),
    };
    // Expiry intentionally not renewed.
    let _ = data.state.voice_ux.set_phase(&session.sid, Phase::Status);
    let mut session = session;
    session.phase = Phase::Status;
    update_message(
        ctx,
        interaction,
        data,
        &voice_ux::panel_content(Phase::Status, &status, is_playable),
        rows_for(&session, is_playable),
    )
    .await;
    Ok(())
}

async fn swap_phase(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    session: Session,
    phase: Phase,
) -> Result<(), Rejection> {
    let runtime = data.voice_for(session.guild);
    let is_playable = match runtime.as_ref() {
        Some(runtime) => live_playable_http(ctx, data, runtime).await,
        None => false,
    };
    let status = match runtime.as_ref() {
        Some(runtime) => status_body_http(ctx, runtime, false, is_playable).await,
        None => "No voice session is prepared in this server.".to_owned(),
    };
    let _ = data.state.voice_ux.set_phase(&session.sid, phase);
    let mut session = session;
    session.phase = phase;
    update_message(
        ctx,
        interaction,
        data,
        &voice_ux::panel_content(phase, &status, is_playable),
        rows_for(&session, is_playable),
    )
    .await;
    Ok(())
}

async fn confirm_leave(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    session: Session,
) -> Result<(), Rejection> {
    let guild_id = GuildId::new(session.guild);
    let Some(runtime) = data.voice_for(session.guild) else {
        let _ = data.state.voice_ux.set_phase(&session.sid, Phase::Left);
        update_message(
            ctx,
            interaction,
            data,
            voice_ux::panel_content(Phase::Left, "", false).as_str(),
            Vec::new(),
        )
        .await;
        let _ = data.state.voice_ux.remove(&session.sid);
        return Ok(());
    };
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let present = cached_participants_from_serenity(ctx, guild_id, channel_id)
        .is_some_and(|users| users.contains(&interaction.user.id.get()));
    let permissions = interaction
        .member
        .as_ref()
        .and_then(|member| member.permissions);
    let Some(closed) = super::acknowledgement::authorize_and_close_media(
        || can_stop_voice(present, permissions),
        || {
            data.cancel_voice_join(session.guild);
            runtime
                .music
                .stop("voice leave", PlaybackTermination::Stopped);
            runtime.cancel_pending_start();
        },
    ) else {
        // Authorized session owner but not allowed to stop — ephemeral explain
        // without tearing down.
        deny_message(
            ctx,
            interaction,
            data,
            "Only someone currently in the configured voice channel or a member with Manage Server can stop Abbey voice.",
        )
        .await;
        return Ok(());
    };
    let verification_run = runtime.verification_run_token();
    let leave_work = perform_leave(ctx, data, guild_id, &runtime, verification_run);
    let ack = interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    );
    let (ack_result, leave_result) =
        super::acknowledgement::acknowledge_with_transition(closed, ack, leave_work).await;
    if let Err(error) = &ack_result {
        crate::gateway::interaction_outcomes::delivery_failed_from(&data.state, error);
    }
    let content = if leave_result.is_ok() {
        let _ = data.state.voice_ux.set_phase(&session.sid, Phase::Left);
        let _ = data.state.voice_ux.remove(&session.sid);
        voice_ux::panel_content(Phase::Left, "", false)
    } else {
        // Mid-fail: keep ConfirmLeave? Design: show failed + Refresh.
        let _ = data.state.voice_ux.set_phase(&session.sid, Phase::Status);
        let mut failed = session;
        failed.phase = Phase::Status;
        let body = format!(
            "Leave did not finish cleanly. Tap Refresh to re-read live voice state.\n{}",
            leave_result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
        );
        edit_deferred(ctx, interaction, data, &body, rows_for(&failed, false)).await;
        return Ok(());
    };
    edit_deferred(ctx, interaction, data, &content, Vec::new()).await;
    Ok(())
}

async fn perform_leave(
    ctx: &serenity::all::Context,
    data: &Data,
    guild_id: GuildId,
    runtime: &Arc<VoiceRuntime>,
    verification_run: Option<u64>,
) -> Result<(), Error> {
    let manager = songbird::get(ctx)
        .await
        .ok_or("Songbird was not registered in the Discord client")?;
    let exact_call = manager.get(guild_id);
    let leave_exact = async {
        if let Some(call) = exact_call {
            pause_call_for_consent(&call).await;
        }
    };
    let (transition, ()) = tokio::join!(runtime.transition.lock(), leave_exact);
    if let Some(call) = manager.get(guild_id) {
        pause_call_for_consent(&call).await;
    }
    runtime
        .disconnect("configured; disconnected by voice UX leave")
        .await;
    let removed = manager.remove(guild_id).await;
    let result = match removed {
        Ok(()) | Err(songbird::error::JoinError::NoCall) => {
            if let Some(run) = verification_run {
                let _ = runtime.note_verification_final_leave(run);
            }
            data.retire_voice_after_leave(guild_id.get(), runtime);
            Ok(())
        }
        Err(error) => Err(error.into()),
    };
    drop(transition);
    result
}

async fn music_act(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    session: Session,
    act: Act,
    _next: Phase,
) -> Result<(), Rejection> {
    let Some(runtime) = data.voice_for(session.guild) else {
        deny_message(
            ctx,
            interaction,
            data,
            &voice_ux::music_controls_unavailable_note("no voice session is prepared"),
        )
        .await;
        return Ok(());
    };
    let snapshot_for_gate = runtime.snapshot().await;
    if !voice_ux::playable(
        true,
        snapshot_for_gate.media_enabled,
        snapshot_for_gate.phase == VoicePhase::Failed,
        snapshot_for_gate.start_pending,
    ) {
        let reason = voice_ux::unplayable_reason(
            true,
            snapshot_for_gate.media_enabled,
            snapshot_for_gate.phase == VoicePhase::Failed,
            snapshot_for_gate.start_pending,
        )
        .unwrap_or("session is not playable");
        deny_message(
            ctx,
            interaction,
            data,
            &voice_ux::music_controls_unavailable_note(reason),
        )
        .await;
        return Ok(());
    }
    // Manager + presence gate (parity with /voice play).
    let permissions = interaction
        .member
        .as_ref()
        .and_then(|m| m.permissions)
        .unwrap_or(Permissions::empty());
    let manager = permissions.intersects(Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR);
    let present = cached_participants_from_serenity(
        ctx,
        GuildId::new(runtime.config.guild_id),
        ChannelId::new(runtime.config.channel_id),
    )
    .is_some_and(|users| users.contains(&interaction.user.id.get()));
    if let Err(error) = crate::music::command_channel_gate(
        runtime.config.guild_id,
        runtime.config.music_command_channel_id,
        Some(session.guild),
        interaction.channel_id.get(),
    ) {
        deny_message(ctx, interaction, data, &error).await;
        return Ok(());
    }
    if let Err(error) = crate::music::gate(true, manager, present, cfg!(target_os = "macos")) {
        deny_message(ctx, interaction, data, error).await;
        return Ok(());
    }

    // Play/Skip can exceed the 3s interaction window (osascript + tap).
    let defer_music = matches!(act, Act::Play | Act::Skip);
    if defer_music {
        let ack = interaction
            .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
            .await;
        if ack.is_err() {
            crate::gateway::interaction_outcomes::delivery_failed(&data.state);
            return Ok(());
        }
    }

    let note = match act {
        Act::Stop => {
            let was_active = runtime.music.is_output_active();
            runtime.music.stop("stopped", PlaybackTermination::Stopped);
            voice_ux::stop_note(was_active).to_owned()
        }
        Act::Play => {
            match super::play::start_empty_for_ux(ctx, &data.state, interaction, runtime.clone())
                .await
            {
                Ok(message) => message,
                Err(error) => {
                    let detail = error.to_string();
                    if detail.contains("/voice play") {
                        detail
                    } else {
                        format!("{detail} Track URI / library search still uses `/voice play`.")
                    }
                }
            }
        }
        Act::Skip => match runtime.music.player() {
            Some(player) => match data.state.host_music.control(runtime.config.guild_id) {
                Ok(lease) => {
                    let script = crate::player_control::next(player);
                    match super::play::execute_script_for_ux(&runtime, script, lease).await {
                        Ok(()) => "Skipped to the next track.".to_owned(),
                        Err(error) => format!("Skip failed: {error}"),
                    }
                }
                Err(error) => error.to_owned(),
            },
            None => voice_ux::skip_no_player_note().to_owned(),
        },
        _ => return Err(Rejection::WrongPhase),
    };
    let is_playable = live_playable_http(ctx, data, &runtime).await;
    let status = status_body_http(ctx, &runtime, false, is_playable).await;
    let body = format!(
        "{}\n\n{}",
        note,
        voice_ux::panel_content(Phase::Status, &status, is_playable)
    );
    let mut session = session;
    session.phase = Phase::Status;
    if defer_music {
        edit_deferred(
            ctx,
            interaction,
            data,
            &body,
            rows_for(&session, is_playable),
        )
        .await;
    } else {
        update_message(
            ctx,
            interaction,
            data,
            &body,
            rows_for(&session, is_playable),
        )
        .await;
    }
    Ok(())
}

fn rows_for(session: &Session, is_playable: bool) -> Vec<CreateActionRow> {
    match session.phase {
        Phase::Status => {
            let buttons = voice_ux::status_buttons(true)
                .iter()
                .copied()
                .map(|act| {
                    let enabled = match act {
                        Act::Play | Act::Stop | Act::Skip => is_playable,
                        _ => true,
                    };
                    let style = match act {
                        Act::Leave => ButtonStyle::Danger,
                        Act::Play | Act::Stop | Act::Skip => ButtonStyle::Primary,
                        _ => ButtonStyle::Secondary,
                    };
                    CreateButton::new(voice_ux::format_custom_id(&session.sid, act))
                        .label(act.label())
                        .style(style)
                        .disabled(!enabled)
                })
                .collect();
            // Non-playable matrix is Ref+Leave only in pure API; UI still shows
            // disabled play controls so Phase C remains discoverable.
            let _ = voice_ux::status_buttons(false);
            vec![CreateActionRow::Buttons(buttons)]
        }
        Phase::ConfirmLeave => {
            let buttons = voice_ux::confirm_buttons()
                .iter()
                .copied()
                .map(|act| {
                    let style = if act == Act::Ok {
                        ButtonStyle::Danger
                    } else {
                        ButtonStyle::Secondary
                    };
                    CreateButton::new(voice_ux::format_custom_id(&session.sid, act))
                        .label(act.label())
                        .style(style)
                })
                .collect();
            vec![CreateActionRow::Buttons(buttons)]
        }
        Phase::Left => Vec::new(),
    }
}

async fn live_playable(data: &Data, runtime: &VoiceRuntime) -> bool {
    let snapshot = runtime.snapshot().await;
    voice_ux::playable(
        data.voice_for(runtime.config.guild_id).is_some(),
        snapshot.media_enabled,
        snapshot.phase == VoicePhase::Failed,
        snapshot.start_pending,
    )
}

async fn live_playable_http(
    _ctx: &serenity::all::Context,
    data: &Data,
    runtime: &VoiceRuntime,
) -> bool {
    live_playable(data, runtime).await
}

async fn status_body(runtime: &VoiceRuntime, resumed: bool, is_playable: bool) -> String {
    let snapshot = runtime.snapshot().await;
    let player = match runtime.music.player() {
        Some(crate::player_control::Player::Spotify) => "spotify",
        Some(crate::player_control::Player::Music) => "music",
        None => "none",
    };
    let pending = if snapshot.start_pending {
        " · start pending"
    } else {
        ""
    };
    let why = voice_ux::unplayable_reason(
        true,
        snapshot.media_enabled,
        snapshot.phase == VoicePhase::Failed,
        snapshot.start_pending,
    )
    .map(|reason| format!(" · {reason}"))
    .unwrap_or_default();
    let music = runtime.music.status();
    format!(
        "{} <#{channel}> · mode `{}` · phase {} · media {} · playable {} · player `{player}`{pending}{why}\n{music}",
        if resumed { "Resumed" } else { "Joined" },
        runtime.effective_mode().label(),
        snapshot.phase.label(),
        if snapshot.media_enabled {
            "open"
        } else {
            "closed"
        },
        if is_playable { "yes" } else { "no" },
        channel = runtime.config.channel_id,
    )
}

async fn status_body_http(
    _ctx: &serenity::all::Context,
    runtime: &VoiceRuntime,
    resumed: bool,
    is_playable: bool,
) -> String {
    status_body(runtime, resumed, is_playable).await
}

async fn update_message(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    content: &str,
    rows: Vec<CreateActionRow>,
) {
    let delivery = interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(clamp_message(content.to_owned()))
                    .components(rows)
                    .allowed_mentions(crate::gateway::no_mentions()),
            ),
        )
        .await;
    if let Err(error) = &delivery {
        crate::gateway::interaction_outcomes::delivery_failed_from(&data.state, error);
    }
}

async fn edit_deferred(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    content: &str,
    rows: Vec<CreateActionRow>,
) {
    let delivery = interaction
        .edit_response(
            &ctx.http,
            serenity::all::EditInteractionResponse::new()
                .content(clamp_message(content.to_owned()))
                .components(rows)
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await;
    if let Err(error) = &delivery {
        crate::gateway::interaction_outcomes::delivery_failed_from(&data.state, error);
    }
}

async fn deny(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    rejection: Rejection,
) {
    deny_message(ctx, interaction, data, rejection.message()).await;
}

async fn deny_message(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    message: &str,
) {
    let delivery = interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(clamp_message(message.to_owned()))
                    .ephemeral(true)
                    .flags(InteractionResponseFlags::EPHEMERAL)
                    .allowed_mentions(crate::gateway::no_mentions()),
            ),
        )
        .await;
    if let Err(error) = &delivery {
        crate::gateway::interaction_outcomes::delivery_failed_from(&data.state, error);
    }
}
