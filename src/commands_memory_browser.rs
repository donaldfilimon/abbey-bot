//! Private fact-browser shell. Current permissions precede each fresh snapshot.
use crate::{Context, Data, Error, command_catalog::DiscordPermission, memory_browser as browser};
use browser::{BrowserRejection, MemoryScope, MemorySession};
use serenity::all::{
    ButtonStyle, ComponentInteraction, ComponentInteractionDataKind, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse,
    InteractionContext,
};

fn rows(session: MemorySession, page: &browser::FactPage<'_>) -> Vec<CreateActionRow> {
    if page.validity.is_err() || page.total_pages <= 1 {
        return Vec::new();
    }
    let mut buttons = Vec::new();
    if page.index > 0 {
        buttons.push(
            CreateButton::new(
                session
                    .navigate(page.index - 1)
                    .expect("bounded page")
                    .custom_id(),
            )
            .label("Previous facts")
            .style(ButtonStyle::Secondary),
        );
    }
    if page.index + 1 < page.total_pages {
        buttons.push(
            CreateButton::new(
                session
                    .navigate(page.index + 1)
                    .expect("bounded page")
                    .custom_id(),
            )
            .label("Next facts")
            .style(ButtonStyle::Secondary),
        );
    }
    vec![CreateActionRow::Buttons(buttons)]
}

pub async fn send_summary(
    ctx: Context<'_>,
    subject: u64,
    content: String,
    has_facts: bool,
) -> Result<(), Error> {
    let scope = ctx
        .guild_id()
        .map_or(MemoryScope::BotDm, |guild| MemoryScope::Guild(guild.get()));
    let controls = if has_facts {
        MemorySession::new(ctx.author().id.get(), subject, scope, crate::runtime::now())
            .map(|session| {
                vec![CreateActionRow::Buttons(vec![
                    CreateButton::new(session.custom_id())
                        .label("Browse facts")
                        .style(ButtonStyle::Secondary),
                ])]
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    ctx.send(
        poise::CreateReply::default()
            .content(crate::commands::clamp_message(content))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions())
            .components(controls),
    )
    .await?;
    Ok(())
}

enum Preparation {
    Rejected(&'static str),
    Ready(MemorySession, Vec<String>),
}

/// Construct neither permission I/O nor a snapshot before acknowledgement and
/// local envelope validation; the production dispatcher consumes this seam.
async fn prepare<A, Validate, Permissions, P, Snapshot>(
    acknowledgement: A,
    validate: Validate,
    permissions: Permissions,
    snapshot: Snapshot,
) -> Result<Preparation, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    Validate: Fn() -> Result<MemorySession, BrowserRejection>,
    Permissions: FnOnce() -> P,
    P: std::future::Future<Output = Result<Vec<DiscordPermission>, Error>>,
    Snapshot: FnOnce(&MemorySession) -> Vec<String>,
{
    acknowledgement.await?;
    let session = match validate() {
        Ok(session) => session,
        Err(error) => return Ok(Preparation::Rejected(error.message())),
    };
    let permissions = match permissions().await {
        Ok(permissions) => permissions,
        Err(_) => {
            return Ok(Preparation::Rejected(
                "Discord could not confirm the current permissions. Open `/recall` to try again.",
            ));
        }
    };
    if !crate::memory_card::subject_authorized(session.owner, session.subject, &permissions) {
        return Ok(Preparation::Rejected(
            "You can view another member's facts only while Discord grants you Manage Messages or Manage Server.",
        ));
    }
    // A REST refresh can cross the original expiry. Recheck before reading facts.
    if let Err(error) = validate() {
        return Ok(Preparation::Rejected(error.message()));
    }
    Ok(Preparation::Ready(session, snapshot(&session)))
}

pub async fn dispatch(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
) -> bool {
    let preparation = prepare(
        async {
            interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Defer(
                        CreateInteractionResponseMessage::new().ephemeral(true),
                    ),
                )
                .await
                .map_err(Error::from)
        },
        || {
            if interaction.user.bot
                || !matches!(interaction.data.kind, ComponentInteractionDataKind::Button)
                || interaction.message.author.id != ctx.cache.current_user().id
            {
                return Err(BrowserRejection::Stale);
            }
            let scope = match (interaction.context, interaction.guild_id) {
                (Some(InteractionContext::Guild), Some(guild)) => MemoryScope::Guild(guild.get()),
                (Some(InteractionContext::BotDm), None) => MemoryScope::BotDm,
                _ => return Err(BrowserRejection::WrongScope),
            };
            browser::validate(
                &interaction.data.custom_id,
                interaction.user.id.get(),
                &scope,
                crate::runtime::now(),
            )
        },
        || async {
            match interaction.guild_id {
                Some(guild) => crate::commands_help::current_permissions(
                    ctx,
                    guild,
                    interaction.channel_id,
                    interaction.user.id,
                )
                .await
                .map(crate::commands_help::permissions_input),
                None => Ok(Vec::new()),
            }
        },
        |session| {
            let guild = match session.scope {
                MemoryScope::Guild(guild) => format!("discord:{guild}"),
                MemoryScope::BotDm => format!("discord:dm:{}", session.owner),
            };
            data.state
                .memory_service()
                .subject_snapshot(&guild, &format!("discord:{}", session.subject))
                .0
        },
    )
    .await;
    let (body, controls) = match preparation {
        Err(_) => return true,
        Ok(Preparation::Rejected(message)) => (message.to_string(), Vec::new()),
        Ok(Preparation::Ready(session, facts)) => {
            let page = browser::page(&facts, session.page);
            (
                browser::render(session.subject, &page),
                rows(session, &page),
            )
        }
    };
    let _ = interaction
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(crate::commands::clamp_message(body))
                .allowed_mentions(crate::gateway::no_mentions())
                .components(controls),
        )
        .await;
    true
}

#[cfg(test)]
mod tests;
