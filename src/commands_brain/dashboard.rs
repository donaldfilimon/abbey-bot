//! Focused dashboard command adapters.
use super::*;

/// Open the owner- and guild-bound classic administration dashboard.
#[poise::command(slash_command, guild_only, ephemeral, rename = "dashboard")]
pub async fn admin_dashboard(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let session = crate::admin_dashboard::AdminSession {
        owner: ctx.author().id.get(),
        guild: guild.get(),
        expiry: runtime::now().saturating_add(crate::admin_dashboard::SESSION_SECONDS),
        page: crate::admin_dashboard::AdminPage::Overview,
    };
    let input = dashboard_input(ctx.data(), guild.get(), ctx.channel_id().get(), None);
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(crate::admin_dashboard::render(
                session.page,
                &input,
            )))
            .components(dashboard_rows(&session))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

/// Caller acknowledges privately and resolves fresh permissions before opening.
pub(crate) async fn open_dashboard_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
    permissions: Permissions,
) -> Result<(), Error> {
    if !permissions.intersects(Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR) {
        return Err("Current Manage Server permission is required.".into());
    }
    let guild = interaction.guild_id.ok_or(NO_GUILD)?;
    let session = crate::admin_dashboard::AdminSession {
        owner: interaction.user.id.get(),
        guild: guild.get(),
        expiry: runtime::now().saturating_add(crate::admin_dashboard::SESSION_SECONDS),
        page: crate::admin_dashboard::AdminPage::Overview,
    };
    let input = dashboard_input(data, guild.get(), interaction.channel_id.get(), None);
    interaction
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(clamp_message(crate::admin_dashboard::render(
                    session.page,
                    &input,
                )))
                .components(dashboard_rows(&session))
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await?;
    Ok(())
}

fn dashboard_settings(state: &AppState, guild_id: u64) -> GuildSettings {
    let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.to_string()));
    let mut stores = AppState::lock(&state.stores);
    AppState::lock(&state.guilds).refresh(&scoped, &mut *stores)
}

fn dashboard_input(
    data: &crate::Data,
    guild_id: u64,
    channel_id: u64,
    result: Option<String>,
) -> crate::admin_dashboard::AdminViewInput {
    let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.to_string()));
    let settings = dashboard_settings(&data.state, guild_id);
    let (epsilon, brain_summary) = {
        let stores = AppState::lock(&data.state.stores);
        let mut brains = AppState::lock(&data.state.brains);
        let brain = brains.brain(&scoped, &*stores, runtime::now());
        (
            brain.epsilon(),
            format!(
                "Steps: {} · replay: {} · experiences: {}",
                brain.step_count(),
                brain.buffer_len(),
                brains.experience_count(&scoped).unwrap_or(0)
            ),
        )
    };
    let description = data
        .state
        .providers
        .request_readiness(crate::provider::RequestClass::VisionDescribe)
        .is_ok();
    let ocr = data
        .state
        .providers
        .request_readiness(crate::provider::RequestClass::VisionOcr)
        .is_ok();
    let mut capabilities = vec!["memory"];
    if data
        .state
        .providers
        .request_readiness(crate::provider::RequestClass::text(
            data.state.providers.tools_enabled(),
        ))
        .is_ok()
    {
        capabilities.push("generation");
    }
    if settings.vision_enabled && description {
        capabilities.push("image description");
    }
    if settings.vision_enabled && ocr {
        capabilities.push("image OCR");
    }
    if data
        .voice
        .as_ref()
        .is_some_and(|voice| voice.config.guild_id == guild_id)
    {
        capabilities.push(
            if data
                .voice
                .as_ref()
                .is_some_and(|voice| voice.media_enabled(voice.current_epoch()))
            {
                "voice active"
            } else {
                "voice configured; not active"
            },
        );
    }
    let now = runtime::now();
    let tokens =
        AppState::lock(&data.state.budget).tokens_left(&scoped, settings.unsolicited_per_hour, now);
    let channel = guild::scoped_channel_id(PLATFORM, &channel_id.to_string());
    let cooldown_ready = AppState::lock(&data.state.cooldown).permitted(
        &channel,
        settings.reply_cooldown_seconds,
        now,
    );
    crate::admin_dashboard::AdminViewInput {
        effective_policy: format!(
            "{}\n{}\nCurrent rate limits: {} · {} (snapshot; rechecked on action).",
            crate::admin_dashboard::unsolicited_status(
                &settings,
                data.state.quiet,
                data.state
                    .providers
                    .request_readiness_for(crate::provider::RequestClass::TextReadOnly, true)
                    .is_ok()
            ),
            crate::admin_dashboard::vision_status(settings.vision_enabled, description, ocr),
            if tokens >= 1.0 {
                "hourly budget available"
            } else {
                "hourly budget exhausted"
            },
            if cooldown_ready {
                "channel cooldown clear"
            } else {
                "channel cooldown active"
            }
        ),
        settings,
        epsilon,
        brain_summary,
        capabilities,
        operation_result: result,
    }
}

/// Classic String Select that opens a dashboard page (shared by `/admin show`
/// and the dashboard itself). Option values are `View(...)` slugs; the menu
/// custom id carries [`AdminAction::SelectPage`].
pub(super) fn page_select_row(session: &crate::admin_dashboard::AdminSession) -> CreateActionRow {
    use crate::admin_dashboard::{AdminAction as A, AdminPage};
    CreateActionRow::SelectMenu(
        CreateSelectMenu::new(
            session.custom_id(A::SelectPage),
            CreateSelectMenuKind::String {
                options: AdminPage::NAV
                    .into_iter()
                    .map(|page| {
                        CreateSelectMenuOption::new(page.label(), A::View(page).slug())
                            .default_selection(page == session.page)
                    })
                    .collect(),
            },
        )
        .placeholder("Choose an administration page")
        .min_values(1)
        .max_values(1),
    )
}

pub(super) fn dashboard_rows(
    session: &crate::admin_dashboard::AdminSession,
) -> Vec<CreateActionRow> {
    use crate::admin_dashboard::{AdminAction as A, AdminPage as P};
    let action_rows = match session.page {
        P::Conversation => vec![
            vec![
                CreateButton::new(session.custom_id(A::SetVision(true)))
                    .label("Vision on")
                    .style(ButtonStyle::Success),
                CreateButton::new(session.custom_id(A::SetVision(false)))
                    .label("Vision off")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetUnsolicited(true)))
                    .label("Act on")
                    .style(ButtonStyle::Success),
                CreateButton::new(session.custom_id(A::SetUnsolicited(false)))
                    .label("Act off")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetPersona(crate::persona::Persona::Abbey)))
                    .label("Persona Abbey")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetPersona(crate::persona::Persona::Aviva)))
                    .label("Persona Aviva")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetPersona(crate::persona::Persona::Abi)))
                    .label("Persona Abi")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetCooldown(0)))
                    .label("Cooldown 0s")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetCooldown(20)))
                    .label("Cooldown 20s")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetCooldown(60)))
                    .label("Cooldown 60s")
                    .style(ButtonStyle::Secondary),
            ],
        ],
        P::Learning => vec![
            vec![
                CreateButton::new(session.custom_id(A::SetLearning(true)))
                    .label("Learning on")
                    .style(ButtonStyle::Success),
                CreateButton::new(session.custom_id(A::SetLearning(false)))
                    .label("Learning off")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetBudget(1)))
                    .label("Budget 1/h")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetBudget(6)))
                    .label("Budget 6/h")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetBudget(60)))
                    .label("Budget 60/h")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetEpsilon(5)))
                    .label("Epsilon .05")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetEpsilon(20)))
                    .label("Epsilon .20")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetEpsilon(50)))
                    .label("Epsilon .50")
                    .style(ButtonStyle::Secondary),
            ],
        ],
        P::Operations => vec![vec![
            CreateButton::new(session.custom_id(A::Flush))
                .label("Flush")
                .style(ButtonStyle::Primary),
            CreateButton::new(session.custom_id(A::Export))
                .label("Export")
                .style(ButtonStyle::Secondary),
            CreateButton::new(session.custom_id(A::RequestReset))
                .label("Reset transcript…")
                .style(ButtonStyle::Danger),
        ]],
        P::ConfirmReset => vec![vec![
            CreateButton::new(session.custom_id(A::ConfirmReset))
                .label("Confirm channel reset")
                .style(ButtonStyle::Danger),
            CreateButton::new(session.custom_id(A::View(P::Operations)))
                .label("Cancel")
                .style(ButtonStyle::Secondary),
        ]],
        P::Overview => Vec::new(),
    };
    let mut rows = vec![page_select_row(session)];
    for actions in action_rows {
        rows.push(CreateActionRow::Buttons(actions));
    }
    rows
}

/// Central admin protocol adapter. It always acknowledges before validation,
/// permission REST reads, state reloads, or mutations.
pub(super) enum AdminPreparation {
    Rejected(crate::admin_dashboard::Rejection),
    LookupUnavailable,
    PermissionDenied,
    Ready(
        crate::admin_dashboard::AdminSession,
        crate::admin_dashboard::AdminAction,
        GuildSettings,
    ),
}

pub(super) async fn acknowledged_admin_preparation<A, Validate, Load, L>(
    acknowledgement: A,
    validate: Validate,
    load: Load,
) -> Result<AdminPreparation, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    Validate: FnOnce() -> Result<
        (
            crate::admin_dashboard::AdminSession,
            crate::admin_dashboard::AdminAction,
        ),
        crate::admin_dashboard::Rejection,
    >,
    Load: FnOnce() -> L,
    L: std::future::Future<Output = Result<(Permissions, GuildSettings), Error>>,
{
    acknowledgement.await?;
    let (session, action) = match validate() {
        Ok(value) => value,
        Err(error) => return Ok(AdminPreparation::Rejected(error)),
    };
    let (permissions, settings) = match load().await {
        Ok(value) => value,
        Err(_) => return Ok(AdminPreparation::LookupUnavailable),
    };
    if !permissions.contains(Permissions::MANAGE_GUILD)
        && !permissions.contains(Permissions::ADMINISTRATOR)
    {
        return Ok(AdminPreparation::PermissionDenied);
    }
    Ok(AdminPreparation::Ready(session, action, settings))
}

pub async fn dispatch_admin_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
) -> bool {
    if !interaction.data.custom_id.starts_with("abbey:admin:") {
        return false;
    }
    let preparation = acknowledged_admin_preparation(
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
            if interaction.user.bot || interaction.message.author.id != ctx.cache.current_user().id
            {
                return Err(crate::admin_dashboard::Rejection::Malformed);
            }
            let (session, action) = crate::admin_dashboard::AdminSession::parse(
                &interaction.data.custom_id,
                interaction.user.id.get(),
                interaction.guild_id.map(|id| id.get()),
                runtime::now(),
            )?;
            let action = match &interaction.data.kind {
                // `SelectPage` is a String Select sentinel: it only becomes a concrete
                // action once `resolve_select_action` reads the chosen option value. A
                // button carries no values, so a `SelectPage` reaching this arm is
                // malformed by construction and must not pass through unresolved.
                ComponentInteractionDataKind::Button
                    if matches!(action, crate::admin_dashboard::AdminAction::SelectPage) =>
                {
                    return Err(crate::admin_dashboard::Rejection::Malformed);
                }
                ComponentInteractionDataKind::Button => action,
                ComponentInteractionDataKind::StringSelect { values } => {
                    crate::admin_dashboard::resolve_select_action(action, values)?
                }
                _ => return Err(crate::admin_dashboard::Rejection::Malformed),
            };
            Ok((session, action))
        },
        || async {
            let guild_id = interaction
                .guild_id
                .ok_or("Administration guild is unavailable.")?;
            let permissions = crate::commands_help::current_permissions(
                ctx,
                guild_id,
                interaction.channel_id,
                interaction.user.id,
            )
            .await?;
            let settings = dashboard_settings(&data.state, guild_id.get());
            Ok((permissions, settings))
        },
    )
    .await;
    let (mut session, action, current) = match preparation {
        Err(_) => {
            crate::gateway::interaction_outcomes::delivery_failed(&data.state);
            return true;
        }
        Ok(AdminPreparation::Rejected(error)) => {
            edit_admin(ctx, interaction, data, error.message(), Vec::new()).await;
            return true;
        }
        Ok(AdminPreparation::LookupUnavailable) => {
            edit_admin(
                ctx,
                interaction,
                data,
                "Discord could not confirm your current permission. Nothing changed.",
                Vec::new(),
            )
            .await;
            return true;
        }
        Ok(AdminPreparation::PermissionDenied) => {
            edit_admin(
                ctx,
                interaction,
                data,
                "Discord could not confirm that you currently have Manage Server.",
                Vec::new(),
            )
            .await;
            return true;
        }
        Ok(AdminPreparation::Ready(session, action, current)) => (session, action, current),
    };
    let guild_id = serenity::all::GuildId::new(session.guild);
    let effect = crate::admin_dashboard::reduce(action, &current);
    let mut result = None;
    use crate::admin_dashboard::AdminEffect;
    match effect {
        AdminEffect::View(page) => session.page = page,
        AdminEffect::SetLearning(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.learning_enabled = value);
            result = Some(format!(
                "Learning is now **{}**.",
                if value { "on" } else { "off" }
            ));
            let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
            if let Some(gate) = data.state.gate_for(&scoped).cloned() {
                let request = LearningToggleRequest {
                    scoped_guild: scoped,
                    scoped_user: guild::scoped_user_id(
                        PLATFORM,
                        &interaction.user.id.get().to_string(),
                    ),
                    now: runtime::now(),
                    nonce: gate.next_nonce(),
                };
                data.state.spawn_episode(async move {
                    gate.record_learning_toggle(request).await;
                });
            }
        }
        AdminEffect::SetVision(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.vision_enabled = value);
            result = Some(format!(
                "Vision is now **{}**.",
                if value { "on" } else { "off" }
            ));
        }
        AdminEffect::SetUnsolicited(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.unsolicited = value);
            result = Some(format!(
                "Unsolicited action is now **{}**.",
                if value { "on" } else { "off" }
            ));
        }
        AdminEffect::SetPersona(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.default_persona = value);
            result = Some(format!(
                "Default persona is now **{}**.",
                guild::persona_name(value)
            ));
        }
        AdminEffect::SetCooldown(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| {
                s.reply_cooldown_seconds = guild::clamp_cooldown(i64::from(value))
            });
            result = Some(format!("Reply cooldown is now **{value}s**."));
        }
        AdminEffect::SetBudget(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| {
                s.unsolicited_per_hour = guild::clamp_budget(i64::from(value))
            });
            result = Some(format!("Unsolicited budget is now **{value}/h**."));
        }
        AdminEffect::SetEpsilon(value) => {
            let epsilon = guild::clamp_epsilon(f64::from(value) / 100.0);
            update_dashboard_setting(data, guild_id.get(), |s| s.epsilon_override = Some(epsilon));
            let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
            let stores = AppState::lock(&data.state.stores);
            AppState::lock(&data.state.brains)
                .brain(&scoped, &*stores, runtime::now())
                .set_epsilon(epsilon);
            result = Some(format!("Exploration epsilon is now **{epsilon:.2}**."));
        }
        AdminEffect::Persist => {
            result = Some(match data.state.request_persistence().await {
                Ok(report) => render_persistence_result(&report),
                Err(error) => error.to_string(),
            });
        }
        AdminEffect::ResetChannel => {
            let scope =
                guild::scoped_channel_id(PLATFORM, &interaction.channel_id.get().to_string());
            result = Some(
                if AppState::lock(&data.state.engine).reset(&scope) {
                    "Conversation transcript for this channel cleared."
                } else {
                    "Conversation transcript for this channel was already clear."
                }
                .into(),
            );
        }
        AdminEffect::Export => {
            let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
            let bytes = {
                let stores = AppState::lock(&data.state.stores);
                let mut brains = AppState::lock(&data.state.brains);
                serde_json::to_vec_pretty(
                    &brains
                        .brain(&scoped, &*stores, runtime::now())
                        .export_weights(),
                )
                .unwrap_or_default()
            };
            let delivery = interaction
                .edit_response(
                    &ctx.http,
                    EditInteractionResponse::new()
                        .content("Brain snapshot attached privately.")
                        .new_attachment(CreateAttachment::bytes(
                            bytes,
                            format!("{}-brain.json", scoped.replace(':', "-")),
                        ))
                        .components(dashboard_rows(&session))
                        .allowed_mentions(crate::gateway::no_mentions()),
                )
                .await;
            if delivery.is_err() {
                crate::gateway::interaction_outcomes::delivery_failed(&data.state);
            }
            return true;
        }
        AdminEffect::None => result = Some("That setting already has the requested value.".into()),
    }
    let input = dashboard_input(data, guild_id.get(), interaction.channel_id.get(), result);
    edit_admin(
        ctx,
        interaction,
        data,
        &crate::admin_dashboard::render(session.page, &input),
        dashboard_rows(&session),
    )
    .await;
    true
}

fn update_dashboard_setting(
    data: &crate::Data,
    guild_id: u64,
    mutate: impl FnOnce(&mut GuildSettings),
) {
    let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.to_string()));
    let mut stores = AppState::lock(&data.state.stores);
    AppState::lock(&data.state.guilds).update(&scoped, &mut *stores, mutate);
}

async fn edit_admin(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
    content: &str,
    rows: Vec<CreateActionRow>,
) {
    let delivery = interaction
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(clamp_message(content.to_owned()))
                .components(rows)
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await;
    if delivery.is_err() {
        crate::gateway::interaction_outcomes::delivery_failed(&data.state);
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;

    #[test]
    fn unsolicited_dashboard_uses_streaming_read_only_route_without_tool_capability() {
        use crate::provider::{
            FmConfig, FmMode, FoundationModels, ProviderCapabilities, ProviderRuntime,
            RequestClass, VerifiedFmCapabilities,
        };
        let fm = FoundationModels::new_qualified(
            FmConfig {
                mode: FmMode::System,
                endpoint: Some("http://127.0.0.1:9".into()),
                cli: std::path::PathBuf::from("synthetic-dashboard-cli-not-executed"),
                fallback: true,
                timeout_secs: 1,
            },
            None,
            true,
            VerifiedFmCapabilities {
                server: Some(ProviderCapabilities {
                    text: true,
                    streaming: true,
                    ..ProviderCapabilities::default()
                }),
                cli: ProviderCapabilities::default(),
            },
        );
        let mut state = AppState::in_memory();
        std::sync::Arc::get_mut(&mut state).unwrap().providers =
            ProviderRuntime::legacy(None, None, Some(fm), None, true, 1, 1);
        let data = crate::Data { state, voice: None };
        assert!(data.state.providers.tools_enabled());
        assert!(
            data.state
                .providers
                .request_readiness(RequestClass::TextReadOnly)
                .is_err()
        );
        assert_eq!(
            data.state
                .providers
                .request_readiness_for(RequestClass::TextReadOnly, true),
            Ok(())
        );
        assert!(
            data.state
                .providers
                .request_readiness(RequestClass::TextWithTools)
                .is_err()
        );
        update_dashboard_setting(&data, 7, |settings| {
            settings.unsolicited = true;
            settings.learning_enabled = true;
        });
        let view = dashboard_input(&data, 7, 8, None);
        assert!(
            view.effective_policy
                .contains("eligible for policy selection"),
            "{}",
            view.effective_policy
        );
        assert!(
            !view.capabilities.contains(&"generation"),
            "interactive tool conversation must remain unavailable"
        );
    }

    #[test]
    fn failed_delivery_keeps_the_setting_change_and_the_original_failure() {
        let state = AppState::in_memory();
        let data = crate::Data { state, voice: None };
        let mut mutations = 0;
        update_dashboard_setting(&data, 7, |settings| {
            settings.unsolicited = true;
            mutations += 1;
        });
        let failure = Err::<(), _>("transport");
        if failure.is_err() {
            crate::gateway::interaction_outcomes::delivery_failed(&data.state);
        }
        assert_eq!(failure, Err("transport"));
        assert_eq!(mutations, 1);
        assert!(dashboard_settings(&data.state, 7).unsolicited);
        let view = dashboard_input(&data, 7, 8, Some("Unsolicited action is now on.".into()));
        assert!(view.effective_policy.contains("learning is off"));
        assert!(view.operation_result.unwrap().contains("now on"));
    }
}
