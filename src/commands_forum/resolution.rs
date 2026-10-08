//! Current REST proof and the one explicit tag-only forum mutation.

use serenity::all::{
    ChannelId, ChannelType, EditThread, ForumTagId, GuildChannel, GuildId, Http, Permissions,
    UserId,
};

use crate::forum_resolution::{ResolutionError, ResolutionFacts, ResolutionState, resolution_tags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResolveError {
    InvalidThread,
    InactiveThread,
    Denied,
    InvalidTags,
    Unavailable,
}

fn valid_thread(thread: &GuildChannel, guild: GuildId) -> Result<ChannelId, ResolveError> {
    if thread.guild_id != guild || thread.kind != ChannelType::PublicThread {
        return Err(ResolveError::InvalidThread);
    }
    let metadata = thread
        .thread_metadata
        .as_ref()
        .ok_or(ResolveError::InvalidThread)?;
    if metadata.archived || metadata.locked {
        return Err(ResolveError::InactiveThread);
    }
    if thread.owner_id.is_none() {
        return Err(ResolveError::InvalidThread);
    }
    thread.parent_id.ok_or(ResolveError::InvalidThread)
}

fn status_tag(channel: &GuildChannel, name: &str) -> Option<u64> {
    let mut matched = channel
        .available_tags
        .iter()
        .filter(|tag| tag.name.eq_ignore_ascii_case(name));
    let id = matched.next()?.id.get();
    matched.next().is_none().then_some(id)
}

/// Invoked only after the command's private acknowledgement. Every permission
/// fact comes from REST; the final thread fetch preserves the latest tag set.
pub(super) async fn resolve_thread(
    http: &Http,
    guild_id: GuildId,
    thread_id: ChannelId,
    actor_id: UserId,
    state: ResolutionState,
) -> Result<bool, ResolveError> {
    let thread = thread_id
        .to_channel(http)
        .await
        .map_err(|_| ResolveError::Unavailable)?
        .guild()
        .ok_or(ResolveError::InvalidThread)?;
    let parent_id = valid_thread(&thread, guild_id)?;
    let (parent, guild, bot) = tokio::try_join!(
        parent_id.to_channel(http),
        guild_id.to_partial_guild(http),
        http.get_current_user()
    )
    .map_err(|_| ResolveError::Unavailable)?;
    let parent = parent.guild().ok_or(ResolveError::InvalidThread)?;
    if parent.id != parent_id
        || parent.guild_id != guild_id
        || parent.kind != ChannelType::Forum
        || guild.id != guild_id
    {
        return Err(ResolveError::InvalidThread);
    }
    let (actor, bot_member) = tokio::try_join!(
        guild_id.member(http, actor_id),
        guild_id.member(http, bot.id)
    )
    .map_err(|_| ResolveError::Unavailable)?;
    if actor.user.id != actor_id || bot_member.user.id != bot.id {
        return Err(ResolveError::Denied);
    }
    let actor_permissions = guild.user_permissions_in(&parent, &actor);
    let bot_permissions = guild.user_permissions_in(&parent, &bot_member);
    if !actor_permissions.contains(Permissions::VIEW_CHANNEL)
        || !bot_permissions.contains(Permissions::VIEW_CHANNEL | Permissions::MANAGE_THREADS)
    {
        return Err(ResolveError::Denied);
    }
    let current = thread_id
        .to_channel(http)
        .await
        .map_err(|_| ResolveError::Unavailable)?
        .guild()
        .ok_or(ResolveError::InvalidThread)?;
    if current.id != thread_id || valid_thread(&current, guild_id)? != parent_id {
        return Err(ResolveError::InvalidThread);
    }
    let before: Vec<u64> = current.applied_tags.iter().map(|tag| tag.get()).collect();
    let can_manage_thread = actor_permissions.contains(Permissions::MANAGE_THREADS);
    let planned = resolution_tags(
        ResolutionFacts {
            is_thread_author: current.owner_id == Some(actor_id),
            can_manage_thread,
            current_tags: &before,
            solved_tag_id: status_tag(&parent, "Solved"),
            unresolved_tag_id: status_tag(&parent, "Unresolved"),
        },
        state,
    )
    .map_err(|error| match error {
        ResolutionError::Denied => ResolveError::Denied,
        ResolutionError::InvalidTags => ResolveError::InvalidTags,
    })?;
    // Abbey's permission to edit the thread cannot grant the actor authority
    // to add or remove a moderated status tag.
    if !can_manage_thread
        && parent.available_tags.iter().any(|tag| {
            tag.moderated
                && (tag.name.eq_ignore_ascii_case("Solved")
                    || tag.name.eq_ignore_ascii_case("Unresolved"))
                && before.contains(&tag.id.get()) != planned.contains(&tag.id.get())
        })
    {
        return Err(ResolveError::Denied);
    }
    if before == planned {
        return Ok(false);
    }
    let updated = thread_id
        .edit_thread(
            http,
            EditThread::new()
                .applied_tags(planned.iter().copied().map(ForumTagId::new))
                .audit_log_reason("Abbey explicit /forum resolve"),
        )
        .await
        .map_err(|_| ResolveError::Unavailable)?;
    let observed: Vec<u64> = updated.applied_tags.iter().map(|tag| tag.get()).collect();
    if updated.id != thread_id
        || updated.guild_id != guild_id
        || updated.parent_id != Some(parent_id)
        || observed.len() != planned.len()
        || !planned.iter().all(|tag| observed.contains(tag))
    {
        return Err(ResolveError::Unavailable);
    }
    Ok(true)
}

pub(super) fn reply(result: Result<bool, ResolveError>, state: ResolutionState) -> &'static str {
    match (result, state) {
        (Ok(true), ResolutionState::Solved) => {
            "Marked this forum post Solved. Other tags and thread history are unchanged."
        }
        (Ok(true), ResolutionState::Unresolved) => {
            "Marked this forum post Unresolved. Other tags and thread history are unchanged."
        }
        (Ok(false), ResolutionState::Solved) => "This forum post is already marked Solved.",
        (Ok(false), ResolutionState::Unresolved) => "This forum post is already marked Unresolved.",
        (Err(ResolveError::InvalidThread), _) => {
            "Choose a forum post in this server, or run `/forum resolve` inside one."
        }
        (Err(ResolveError::InactiveThread), _) => {
            "This forum post is archived or locked. Resolution will not reopen or unlock it."
        }
        (Err(ResolveError::Denied), _) => {
            "You must be able to view the forum and own the post or have Manage Threads. Changing moderated status tags also requires Manage Threads. Abbey needs View Channel and Manage Threads."
        }
        (Err(ResolveError::InvalidTags), _) => {
            "This forum needs exactly one Solved tag and one Unresolved tag. Its existing tags must fit the five-tag limit; resolution will not remove unrelated tags."
        }
        (Err(ResolveError::Unavailable), _) => {
            "Discord could not confirm the current forum state or update. Check the post's tags before trying again."
        }
    }
}

#[cfg(test)]
mod tests;
