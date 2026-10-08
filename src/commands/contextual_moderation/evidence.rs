//! Current native contextual facts; plaintext is discarded after a scoped digest.
use super::*;
use crate::moderation::shadow::{FreshAuthority, NativeAuthorityFacts, SourceVersion};
use serenity::all::{CommandType, InteractionContext, MessageId, RoleId, UserId};
use std::sync::atomic::Ordering;

const UNPROVED: &str =
    "Current source or moderator access could not be confirmed. No contextual proposal qualified.";

#[derive(Clone, Copy)]
pub(super) struct Envelope {
    pub(super) guild: u64,
    pub(super) channel: u64,
    pub(super) actor: u64,
}
pub(super) fn envelope(ctx: Context<'_>) -> Result<Envelope, &'static str> {
    let poise::Context::Application(app) = ctx else {
        return Err(UNPROVED);
    };
    let native = app.interaction;
    let binding = app
        .command
        .custom_data
        .downcast_ref::<crate::commands_help::CatalogBinding>()
        .ok_or(UNPROVED)?;
    if app.interaction_type != poise::CommandInteractionType::Command
        || native.data.kind != CommandType::ChatInput
        || native.data.name != "modcall"
        || native.context != Some(InteractionContext::Guild)
        || binding.key != crate::command_catalog::CommandKey::Modcall
        || app.command.qualified_name != "modcall"
        || !app.command.guild_only
        || !app.command.ephemeral
        || !app.has_sent_initial_response.load(Ordering::SeqCst)
        || native.application_id.get() != app.serenity_context.cache.current_user().id.get()
        || native.user.bot
        || ctx.author().bot
    {
        return Err(UNPROVED);
    }
    let facts = Envelope {
        guild: native.guild_id.ok_or(UNPROVED)?.get(),
        channel: native.channel_id.get(),
        actor: native.user.id.get(),
    };
    if [facts.guild, facts.channel, facts.actor].contains(&0) {
        return Err(UNPROVED);
    }
    Ok(facts)
}
pub(super) struct Facts {
    pub(super) owner: u64,
    pub(super) source: SourceVersion,
    pub(super) input: contextual::Input,
    native: Envelope,
    started: u64,
}
impl Facts {
    pub(super) fn authority(&self) -> Result<FreshAuthority, &'static str> {
        FreshAuthority::verified(NativeAuthorityFacts {
            guild: self.native.guild,
            owner: self.owner,
            actor: self.native.actor,
            origin: self.native.channel,
            at: self.started,
            current_member: true,
            can_view_origin: true,
            can_view_source: true,
            complete_permissions: true,
            can_delete: self.input.moderator_can_delete,
            can_timeout: self.input.moderator_can_timeout,
            observed_source: Some(self.source.clone()),
        })
    }
}
pub(super) async fn prove(
    http: &serenity::all::Http,
    native: Envelope,
    target: u64,
    message: u64,
    severity: Severity,
    assessment: Assessment,
) -> Result<Facts, &'static str> {
    if target == 0 || message == 0 {
        return Err(UNPROVED);
    }
    let started = crate::runtime::now();
    let gid = GuildId::new(native.guild);
    let (guild, moderator, subject, channel) = tokio::try_join!(
        gid.to_partial_guild(http),
        gid.member(http, UserId::new(native.actor)),
        gid.member(http, UserId::new(target)),
        ChannelId::new(native.channel).to_channel(http),
    )
    .map_err(|_| UNPROVED)?;
    let channel = channel.guild().ok_or(UNPROVED)?;
    if !member_facts_match(&moderator, &guild, gid, UserId::new(native.actor))
        || !member_facts_match(&subject, &guild, gid, UserId::new(target))
        || guild.owner_id.get() == 0
        || moderator.user.bot
        || channel.id.get() != native.channel
        || channel.guild_id != gid
        || !matches!(channel.kind, ChannelType::Text | ChannelType::News)
        || guild
            .roles
            .iter()
            .any(|(id, role)| id.get() == 0 || *id != role.id)
        || !guild.roles.contains_key(&RoleId::new(native.guild))
    {
        return Err(UNPROVED);
    }
    let held = guild.user_permissions_in(&channel, &moderator);
    if !held.contains(Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY) {
        return Err(UNPROVED);
    }
    // Read the exact source after all permission/hierarchy facts are observed.
    let source = ChannelId::new(native.channel)
        .message(http, MessageId::new(message))
        .await
        .map_err(|_| UNPROVED)?;
    if source.id.get() != message
        || source.channel_id.get() != native.channel
        || source.guild_id.is_some_and(|id| id != gid)
        || source.author.id.get() != target
        || source.author.bot != subject.user.bot
    {
        return Err(UNPROVED);
    }
    let created = u64::try_from(source.timestamp.unix_timestamp()).map_err(|_| UNPROVED)?;
    let edited = source
        .edited_timestamp
        .map(|at| u64::try_from(at.unix_timestamp()).map_err(|_| UNPROVED))
        .transpose()?;
    let version = SourceVersion::capture(
        native.guild,
        native.channel,
        message,
        target,
        created,
        edited,
        &source.content,
    )?;
    let now = crate::runtime::now();
    if created > now || edited.is_some_and(|at| at > now) {
        return Err(UNPROVED);
    }
    let staff = Permissions::ADMINISTRATOR
        | Permissions::MANAGE_GUILD
        | Permissions::MODERATE_MEMBERS
        | Permissions::MANAGE_MESSAGES
        | Permissions::KICK_MEMBERS
        | Permissions::BAN_MEMBERS;
    let target_guild = guild.member_permissions(&subject);
    let target_channel = guild.user_permissions_in(&channel, &subject);
    let input = contextual::Input {
        guild: native.guild,
        channel: native.channel,
        message,
        source_author: target,
        target,
        source_matches_scope: true,
        content_available: true,
        target_is_bot: subject.user.bot || source.author.bot,
        target_is_staff: UserId::new(target) == guild.owner_id
            || target_guild.intersects(staff)
            || target_channel.intersects(staff),
        moderator_can_delete: held.contains(Permissions::MANAGE_MESSAGES),
        moderator_can_timeout: held.contains(Permissions::MODERATE_MEMBERS),
        hierarchy_allows: moderation::hierarchy_blocker(
            UserId::new(native.actor) == guild.owner_id,
            top_role_position(&moderator, &guild),
            UserId::new(target) == guild.owner_id,
            target_guild.contains(Permissions::ADMINISTRATOR),
            top_role_position(&subject, &guild),
            true,
        )
        .is_none(),
        assessment,
        severity,
    };
    Ok(Facts {
        owner: guild.owner_id.get(),
        source: version,
        input,
        native,
        started,
    })
}
