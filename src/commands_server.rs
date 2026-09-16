//! `/server` surface: blueprint (emit-only) plus permission-mirrored guild actions.
//!
//! Mutations go through [`crate::permission_mirror`]: the requester and Abbey
//! must both hold the Discord permission for the action. Destructive ops need
//! `confirm:true`. Role deletion is not exposed here (holders > 0 is a hard ban
//! in the gate module for any future caller).

use serenity::all::{
    ChannelId, ChannelType, CreateChannel, EditChannel, EditMember, GuildId, Permissions, RoleId,
    User,
};

use crate::commands::ArchetypeChoice;
use crate::commands::clamp_message;
use crate::permission_mirror::{self, ActionContext, ServerAction};
use crate::server;
use crate::{Context, Error};

const NO_GUILD: &str = "This one only works inside a server.";

/// Parent — Discord forces a subcommand; body is unreachable wiring.
#[poise::command(
    slash_command,
    subcommands(
        "blueprint",
        "create_channel",
        "rename_channel",
        "slowmode",
        "delete_channel",
        "assign_role",
        "remove_role",
        "move_member",
        "purge"
    )
)]
pub async fn server(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Produce a server blueprint: role hierarchy, channel structure, numbered steps.
///
/// Emits a plan; it creates nothing. Live guild builds stay on the operator CLI
/// (`abbey-bot --server-plan`).
#[poise::command(slash_command, ephemeral, rename = "blueprint")]
pub async fn blueprint(
    ctx: Context<'_>,
    #[description = "What kind of server"] kind: ArchetypeChoice,
) -> Result<(), Error> {
    ctx.say(clamp_message(server::render(kind.into()))).await?;
    Ok(())
}

/// Create a text channel (hidden from @everyone until you set overwrites yourself).
#[poise::command(slash_command, guild_only, ephemeral, rename = "create-channel")]
pub async fn create_channel(
    ctx: Context<'_>,
    #[description = "Channel name"] name: String,
    #[description = "Optional category to place it under"] category: Option<ChannelId>,
    #[description = "Optional topic"] topic: Option<String>,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::ChannelCreate,
        requester,
        bot,
        ActionContext::default(),
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let name = normalize_channel_name(&name);
    if name.is_empty() {
        ctx.say("Channel name cannot be empty after normalization.").await?;
        return Ok(());
    }

    let mut builder = CreateChannel::new(&name).kind(ChannelType::Text);
    if let Some(parent) = category {
        builder = builder.category(parent);
    }
    if let Some(topic) = topic.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        builder = builder.topic(topic);
    }
    let reason = audit_reason(ctx);
    builder = builder.audit_log_reason(&reason);

    let channel = guild_id.create_channel(ctx.http(), builder).await?;
    ctx.say(clamp_message(format!(
        "Created <#{}> (`{}`). Set overwrites next if it should stay private.",
        channel.id, channel.name
    )))
    .await?;
    Ok(())
}

/// Rename a channel.
#[poise::command(slash_command, guild_only, ephemeral, rename = "rename-channel")]
pub async fn rename_channel(
    ctx: Context<'_>,
    #[description = "Channel to rename"] channel: ChannelId,
    #[description = "New name"] name: String,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::ChannelEdit,
        requester,
        bot,
        ActionContext::default(),
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let name = normalize_channel_name(&name);
    if name.is_empty() {
        ctx.say("Channel name cannot be empty after normalization.").await?;
        return Ok(());
    }
    ensure_guild_channel(ctx, guild_id, channel).await?;
    let reason = audit_reason(ctx);
    channel
        .edit(
            ctx.http(),
            EditChannel::new().name(&name).audit_log_reason(&reason),
        )
        .await?;
    ctx.say(clamp_message(format!("Renamed <#{}> to `{}`.", channel, name)))
        .await?;
    Ok(())
}

/// Set text-channel slowmode (0–21600 seconds).
#[poise::command(slash_command, guild_only, ephemeral, rename = "slowmode")]
pub async fn slowmode(
    ctx: Context<'_>,
    #[description = "Text channel"] channel: ChannelId,
    #[description = "Seconds (0 disables)"] seconds: i64,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::Slowmode,
        requester,
        bot,
        ActionContext::default(),
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let secs = seconds.clamp(0, 21_600) as u16;
    ensure_guild_channel(ctx, guild_id, channel).await?;
    let reason = audit_reason(ctx);
    channel
        .edit(
            ctx.http(),
            EditChannel::new()
                .rate_limit_per_user(secs)
                .audit_log_reason(&reason),
        )
        .await?;
    ctx.say(clamp_message(format!(
        "Slowmode on <#{}> is now **{secs}s**.",
        channel
    )))
    .await?;
    Ok(())
}

/// Delete a channel. Requires `confirm:true` (non-destructive by default).
#[poise::command(slash_command, guild_only, ephemeral, rename = "delete-channel")]
pub async fn delete_channel(
    ctx: Context<'_>,
    #[description = "Channel to delete"] channel: ChannelId,
    #[description = "Must be true to proceed"] confirm: bool,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::ChannelDelete,
        requester,
        bot,
        ActionContext {
            confirm,
            ..ActionContext::default()
        },
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let guild_channel = ensure_guild_channel(ctx, guild_id, channel).await?;
    let label = guild_channel.name.clone();
    channel.delete(ctx.http()).await?;
    ctx.say(clamp_message(format!("Deleted channel `#{label}`.")))
        .await?;
    Ok(())
}

/// Assign a role to a member.
#[poise::command(slash_command, guild_only, ephemeral, rename = "assign-role")]
pub async fn assign_role(
    ctx: Context<'_>,
    #[description = "Member"] user: User,
    #[description = "Role to add"] role: RoleId,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::RoleAssign,
        requester,
        bot,
        ActionContext::default(),
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let member = guild_id.member(ctx.http(), user.id).await?;
    member.add_role(ctx.http(), role).await?;
    ctx.say(clamp_message(format!(
        "Assigned <@&{}> to **{}**.",
        role, user.name
    )))
    .await?;
    Ok(())
}

/// Remove a role from a member.
#[poise::command(slash_command, guild_only, ephemeral, rename = "remove-role")]
pub async fn remove_role(
    ctx: Context<'_>,
    #[description = "Member"] user: User,
    #[description = "Role to remove"] role: RoleId,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::RoleRemove,
        requester,
        bot,
        ActionContext::default(),
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let member = guild_id.member(ctx.http(), user.id).await?;
    member.remove_role(ctx.http(), role).await?;
    ctx.say(clamp_message(format!(
        "Removed <@&{}> from **{}**.",
        role, user.name
    )))
    .await?;
    Ok(())
}

/// Move a member to another voice channel (both need Move Members).
#[poise::command(slash_command, guild_only, ephemeral, rename = "move-member")]
pub async fn move_member(
    ctx: Context<'_>,
    #[description = "Member in voice"] user: User,
    #[description = "Destination voice channel"] channel: ChannelId,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::MoveMember,
        requester,
        bot,
        ActionContext::default(),
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let dest = ensure_guild_channel(ctx, guild_id, channel).await?;
    if !matches!(dest.kind, ChannelType::Voice | ChannelType::Stage) {
        ctx.say("Destination must be a voice or stage channel.").await?;
        return Ok(());
    }

    let reason = audit_reason(ctx);
    guild_id
        .edit_member(
            ctx.http(),
            user.id,
            EditMember::new()
                .voice_channel(channel)
                .audit_log_reason(&reason),
        )
        .await?;
    ctx.say(clamp_message(format!(
        "Moved **{}** to <#{}>.",
        user.name, channel
    )))
    .await?;
    Ok(())
}

/// Bulk-delete recent messages. Requires `confirm:true` and Manage Messages.
#[poise::command(slash_command, guild_only, ephemeral, rename = "purge")]
pub async fn purge(
    ctx: Context<'_>,
    #[description = "Channel to purge"] channel: ChannelId,
    #[description = "How many messages (2–100)"] count: i64,
    #[description = "Must be true to proceed"] confirm: bool,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let (requester, bot) = load_mirror_perms(ctx, guild_id).await?;
    if let Err(denial) = permission_mirror::authorize(
        ServerAction::PurgeMessages,
        requester,
        bot,
        ActionContext {
            confirm,
            ..ActionContext::default()
        },
    ) {
        ctx.say(clamp_message(denial.message())).await?;
        return Ok(());
    }

    let count = count.clamp(2, 100) as u8;
    ensure_guild_channel(ctx, guild_id, channel).await?;
    let deleted = channel
        .messages(ctx.http(), serenity::all::GetMessages::new().limit(count))
        .await?;
    let ids: Vec<_> = deleted.iter().map(|m| m.id).collect();
    if ids.len() < 2 {
        ctx.say("Need at least two messages younger than two weeks to bulk-delete.")
            .await?;
        return Ok(());
    }
    channel.delete_messages(ctx.http(), &ids).await?;
    ctx.say(clamp_message(format!(
        "Purged **{}** message(s) in <#{}>.",
        ids.len(),
        channel
    )))
    .await?;
    Ok(())
}

async fn load_mirror_perms(
    ctx: Context<'_>,
    guild_id: GuildId,
) -> Result<(Permissions, Permissions), Error> {
    let guild = guild_id.to_partial_guild(ctx.http()).await?;
    let author_id = ctx.author().id;
    let requester_member = guild_id.member(ctx.http(), author_id).await?;
    let requester = guild.member_permissions(&requester_member);

    let bot_user = ctx.http().get_current_user().await?;
    let bot_member = guild_id.member(ctx.http(), bot_user.id).await?;
    let bot = guild.member_permissions(&bot_member);
    Ok((requester, bot))
}

async fn ensure_guild_channel(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel: ChannelId,
) -> Result<serenity::all::GuildChannel, Error> {
    let channel = channel.to_channel(ctx.http()).await?;
    let Some(guild_channel) = channel.guild() else {
        return Err("That is not a server channel.".into());
    };
    if guild_channel.guild_id != guild_id {
        return Err("That channel is not in this server.".into());
    }
    Ok(guild_channel)
}

fn normalize_channel_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_whitespace() {
                '-'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn audit_reason(ctx: Context<'_>) -> String {
    format!(
        "abbey-bot /server by {} ({})",
        ctx.author().name,
        ctx.author().id
    )
}
