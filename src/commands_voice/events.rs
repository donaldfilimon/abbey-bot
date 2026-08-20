//! One Discord voice adapter for operational text, voice-state evidence, and
//! permission-affecting gateway events.

use serenity::all::{CreateMessage, FullEvent, MessageReference};

use super::discord::{
    bot_has_required_voice_permissions, member_has_required_voice_permissions,
    required_voice_permissions,
};
use super::supervision::{
    on_voice_permissions_changed, on_voice_state_update, withdraw_voice_from_text,
};
use crate::voice_session::{authoritative_text_reply, requests_consent_withdrawal};

/// Handle the voice-specific portion of a gateway event. `true` means an
/// operational text message was answered and must not also enter the social
/// generation pipeline. Other events return `false` after supervision so the
/// ordinary gateway adapter can still process its own social bookkeeping.
pub async fn on_gateway_event(
    ctx: &serenity::all::Context,
    event: &FullEvent,
    data: &crate::Data,
) -> bool {
    match event {
        FullEvent::Message { new_message } => {
            handle_voice_control_message(ctx, new_message, data).await
        }
        FullEvent::VoiceStateUpdate { old, new } => {
            on_voice_state_update(ctx, old, new, data).await;
            false
        }
        FullEvent::ChannelDelete { channel, .. } => {
            on_voice_permissions_changed(ctx, channel.guild_id, Some(channel.id), true, data).await;
            false
        }
        FullEvent::ChannelUpdate { new, .. } => {
            let payload_revoked = new.kind != serenity::all::ChannelType::Voice
                || !bot_has_required_voice_permissions(ctx, new);
            on_voice_permissions_changed(ctx, new.guild_id, Some(new.id), payload_revoked, data)
                .await;
            false
        }
        FullEvent::GuildRoleUpdate {
            old_data_if_available,
            new,
        } => {
            let payload_revoked = role_is_relevant_to_bot(ctx, new.guild_id, new.id)
                && old_data_if_available.as_ref().is_some_and(|old| {
                    permission_grant_was_reduced(old.permissions, new.permissions)
                });
            // Always run the supplemental effective-permission check. The
            // newest member cache may already have removed this role, so it
            // cannot be the sole authority for whether the payload mattered.
            on_voice_permissions_changed(ctx, new.guild_id, None, payload_revoked, data).await;
            false
        }
        FullEvent::GuildRoleDelete {
            guild_id,
            removed_role_id: _,
            removed_role_data_if_available,
        } => {
            let carried_required = removed_role_data_if_available.as_ref().is_some_and(|role| {
                role.permissions.administrator()
                    || !(role.permissions & required_voice_permissions()).is_empty()
            });
            // Discord does not include the affected members in a role-delete
            // payload, and the cache may already have removed the assignment.
            // Deleting any grant-capable role is therefore treated as adverse
            // while voice is active; deletion is rare and a false-safe stop is
            // preferable to losing the only bidirectional grant transiently.
            on_voice_permissions_changed(ctx, *guild_id, None, carried_required, data).await;
            false
        }
        FullEvent::GuildMemberUpdate {
            old_if_available,
            new,
            event,
        } if event.user.id == ctx.cache.current_user().id => {
            let configured_channel = data.voice.as_ref().and_then(|runtime| {
                ctx.cache.guild(event.guild_id).and_then(|guild| {
                    guild
                        .channels
                        .get(&serenity::all::ChannelId::new(runtime.config.channel_id))
                        .cloned()
                })
            });
            // `new` is a cache convenience and may already represent a later
            // update. Use it (or `old`) only as a structural Member value and
            // overwrite roles with the immutable roles from this payload.
            let payload_member = new.as_ref().or(old_if_available.as_ref()).map(|member| {
                let mut member = member.clone();
                member.roles.clone_from(&event.roles);
                member
            });
            let payload_revoked = match (payload_member.as_ref(), configured_channel.as_ref()) {
                (Some(member), Some(channel)) => {
                    !member_has_required_voice_permissions(ctx, channel, member)
                }
                // The payload identifies the current bot, but without both
                // the payload member and configured channel we cannot prove
                // the updated role set retains every effective grant. Fail
                // closed instead of allowing a newer/incomplete cache to
                // erase this event's evidence.
                _ => role_membership_changed(
                    old_if_available.as_ref().map(|old| old.roles.as_slice()),
                    &event.roles,
                ),
            };
            on_voice_permissions_changed(ctx, event.guild_id, None, payload_revoked, data).await;
            false
        }
        FullEvent::GuildDelete { incomplete, .. } => {
            on_voice_permissions_changed(ctx, incomplete.id, None, true, data).await;
            false
        }
        _ => false,
    }
}

fn permission_grant_was_reduced(
    old: serenity::all::Permissions,
    new: serenity::all::Permissions,
) -> bool {
    let old_required = old & required_voice_permissions();
    !new.contains(old_required) || (old.administrator() && !new.administrator())
}

fn role_membership_changed(
    old: Option<&[serenity::all::RoleId]>,
    new: &[serenity::all::RoleId],
) -> bool {
    let Some(old) = old else {
        return true;
    };
    old.len() != new.len() || old.iter().any(|role| !new.contains(role))
}

fn role_is_relevant_to_bot(
    ctx: &serenity::all::Context,
    guild_id: serenity::all::GuildId,
    role_id: serenity::all::RoleId,
) -> bool {
    if role_id == guild_id.everyone_role() {
        return true;
    }
    let bot_id = ctx.cache.current_user().id;
    ctx.cache
        .guild(guild_id)
        .and_then(|guild| guild.members.get(&bot_id).cloned())
        .is_some_and(|member| member.roles.contains(&role_id))
}

async fn handle_voice_control_message(
    ctx: &serenity::all::Context,
    message: &serenity::all::Message,
    data: &crate::Data,
) -> bool {
    let Some(runtime) = data.voice.as_ref() else {
        return false;
    };
    let bot_id = ctx.cache.current_user().id;
    if !voice_control_scope(
        runtime.config.guild_id,
        runtime.config.channel_id,
        message.guild_id.map(|guild| guild.get()),
        message.channel_id.get(),
        message.author.bot,
        message.author.id.get(),
        bot_id.get(),
    ) {
        return false;
    }

    let translated_text = crate::gateway::strip_bot_mention(&message.content, bot_id.get());
    let mut snapshot = runtime.snapshot().await;
    let Some(mut text) = authoritative_text_reply(&translated_text, &snapshot) else {
        return false;
    };
    if requests_consent_withdrawal(&translated_text, &snapshot) {
        let _ = withdraw_voice_from_text(ctx, runtime, message.author.id.get()).await;
        snapshot = runtime.snapshot().await;
        if let Some(updated) = authoritative_text_reply(&translated_text, &snapshot) {
            text = updated;
        }
    }

    let mut reference: MessageReference = (message.channel_id, message.id).into();
    reference.fail_if_not_exists = Some(false);
    let builder = CreateMessage::new()
        .content(text)
        .allowed_mentions(crate::gateway::no_mentions())
        .reference_message(reference);
    match message.channel_id.send_message(&ctx.http, builder).await {
        Ok(sent) => tracing::info!(
            message = message.id.get(),
            reply = sent.id.get(),
            phase = snapshot.phase.label(),
            "voice-state text answered from runtime snapshot"
        ),
        Err(error) => tracing::warn!(
            message = message.id.get(),
            %error,
            "voice-state text reply failed"
        ),
    }
    true
}

fn voice_control_scope(
    configured_guild: u64,
    configured_channel: u64,
    message_guild: Option<u64>,
    message_channel: u64,
    author_is_bot: bool,
    author_id: u64,
    bot_id: u64,
) -> bool {
    !author_is_bot
        && author_id != bot_id
        && message_guild == Some(configured_guild)
        && message_channel == configured_channel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_boundary_scope_is_exact_and_excludes_bots_and_self() {
        let in_scope = |guild, channel, bot, author| {
            voice_control_scope(10, 20, guild, channel, bot, author, 99)
        };
        assert!(in_scope(Some(10), 20, false, 42));
        assert!(!in_scope(Some(11), 20, false, 42));
        assert!(!in_scope(Some(10), 21, false, 42));
        assert!(!in_scope(None, 20, false, 42));
        assert!(!in_scope(Some(10), 20, true, 42));
        assert!(!in_scope(Some(10), 20, false, 99));
    }

    #[test]
    fn required_bits_detect_role_permission_reduction() {
        let old = serenity::all::Permissions::VIEW_CHANNEL
            | serenity::all::Permissions::CONNECT
            | serenity::all::Permissions::SPEAK;
        let new = serenity::all::Permissions::VIEW_CHANNEL | serenity::all::Permissions::CONNECT;
        assert!(permission_grant_was_reduced(old, new));
        assert!(!permission_grant_was_reduced(new, old));
        assert!(permission_grant_was_reduced(
            serenity::all::Permissions::ADMINISTRATOR,
            required_voice_permissions()
        ));
    }

    #[test]
    fn incomplete_member_payload_fails_closed_only_when_membership_cannot_be_preserved() {
        use serenity::all::RoleId;

        let old = [RoleId::new(1), RoleId::new(2)];
        assert!(!role_membership_changed(
            Some(&old),
            &[RoleId::new(2), RoleId::new(1)]
        ));
        assert!(role_membership_changed(Some(&old), &[RoleId::new(1)]));
        assert!(role_membership_changed(
            Some(&old),
            &[RoleId::new(1), RoleId::new(2), RoleId::new(3)]
        ));
        assert!(role_membership_changed(None, &old));
    }
}
