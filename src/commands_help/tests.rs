use super::*;
use crate::command_catalog::{self, CommandKind};

#[test]
fn recursive_poise_catalog_and_runtime_binding_parity() {
    fn flatten<'a>(
        commands: &'a [poise::Command<Data, Error>],
        leaves: &mut Vec<&'a poise::Command<Data, Error>>,
    ) {
        for command in commands {
            assert!(command.required_permissions.is_empty());
            assert!(command.required_bot_permissions.is_empty());
            assert!(!command.owners_only && !command.nsfw_only && !command.dm_only);
            if command.subcommands.is_empty() {
                leaves.push(command);
            } else {
                let permission = command
                    .subcommands
                    .iter()
                    .map(|child| child.default_member_permissions)
                    .reduce(|a, b| a & b)
                    .unwrap();
                assert_eq!(command.default_member_permissions, permission);
                assert_eq!(
                    command.guild_only,
                    command.subcommands.iter().all(|child| child.guild_only)
                );
                assert!(command.checks.is_empty());
                flatten(&command.subcommands, leaves);
            }
        }
    }
    let commands = crate::application_commands();
    let mut leaves = Vec::new();
    flatten(&commands, &mut leaves);
    assert_eq!(leaves.len(), command_catalog::registered_commands().len());
    let mut seen = std::collections::HashSet::new();
    for leaf in leaves {
        let binding = leaf.custom_data.downcast_ref::<CatalogBinding>().unwrap();
        assert!(seen.insert(binding.key));
        let spec = catalog::command(binding.key);
        assert_eq!(spec.status, catalog::ImplementationStatus::Registered);
        assert_eq!(leaf.qualified_name, spec.name);
        assert_eq!(leaf.ephemeral, spec.private);
        assert_eq!(binding.eligibility, spec.eligibility);
        assert_eq!(
            leaf.default_member_permissions,
            spec.registration
                .default_member_permissions
                .map_or(Permissions::empty(), discord_permission)
        );
        let contexts: Vec<_> = spec
            .registration
            .contexts
            .iter()
            .map(|context| match context {
                InteractionContext::Guild => serenity::all::InteractionContext::Guild,
                InteractionContext::BotDm => serenity::all::InteractionContext::BotDm,
            })
            .collect();
        assert_eq!(leaf.interaction_context, Some(contexts));
        let kind = match &leaf.context_menu_action {
            Some(poise::ContextMenuCommandAction::User(_)) => CommandKind::UserContext,
            Some(poise::ContextMenuCommandAction::Message(_)) => CommandKind::MessageContext,
            None => CommandKind::Slash,
            Some(_) => panic!("unsupported command kind"),
        };
        assert_eq!(kind, spec.kind);
        assert_eq!(leaf.checks.len(), 1);
        assert!(std::ptr::fn_addr_eq(
            leaf.checks[0],
            catalog_check as for<'a> fn(Context<'a>) -> poise::BoxFuture<'a, Result<bool, Error>>
        ));
    }
    assert!(!seen.contains(&CommandKey::MemoryMenu));
}
#[test]
fn discord_payload_contexts_parent_permissions_and_limits() {
    let commands = poise::builtins::create_application_commands(&crate::application_commands());
    let payload = serde_json::to_value(commands).unwrap();
    let commands = payload.as_array().unwrap();
    for command in commands {
        assert!(command["name"].as_str().unwrap().chars().count() <= 32);
        let contexts = command["contexts"].as_array().unwrap();
        assert!(!contexts.iter().any(|value| value == 2)); // no arbitrary private/group DMs
        let name = command["name"].as_str().unwrap();
        if [
            "voice",
            "remember",
            "forget",
            "pending",
            "recall",
            "reputation",
        ]
        .contains(&name)
        {
            assert!(command.get("default_member_permissions").is_none());
        }
        if name == "admin" {
            assert_eq!(
                command["default_member_permissions"],
                Permissions::MANAGE_GUILD.bits().to_string()
            );
        }
    }
    for section in HelpSection::ALL {
        let rows = help_rows(help_center::HelpSession::new(u64::MAX, 1000, section).unwrap());
        let json = serde_json::to_value(rows).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
        let menu = &json[0]["components"][0];
        assert!(menu["custom_id"].as_str().unwrap().len() <= 100);
        assert_eq!(menu["options"].as_array().unwrap().len(), 8);
        for option in menu["options"].as_array().unwrap() {
            assert!(option["label"].as_str().unwrap().len() <= 100);
        }
    }
}
#[test]
fn real_registered_guard_special_cases_use_the_evaluator() {
    // These are the two intentionally staged guards. Their real command bodies
    // delegate after target resolution or before closing the media gate.
    for present in [false, true] {
        for permissions in [
            None,
            Some(Permissions::MANAGE_GUILD),
            Some(Permissions::ADMINISTRATOR),
            Some(Permissions::VIEW_CHANNEL),
        ] {
            let mut input = EligibilityInput::new(InteractionContext::Guild);
            input.caller_present_in_voice = Some(present);
            input.permissions = permissions_input(permissions.unwrap_or_default());
            input.capabilities = vec![Capability::VoiceConfigured];
            assert_eq!(
                leave_allowed(present, permissions),
                catalog::eligible(
                    catalog::command(CommandKey::VoiceLeave),
                    &input,
                    EvaluationMode::Invocation
                )
            );
        }
    }
    assert!(!resolved_modcall_allowed(
        Permissions::MODERATE_MEMBERS,
        None
    ));
    assert!(!resolved_modcall_allowed(
        Permissions::MODERATE_MEMBERS,
        Some(false)
    ));
    assert!(resolved_modcall_allowed(
        Permissions::MODERATE_MEMBERS,
        Some(true)
    ));
    assert!(!resolved_modcall_allowed(
        Permissions::VIEW_CHANNEL,
        Some(true)
    ));
    assert!(
        include_str!("../commands.rs")
            .contains("commands_help::resolved_modcall_allowed(held, Some(blocker.is_none()))")
    );
    assert!(include_str!("../commands_voice/discord.rs").contains("crate::commands_help::leave_allowed(present_in_configured_channel, interaction_permissions)"));
}
#[test]
fn help_is_bounded_and_does_not_expose_hidden_voice_channels_or_mentions() {
    let mut input = EligibilityInput::new(InteractionContext::Guild);
    input.permissions = vec![DiscordPermission::Administrator];
    input.self_subject = Some(true);
    input.follow_up_absent = Some(true);
    input.caller_present_in_voice = Some(true);
    input.selected_voice_mode = SelectedVoiceMode::Local;
    input.capabilities = vec![
        Capability::Generation,
        Capability::Vision,
        Capability::VoiceConfigured,
        Capability::VoiceLocal,
        Capability::VoiceOpenAi,
    ];
    for section in HelpSection::ALL {
        let raw = catalog::render_help(section, &input);
        // Do not merely rely on clamping to accidentally drop an eligible leaf.
        assert!(
            raw.chars().count() <= 2000,
            "{}: {}",
            section.label(),
            raw.chars().count()
        );
        assert_eq!(crate::commands::clamp_message(raw.clone()), raw);
        for private in [
            "<#",
            "<@",
            "127.0.0.1",
            "http",
            "model",
            "epoch",
            "channel_id",
        ] {
            assert!(!raw.contains(private));
        }
    }
    let mentions = serde_json::to_value(crate::gateway::no_mentions()).unwrap();
    assert_eq!(mentions["parse"], serde_json::json!([]));
    assert_eq!(mentions["replied_user"], false);
    let huge = crate::commands::clamp_message("🦀".repeat(2100));
    assert!(huge.chars().count() <= 2000);
}
#[test]
fn runtime_projection_observes_configuration_guild_opt_out_and_scope() {
    let data = Data {
        state: runtime::AppState::in_memory(),
        voice: None,
    };
    let absent = runtime_input(&data, InteractionContext::Guild, Some(123));
    assert!(absent.capabilities.is_empty());
    assert_eq!(absent.selected_voice_mode, SelectedVoiceMode::Off);
}

#[test]
fn projection_requires_qualified_routable_fm_and_respects_guild_vision() {
    use crate::provider::{
        FmConfig, FmMode, FoundationModels, ProviderCapabilities, VerifiedFmCapabilities,
    };
    use std::sync::Arc;
    let mut data = Data {
        state: runtime::AppState::in_memory(),
        voice: None,
    };
    let config = |fallback| FmConfig {
        mode: FmMode::System,
        endpoint: None,
        cli: "/does-not-run/fm".into(),
        fallback,
        timeout_secs: 1,
    };
    for qualified in [false, true] {
        for fallback in [false, true] {
            let state = Arc::get_mut(&mut data.state).unwrap();
            state.foundation_models = Some(if qualified {
                FoundationModels::new_qualified(
                    config(fallback),
                    None,
                    true,
                    VerifiedFmCapabilities {
                        server: None,
                        cli: ProviderCapabilities::text_with_tools(),
                    },
                )
            } else {
                FoundationModels::new(config(fallback), None, true)
            });
            assert_eq!(
                runtime_input(&data, InteractionContext::Guild, Some(123))
                    .capabilities
                    .contains(&Capability::Generation),
                qualified && fallback
            );
        }
    }
    let state = Arc::get_mut(&mut data.state).unwrap();
    state.vision = Some(crate::vision::ConfiguredVision::Remote(
        crate::vision::RemoteVision {
            config: crate::vision::VisionConfig {
                base_url: "http://127.0.0.1:1111/v1".into(),
                model: "hidden-model-canary".into(),
                api_key: "hidden-credential-canary".into(),
            },
            transport: runtime::HttpVisionTransport::default(),
        },
    ));
    runtime::AppState::lock(&state.stores).guilds.insert(
        "discord:123".into(),
        crate::guild::GuildSettings {
            vision_enabled: false,
            ..Default::default()
        },
    );
    assert!(
        !runtime_input(&data, InteractionContext::Guild, Some(123))
            .capabilities
            .contains(&Capability::Vision)
    );
    assert!(
        runtime_input(&data, InteractionContext::Guild, Some(456))
            .capabilities
            .contains(&Capability::Vision)
    );
    assert!(
        runtime_input(&data, InteractionContext::BotDm, None)
            .capabilities
            .contains(&Capability::Vision)
    );
    assert!(
        !format!(
            "{:?}",
            runtime_input(&data, InteractionContext::Guild, Some(456))
        )
        .contains("canary")
    );
}

#[test]
fn voice_projection_requires_exact_guild_and_complete_selected_local_backend() {
    use std::sync::Arc;
    let local = crate::offline_voice::OfflineVoiceConfig::from_values(None, None, None, None, None)
        .unwrap();
    let voice = crate::voice_session::VoiceRuntime::new(crate::voice::VoiceConfig::selected_only(
        123,
        456,
        crate::voice::VoiceBackendConfig::Local(local),
        true,
    ));
    let mut data = Data {
        state: runtime::AppState::in_memory(),
        voice: Some(Arc::new(voice)),
    };
    assert!(
        runtime_input(&data, InteractionContext::Guild, Some(999))
            .capabilities
            .is_empty()
    );
    assert!(
        runtime_input(&data, InteractionContext::BotDm, None)
            .capabilities
            .is_empty()
    );
    let missing_text = runtime_input(&data, InteractionContext::Guild, Some(123));
    assert!(
        missing_text
            .capabilities
            .contains(&Capability::VoiceConfigured)
    );
    assert!(!missing_text.capabilities.contains(&Capability::VoiceLocal));
    Arc::get_mut(&mut data.state).unwrap().backend = crate::llm::Backend::from_values(
        None,
        Some("http://127.0.0.1:11434".into()),
        Some("test-model".into()),
    );
    let configured = runtime_input(&data, InteractionContext::Guild, Some(123));
    assert!(configured.capabilities.contains(&Capability::VoiceLocal));
    assert!(!configured.capabilities.contains(&Capability::VoiceOpenAi));
    assert_eq!(configured.selected_voice_mode, SelectedVoiceMode::Local);
    data.voice
        .as_ref()
        .unwrap()
        .set_effective_mode(crate::voice::VoiceMode::Disabled);
    let disabled = runtime_input(&data, InteractionContext::Guild, Some(123));
    assert_eq!(disabled.selected_voice_mode, SelectedVoiceMode::Off);
    assert!(!catalog::condition_allows(
        catalog::ConditionId::C5.rule(),
        &disabled,
        EvaluationMode::Invocation
    ));
}

#[tokio::test]
async fn registered_ordinary_guard_seam_acknowledges_before_lookup_and_evaluates_rules() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn bindings(commands: &[poise::Command<Data, Error>], result: &mut Vec<CatalogBinding>) {
        for command in commands {
            if let Some(binding) = command.custom_data.downcast_ref::<CatalogBinding>() {
                result.push(*binding);
            }
            bindings(&command.subcommands, result);
        }
    }
    let mut registered = Vec::new();
    bindings(&crate::application_commands(), &mut registered);
    for binding in registered {
        if matches!(
            binding.key,
            CommandKey::VoiceLeave | CommandKey::Modcall | CommandKey::Help
        ) {
            continue;
        }
        let spec = catalog::command(binding.key);
        if spec.eligibility.access == catalog::AccessId::A0
            && spec.eligibility.condition == catalog::ConditionId::C0
        {
            continue;
        }
        for all_available in [false, true] {
            let steps = AtomicUsize::new(0);
            let mut input = EligibilityInput::new(InteractionContext::Guild);
            if all_available {
                input.permissions = vec![DiscordPermission::Administrator];
                input.self_subject = Some(true);
                input.caller_present_in_voice = Some(true);
                input.selected_voice_mode = SelectedVoiceMode::Local;
                input.capabilities = vec![
                    Capability::Generation,
                    Capability::Vision,
                    Capability::VoiceConfigured,
                    Capability::VoiceLocal,
                    Capability::VoiceOpenAi,
                ];
                input.follow_up_absent = Some(true);
                input.action_target_resolved = true;
                input.hierarchy_allows_action = Some(true);
            }
            let expected = catalog::eligible(spec, &input, EvaluationMode::Invocation);
            let allowed = acknowledged_eligibility(
                binding,
                async {
                    assert_eq!(steps.fetch_add(1, Ordering::SeqCst), 0);
                    Ok(())
                },
                || {
                    assert_eq!(steps.fetch_add(1, Ordering::SeqCst), 1);
                    async { Ok(input) }
                },
            )
            .await
            .unwrap();
            assert_eq!(allowed, expected, "{}", spec.name);
            assert_eq!(steps.load(Ordering::SeqCst), 2);
        }
        let steps = AtomicUsize::new(0);
        let result = acknowledged_eligibility(
            binding,
            async { Err("acknowledgement failed".into()) },
            || {
                steps.fetch_add(1, Ordering::SeqCst);
                async { Ok(EligibilityInput::new(InteractionContext::Guild)) }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(steps.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn autocomplete_never_enters_the_defer_or_permission_lookup_path() {
    assert!(!requires_command_guard(
        poise::CommandInteractionType::Autocomplete
    ));
    assert!(requires_command_guard(
        poise::CommandInteractionType::Command
    ));
    let source = include_str!("../commands_brain.rs");
    for name in ["autocomplete_fact", "autocomplete_pending"] {
        let start = source.find(&format!("async fn {name}(")).unwrap();
        let body = &source[start..source[start..].find("\n}").unwrap() + start];
        assert!(body.contains("scoped_user(ctx.author())"));
        assert!(!body.contains("author_member") && !body.contains("ctx.http()"));
    }
}

#[tokio::test]
async fn component_dispatch_acknowledges_then_rejects_bad_controls_without_any_lookup() {
    use std::sync::Mutex;
    for (id, actor, now, rejection) in [
        (
            "abbey:help:v1:1:1000:administration",
            2,
            100,
            help_center::Rejection::NotOwner,
        ),
        (
            "abbey:help:v1:1:1000:administration",
            1,
            1000,
            help_center::Rejection::Expired,
        ),
        (
            "abbey:help:v2:1:1000:administration",
            1,
            100,
            help_center::Rejection::Stale,
        ),
        (
            "abbey:help:v1:1:1000:unknown",
            1,
            100,
            help_center::Rejection::Stale,
        ),
        (
            "abbey:voice:v99:unknown",
            1,
            100,
            help_center::Rejection::Stale,
        ),
    ] {
        let events = Mutex::new(Vec::new());
        let prepared = acknowledged_help(
            async {
                events.lock().unwrap().push("ack");
                Ok(())
            },
            || {
                events.lock().unwrap().push("validate");
                help_center::validate(id, actor, now)
            },
            || {
                events.lock().unwrap().push("lookup");
                async { Ok(EligibilityInput::new(InteractionContext::Guild)) }
            },
        )
        .await
        .unwrap();
        assert!(matches!(prepared, HelpPreparation::Rejected(actual) if actual == rejection));
        assert_eq!(*events.lock().unwrap(), ["ack", "validate"]);
    }
    let result = acknowledged_help(
        async { Err("no acknowledgement".into()) },
        || panic!("validation must not run"),
        || async { panic!("lookup must not run") },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn component_dispatch_reloads_revoked_permissions_and_does_not_extend_expiry() {
    use std::sync::Mutex;
    let id = "abbey:help:v1:1:1000:administration";
    for (now, manager) in [(100, true), (200, false)] {
        let events = Mutex::new(Vec::new());
        let prepared = acknowledged_help(
            async {
                events.lock().unwrap().push("ack");
                Ok(())
            },
            || {
                events.lock().unwrap().push("validate");
                help_center::validate(id, 1, now)
            },
            || {
                events.lock().unwrap().push("lookup");
                async {
                    let mut input = EligibilityInput::new(InteractionContext::Guild);
                    if manager {
                        input.permissions = vec![DiscordPermission::ManageServer];
                    }
                    Ok(input)
                }
            },
        )
        .await
        .unwrap();
        let HelpPreparation::Ready(session, input) = prepared else {
            panic!("valid control")
        };
        assert_eq!(session.expiry, 1000);
        let body = catalog::render_help(session.section, &input);
        assert_eq!(body.contains("`/admin show`"), manager);
        assert_eq!(*events.lock().unwrap(), ["ack", "validate", "lookup"]);
    }
    let failed_lookup = acknowledged_help(
        async { Ok(()) },
        || help_center::validate(id, 1, 200),
        || async { Err("private transport details".into()) },
    )
    .await
    .unwrap();
    assert!(matches!(
        failed_lookup,
        HelpPreparation::PermissionsUnavailable
    ));
}

#[tokio::test]
async fn established_self_owner_and_presence_access_needs_no_permission_lookup() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (rule, self_subject, owner, present) in [
        (catalog::AccessId::A1, true, false, false),
        (catalog::AccessId::A7, false, true, false),
        (catalog::AccessId::A6, false, false, true),
    ] {
        let lookups = AtomicUsize::new(0);
        let mut input = EligibilityInput::new(InteractionContext::Guild);
        input.self_subject = Some(self_subject);
        input.application_owner = owner;
        input.caller_present_in_voice = Some(present);
        let resolved = resolve_access_permissions(input, rule.rule(), || {
            lookups.fetch_add(1, Ordering::SeqCst);
            async { Err("permission service is unavailable".into()) }
        })
        .await
        .unwrap();
        assert_eq!(lookups.load(Ordering::SeqCst), 0);
        assert!(catalog::access_allows(rule.rule(), &resolved));
        assert!(resolved.permissions.is_empty());
    }
    for rule in [
        catalog::AccessId::A1,
        catalog::AccessId::A4,
        catalog::AccessId::A5,
    ] {
        let mut input = EligibilityInput::new(InteractionContext::Guild);
        input.self_subject = Some(false);
        // Neither owner identity nor presence substitutes for a mandatory
        // manager permission; cross-member memory also needs a current grant.
        input.application_owner = true;
        input.caller_present_in_voice = Some(true);
        let lookups = AtomicUsize::new(0);
        assert!(
            resolve_access_permissions(input.clone(), rule.rule(), || {
                lookups.fetch_add(1, Ordering::SeqCst);
                async { Err("permission service is unavailable".into()) }
            })
            .await
            .is_err()
        );
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
        let denied = resolve_access_permissions(input.clone(), rule.rule(), || async {
            Ok(Permissions::VIEW_CHANNEL)
        })
        .await
        .unwrap();
        assert!(!catalog::access_allows(rule.rule(), &denied));
        let allowed = resolve_access_permissions(input, rule.rule(), || async {
            Ok(Permissions::MANAGE_GUILD)
        })
        .await
        .unwrap();
        assert!(catalog::access_allows(rule.rule(), &allowed));
    }
}
