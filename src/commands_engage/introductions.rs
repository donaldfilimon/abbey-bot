//! Invoker-bound private approval controls. No invitation DM or memory retrieval.
use super::*;
use crate::engagement::lifecycle::{EngagementReservation, IntroductionReservation};
use crate::runtime::engagement_delivery::EngagementTransport;
use serenity::all::{
    ButtonStyle, ComponentInteractionDataKind, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse,
    InteractionContext, User,
};
use std::time::Duration;

#[derive(Clone)]
struct Session {
    command: u64,
    owner: u64,
    guild: u64,
    channel: u64,
    introduction: u64,
    revision: u64,
}
struct Envelope {
    owner: u64,
    bot: bool,
    guild: Option<u64>,
    channel: u64,
    guild_context: bool,
    message_is_ours: bool,
    button: bool,
}
impl Session {
    fn custom_id(&self, action: &str) -> String {
        format!(
            "i:{}:{}:{}:{}:{action}",
            self.command, self.owner, self.introduction, self.revision
        )
    }
    fn authorize(&self, id: &str, envelope: &Envelope) -> Option<bool> {
        if envelope.bot
            || envelope.guild != Some(self.guild)
            || envelope.channel != self.channel
            || !envelope.guild_context
            || !envelope.message_is_ours
            || !envelope.button
        {
            return None;
        }
        self.action(id, envelope.owner)
    }
    fn action(&self, id: &str, member: u64) -> Option<bool> {
        if member != self.owner {
            return None;
        }
        if id == self.custom_id("a") {
            Some(true)
        } else if id == self.custom_id("w") {
            Some(false)
        } else {
            None
        }
    }
}
async fn access(ctx: Context<'_>, i: &Introduction) -> Result<(), Error> {
    let r = EngagementReservation {
        candidate_id: 0,
        revision: i.revision,
        policy_revision: 0,
        scope: i.scope.clone(),
        member: None,
        destination: DestinationPreference::Origin,
        introduction: Some(IntroductionReservation {
            introduction: i.clone(),
            policy_revisions: [0; 2],
        }),
    };
    let adapter = crate::gateway::engagement_delivery::DiscordEngagementDelivery(
        ctx.serenity_context().http.clone(),
    );
    tokio::time::timeout(Duration::from_secs(30), adapter.authorize(&r))
        .await
        .map_err(|_| crate::work::WorkError::Denied)??;
    Ok(())
}
async fn current_invoker(ctx: Context<'_>, guild: u64) -> Result<(), Error> {
    let permissions = crate::commands_help::current_permissions(
        ctx.serenity_context(),
        serenity::all::GuildId::new(guild),
        ctx.channel_id(),
        ctx.author().id,
    )
    .await?;
    if !permissions.contains(Permissions::VIEW_CHANNEL) {
        return Err(crate::work::WorkError::Denied.into());
    }
    Ok(())
}
#[poise::command(slash_command, ephemeral)]
pub async fn introduce(
    ctx: Context<'_>,
    #[description = "Member in this server; no automatic DM is sent"] member: User,
    #[description = "Common public destination in this server"] destination: ChannelId,
    #[description = "Your own exact public description; maximum 300 Unicode characters"]
    self_description: String,
) -> Result<(), Error> {
    let guild = ctx.guild_id().ok_or(crate::work::WorkError::Denied)?.get();
    let owner = ctx.author().id.get();
    if member.bot
        || member.id.get() == owner
        || !crate::engagement::introductions::description_valid(&self_description)
    {
        return reply(ctx,"Choose another human member and supply your own description of at most 300 characters.").await;
    }
    current_invoker(ctx, guild).await?;
    let scope = EngagementScope::Guild {
        guild,
        channel: destination.get(),
    };
    let policy_revision = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        stores
            .work
            .engagement
            .guild_features
            .get(&guild)
            .map(|p| p.revision)
            .ok_or(crate::work::WorkError::Denied)?
    };
    let i = Introduction {
        id: 0,
        revision: 1,
        scope: scope.clone(),
        members: [owner, member.id.get()],
        approved_self_descriptions: [Some(self_description.clone()), None],
        approvals: [None; 2],
        destination: destination.get(),
        state: IntroductionState::Pending,
    };
    access(ctx, &i).await?;
    if !ctx
        .data()
        .state
        .engagement_guild_gate(&scope, crate::runtime::now(), false)
    {
        return reply(
            ctx,
            "The common destination is blocked by the server’s current engagement policy.",
        )
        .await;
    }
    let at = crate::runtime::now();
    let state = ctx.data().state.clone();
    let check = state.clone();
    let result = state
        .commit_engagement(move |s| {
            if s.guild_features
                .get(&guild)
                .is_none_or(|p| p.revision != policy_revision)
                || !check.engagement_guild_gate(&scope, at, false)
            {
                return Err(crate::work::WorkError::Stale);
            }
            s.create_introduction(i.members, scope, self_description, at)
        })
        .await;
    match result {
        Ok(id) => review(ctx, id).await,
        Err(e) => saved(ctx, Err(e), "").await,
    }
}
#[poise::command(slash_command, ephemeral)]
pub async fn introduction(
    ctx: Context<'_>,
    #[description = "Your introduction number; omit to list your proposals"] proposal: Option<u64>,
    #[description = "Supply or edit only your own exact description, maximum 300 characters"]
    self_description: Option<String>,
    #[description = "Change the common destination in this server; clears both approvals"]
    destination: Option<ChannelId>,
) -> Result<(), Error> {
    let guild = ctx.guild_id().ok_or(crate::work::WorkError::Denied)?.get();
    current_invoker(ctx, guild).await?;
    let owner = ctx.author().id.get();
    let Some(id) = proposal else {
        let text = {
            let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
            own_list(&stores.work.engagement, owner, guild)
        };
        return reply(ctx, text).await;
    };
    let i = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        own_record(&stores.work.engagement, id, owner, guild)?.clone()
    };
    if self_description.is_some() || destination.is_some() {
        let index = i
            .members
            .iter()
            .position(|m| *m == owner)
            .ok_or(crate::work::WorkError::Denied)?;
        let description = self_description
            .or_else(|| i.approved_self_descriptions[index].clone())
            .ok_or(crate::work::WorkError::Invalid)?;
        let mut proposed = i.clone();
        if let Some(d) = destination {
            proposed.destination = d.get();
            proposed.scope = EngagementScope::Guild {
                guild,
                channel: d.get(),
            };
        }
        access(ctx, &proposed).await?;
        if !ctx
            .data()
            .state
            .engagement_guild_gate(&proposed.scope, crate::runtime::now(), false)
        {
            return reply(
                ctx,
                "The destination is blocked by the server’s current engagement policy.",
            )
            .await;
        }
        let result = ctx
            .data()
            .state
            .commit_engagement(move |s| {
                s.edit_introduction(
                    id,
                    owner,
                    i.revision,
                    description,
                    destination.map(|d| d.get()),
                )
            })
            .await;
        if let Err(e) = result {
            return saved(ctx, Err(e), "").await;
        }
    }
    review(ctx, id).await
}
fn own_record(
    s: &EngagementStore,
    id: u64,
    owner: u64,
    guild: u64,
) -> Result<&Introduction, crate::work::WorkError> {
    s.introductions
        .get(&id)
        .filter(|i| {
            i.members.contains(&owner)
                && matches!(i.scope,EngagementScope::Guild{guild:g,..}if g==guild)
        })
        .ok_or(crate::work::WorkError::Denied)
}
fn own_list(s: &EngagementStore, owner: u64, guild: u64) -> String {
    let entries = s
        .introductions
        .values()
        .rev()
        .filter(|i| {
            i.members.contains(&owner)
                && matches!(i.scope,EngagementScope::Guild{guild:g,..}if g==guild)
        })
        .take(10)
        .map(|i| {
            let outcome = s
                .candidates
                .values()
                .find(|c| c.introduction_id == Some(i.id))
                .map_or_else(|| "unknown".to_owned(), |c| format!("{:?}", c.state));
            format!(
                "{}: {:?}, revision {}; delivery {outcome}",
                i.id, i.state, i.revision
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Your introduction proposals in this server: {entries}\nUse `/engage introduction proposal:<number> self_description:<your own description>` to review privately. Both members must approve the same revision. No automatic DM is sent to request approval."
    )
}
fn render_review(
    s: &EngagementStore,
    id: u64,
    owner: u64,
    guild: u64,
) -> Result<String, crate::work::WorkError> {
    let i = own_record(s, id, owner, guild)?;
    let mut text = crate::engagement::introductions::private_preview(i, owner)?;
    let candidate = s
        .candidates
        .values()
        .find(|c| c.introduction_id == Some(id))
        .ok_or(crate::work::WorkError::Missing)?;
    text.push_str(&format!("\nDelivery: {:?}.", candidate.state));
    match candidate.state {
        CandidateState::Sent => {
            if let Some(message) = candidate.message_id {
                text.push_str(&format!(
                    " Saved Discord receipt: https://discord.com/channels/{guild}/{}/{message}.",
                    i.destination
                ));
            }
        }
        CandidateState::ReviewRequired => text.push_str(
            " The send outcome is uncertain; this proposal will not be retried automatically.",
        ),
        CandidateState::Reserved => text.push_str(
            " Both members’ attempted capacity is saved; publication has not been confirmed.",
        ),
        CandidateState::Rejected | CandidateState::Cancelled => {
            text.push_str(" This proposal will not be sent again.")
        }
        CandidateState::Pending => text.push_str(
            " Both current approvals and policy/access checks are required before publication.",
        ),
    }
    Ok(text)
}
async fn review(ctx: Context<'_>, id: u64) -> Result<(), Error> {
    let guild = ctx.guild_id().ok_or(crate::work::WorkError::Denied)?.get();
    let owner = ctx.author().id.get();
    let (i, text) = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        (
            own_record(&stores.work.engagement, id, owner, guild)?.clone(),
            render_review(&stores.work.engagement, id, owner, guild)?,
        )
    };
    if !matches!(
        i.state,
        IntroductionState::Pending | IntroductionState::Ready | IntroductionState::Consumed
    ) {
        return reply(ctx, text).await;
    }
    let session = Session {
        command: ctx.id(),
        owner,
        guild,
        channel: ctx.channel_id().get(),
        introduction: id,
        revision: i.revision,
    };
    let components = vec![CreateActionRow::Buttons(vec![
        CreateButton::new(session.custom_id("a"))
            .label("Approve my exact description here")
            .style(ButtonStyle::Primary),
        CreateButton::new(session.custom_id("w"))
            .label("Withdraw introduction")
            .style(ButtonStyle::Danger),
    ])];
    ctx.send(
        poise::CreateReply::default()
            .content(text)
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions())
            .components(components),
    )
    .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let prefix = format!("i:{}:", session.command);
        let collector =
            serenity::collector::ComponentInteractionCollector::new(ctx.serenity_context())
                .filter(move |p| p.data.custom_id.starts_with(&prefix));
        let Ok(Some(press)) = tokio::time::timeout_at(deadline, collector).await else {
            break;
        };
        press
            .create_response(
                ctx.http(),
                CreateInteractionResponse::Defer(
                    CreateInteractionResponseMessage::new().ephemeral(true),
                ),
            )
            .await?;
        let action = session.authorize(
            &press.data.custom_id,
            &Envelope {
                owner: press.user.id.get(),
                bot: press.user.bot,
                guild: press.guild_id.map(|g| g.get()),
                channel: press.channel_id.get(),
                guild_context: press.context == Some(InteractionContext::Guild),
                message_is_ours: press.message.author.id
                    == ctx.serenity_context().cache.current_user().id,
                button: matches!(press.data.kind, ComponentInteractionDataKind::Button),
            },
        );
        let result = async {
            let approve = action.ok_or(crate::work::WorkError::Denied)?;
            current_invoker(ctx, guild).await?;
            let current = {
                let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
                own_record(&stores.work.engagement, id, owner, guild)?.clone()
            };
            if current.revision != session.revision {
                return Err(crate::work::WorkError::Stale.into());
            }
            // Withdrawal remains possible after destination access was lost.
            if approve {
                access(ctx, &current).await?;
                if !ctx.data().state.engagement_guild_gate(&current.scope, crate::runtime::now(), false) {
                    return Err(crate::work::WorkError::Denied.into());
                }
            }
            let revision = session.revision;
            let state = ctx.data().state.clone();
            let check = state.clone();
            state.commit_engagement(move |s| {
                if approve {
                    if !check.engagement_guild_gate(&current.scope, crate::runtime::now(), false) {
                        return Err(crate::work::WorkError::Denied);
                    }
                    s.approve_introduction(id, owner, revision).map(|ready| {
                        if ready {
                            "Both members approved this revision. Publication still requires current access and both contact budgets."
                        } else {
                            "Saved your exact approval. The other member must separately approve this same revision."
                        }
                    })
                } else {
                    s.withdraw_introduction(id, owner, revision).map(|()| {
                        "Withdrew the introduction and cancelled its unfinished delivery."
                    })
                }
            }).await.map_err(Error::from)
        }.await;
        let text = match &result {
            Ok(text) => *text,
            Err(_) => {
                "This control was refused. It may be stale, belong to another member, or be blocked by current policy/access. Open `/engage introduction` to review your current proposal."
            }
        };
        press
            .edit_response(
                ctx.http(),
                EditInteractionResponse::new()
                    .content(text)
                    .allowed_mentions(crate::gateway::no_mentions()),
            )
            .await?;
        if result.is_ok() {
            break;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
