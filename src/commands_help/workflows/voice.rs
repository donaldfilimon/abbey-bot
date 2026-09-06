//! Read the existing voice lifecycle and music output state without changing consent.
use crate::Data;
use serenity::all::{ChannelId, ComponentInteraction, GuildId, Permissions};

pub(super) async fn render(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
) -> String {
    let Some(runtime) = data.voice.as_ref().filter(|v| {
        interaction
            .guild_id
            .is_some_and(|g| g.get() == v.config.guild_id)
    }) else {
        return "**Voice & Music**\nNo voice destination is configured for this server. A manager can review `/voice status` and configure the service. Music needs that configured voice output and a working host player/audio sidecar. Opening this view starts neither music nor listening.".into();
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
        body.push_str(&format!("\n\n**Music output**\n{}\nThis is the current local output state, not evidence of an audible Discord result.", runtime.music.status()));
        if permissions.contains(Permissions::MANAGE_GUILD) {
            body.push_str("\nUse `/voice play` for the host player's current selection, `/voice pause` to pause music, `/voice resume-music` to resume, and `/voice stop-music` to stop it. Playback needs a working host player/audio sidecar and voice output; this view does not probe the sidecar.");
        } else {
            body.push_str("\nA server manager controls music playback.");
        }
    }
    body.push_str("\nMusic never grants listening agreement. Review `/voice consent` for your agreement; `/voice leave` stops voice for a present member or manager.");
    body
}
