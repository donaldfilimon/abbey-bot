//! Private invoking-member engagement controls. Catalog guards acknowledge first.
mod controls;
mod follow_up;
use follow_up::follow_up;
mod introductions;
mod invitations;
use introductions::{introduce, introduction};
use invitations::invite;
#[cfg(test)]
mod tests;
use crate::engagement::*;
use crate::{Context, Error};
use controls::StopScope;
use serenity::all::{ChannelId, Permissions};

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum Destination {
    #[name = "origin"]
    Origin,
    #[name = "private"]
    Private,
}
impl From<Destination> for DestinationPreference {
    fn from(v: Destination) -> Self {
        match v {
            Destination::Origin => Self::Origin,
            Destination::Private => Self::Private,
        }
    }
}
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum Feature {
    #[name = "starters"]
    Starters,
    #[name = "questions"]
    Questions,
    #[name = "welcomes"]
    Welcomes,
    #[name = "projects"]
    Projects,
    #[name = "introductions"]
    Introductions,
}
impl From<Feature> for CommunityFeature {
    fn from(v: Feature) -> Self {
        match v {
            Feature::Starters => Self::Starters,
            Feature::Questions => Self::Questions,
            Feature::Welcomes => Self::Welcomes,
            Feature::Projects => Self::Projects,
            Feature::Introductions => Self::Introductions,
        }
    }
}
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum Feedback {
    #[name = "useful"]
    Useful,
    #[name = "dismissed"]
    Dismissed,
}
impl From<Feedback> for FeedbackKind {
    fn from(v: Feedback) -> Self {
        match v {
            Feedback::Useful => Self::Useful,
            Feedback::Dismissed => Self::Dismissed,
        }
    }
}
fn origin(ctx: Context<'_>) -> EngagementScope {
    match ctx.guild_id() {
        Some(g) => EngagementScope::Guild {
            guild: g.get(),
            channel: ctx.channel_id().get(),
        },
        None => EngagementScope::Dm {
            member: ctx.author().id.get(),
            channel: ctx.channel_id().get(),
        },
    }
}
async fn reply(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(
        poise::CreateReply::default()
            .content(crate::commands::clamp_message(text.into()))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}
async fn saved(
    ctx: Context<'_>,
    result: Result<(), crate::work::WorkError>,
    text: &str,
) -> Result<(), Error> {
    match result {Ok(())=>reply(ctx,text).await,Err(crate::work::WorkError::Persistence)=>reply(ctx,"The change could not be saved. No saved change is confirmed; check `/engage status` before trying again.").await,Err(_)=>reply(ctx,"The change was not saved. Check the values and your access to this setting or receipt.").await}
}
async fn update_policy(
    ctx: Context<'_>,
    change: impl FnOnce(&mut MemberPolicy) -> Result<(), crate::work::WorkError> + Send + 'static,
    text: &str,
) -> Result<(), Error> {
    let actor = ctx.author().id.get();
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| {
            let mut p = s.member_policies.get(&actor).cloned().unwrap_or_default();
            change(&mut p)?;
            controls::save_policy(s, actor, p)
        })
        .await;
    saved(ctx, result, text).await
}
#[poise::command(
    slash_command,
    subcommands(
        "follow_up",
        "invite",
        "introduce",
        "introduction",
        "preferences",
        "configure",
        "status",
        "snooze",
        "stop",
        "resume",
        "weekly",
        "dismiss",
        "feedback",
        "community",
        "community_status"
    )
)]
pub async fn engage(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}
#[poise::command(slash_command, ephemeral)]
pub async fn preferences(ctx: Context<'_>) -> Result<(), Error> {
    show_status(ctx).await
}
#[poise::command(slash_command, ephemeral)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    show_status(ctx).await
}
async fn show_status(ctx: Context<'_>) -> Result<(), Error> {
    let mut text = controls::render_status(
        &crate::runtime::AppState::lock(&ctx.data().state.stores)
            .work
            .engagement,
        ctx.author().id.get(),
        &origin(ctx),
        crate::runtime::now(),
    );
    let activity = match crate::runtime::activity_readiness::current_activity(crate::runtime::now()).await {
        Ok(_) => "Activity invitations: operator acceptance and current public deployment digest validated.".to_owned(),
        Err(reason) => reason.to_owned(),
    };
    let voice = if ctx.data().state.voice_invitation_available(&origin(ctx)) {
        "Voice invitations: local backend and local text route configured; audible human acceptance remains separate."
    } else {
        "Voice invitations are disabled: selected local backend and eligible local text route required."
    };
    let invitations = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        stores
            .work
            .engagement
            .candidates
            .values()
            .rev()
            .filter(|c| {
                c.member == Some(ctx.author().id.get())
                    && c.scope == origin(ctx)
                    && crate::engagement::invitations::invitation_kind(c.kind)
            })
            .take(8)
            .map(|c| format!("{} {:?}: {:?}", c.id, c.kind, c.state))
            .collect::<Vec<_>>()
            .join("; ")
    };
    text.push_str(&format!(
        "\n{activity}\n{voice}\nYour invitations in this origin: {invitations}"
    ));
    reply(ctx, text).await?;
    let tasks = follow_up::status_text(ctx).await;
    if !tasks.is_empty() {
        reply(ctx, tasks).await?;
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
#[poise::command(slash_command, ephemeral)]
pub async fn configure(
    ctx: Context<'_>,
    #[description = "Explicit daily contact limit, 1–4"] daily_limit: u8,
    #[description = "IANA timezone, for example America/New_York"] timezone: String,
    #[description = "Optional weekly limit, 1–28, at most seven daily limits"] weekly_limit: Option<
        u8,
    >,
    #[description = "Quiet hours starting local hour, 0–23"] quiet_start: Option<u8>,
    #[description = "Quiet hours ending local hour, 0–23"] quiet_end: Option<u8>,
    #[description = "Delivery preference for this origin only"] destination: Option<Destination>,
) -> Result<(), Error> {
    let actor = ctx.author().id.get();
    let origin = origin(ctx);
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| {
            controls::configure(
                s,
                actor,
                &origin,
                daily_limit,
                timezone,
                weekly_limit,
                quiet_start,
                quiet_end,
                destination.map(Into::into),
            )
        })
        .await;
    saved(ctx,result,"Saved your explicit contact settings. Stops remain in effect; `/engage status` shows blockers.").await
}
#[poise::command(slash_command, ephemeral)]
pub async fn snooze(
    ctx: Context<'_>,
    #[description = "Resume after this Unix timestamp"] until: u64,
) -> Result<(), Error> {
    let now = crate::runtime::now();
    update_policy(
        ctx,
        move |p| {
            if until <= now || until > now.saturating_add(366 * 86400) {
                return Err(crate::work::WorkError::Invalid);
            }
            p.snoozed_until = Some(until);
            Ok(())
        },
        "Saved your snooze. Existing candidates are postponed; snooze does not enable contact.",
    )
    .await
}
#[poise::command(slash_command, ephemeral)]
pub async fn stop(
    ctx: Context<'_>,
    #[description = "Stop scope; defaults to global"] scope: Option<StopScope>,
) -> Result<(), Error> {
    let origin = origin(ctx);
    let actor = ctx.author().id.get();
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| {
            controls::stop_store(s, actor, &origin, scope.unwrap_or(StopScope::Global))
        })
        .await;
    saved(ctx, result, "Saved your stop and cancelled matching unfinished candidates. Inbound messages and resume do not rearm those sources.").await
}
#[poise::command(slash_command, ephemeral)]
pub async fn resume(
    ctx: Context<'_>,
    #[description = "Resume scope; defaults to global"] scope: Option<StopScope>,
) -> Result<(), Error> {
    let origin = origin(ctx);
    update_policy(ctx,move |p|controls::set_stop(p,&origin,scope.unwrap_or(StopScope::Global),false),"Saved your explicit resume for that scope. Other stops, limits, eligibility and snooze still apply; check `/engage status`.").await
}
#[poise::command(slash_command, ephemeral)]
pub async fn weekly(
    ctx: Context<'_>,
    #[description = "Explicitly subscribe or unsubscribe"] enabled: bool,
    #[description = "Chosen weekday: Monday 0 through Sunday 6"] weekday: Option<u8>,
    #[description = "Chosen local hour, 0–23"] hour: Option<u8>,
) -> Result<(), Error> {
    let origin = origin(ctx);
    update_policy(
        ctx,
        move |p| controls::set_weekly(p, origin, enabled, weekday, hour),
        "Saved your weekly subscription choice. Subscription establishes eligibility; delivery still requires your explicit daily limit, timezone and all policy checks.",
    ).await
}
#[poise::command(slash_command, ephemeral)]
pub async fn dismiss(
    ctx: Context<'_>,
    #[description = "Your pending candidate number"] candidate: u64,
) -> Result<(), Error> {
    let actor = ctx.author().id.get();
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| controls::dismiss(s, actor, candidate))
        .await;
    saved(
        ctx,
        result,
        "Dismissed your pending candidate. It will not be rearmed by a preference update.",
    )
    .await
}
#[poise::command(slash_command, ephemeral)]
pub async fn feedback(
    ctx: Context<'_>,
    #[description = "Your sent delivery candidate number"] delivery: u64,
    #[description = "Explicit delivery feedback"] feedback: Feedback,
) -> Result<(), Error> {
    let actor = ctx.author().id.get();
    let at = crate::runtime::now();
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| {
            controls::feedback(s, actor, delivery, feedback.into(), at)?;
            Ok(s.candidates[&delivery].scope.clone())
        })
        .await;
    if let Ok(EngagementScope::Guild { guild, .. }) = &result
        && ctx.guild_id().is_some_and(|id| id.get() == *guild)
        && matches!(feedback, Feedback::Dismissed)
        && ctx
            .data()
            .state
            .addenda_status(&format!("discord:{guild}"), at)
            .learning_enabled
    {
        ctx.data().state.observe_style(
            &format!("discord:{guild}"),
            &format!("discord:{actor}"),
            "fewer follow-ups",
            at,
        );
    }
    saved(ctx, result.map(|_| ()), "Saved your explicit delivery feedback. Useful is not inferred from successful delivery; dismissed feedback may only reduce optional server follow-ups.").await
}
#[poise::command(slash_command, subcommands("community_feature"))]
pub async fn community(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}
async fn manager_guild(ctx: Context<'_>) -> Result<u64, Error> {
    let guild = ctx.guild_id().ok_or(crate::work::WorkError::Invalid)?;
    let perms = crate::commands_help::current_permissions(
        ctx.serenity_context(),
        guild,
        ctx.channel_id(),
        ctx.author().id,
    )
    .await?;
    if !perms.contains(Permissions::MANAGE_GUILD) {
        return Err(crate::work::WorkError::Invalid.into());
    }
    Ok(guild.get())
}
#[poise::command(slash_command, ephemeral, rename = "feature")]
pub async fn community_feature(
    ctx: Context<'_>,
    #[description = "Community behavior"] feature: Feature,
    #[description = "Enable or disable this feature"] enabled: bool,
    #[description = "Explicit channel in this server when enabling"] channel: Option<ChannelId>,
) -> Result<(), Error> {
    let guild = manager_guild(ctx).await?;
    let channel = if enabled {
        let id = channel.ok_or(crate::work::WorkError::Invalid)?;
        let fetched = id.to_channel(ctx.http()).await?;
        let actual = fetched.guild().ok_or(crate::work::WorkError::Invalid)?;
        if actual.guild_id.get() != guild {
            return Err(crate::work::WorkError::Invalid.into());
        }
        let permissions = crate::commands_help::current_permissions(
            ctx.serenity_context(),
            actual.guild_id,
            id,
            ctx.author().id,
        )
        .await?;
        if !permissions.contains(Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD) {
            return Err(crate::work::WorkError::Invalid.into());
        }
        Some(id.get())
    } else {
        None
    };
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| {
            controls::set_community(s, guild, true, feature.into(), enabled, channel)
        })
        .await;
    saved(ctx,result,"Saved the community feature switch. Existing guild act and access policy still applies; learning is independent and this does not enable `/admin act`.").await
}
#[poise::command(slash_command, ephemeral, rename = "community-status")]
pub async fn community_status(ctx: Context<'_>) -> Result<(), Error> {
    let guild = manager_guild(ctx).await?;
    let text = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        let s = &stores.work.engagement;
        let p = s.guild_features.get(&guild).cloned().unwrap_or_default();
        let counts = controls::receipt_counts(
            s,
            |c| matches!(c.scope, EngagementScope::Guild {guild:g,..} if g == guild),
        );
        format!(
            "Community settings: enabled {:?}.\nGuild receipt totals: {counts}.\nAll features default off. Guild act and access checks still apply. Learning is independent. Welcomes: unavailable (the current gateway does not request member-join events). No privileged intents are enabled here. Projects require a closed channel audience matching current Work membership; unverifiable audiences are refused.",
            p.enabled
        )
    };
    let timing = ctx.data().state.engagement_timing_status(guild);
    reply(ctx, format!("{text}\n{timing}")).await
}

#[cfg(test)]
pub(crate) fn stop_for_test(
    s: &mut EngagementStore,
    actor: u64,
    scope: &EngagementScope,
) -> Result<(), crate::work::WorkError> {
    controls::stop_store(s, actor, scope, StopScope::Global)
}
