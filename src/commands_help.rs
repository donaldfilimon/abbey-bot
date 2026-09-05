//! Thin Discord catalog binding and centrally dispatched private help.
use crate::{Context, Data, Error, command_catalog as catalog, help_center, runtime};
use catalog::{
    Capability, CommandKey, DiscordPermission, EligibilityInput, EvaluationMode, HelpSection,
    InteractionContext, SelectedVoiceMode,
};
use serenity::all::{
    CommandDataOption, CommandDataOptionValue, ComponentInteraction, ComponentInteractionDataKind,
    CreateActionRow, CreateInteractionResponse, CreateInteractionResponseMessage, CreateSelectMenu,
    CreateSelectMenuKind, CreateSelectMenuOption, EditInteractionResponse, GuildId, Permissions,
    UserId,
};

/// Attached to every executable adapter and read by the guard itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogBinding {
    pub key: CommandKey,
    pub eligibility: catalog::EligibilityRule,
}

impl CatalogBinding {
    fn evaluate(self, input: &EligibilityInput) -> bool {
        let spec = catalog::command(self.key);
        self.eligibility == spec.eligibility
            && catalog::eligible(spec, input, EvaluationMode::Invocation)
    }
}
/// The runtime seam used by every ordinary registered guard. Construct the
/// permission/capability lookup only after acknowledgement has completed.
async fn acknowledged_eligibility<A, Load, L>(
    binding: CatalogBinding,
    acknowledgement: A,
    load: Load,
) -> Result<bool, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    Load: FnOnce() -> L,
    L: std::future::Future<Output = Result<EligibilityInput, Error>>,
{
    acknowledgement.await?;
    let input = load().await?;
    Ok(binding.evaluate(&input))
}

fn discord_permission(permission: DiscordPermission) -> Permissions {
    match permission {
        DiscordPermission::ManageMessages => Permissions::MANAGE_MESSAGES,
        DiscordPermission::ModerateMembers => Permissions::MODERATE_MEMBERS,
        DiscordPermission::ManageWebhooks => Permissions::MANAGE_WEBHOOKS,
        DiscordPermission::ManageServer => Permissions::MANAGE_GUILD,
        DiscordPermission::Administrator => Permissions::ADMINISTRATOR,
    }
}
pub fn permissions_input(permissions: Permissions) -> Vec<DiscordPermission> {
    [
        DiscordPermission::ManageMessages,
        DiscordPermission::ModerateMembers,
        DiscordPermission::ManageWebhooks,
        DiscordPermission::ManageServer,
        DiscordPermission::Administrator,
    ]
    .into_iter()
    .filter(|permission| permissions.contains(discord_permission(*permission)))
    .collect()
}
/// Derive leaf registration metadata from the same policy consumed at runtime.
/// Structural parents receive the union of child contexts and only a permission
/// required by every descendant. In particular voice/verify cannot hide owners.
pub fn bind_commands(commands: &mut [poise::Command<Data, Error>]) {
    fn bind(command: &mut poise::Command<Data, Error>, prefix: &str) {
        let name = if prefix.is_empty() {
            command
                .context_menu_name
                .as_ref()
                .unwrap_or(&command.name)
                .clone()
        } else {
            format!("{prefix} {}", command.name)
        };
        command.qualified_name.clone_from(&name);
        // Poise checks these before custom guards, potentially using REST before
        // defer. All runtime access is instead evaluated in our acknowledged guard.
        command.required_permissions = Permissions::empty();
        command.required_bot_permissions = Permissions::empty();
        if command.subcommands.is_empty() {
            let spec = catalog::registered_commands()
                .iter()
                .find(|spec| spec.name == name)
                .expect("every adapter must have a registered catalog leaf");
            command.custom_data = Box::new(CatalogBinding {
                key: spec.key,
                eligibility: spec.eligibility,
            });
            command.checks = vec![catalog_check];
            command.ephemeral = spec.private;
            command.default_member_permissions = spec
                .registration
                .default_member_permissions
                .map_or(Permissions::empty(), discord_permission);
            command.guild_only = !spec
                .registration
                .contexts
                .contains(&InteractionContext::BotDm);
            command.interaction_context = Some(
                spec.registration
                    .contexts
                    .iter()
                    .map(|context| match context {
                        InteractionContext::Guild => serenity::all::InteractionContext::Guild,
                        InteractionContext::BotDm => serenity::all::InteractionContext::BotDm,
                    })
                    .collect(),
            );
        } else {
            for child in &mut command.subcommands {
                bind(child, &name);
            }
            command.default_member_permissions = command
                .subcommands
                .iter()
                .map(|child| child.default_member_permissions)
                .reduce(|left, right| left & right)
                .unwrap_or_default();
            command.guild_only = command.subcommands.iter().all(|child| child.guild_only);
            command.interaction_context = Some(if command.guild_only {
                vec![serenity::all::InteractionContext::Guild]
            } else {
                vec![
                    serenity::all::InteractionContext::Guild,
                    serenity::all::InteractionContext::BotDm,
                ]
            });
            command.checks.clear();
        }
    }
    for command in commands {
        bind(command, "");
    }
}

pub fn runtime_input(
    data: &Data,
    context: InteractionContext,
    guild: Option<u64>,
) -> EligibilityInput {
    let mut input = EligibilityInput::new(context);
    let generation = data.state.backend.is_some()
        || data
            .state
            .foundation_models
            .as_ref()
            .filter(|fm| fm.is_qualified())
            .is_some_and(|fm| crate::generation::fm_cli_text_available(Some(fm)));
    if generation {
        input.capabilities.push(Capability::Generation);
    }
    let vision_allowed = guild.is_none_or(|guild| {
        runtime::AppState::lock(&data.state.stores)
            .guilds
            .get(&format!("discord:{guild}"))
            .is_none_or(|settings| settings.vision_enabled)
    });
    if vision_allowed && data.state.vision.is_some() {
        input.capabilities.push(Capability::Vision);
    }
    if let Some(voice) = data
        .voice
        .as_ref()
        .filter(|voice| guild == Some(voice.config.guild_id))
    {
        input.capabilities.push(Capability::VoiceConfigured);
        let local_text = data
            .state
            .backend
            .as_ref()
            .into_iter()
            .chain(data.state.fallback.as_ref())
            .any(|backend| backend.is_loopback_openai_compatible());
        if voice
            .config
            .backend_for(crate::voice::VoiceMode::Local)
            .is_some()
            && local_text
        {
            input.capabilities.push(Capability::VoiceLocal);
        }
        if voice
            .config
            .backend_for(crate::voice::VoiceMode::OpenAi)
            .is_some()
        {
            input.capabilities.push(Capability::VoiceOpenAi);
        }
        input.selected_voice_mode = match voice.effective_mode() {
            crate::voice::VoiceMode::Disabled => SelectedVoiceMode::Off,
            crate::voice::VoiceMode::Local => SelectedVoiceMode::Local,
            crate::voice::VoiceMode::OpenAi => SelectedVoiceMode::OpenAi,
        };
    }
    input
}
async fn current_permissions(
    ctx: &serenity::all::Context,
    guild: GuildId,
    channel: serenity::all::ChannelId,
    actor: UserId,
) -> Result<Permissions, Error> {
    let (member, guild, channel) = tokio::try_join!(
        guild.member(&ctx.http, actor),
        guild.to_partial_guild(&ctx.http),
        channel.to_channel(&ctx.http)
    )?;
    let channel = channel
        .guild()
        .ok_or("Permission context is unavailable.")?;
    if channel.guild_id != guild.id {
        return Err("Permission context changed.".into());
    }
    Ok(guild.user_permissions_in(&channel, &member))
}
fn presence(
    ctx: &serenity::all::Context,
    data: &Data,
    guild: Option<GuildId>,
    actor: UserId,
) -> Option<bool> {
    let voice = data
        .voice
        .as_ref()
        .filter(|voice| guild.is_some_and(|guild| guild.get() == voice.config.guild_id))?;
    let guild = ctx.cache.guild(guild?)?;
    Some(
        guild
            .voice_states
            .get(&actor)
            .and_then(|state| state.channel_id)
            .is_some_and(|id| id.get() == voice.config.channel_id),
    )
}
fn option<'a>(options: &'a [CommandDataOption], name: &str) -> Option<&'a CommandDataOptionValue> {
    for value in options {
        if value.name == name {
            return Some(&value.value);
        }
        match &value.value {
            CommandDataOptionValue::SubCommand(children)
            | CommandDataOptionValue::SubCommandGroup(children) => {
                if let Some(value) = option(children, name) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}
async fn invocation_input(
    ctx: Context<'_>,
    need_permissions: bool,
) -> Result<EligibilityInput, Error> {
    let mut input = runtime_input(
        ctx.data(),
        if ctx.guild_id().is_some() {
            InteractionContext::Guild
        } else {
            InteractionContext::BotDm
        },
        ctx.guild_id().map(|id| id.get()),
    );
    input.application_owner = ctx.framework().options().owners.contains(&ctx.author().id);
    input.caller_present_in_voice = presence(
        ctx.serenity_context(),
        ctx.data(),
        ctx.guild_id(),
        ctx.author().id,
    );
    input.self_subject = Some(true);
    if let poise::Context::Application(application) = ctx {
        let options = &application.interaction.data.options;
        if let Some(CommandDataOptionValue::User(subject)) = option(options, "user") {
            input.self_subject = Some(*subject == ctx.author().id);
        }
        input.follow_up_absent = Some(option(options, "question").is_none());
    }
    if need_permissions && let Some(guild) = ctx.guild_id() {
        input.permissions = permissions_input(
            current_permissions(
                ctx.serenity_context(),
                guild,
                ctx.channel_id(),
                ctx.author().id,
            )
            .await?,
        );
    }
    Ok(input)
}
/// Self, application-owner and caller-presence alternatives can establish
/// access without Discord permission grants. A failed REST lookup must not
/// disable those paths; permission-dependent alternatives still fail closed.
async fn resolve_access_permissions<Load, L>(
    mut input: EligibilityInput,
    rule: catalog::AccessRule,
    load: Load,
) -> Result<EligibilityInput, Error>
where
    Load: FnOnce() -> L,
    L: std::future::Future<Output = Result<Permissions, Error>>,
{
    if input.context == InteractionContext::Guild && !catalog::access_allows(rule, &input) {
        input.permissions = permissions_input(load().await?);
    }
    Ok(input)
}
async fn invocation_guard_input(
    ctx: Context<'_>,
    rule: catalog::AccessRule,
) -> Result<EligibilityInput, Error> {
    let input = invocation_input(ctx, false).await?;
    resolve_access_permissions(input, rule, || async {
        let guild = ctx.guild_id().ok_or("Permission context is unavailable.")?;
        current_permissions(
            ctx.serenity_context(),
            guild,
            ctx.channel_id(),
            ctx.author().id,
        )
        .await
    })
    .await
}
fn requires_command_guard(interaction: poise::CommandInteractionType) -> bool {
    !matches!(interaction, poise::CommandInteractionType::Autocomplete)
}
pub fn catalog_check(ctx: Context<'_>) -> poise::BoxFuture<'_, Result<bool, Error>> {
    Box::pin(async move {
        // Existing autocomplete callbacks only read the caller's own facts or
        // fixed suggestions. Discord requires an immediate autocomplete result;
        // Poise also runs these checks there, but a normal defer is invalid.
        if let poise::Context::Application(application) = ctx
            && !requires_command_guard(application.interaction_type)
        {
            return Ok(true);
        }

        let binding = ctx
            .command()
            .custom_data
            .downcast_ref::<CatalogBinding>()
            .ok_or("Missing command policy.")?;
        // Leave authorizes synchronously and closes its gate in its adapter.
        if binding.key == CommandKey::VoiceLeave {
            return Ok(true);
        }
        let spec = catalog::command(binding.key);
        let acknowledgement = async {
            if spec.private {
                ctx.defer_ephemeral().await?;
            } else {
                ctx.defer().await?;
            }
            Ok::<(), Error>(())
        };
        // A moderation target and its action must be resolved by the typed
        // adapter; it calls resolved_modcall_allowed before rendering anything.
        // Help loads permissions inside its own acknowledged adapter so a failed
        // lookup can still leave /help itself visible.
        if matches!(binding.key, CommandKey::Modcall | CommandKey::Help) {
            acknowledgement.await?;
            return Ok(true);
        }
        let allowed = match acknowledged_eligibility(*binding, acknowledgement, || {
            invocation_guard_input(ctx, spec.eligibility.access.rule())
        })
        .await
        {
            Ok(allowed) => allowed,
            Err(_) => {
                ctx.say("Discord could not confirm the current permissions. Please try again.")
                    .await?;
                return Ok(false);
            }
        };
        if !allowed {
            ctx.say("This command is unavailable with the current permissions, context, or capabilities. Open /help to see available commands.").await?;
        }
        Ok(allowed)
    })
}
pub fn modcall_access_allowed(permissions: Permissions) -> bool {
    let mut input = EligibilityInput::new(InteractionContext::Guild);
    input.permissions = permissions_input(permissions);
    catalog::access_allows(
        catalog::command(CommandKey::Modcall)
            .eligibility
            .access
            .rule(),
        &input,
    )
}
pub fn resolved_modcall_allowed(permissions: Permissions, hierarchy: Option<bool>) -> bool {
    let mut input = EligibilityInput::new(InteractionContext::Guild);
    input.permissions = permissions_input(permissions);
    input.action_target_resolved = hierarchy.is_some();
    input.hierarchy_allows_action = hierarchy;
    catalog::eligible(
        catalog::command(CommandKey::Modcall),
        &input,
        EvaluationMode::Invocation,
    )
}
pub fn leave_allowed(present: bool, permissions: Option<Permissions>) -> bool {
    let mut input = EligibilityInput::new(InteractionContext::Guild);
    input.caller_present_in_voice = Some(present);
    input.permissions = permissions_input(permissions.unwrap_or_default());
    // The synchronous adapter calls this only after resolving its exact runtime.
    input.capabilities.push(Capability::VoiceConfigured);
    catalog::eligible(
        catalog::command(CommandKey::VoiceLeave),
        &input,
        EvaluationMode::Invocation,
    )
}

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum SectionChoice {
    Start,
    Conversation,
    Memory,
    Images,
    Moderation,
    Server,
    Voice,
    Administration,
}
impl From<SectionChoice> for HelpSection {
    fn from(section: SectionChoice) -> Self {
        match section {
            SectionChoice::Start => Self::Start,
            SectionChoice::Conversation => Self::Conversation,
            SectionChoice::Memory => Self::Memory,
            SectionChoice::Images => Self::Images,
            SectionChoice::Moderation => Self::Moderation,
            SectionChoice::Server => Self::Server,
            SectionChoice::Voice => Self::Voice,
            SectionChoice::Administration => Self::Administration,
        }
    }
}
fn help_rows(session: help_center::HelpSession) -> Vec<CreateActionRow> {
    vec![CreateActionRow::SelectMenu(
        CreateSelectMenu::new(
            session.custom_id(),
            CreateSelectMenuKind::String {
                options: HelpSection::ALL
                    .into_iter()
                    .map(|section| {
                        CreateSelectMenuOption::new(section.label(), section.slug())
                            .default_selection(section == session.section)
                    })
                    .collect(),
            },
        )
        .placeholder("Choose a help section")
        .min_values(1)
        .max_values(1),
    )]
}
/// Browse commands available here using private, owner-bound controls.
#[poise::command(slash_command, ephemeral)]
pub async fn help(
    ctx: Context<'_>,
    #[description = "Which commands to browse"] section: Option<SectionChoice>,
) -> Result<(), Error> {
    let section = section.map_or(HelpSection::Start, Into::into);
    let prepared = acknowledged_help(
        async { ctx.defer_ephemeral().await.map_err(Error::from) },
        || {
            help_center::HelpSession::new(ctx.author().id.get(), runtime::now(), section)
                .ok_or(help_center::Rejection::Stale)
        },
        || async {
            match invocation_input(ctx, true).await {
                Ok(input) => Ok(input),
                Err(_) => invocation_input(ctx, false).await,
            }
        },
    )
    .await?;
    match prepared {
        HelpPreparation::Ready(session, input) => {
            ctx.send(
                poise::CreateReply::default()
                    .content(crate::commands::clamp_message(catalog::render_help(
                        section, &input,
                    )))
                    .components(help_rows(session))
                    .ephemeral(true)
                    .allowed_mentions(crate::gateway::no_mentions()),
            )
            .await?;
        }
        HelpPreparation::Rejected(rejection) => {
            ctx.say(rejection.message()).await?;
        }
        HelpPreparation::PermissionsUnavailable => {
            ctx.say("Discord could not confirm the current permissions. Open /help to try again.")
                .await?;
        }
    }
    Ok(())
}
enum HelpPreparation {
    Ready(help_center::HelpSession, EligibilityInput),
    Rejected(help_center::Rejection),
    PermissionsUnavailable,
}
/// Production command/component seam: acknowledge first, validate the local
/// envelope second, then and only then construct the live fact lookup.
async fn acknowledged_help<A, Validate, Load, L>(
    acknowledgement: A,
    validate: Validate,
    load: Load,
) -> Result<HelpPreparation, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    Validate: FnOnce() -> Result<help_center::HelpSession, help_center::Rejection>,
    Load: FnOnce() -> L,
    L: std::future::Future<Output = Result<EligibilityInput, Error>>,
{
    acknowledgement.await?;
    let session = match validate() {
        Ok(session) => session,
        Err(rejection) => return Ok(HelpPreparation::Rejected(rejection)),
    };
    Ok(match load().await {
        Ok(input) => HelpPreparation::Ready(session, input),
        Err(_) => HelpPreparation::PermissionsUnavailable,
    })
}
/// Central component dispatch. Consent retains its existing stop-first safety
/// ordering; all other abbey protocols fail closed unless explicitly understood.
pub async fn dispatch_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &Data,
    application_owner: bool,
) -> bool {
    let id = &interaction.data.custom_id;
    if !id.starts_with("abbey:") || crate::voice_consent::parse_button(id).is_some() {
        return false;
    }
    let preparation = acknowledged_help(
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
            let session = help_center::validate(id, interaction.user.id.get(), runtime::now())?;
            if interaction.user.bot || interaction.message.author.id != ctx.cache.current_user().id
            {
                return Err(help_center::Rejection::Stale);
            }
            match &interaction.data.kind {
                ComponentInteractionDataKind::StringSelect { values } if values.len() == 1 => {
                    HelpSection::parse(&values[0])
                        .map(|section| session.navigate(section))
                        .ok_or(help_center::Rejection::Stale)
                }
                _ => Err(help_center::Rejection::Stale),
            }
        },
        || async {
            let mut input = runtime_input(
                data,
                if interaction.guild_id.is_some() {
                    InteractionContext::Guild
                } else {
                    InteractionContext::BotDm
                },
                interaction.guild_id.map(|id| id.get()),
            );
            input.self_subject = Some(true);
            input.follow_up_absent = Some(true);
            input.application_owner = application_owner;
            input.caller_present_in_voice =
                presence(ctx, data, interaction.guild_id, interaction.user.id);
            if let Some(guild) = interaction.guild_id {
                input.permissions = permissions_input(
                    current_permissions(ctx, guild, interaction.channel_id, interaction.user.id)
                        .await?,
                );
            }
            Ok(input)
        },
    )
    .await;
    let (body, rows) = match preparation {
        Err(_) => return true,
        Ok(HelpPreparation::Rejected(error)) => (error.message().to_string(), Vec::new()),
        Ok(HelpPreparation::PermissionsUnavailable) => (
            "Discord could not confirm the current permissions. Open /help to try again."
                .to_string(),
            Vec::new(),
        ),
        Ok(HelpPreparation::Ready(session, input)) => (
            catalog::render_help(session.section, &input),
            help_rows(session),
        ),
    };
    let _ = interaction
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(crate::commands::clamp_message(body))
                .components(rows)
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await;
    true
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod dispatch_tests;
