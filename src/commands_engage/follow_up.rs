//! Human-only task requests. No generated proposal or tool mints contact consent.
use super::*;
use crate::runtime::engagement_follow_up::{TaskFollowUpRequest, TaskFollowUpResult};
use serenity::all::{CommandType, InteractionContext};
use std::{sync::Arc, sync::atomic::Ordering};

fn native_origin(ctx: Context<'_>) -> Result<EngagementScope, crate::work::WorkError> {
    let poise::Context::Application(app) = ctx else {
        return Err(crate::work::WorkError::Denied);
    };
    let interaction = app.interaction;
    let actor = interaction.user.id.get();
    let channel = interaction.channel_id.get();
    if app.interaction_type != poise::CommandInteractionType::Command
        || interaction.data.kind != CommandType::ChatInput
        || interaction.data.name != "engage"
        || app.command.qualified_name != "engage follow_up"
        || !app.command.ephemeral
        || !app.has_sent_initial_response.load(Ordering::SeqCst)
        || actor == 0
        || channel == 0
        || ctx.author().bot
        || interaction.user.bot
        || interaction.application_id.get() != app.serenity_context.cache.current_user().id.get()
    {
        return Err(crate::work::WorkError::Denied);
    }
    match (interaction.context, interaction.guild_id) {
        (Some(InteractionContext::Guild), Some(guild)) if guild.get() != 0 => {
            Ok(EngagementScope::Guild {
                guild: guild.get(),
                channel,
            })
        }
        (Some(InteractionContext::BotDm), None) => Ok(EngagementScope::Dm {
            member: actor,
            channel,
        }),
        _ => Err(crate::work::WorkError::Denied),
    }
}

#[poise::command(slash_command, ephemeral)]
pub async fn follow_up(
    ctx: Context<'_>,
    #[description = "Existing native task number in this origin"] task: u64,
    #[description = "Exact current task revision; zero is valid"] revision: u64,
    #[description = "Exact decimal ID of your eligible completed human message in this origin"]
    source_message: String,
    #[description = "Expiry from this request: 60–604800 seconds"] expiry_seconds: u64,
) -> Result<(), Error> {
    let scope = native_origin(ctx)?;
    let source_message = (source_message.len() <= 20)
        .then_some(source_message.as_str())
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|id| *id != 0);
    let Some(source_message) = source_message else {
        return reply(
            ctx,
            "Task follow-up was not saved. Enter the exact positive decimal message ID.",
        )
        .await;
    };
    let request = TaskFollowUpRequest {
        member: ctx.author().id.get(),
        origin: scope,
        task,
        revision,
        source_message,
        expiry_seconds,
    };
    let transport = Arc::new(
        crate::gateway::engagement_delivery::DiscordEngagementDelivery(
            ctx.serenity_context().http.clone(),
        ),
    );
    let result = ctx
        .data()
        .state
        .request_task_follow_up(request, transport, crate::runtime::now)
        .await;
    match result {
        Ok(TaskFollowUpResult::Saved{candidate,due_at,expires_at})=>reply(ctx,format!(
            "Saved task follow-up candidate {candidate}: eligible from <t:{due_at}:R>, expires <t:{expires_at}:R>. Delivery will recheck your contact settings, shared limits, task and current access. `/engage status` shows the receipt; `/engage stop` cancels unfinished contact. No message was sent by this request."
        )).await,
        Ok(TaskFollowUpResult::Refused(reason))=>reply(ctx,format!("Task follow-up was not saved. {}",reason.message())).await,
        Err(crate::work::WorkError::Invalid)=>reply(ctx,"Task follow-up was not saved. Use an existing task revision and your eligible human exchange, with 60–604800 seconds until expiry. The existing next-day source time must be before expiry.").await,
        Err(error)=>saved(ctx,Err(error),"").await,
    }
}

pub(super) async fn status_text(ctx: Context<'_>) -> String {
    let transport = crate::gateway::engagement_delivery::DiscordEngagementDelivery(
        ctx.serenity_context().http.clone(),
    );
    ctx.data()
        .state
        .task_follow_up_status(
            ctx.author().id.get(),
            &origin(ctx),
            &transport,
            crate::runtime::now,
        )
        .await
}
