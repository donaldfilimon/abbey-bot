//! Private explicit learning-category erasure and separate confirmed guild reset.
use super::*;
const RESET_WARNING: &str = "Reset current learning for this server, including additions before confirmation? This discards all learned policy weights, replay, aggregate learning statistics, reputation, pending learning and style influence, plus scoped engagement records and continuity cards. Stored facts, transcript history, server settings, global contact safety settings, minimized member-linked contact and project-linked budget counters, erasure/replay safety markers and other scopes are preserved. This cannot be undone. Confirm within 60 seconds.";
const FAILED: &str = "Learning deletion is incomplete. Some learning or continuity deletions may already have taken effect. Current-scope protection remains active where reconciliation is incomplete. Inspect current state before retrying. No individual aggregate-weight unlearning is claimed.";

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum LearningCategory {
    #[name = "learning"]
    Learning,
}
/// Remove your linkable learning records in this server or your own DM scope.
#[poise::command(slash_command, ephemeral)]
pub async fn forget_learning(
    ctx: Context<'_>,
    #[description = "Explicit category to erase"] category: LearningCategory,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let LearningCategory::Learning = category;
    let text = match ctx
        .data()
        .state
        .erase_personal_learning(scoped_guild(ctx), ctx.author().id.get())
        .await
    {
        Ok(report) => report.render(),
        Err(_) => FAILED.into(),
    };
    send_private_no_mentions(ctx, text).await
}

pub(crate) struct Confirmation {
    pub(crate) actor: u64,
    pub(crate) guild: u64,
    pub(crate) channel: u64,
    pub(crate) token: String,
    pub(crate) created: u64,
    pub(crate) used: bool,
}
impl Confirmation {
    fn consume(
        &mut self,
        actor: u64,
        guild: Option<u64>,
        channel: u64,
        token: &str,
        manager: bool,
        now: u64,
    ) -> bool {
        if self.used
            || !manager
            || actor != self.actor
            || guild != Some(self.guild)
            || channel != self.channel
            || token != self.token
            || now < self.created
            || now - self.created > 60
        {
            return false;
        }
        self.used = true;
        true
    }
}
async fn manager(ctx: Context<'_>) -> bool {
    let Some(guild) = ctx.guild_id() else {
        return false;
    };
    crate::commands_help::current_permissions(
        ctx.serenity_context(),
        guild,
        ctx.channel_id(),
        ctx.author().id,
    )
    .await
    .is_ok_and(brain_diagnostics_authorized)
}
/// Confirm a complete server learning reset; facts and settings are preserved.
#[poise::command(slash_command, guild_only, ephemeral, rename = "reset_learning")]
pub async fn admin_reset_learning(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild) = ctx.guild_id() else {
        return send_private_no_mentions(ctx, NO_GUILD.into()).await;
    };
    if !manager(ctx).await {
        return send_private_no_mentions(
            ctx,
            "Current Manage Server permission is required.".into(),
        )
        .await;
    }
    let mut confirmation = Confirmation {
        actor: ctx.author().id.get(),
        guild: guild.get(),
        channel: ctx.channel_id().get(),
        token: format!("{}:reset_learning", ctx.id()),
        created: runtime::now(),
        used: false,
    };
    let reply = ctx
        .send(
            poise::CreateReply::default()
                .ephemeral(true)
                .content(RESET_WARNING)
                .allowed_mentions(crate::gateway::no_mentions())
                .components(vec![CreateActionRow::Buttons(vec![
                    CreateButton::new(&confirmation.token)
                        .label("Confirm learning reset")
                        .style(ButtonStyle::Danger),
                ])]),
        )
        .await?;
    let message_id = reply.message().await?.id;
    let token = confirmation.token.clone();
    let press = serenity::collector::ComponentInteractionCollector::new(ctx.serenity_context())
        .message_id(message_id)
        .filter(move |p| p.data.custom_id == token)
        .timeout(Duration::from_secs(60))
        .await;
    let Some(press) = press else {
        reply
            .edit(
                ctx,
                poise::CreateReply::default()
                    .content("Learning reset expired. Nothing was reset.")
                    .components(vec![]),
            )
            .await?;
        return Ok(());
    };
    handle_reset_press(
        ctx.serenity_context(),
        &press,
        &ctx.data().state,
        &mut confirmation,
    )
    .await
}
pub(crate) async fn handle_reset_press(
    ctx: &serenity::all::Context,
    press: &ComponentInteraction,
    state: &AppState,
    confirmation: &mut Confirmation,
) -> Result<(), Error> {
    press
        .create_response(ctx, CreateInteractionResponse::Acknowledge)
        .await?;
    let authorized = if press.user.id.get() == confirmation.actor
        && press.guild_id.map(|g| g.get()) == Some(confirmation.guild)
        && press.channel_id.get() == confirmation.channel
    {
        crate::commands_help::current_permissions(
            ctx,
            serenity::all::GuildId::new(confirmation.guild),
            press.channel_id,
            press.user.id,
        )
        .await
        .is_ok_and(brain_diagnostics_authorized)
    } else {
        false
    };
    let text = if confirmation.consume(
        press.user.id.get(),
        press.guild_id.map(|g| g.get()),
        press.channel_id.get(),
        &press.data.custom_id,
        authorized,
        runtime::now(),
    ) {
        match state
            .reset_learning_scope(
                format!("discord:{}", confirmation.guild),
                confirmation.created,
            )
            .await
        {
            Ok(report) => report.render(),
            Err(_) => FAILED.into(),
        }
    } else {
        "Learning reset refused: the confirmation or current manager authorization is no longer valid.".into()
    };
    press
        .edit_response(
            ctx,
            EditInteractionResponse::new()
                .content(clamp_message(text))
                .components(vec![])
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn erasure_confirmation_is_actor_scope_bound_fresh_and_single_use() {
        let make = || Confirmation {
            actor: 1,
            guild: 2,
            channel: 3,
            token: "confirmation".into(),
            created: 100,
            used: false,
        };
        for (actor, guild, channel, token, manager, now) in [
            (9, Some(2), 3, "confirmation", true, 100),
            (1, Some(9), 3, "confirmation", true, 100),
            (1, None, 3, "confirmation", true, 100),
            (1, Some(2), 9, "confirmation", true, 100),
            (1, Some(2), 3, "stale", true, 100),
            (1, Some(2), 3, "confirmation", false, 100),
            (1, Some(2), 3, "confirmation", true, 161),
            (1, Some(2), 3, "confirmation", true, 99),
        ] {
            assert!(!make().consume(actor, guild, channel, token, manager, now));
        }
        let mut c = make();
        assert!(c.consume(1, Some(2), 3, "confirmation", true, 160));
        assert!(!c.consume(1, Some(2), 3, "confirmation", true, 160));
    }
    #[test]
    fn erasure_prose_states_counts_limits_and_reset_warning() {
        for aggregate_reset in [false, true] {
            let report = crate::brain::erasure::LearningEraseReport {
                pending: 7,
                reactions: 8,
                social: 9,
                style: 10,
                engagement: 11,
                aggregate_reset,
                ..Default::default()
            };
            let text = report.render();
            println!("Receipt ({} chars):\n{text}", text.chars().count());
            assert!(text.chars().count() <= 2000);
            assert_eq!(clamp_message(text.clone()), text);
            assert!(text.contains("7 pending"));
            if !aggregate_reset {
                assert!(text.contains("cannot undo"));
            }
        }
        println!(
            "Warning ({} chars):\n{RESET_WARNING}\nFailure ({} chars):\n{FAILED}",
            RESET_WARNING.chars().count(),
            FAILED.chars().count()
        );
        assert!(RESET_WARNING.chars().count() < 2000);
    }
}
