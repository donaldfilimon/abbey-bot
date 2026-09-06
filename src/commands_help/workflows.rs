//! Guided task adapters: no fabricated command interactions or detached collectors.
//! The central managed framework callback owns each complete submission.
mod protocol;
mod voice;
use crate::{Data, Error, command_catalog as catalog};
use protocol::{Action, Session};
use serenity::all::{
    ActionRowComponent, ButtonStyle, ComponentInteraction, ComponentInteractionDataKind,
    CreateActionRow, CreateButton, CreateInputText, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateModal, EditInteractionResponse, InputTextStyle,
    ModalInteraction, Permissions,
};

fn command(action: Action) -> catalog::CommandKey {
    use catalog::CommandKey;
    match action {
        Action::Conversation => CommandKey::PersonaAsk,
        Action::Memory => CommandKey::Recall,
        Action::Images => CommandKey::DescribeImage,
        Action::Voice => CommandKey::VoiceStatus,
        Action::Administration => CommandKey::AdminDashboard,
    }
}

pub(super) fn rows(
    owner: u64,
    guild: Option<u64>,
    channel: u64,
    expiry: u64,
    input: &catalog::EligibilityInput,
) -> Vec<CreateActionRow> {
    let mut input = input.clone();
    input.self_subject = Some(true);
    let buttons = Action::ALL
        .into_iter()
        .filter(|action| {
            catalog::eligible(
                catalog::command(command(*action)),
                &input,
                catalog::EvaluationMode::Discoverability,
            )
        })
        .map(|action| {
            CreateButton::new(
                Session {
                    owner,
                    guild,
                    channel,
                    expiry,
                    action,
                }
                .custom_id(),
            )
            .label(action.label())
            .style(ButtonStyle::Secondary)
        })
        .collect::<Vec<_>>();
    if buttons.is_empty() {
        Vec::new()
    } else {
        vec![CreateActionRow::Buttons(buttons)]
    }
}

fn context_valid(
    context: Option<serenity::all::InteractionContext>,
    guild: Option<serenity::all::GuildId>,
) -> bool {
    matches!(
        (context, guild),
        (Some(serenity::all::InteractionContext::Guild), Some(_))
            | (Some(serenity::all::InteractionContext::BotDm), None)
    )
}

async fn input(
    ctx: &serenity::all::Context,
    data: &Data,
    guild: Option<serenity::all::GuildId>,
    channel: serenity::all::ChannelId,
    user: serenity::all::UserId,
) -> Result<(catalog::EligibilityInput, Permissions), Error> {
    let permissions = match guild {
        Some(guild) => super::current_permissions(ctx, guild, channel, user).await?,
        None => Permissions::empty(),
    };
    if guild.is_some() && !permissions.contains(Permissions::VIEW_CHANNEL) {
        return Err("current channel access denied".into());
    }
    let mut input = super::runtime_input(
        data,
        if guild.is_some() {
            catalog::InteractionContext::Guild
        } else {
            catalog::InteractionContext::BotDm
        },
        guild.map(|id| id.get()),
    );
    input.permissions = super::permissions_input(permissions);
    input.self_subject = Some(true);
    input.follow_up_absent = Some(true);
    input.caller_present_in_voice = super::presence(ctx, data, guild, user);
    Ok((input, permissions))
}

fn edit(body: String) -> EditInteractionResponse {
    EditInteractionResponse::new()
        .content(crate::commands::clamp_message(body))
        .components(Vec::new())
        .allowed_mentions(crate::gateway::no_mentions())
}

fn delivery_failed(data: &Data) {
    crate::startup::command_errors::record_failure(
        &data.state,
        crate::observability::EventCode::ResponseDelivery,
        crate::observability::OperationalErrorCategory::Unavailable,
    );
}

async fn reject_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    message: &str,
) {
    if interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(message)
                    .ephemeral(true)
                    .allowed_mentions(crate::gateway::no_mentions()),
            ),
        )
        .await
        .is_err()
    {
        delivery_failed(data);
    }
}

pub(super) async fn dispatch_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
) -> bool {
    let session = if interaction.user.bot
        || interaction.message.author.id != ctx.cache.current_user().id
        || !matches!(interaction.data.kind, ComponentInteractionDataKind::Button)
        || !context_valid(interaction.context, interaction.guild_id)
    {
        Err(protocol::STALE)
    } else {
        protocol::validate(
            &interaction.data.custom_id,
            interaction.user.id.get(),
            interaction.guild_id.map(|g| g.get()),
            interaction.channel_id.get(),
            crate::runtime::now(),
        )
    };
    let session = match session {
        Ok(s) => s,
        Err(message) => {
            reject_component(ctx, interaction, data, message).await;
            return true;
        }
    };
    // A modal is itself the initial acknowledgement. No permission or provider
    // I/O precedes it; submission performs fresh checks after its own defer.
    if session.action == Action::Conversation {
        let response = CreateInteractionResponse::Modal(
            CreateModal::new(session.custom_id(), "Ask Abbey privately").components(vec![
                CreateActionRow::InputText(
                    CreateInputText::new(
                        InputTextStyle::Paragraph,
                        "Your question (private reply)",
                        "question",
                    )
                    .required(true)
                    .min_length(1)
                    .max_length(2000),
                ),
            ]),
        );
        if interaction
            .create_response(&ctx.http, response)
            .await
            .is_err()
        {
            delivery_failed(data);
        }
        return true;
    }
    if interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true),
            ),
        )
        .await
        .is_err()
    {
        delivery_failed(data);
        return true;
    }
    let result = run_component(ctx, interaction, data, session).await;
    if result.is_err() {
        tracing::warn!(
            category = "task_operation",
            "guided task did not deliver its result"
        );
        // Retry only the explanatory response; never re-run a domain operation.
        if interaction.edit_response(&ctx.http, edit("The task could not finish delivering its result. Open `/help` and refresh the current state before retrying.".into())).await.is_err() { delivery_failed(data); }
    }
    true
}

async fn run_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    session: Session,
) -> Result<(), Error> {
    let (input, permissions) = match input(
        ctx,
        data,
        interaction.guild_id,
        interaction.channel_id,
        interaction.user.id,
    )
    .await
    {
        Ok(input) => input,
        Err(_) => {
            interaction.edit_response(&ctx.http, edit("Discord could not confirm your current permissions. Open `/help` to try again.".into())).await?;
            return Ok(());
        }
    };
    if let Err(message) = protocol::validate(
        &interaction.data.custom_id,
        session.owner,
        session.guild,
        session.channel,
        crate::runtime::now(),
    ) {
        interaction
            .edit_response(&ctx.http, edit(message.into()))
            .await?;
        return Ok(());
    }
    // Guidance/read-only launchers remain discoverable while a provider is down.
    if !catalog::eligible(
        catalog::command(command(session.action)),
        &input,
        catalog::EvaluationMode::Discoverability,
    ) {
        interaction.edit_response(&ctx.http, edit("That task is not available with your current Discord access here. Open `/help` for permitted tasks.".into())).await?;
        return Ok(());
    }
    match session.action {
        Action::Conversation => unreachable!("modal handles questions"),
        Action::Memory => {
            crate::commands_memory_browser::open_self_component(ctx, interaction, data).await?
        }
        Action::Administration => {
            crate::commands_brain::open_dashboard_component(ctx, interaction, data, permissions)
                .await?
        }
        Action::Images => {
            let body = image_guidance(&input);
            interaction.edit_response(&ctx.http, edit(body)).await?;
        }
        Action::Voice => {
            let body = voice::render(ctx, interaction, data).await;
            let refresh = CreateActionRow::Buttons(vec![
                CreateButton::new(session.custom_id())
                    .label("Refresh voice & music")
                    .style(ButtonStyle::Secondary),
            ]);
            interaction
                .edit_response(&ctx.http, edit(body).components(vec![refresh]))
                .await?;
        }
    }
    Ok(())
}

fn image_guidance(input: &catalog::EligibilityInput) -> String {
    let description =
        catalog::availability(catalog::command(catalog::CommandKey::DescribeImage), input);
    let ocr = catalog::availability(catalog::command(catalog::CommandKey::Ocr), input);
    format!(
        "**Use an image**\nAttach an image to `/see image:<attachment>` for a description or `/ocr image:<attachment>` for text extraction. These slash replies are channel-visible.\nFor a private result, right-click a message with a supported image attachment → Apps → **Abbey: describe image** or **Abbey: read image text**.\nDescription: {}\nText extraction: {}\nThis view has not read or submitted an image.",
        description.message(),
        ocr.message()
    )
}

fn modal_question(components: &[serenity::all::ActionRow]) -> Option<&str> {
    let [row] = components else {
        return None;
    };
    let [ActionRowComponent::InputText(field)] = row.components.as_slice() else {
        return None;
    };
    if field.custom_id != "question" {
        return None;
    }
    protocol::question(field.value.as_deref()?)
}

pub async fn dispatch_modal(
    ctx: &serenity::all::Context,
    interaction: &ModalInteraction,
    data: &Data,
) -> bool {
    if !interaction.data.custom_id.starts_with("abbey:task:") {
        return false;
    }
    if interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true),
            ),
        )
        .await
        .is_err()
    {
        delivery_failed(data);
        return true;
    }
    let result = run_modal(ctx, interaction, data).await;
    if result.is_err() {
        delivery_failed(data);
    }
    true
}

async fn run_modal(
    ctx: &serenity::all::Context,
    interaction: &ModalInteraction,
    data: &Data,
) -> Result<(), Error> {
    let session = protocol::validate(
        &interaction.data.custom_id,
        interaction.user.id.get(),
        interaction.guild_id.map(|g| g.get()),
        interaction.channel_id.get(),
        crate::runtime::now(),
    );
    let session = match session {
        Ok(s)
            if s.action == Action::Conversation
                && !interaction.user.bot
                && interaction.application_id.get() == ctx.cache.current_user().id.get()
                && interaction
                    .message
                    .as_ref()
                    .is_some_and(|m| m.author.id == ctx.cache.current_user().id) =>
        {
            s
        }
        _ => {
            interaction
                .edit_response(&ctx.http, edit(protocol::STALE.into()))
                .await?;
            return Ok(());
        }
    };
    let Some(question) = modal_question(&interaction.data.components) else {
        interaction.edit_response(&ctx.http, edit("Enter a question of 1–2,000 characters. Open `/help` → Talk with Abbey to try again.".into())).await?;
        return Ok(());
    };
    let (input, _) = match input(
        ctx,
        data,
        interaction.guild_id,
        interaction.channel_id,
        interaction.user.id,
    )
    .await
    {
        Ok(input) => input,
        Err(_) => {
            interaction.edit_response(&ctx.http, edit("Discord could not confirm your current permissions. Open `/help` to try again.".into())).await?;
            return Ok(());
        }
    };
    if let Err(message) = protocol::validate(
        &interaction.data.custom_id,
        session.owner,
        session.guild,
        session.channel,
        crate::runtime::now(),
    ) {
        interaction
            .edit_response(&ctx.http, edit(message.into()))
            .await?;
        return Ok(());
    }
    let availability =
        catalog::availability(catalog::command(catalog::CommandKey::PersonaAsk), &input);
    if !catalog::eligible(
        catalog::command(catalog::CommandKey::PersonaAsk),
        &input,
        catalog::EvaluationMode::Invocation,
    ) {
        interaction
            .edit_response(&ctx.http, edit(availability.message().into()))
            .await?;
        return Ok(());
    }
    let answer = crate::commands::answer_question_in_scope(
        &data.state,
        session.guild,
        session.channel,
        session.owner,
        question,
        None,
        crate::commands::Commit::No,
    )
    .await;
    crate::commands::deliver_generated_reply(
        &data.state,
        interaction.edit_response(&ctx.http, edit(answer)),
    )
    .await?;
    Ok(())
}

/// The acknowledgement owns the right to construct permission and domain work.
async fn acknowledged<A, Load, Work, T>(acknowledgement: A, load: Load) -> Result<T, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    Load: FnOnce() -> Work,
    Work: std::future::Future<Output = Result<T, Error>>,
{
    drop(acknowledgement);
    load().await
}

#[cfg(test)]
mod tests;
