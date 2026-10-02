//! Discord evidence lookup for the bounded contextual moderation pilot.
use super::*;
use crate::moderation::contextual::{self, Assessment};

#[derive(Debug, poise::ChoiceParameter)]
pub enum ContextAssessmentChoice {
    #[name = "confirmed offending in context"]
    Confirmed,
    #[name = "ambiguous or insufficient context"]
    Ambiguous,
    #[name = "quotation or member report"]
    Quotation,
}

pub(super) async fn propose(
    ctx: Context<'_>,
    guild: &PartialGuild,
    moderator: &Member,
    user: &User,
    severity: Severity,
    source_message: &str,
    assessment: Option<ContextAssessmentChoice>,
) -> Result<(), Error> {
    let Some(id) = source_message.parse::<u64>().ok().filter(|id| *id != 0) else {
        ctx.say("Provide the numeric ID of a message in this channel. No contextual proposal qualified.").await?;
        return Ok(());
    };
    let message = match ctx
        .channel_id()
        .message(ctx.http(), serenity::all::MessageId::new(id))
        .await
    {
        Ok(message) => message,
        Err(_) => {
            ctx.say("The source message could not be read in this channel. No contextual proposal qualified.").await?;
            return Ok(());
        }
    };
    let target = guild.id.member(ctx.http(), user.id).await?;
    let held = crate::commands_help::current_permissions(
        ctx.serenity_context(),
        guild.id,
        ctx.channel_id(),
        ctx.author().id,
    )
    .await?;
    let target_permissions = guild.member_permissions(&target);
    let target_is_staff = user.id == guild.owner_id
        || target_permissions.intersects(
            Permissions::ADMINISTRATOR
                | Permissions::MANAGE_GUILD
                | Permissions::MODERATE_MEMBERS
                | Permissions::MANAGE_MESSAGES
                | Permissions::KICK_MEMBERS
                | Permissions::BAN_MEMBERS,
        );
    let decision = contextual::qualify(contextual::Input {
        guild: guild.id.get(),
        channel: ctx.channel_id().get(),
        message: message.id.get(),
        source_author: message.author.id.get(),
        target: user.id.get(),
        source_matches_scope: message.channel_id == ctx.channel_id()
            && message.guild_id.is_none_or(|id| id == guild.id),
        content_available: !message.content.trim().is_empty(),
        target_is_bot: user.bot || message.author.bot,
        target_is_staff,
        moderator_can_delete: held.contains(Permissions::MANAGE_MESSAGES),
        moderator_can_timeout: held.contains(Permissions::MODERATE_MEMBERS),
        hierarchy_allows: moderation::hierarchy_blocker(
            ctx.author().id == guild.owner_id,
            top_role_position(moderator, guild),
            user.id == guild.owner_id,
            target_permissions.contains(Permissions::ADMINISTRATOR),
            top_role_position(&target, guild),
            true,
        )
        .is_none(),
        assessment: match assessment {
            Some(ContextAssessmentChoice::Confirmed) => Assessment::ConfirmedOffending,
            Some(ContextAssessmentChoice::Quotation) => Assessment::QuotationOrReport,
            _ => Assessment::Ambiguous,
        },
        severity,
    });
    let reply = match decision {
        Ok(proposal) => contextual::render(proposal),
        Err(reason) => format!(
            "{reason} No action taken. Operational review persistence and live pilot qualification are unavailable."
        ),
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}
