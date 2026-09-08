//! Native-player and Songbird adapters for an independently owned music output.
use crate::{
    Context, Error,
    audio_tap::{AudioTapClient, PcmBuffer, TapStream},
    gateway::shared::clamp_message,
    player_control::{self, Player, Script},
    voice_session::{PlaybackTermination, VoicePhase, VoiceRuntime},
};
use serenity::all::{ChannelId, CreateInteractionResponseFollowup, GuildId, Permissions};
use songbird::input::RawAdapter;
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum MusicPlayer {
    Spotify,
    Music,
}
impl From<MusicPlayer> for Player {
    fn from(p: MusicPlayer) -> Self {
        match p {
            MusicPlayer::Spotify => Self::Spotify,
            MusicPlayer::Music => Self::Music,
        }
    }
}

async fn authorized(ctx: Context<'_>) -> Result<Arc<VoiceRuntime>, Error> {
    let guild = ctx.guild_id().ok_or("This command requires a server.")?;
    let runtime = ctx.data().voice_for(guild.get())
        .ok_or("No voice destination is selected in this server. Join a voice channel and use `/voice join` first; listening still requires each participant's agreement.")?;
    crate::music::command_channel_gate(
        runtime.config.guild_id,
        runtime.config.music_command_channel_id,
        ctx.guild_id().map(|guild| guild.get()),
        ctx.channel_id().get(),
    )?;
    let guild = ctx.guild_id().ok_or("This command requires a server.")?;
    let member = guild.member(ctx.http(), ctx.author().id).await?;
    let partial = guild.to_partial_guild(ctx.http()).await?;
    let manager = member.user.id == partial.owner_id
        || member
            .roles
            .iter()
            .filter_map(|id| partial.roles.get(id))
            .chain(partial.roles.get(&serenity::all::RoleId::new(guild.get())))
            .any(|role| {
                role.permissions
                    .intersects(Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR)
            });
    let present = super::cached_participants(ctx, guild, ChannelId::new(runtime.config.channel_id))
        .is_ok_and(|(present, _)| present);
    crate::music::gate(
        guild.get() == runtime.config.guild_id,
        manager,
        present,
        cfg!(target_os = "macos"),
    )?;
    Ok(runtime)
}

/// Component-safe native player control used by classic voice UX Skip.
pub(super) async fn execute_script_for_ux(
    runtime: &VoiceRuntime,
    script: Script,
    lease: crate::host_music::HostMusicLease,
) -> Result<(), Error> {
    execute(runtime, script, lease).await
}

async fn execute(
    runtime: &VoiceRuntime,
    script: Script,
    lease: crate::host_music::HostMusicLease,
) -> Result<(), Error> {
    if !lease.for_guild(runtime.config.guild_id) {
        return Err("Host music ownership changed.".into());
    }
    runtime
        .spawn_result(move |cancel| async move {
            let _lease = lease;
            execute_owned(script, cancel, None).await
        })?
        .await
        .map_err(|_| "Player control ownership failed.")?
}

async fn execute_owned(
    script: Script,
    cancel: tokio_util::sync::CancellationToken,
    music: Option<(Arc<VoiceRuntime>, u64)>,
) -> Result<(), Error> {
    if !cfg!(target_os = "macos") {
        return Err("Native music control requires macOS.".into());
    }
    let mut command = tokio::process::Command::new("/usr/bin/osascript");
    command
        .arg("-e")
        .arg(script.source)
        .arg("--")
        .arg(script.argument)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let (child, music_cancel) = if let Some((runtime, generation)) = music {
        runtime
            .music
            .launch_current(generation, || spawn_if_running(&cancel, || command.spawn()))
            .ok_or("Music start cancelled before player launch.")?
    } else {
        (
            spawn_if_running(&cancel, || command.spawn()),
            tokio_util::sync::CancellationToken::new(),
        )
    };
    let mut child = child?;
    wait_player(&mut child, &cancel, &music_cancel).await
}

async fn wait_player(
    child: &mut tokio::process::Child,
    cancel: &tokio_util::sync::CancellationToken,
    music_cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), Error> {
    use tokio::io::AsyncReadExt;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("Player stderr unavailable")?
        .take(4096);
    let mut bytes = Vec::new();
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        _ = music_cancel.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(8), async {
            tokio::try_join!(child.wait(), stderr.read_to_end(&mut bytes))
        }) => result.ok(),
    };
    let (status, _) = match result {
        Some(Ok(result)) => result,
        Some(Err(_)) | None => {
            // Root owns this future through kill AND wait, even when the
            // interaction waiter disappears. An unresponsive wait remains an
            // outstanding service operation at the terminal deadline.
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err("Player control stopped before completion; music stayed off.".into());
        }
    };
    if !status.success() {
        return Err(format!(
            "Player refused playback: {}",
            String::from_utf8_lossy(&bytes)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(240)
                .collect::<String>()
        )
        .into());
    }
    Ok(())
}

fn spawn_if_running<T>(
    cancel: &tokio_util::sync::CancellationToken,
    spawn: impl FnOnce() -> std::io::Result<T>,
) -> Result<T, Error> {
    if cancel.is_cancelled() {
        return Err("Player control cancelled before launch; music stayed off.".into());
    }
    spawn().map_err(Into::into)
}

fn tap_client() -> Result<AudioTapClient, Error> {
    Ok(AudioTapClient::new(
        &std::env::var("ABBEY_AUDIO_TAP_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:8182".into()),
    )?)
}

/// Play a native Spotify track or Music library search and mirror eligible host audio.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "play"
)]
pub async fn voice_play(
    ctx: Context<'_>,
    #[description="Spotify track URI or Music library search; omit for current selection"] query:Option<String>,
    #[description = "Native player (default Spotify)"] player: Option<MusicPlayer>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let result = async {
        let runtime = authorized(ctx).await?;
        let player = player.unwrap_or(MusicPlayer::Spotify).into();
        start(ctx, runtime, player, query.as_deref().unwrap_or("")).await
    }
    .await;
    reply(ctx, result).await
}
/// Pause local music and close its capture stream without changing listening consent.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "pause"
)]
pub async fn voice_pause(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let result = async {
        let runtime = authorized(ctx).await?;
        let lease = ctx
            .data()
            .state
            .host_music
            .control(runtime.config.guild_id)?;
        runtime.music.stop("paused", PlaybackTermination::Stopped);
        if let Some(player) = runtime.music.player() {
            execute(&runtime, player_control::pause(player), lease).await?;
        }
        Ok("Music paused; listening consent is unchanged.".into())
    }
    .await;
    reply(ctx, result).await
}
/// Resume music only; this never renews permission to listen to Discord participants.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "resume-music"
)]
pub async fn voice_resume_music(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let result = async {
        let runtime = authorized(ctx).await?;
        let player = runtime
            .music
            .player()
            .ok_or("Select a player with /voice play first.")?;
        start(ctx, runtime, player, "").await
    }
    .await;
    reply(ctx, result).await
}
/// Stop mirroring the host audio mix and discard all queued music.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "stop-music"
)]
pub async fn voice_stop_music(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let result = async {
        let runtime = authorized(ctx).await?;
        runtime.music.stop("stopped", PlaybackTermination::Stopped);
        Ok("Music capture and playback stopped; listening consent is unchanged.".into())
    }
    .await;
    reply(ctx, result).await
}
/// Set the mirrored music volume; Abbey's speaking voice ducks it to one quarter.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "volume"
)]
pub async fn voice_volume(
    ctx: Context<'_>,
    #[description = "Music level from 0 to 100"]
    #[min = 0]
    #[max = 100]
    level: u8,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let result = async {
        let runtime = authorized(ctx).await?;
        runtime.music.set_volume(level);
        Ok(runtime.music.status())
    }
    .await;
    reply(ctx, result).await
}
async fn reply(ctx: Context<'_>, result: Result<String, Error>) -> Result<(), Error> {
    if result.is_err() {
        crate::gateway::interaction_outcomes::record_failure(
            &ctx.data().state,
            crate::observability::EventCode::CommandFailure,
            crate::observability::OperationalErrorCategory::Unavailable,
        );
    }
    let delivered = ctx
        .send(
            poise::CreateReply::default()
                .ephemeral(true)
                .content(clamp_message(result.unwrap_or_else(|e| e.to_string())))
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await;
    match delivered {
        Ok(_) => Ok(()),
        Err(error) => {
            crate::gateway::interaction_outcomes::delivery_failed(&ctx.data().state);
            Err(error.into())
        }
    }
}

/// Caller holds `transition`. Replacing a Decode driver is mandatory before any
/// output-only rejoin, because self-deafen alone is not a decoding boundary.
async fn output_call(
    ctx: &serenity::all::Context,
    runtime: &VoiceRuntime,
) -> Result<Arc<tokio::sync::Mutex<songbird::Call>>, Error> {
    let guild = GuildId::new(runtime.config.guild_id);
    let channel = ChannelId::new(runtime.config.channel_id);
    super::verify_required_voice_permissions_live(ctx, guild, channel).await?;
    let manager = songbird::get(ctx).await.ok_or("Songbird unavailable")?;
    let snapshot = runtime.snapshot().await;
    if snapshot.media_enabled {
        let call = manager
            .get(guild)
            .ok_or("Consented voice call unavailable")?;
        if call.lock().await.current_channel() != Some(channel.into()) {
            return Err("Voice destination changed; music stayed off.".into());
        }
        return Ok(call);
    }
    if snapshot.start_pending || snapshot.phase == VoicePhase::Connecting {
        return Err("Voice is connecting; retry music after it settles.".into());
    }
    if snapshot.phase == VoicePhase::AwaitingConsent
        && !runtime.music_may_restore_output(snapshot.epoch)
    {
        return Err("Listening teardown is still in progress; retry music when it settles.".into());
    }
    if snapshot.phase == VoicePhase::Failed {
        return Err("Voice failed; use /voice leave before starting music again.".into());
    }
    if manager.get(guild).is_some() {
        manager.remove(guild).await?;
    }
    let call = manager.get_or_insert(guild);
    {
        let mut call = call.lock().await;
        call.set_config(super::no_audio_songbird_config());
        call.deafen(true).await?;
        call.mute(true).await?;
    }
    manager.join(guild, channel).await?;
    {
        let mut call = call.lock().await;
        call.deafen(true).await?;
        call.mute(false).await?;
    }
    super::verify_required_voice_permissions_live(ctx, guild, channel).await?;
    if let Some(state) = super::cached_bot_voice_state_from_serenity(ctx, guild)
        && (state.channel_id != Some(channel) || state.mute || state.deaf || state.suppress)
    {
        let _ = manager.remove(guild).await;
        return Err("Discord cannot transmit music in the configured channel.".into());
    }
    Ok(call)
}

async fn start(
    ctx: Context<'_>,
    runtime: Arc<VoiceRuntime>,
    player: Player,
    query: &str,
) -> Result<String, Error> {
    let script = player_control::play(player, query)?;
    let client = tap_client()?;
    let lease = ctx
        .data()
        .state
        .host_music
        .try_start(runtime.config.guild_id)?;
    let generation = runtime.music.begin(player);
    let setup = async {
        client.health().await?; // health never starts capture or requests TCC permission
        let _transition = runtime.transition.lock().await;
        if !runtime.music.current(generation) {
            return Err::<_, Error>("Music start cancelled.".into());
        }
        let call = output_call(ctx.serenity_context(), &runtime).await?;
        let owner = runtime.clone();
        let child_lease = lease.clone();
        runtime
            .spawn_result(move |cancel| async move {
                let _lease = child_lease;
                execute_owned(script, cancel, Some((owner, generation))).await
            })?
            .await
            .map_err(|_| "Player control ownership failed.")??;
        if !runtime.music.current(generation) {
            return Err("Music start cancelled.".into());
        }
        let stream = connect_tap(&client, &runtime, generation).await?;
        Ok((call, stream))
    }
    .await;
    let (call, stream) = match setup {
        Ok(v) => v,
        Err(e) => {
            runtime
                .music
                .finish(generation, "start failed", PlaybackTermination::Errored);
            return Err(e);
        }
    };
    let context = ctx.serenity_context().clone();
    let interaction = match ctx {
        poise::Context::Application(app) => Some(app.interaction.clone()),
        _ => None,
    };
    let owner = Arc::clone(&runtime);
    let state = Arc::clone(&ctx.data().state);
    owner.spawn_owned(async move {
        let result = run_music(&context, &runtime, generation, client, call, stream).await;
        drop(lease);
        if runtime.music.current(generation) {
            let message = result
                .err()
                .map_or_else(|| "Music stopped.".into(), |e| e.to_string());
            runtime
                .music
                .finish(generation, &message, PlaybackTermination::Errored);
            if let Some(interaction) = interaction {
                let delivered = interaction
                    .create_followup(
                        &context.http,
                        CreateInteractionResponseFollowup::new()
                            .ephemeral(true)
                            .content(clamp_message(message))
                            .allowed_mentions(crate::gateway::no_mentions()),
                    )
                    .await;
                if delivered.is_err() {
                    crate::gateway::interaction_outcomes::delivery_failed(&state);
                }
            }
        }
    })?;
    Ok("Mirroring the eligible host application mix, excluding Discord and browser/terminal audio. Other eligible apps can still be heard. Music ducks while Abbey speaks; listening consent is unchanged.".into())
}

async fn run_music(
    ctx: &serenity::all::Context,
    runtime: &VoiceRuntime,
    generation: u64,
    client: AudioTapClient,
    mut call: Arc<tokio::sync::Mutex<songbird::Call>>,
    mut stream: TapStream,
) -> Result<(), Error> {
    struct DetachOnExit<'a>(&'a VoiceRuntime, u64);
    impl Drop for DetachOnExit<'_> {
        fn drop(&mut self) {
            self.0.music.detach(self.1);
        }
    }
    let _cleanup = DetachOnExit(runtime, generation);
    loop {
        let buffer = PcmBuffer::new();
        let input = RawAdapter::new(buffer.reader(), 48_000, 2).into();
        let handle = call.lock().await.play_input(input);
        if !runtime
            .music
            .install(generation, buffer.clone(), handle.clone())
        {
            return Ok(());
        }
        let (events, mut terminal) = tokio::sync::mpsc::unbounded_channel();
        crate::voice_session::register_playback_termination(&handle, &events, generation)?;
        let mut epoch = runtime.current_epoch();
        let mut next = Box::pin(stream.next());
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! { biased;
                _=tick.tick()=>{
                    if !runtime.music.current(generation){return Ok(());}
                    let snapshot=runtime.snapshot().await;
                    if snapshot.phase==VoicePhase::Failed {return Err("Voice failed; music stopped.".into());}
                    if snapshot.epoch!=epoch {
                        if !snapshot.media_enabled {break;}
                        // A fresh listening call needs a fresh tap/track, too.
                        let manager=songbird::get(ctx).await.ok_or("Songbird unavailable")?;
                        if !manager.get(GuildId::new(runtime.config.guild_id)).is_some_and(|c|Arc::ptr_eq(&c,&call)){break;}
                        epoch=snapshot.epoch;
                    }
                    if super::cached_bot_voice_state_from_serenity(ctx,GuildId::new(runtime.config.guild_id)).is_some_and(|state| state.mute || state.deaf || state.suppress || state.channel_id != Some(ChannelId::new(runtime.config.channel_id))) { return Err("Discord can no longer transmit music safely.".into()); }
                    if let Ok(call) = call.try_lock()
                        && call.current_channel()!=Some(ChannelId::new(runtime.config.channel_id).into()) {return Err("Discord music transport disconnected.".into());}
                }
                event=terminal.recv()=>{
                    if runtime.current_epoch()!=epoch {break;}
                    return Err(format!("Music track terminated ({event:?}); capture stopped.").into());
                }
                frame=&mut next=>{buffer.push(&frame?)?;drop(next);next=Box::pin(stream.next());}
            }
        }
        drop(next);
        drop(stream);
        runtime.music.detach(generation);
        // Wait for the exact consent teardown marker, not just a phase change.
        // The marker is published after the old driver is gone; it cannot be
        // inherited by a later listening epoch.
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if !runtime.music.current(generation) {
                    return;
                }
                let snapshot = runtime.snapshot().await;
                if snapshot.media_enabled
                    || (snapshot.phase == VoicePhase::AwaitingConsent
                        && runtime.music_may_restore_output(snapshot.epoch))
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| "Voice transition did not settle; music stopped.")?;
        // Let the old consent teardown finish before replacing its driver. A new
        // /voice leave invalidates this music token before it waits for this lock.
        let _transition = runtime.transition.lock().await;
        if !runtime.music.current(generation) {
            return Ok(());
        }
        call = output_call(ctx, runtime).await?;
        stream = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if !runtime.music.current(generation) {
                    return Err(crate::audio_tap::TapError::Ended);
                }
                match connect_tap(&client, runtime, generation).await {
                    Err(crate::audio_tap::TapError::Busy) => {
                        tokio::time::sleep(Duration::from_millis(50)).await
                    }
                    result => return result,
                }
            }
        })
        .await
        .map_err(|_| "Previous capture did not finish stopping; music stayed off.")??;
    }
}

async fn connect_tap(
    client: &AudioTapClient,
    runtime: &VoiceRuntime,
    generation: u64,
) -> Result<TapStream, crate::audio_tap::TapError> {
    tokio::select! {
        biased;
        _ = async {
            while runtime.music.current(generation) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        } => Err(crate::audio_tap::TapError::Ended),
        result = client.connect() => result,
    }
}

#[cfg(test)]
mod tests {
    fn fixture_runtime() -> Arc<VoiceRuntime> {
        Arc::new(VoiceRuntime::new(crate::voice::VoiceConfig::selected_only(
            1,
            2,
            crate::voice::VoiceBackendConfig::Disabled,
            true,
        )))
    }

    #[tokio::test]
    async fn invalidated_music_generation_never_launches_after_a_pending_setup() {
        let music = fixture_runtime();
        let generation = music.music.begin(Player::Spotify);
        let (release, pending) = tokio::sync::oneshot::channel();
        let owner = music.clone();
        let task = tokio::spawn(async move {
            pending.await.unwrap();
            owner.music.launch_current(generation, || -> () {
                panic!("invalidated generation launched player")
            })
        });
        music.music.stop("leave", PlaybackTermination::Stopped);
        release.send(()).unwrap();
        assert!(task.await.unwrap().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stopping_music_cancels_and_reaps_the_owned_player_child() {
        let music = fixture_runtime();
        let generation = music.music.begin(Player::Spotify);
        let (child, token) = music
            .music
            .launch_current(generation, || {
                tokio::process::Command::new("/bin/sh")
                    .args(["-c", "exec sleep 60"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
            })
            .unwrap();
        let mut child = child.unwrap();
        music.music.stop("leave", PlaybackTermination::Stopped);
        let service_cancel = tokio_util::sync::CancellationToken::new();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(2),
                wait_player(&mut child, &service_cancel, &token)
            )
            .await
            .unwrap()
            .is_err()
        );
        assert!(
            child.try_wait().unwrap().is_some(),
            "cancellation returns only after child wait"
        );
    }

    #[test]
    fn cancelled_player_request_never_calls_the_process_launcher() {
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let launched = std::cell::Cell::new(false);
        let result = super::spawn_if_running(&cancel, || {
            launched.set(true);
            Ok(())
        });
        assert!(result.is_err());
        assert!(!launched.get());
    }
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn leave_cancels_pending_capture_request_before_headers_arrive() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (requested, request) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let received = socket.read(&mut bytes).await.unwrap();
            assert!(received > 0);
            requested.send(()).unwrap();
            // Header delay simulates a sidecar awaiting its first capture frame.
            let result = tokio::time::timeout(Duration::from_secs(1), socket.read(&mut bytes))
                .await
                .unwrap();
            assert_eq!(result.unwrap(), 0, "cancel must close the capture consumer");
            let _ = socket.shutdown().await;
        });
        let runtime = Arc::new(VoiceRuntime::new(crate::voice::VoiceConfig::selected_only(
            1,
            2,
            crate::voice::VoiceBackendConfig::Disabled,
            true,
        )));
        let generation = runtime.music.begin(Player::Spotify);
        let pending_runtime = runtime.clone();
        let pending = tokio::spawn(async move {
            connect_tap(
                &AudioTapClient::new(&endpoint).unwrap(),
                &pending_runtime,
                generation,
            )
            .await
            .err()
        });
        request.await.unwrap();
        runtime.music.stop("leave", PlaybackTermination::Stopped);
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), pending)
                .await
                .unwrap()
                .unwrap(),
            Some(crate::audio_tap::TapError::Ended)
        );
        assert!(!runtime.snapshot().await.media_enabled);
        server.await.unwrap();
    }
}
