//! Read the existing voice lifecycle and music output state without changing consent.
use crate::Data;
use serenity::all::{ChannelId, ComponentInteraction, GuildId, Permissions};

pub(super) async fn render(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
) -> String {
    let Some(runtime) = interaction.guild_id.and_then(|g| data.voice_for(g.get())) else {
        return "**Voice & Music**\nNo voice session is bound in this server. A manager can join a voice channel and use `/voice join` to select it, then each participant can review `/voice consent` before listening starts. `/voice status` shows configuration guidance. Music also needs a working host player/audio sidecar. Opening this view starts neither music nor listening.".into();
    };
    let permissions = match super::super::current_permissions(ctx, GuildId::new(runtime.config.guild_id), ChannelId::new(runtime.config.channel_id), interaction.user.id).await {
        Ok(p) => p,
        Err(_) => return "Discord could not confirm access to the voice destination. Use `/voice status` to try again.".into(),
    };
    let present =
        super::super::presence(ctx, data, interaction.guild_id, interaction.user.id) == Some(true);
    let mode = runtime.effective_mode();
    let snapshot = runtime.snapshot().await;
    let view = crate::voice_views::MemberVoiceView::project(crate::voice_views::MemberVoiceInput {
        configured: true,
        phase: Some(snapshot.phase),
        mode,
        caller_agrees: runtime.consent.agrees(interaction.user.id.get(), mode),
        channel_id: Some(runtime.config.channel_id),
        caller_can_view_channel: permissions.contains(Permissions::VIEW_CHANNEL),
        caller_present: present,
        caller_can_manage: permissions.contains(Permissions::MANAGE_GUILD),
    });
    let mut body = view.render();
    if permissions.contains(Permissions::VIEW_CHANNEL) {
        body.push_str(&format!("\n\n**Music output**\n{}\nThis is the current local output state, not evidence of an audible Discord result.", member_music_status(&runtime.music.status())));
        if permissions.contains(Permissions::MANAGE_GUILD) {
            body.push_str("\nUse `/voice play` for the host player's current selection, `/voice pause` to pause music, `/voice resume-music` to resume, and `/voice stop-music` to stop it. Playback needs a working host player/audio sidecar and voice output; this view does not probe the sidecar.");
        } else {
            body.push_str("\nA server manager controls music playback.");
        }
    }
    body.push_str("\nMusic never grants listening agreement. Review `/voice consent` for your agreement; `/voice leave` stops voice for a present member or manager.");
    body
}

/// Music's internal status can contain a sidecar error. Members receive only a
/// closed phase label, never the error text, host paths, or player metadata.
fn member_music_status(status: &str) -> &'static str {
    match status
        .strip_prefix("Music: ")
        .and_then(|s| s.split_once("; volume "))
        .map(|(phase, _)| phase)
    {
        Some("starting") => "Music: starting",
        Some("playing") => "Music: playing",
        Some("paused") => "Music: paused",
        Some("stopped" | "Music stopped.") => "Music: stopped",
        _ => "Music: stopped or unavailable; a manager can review `/voice diagnostics`.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn member_music_projection_never_exposes_operational_error_text() {
        assert_eq!(
            member_music_status("Music: playing; volume 100%"),
            "Music: playing"
        );
        let rendered =
            member_music_status("Music: failure at /private/host token=secret; volume 100%");
        assert!(!rendered.contains("private") && !rendered.contains("secret"));
        assert!(rendered.contains("unavailable"));
    }
}
