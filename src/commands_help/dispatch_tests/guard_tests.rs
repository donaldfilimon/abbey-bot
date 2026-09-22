//! Registered command guards: access, capability, guild-only, memory-subject and autocomplete checks.
use super::*;

fn ordinary(command: &&poise::Command<Data, Error>) -> bool {
    !matches!(
        binding(command).key,
        CommandKey::Help | CommandKey::Modcall | CommandKey::VoiceLeave
    )
}

async fn check(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    in_guild: bool,
    subject: Option<u64>,
) -> bool {
    let invocation = Invocation::new(command, in_guild, subject);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    command.checks[0](poise::Context::Application(context))
        .await
        .unwrap()
}

#[tokio::test]
async fn registered_ordinary_guards_deny_missing_access_after_acknowledgement() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(ordinary)
        .filter(|command| binding(command).eligibility.access != AccessId::A0)
    {
        assert!(
            !check(&fixture, command, &data, true, Some(OTHER)).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "GET")
                .count(),
            3,
            "{}",
            command.qualified_name
        );
        let body = requests.last().unwrap().body["content"].as_str().unwrap();
        assert!(
            [
                catalog::Blocker::Permission.message(),
                catalog::Blocker::VoicePresence.message()
            ]
            .contains(&body),
            "{body}"
        );
    }
}

#[tokio::test]
async fn registered_ordinary_guards_allow_current_access_and_capabilities() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands).into_iter().filter(ordinary) {
        assert!(
            check(&fixture, command, &data, true, Some(OTHER)).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "GET")
                .count(),
            if binding(command).eligibility.access == AccessId::A0 {
                0
            } else {
                3
            },
            "{}",
            command.qualified_name
        );
    }
}

#[tokio::test]
async fn registered_ordinary_guards_deny_missing_capabilities() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(ordinary)
        .filter(|command| binding(command).eligibility.condition != ConditionId::C0)
    {
        let mut data = configured_data();
        let state = Arc::get_mut(&mut data.state).unwrap();
        match binding(command).eligibility.condition {
            ConditionId::C1 | ConditionId::C8 | ConditionId::C5 | ConditionId::C6 => {
                state.providers.set_primary(None)
            }
            ConditionId::C2 | ConditionId::C9 | ConditionId::C3 => state.providers.clear_vision(),
            ConditionId::C4 => data.voice = None,
            other => panic!("uncovered condition: {other:?}"),
        }
        assert!(
            !check(&fixture, command, &data, true, None).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        let reason = match binding(command).eligibility.condition {
            ConditionId::C1 | ConditionId::C8 => catalog::Blocker::Generation,
            ConditionId::C2 | ConditionId::C3 => catalog::Blocker::Vision,
            ConditionId::C9 => catalog::Blocker::Ocr,
            ConditionId::C4 if binding(command).eligibility.access == AccessId::A5 => {
                catalog::Blocker::VoicePresence
            }
            ConditionId::C4 => catalog::Blocker::VoiceSetup,
            ConditionId::C5 | ConditionId::C6 => catalog::Blocker::VoiceMode,
            _ => unreachable!(),
        };
        assert_eq!(requests.last().unwrap().body["content"], reason.message());
    }
}

#[tokio::test]
async fn registered_voice_start_guards_deny_a_manager_absent_from_voice() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(false);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A5)
    {
        assert!(
            !check(&fixture, command, &data, true, None).await,
            "{}",
            command.qualified_name
        );
    }
}

#[tokio::test]
async fn registered_voice_verification_guards_allow_the_application_owner() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let commands = crate::application_commands();
    let mut options = poise::FrameworkOptions::default();
    options.owners.insert(UserId::new(ACTOR));
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A7)
    {
        let invocation = Invocation::new(command, true, None);
        let context = invocation.context(
            &fixture,
            command,
            &options,
            &data,
            poise::CommandInteractionType::Command,
        );
        assert!(
            command.checks[0](poise::Context::Application(context))
                .await
                .unwrap(),
            "{}",
            command.qualified_name
        );
    }
}

#[tokio::test]
async fn registered_guild_only_guards_reject_dm_contexts() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(ordinary)
        .filter(|command| command.guild_only)
    {
        assert!(
            !check(&fixture, command, &data, false, None).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert!(requests.iter().all(|request| request.method != "GET"));
    }
}

#[tokio::test]
async fn registered_self_memory_guards_need_no_permission_rest() {
    let fixture = DiscordFixture::new().await;
    fixture.fail_permissions.store(true, Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A1)
    {
        let contexts: &[bool] = if command.guild_only {
            &[true]
        } else {
            &[false, true]
        };
        for &in_guild in contexts {
            for subject in [None, Some(ACTOR)] {
                assert!(
                    check(&fixture, command, &data, in_guild, subject).await,
                    "{} guild={in_guild} subject={subject:?}",
                    command.qualified_name
                );
                let requests = fixture.take_requests();
                assert_deferred_first(&requests, command);
                assert_eq!(requests.len(), 1, "{}", command.qualified_name);
            }
        }
    }
}

#[tokio::test]
async fn registered_memory_guards_reject_other_subjects_in_dms_without_permission_rest() {
    let fixture = DiscordFixture::new().await;
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A1)
    {
        assert!(
            !check(&fixture, command, &data, false, Some(OTHER)).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert!(requests.iter().all(|request| request.method != "GET"));
    }
}

#[tokio::test]
async fn registered_guard_permission_failure_denies_privately_after_defer() {
    let fixture = DiscordFixture::new().await;
    fixture.fail_permissions.store(true, Ordering::SeqCst);
    let commands = crate::application_commands();
    let command = leaves(&commands)
        .into_iter()
        .find(|command| binding(command).key == CommandKey::AdminShow)
        .unwrap();
    assert!(!check(&fixture, command, &configured_data(), true, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    let denial = &requests.last().unwrap().body;
    assert_eq!(
        denial["content"],
        "Discord could not confirm the current permissions. Please try again."
    );
    assert!(denial["content"].as_str().unwrap().chars().count() <= 2000);
}

#[tokio::test]
async fn registered_guard_failed_acknowledgement_never_loads_permissions() {
    let fixture = DiscordFixture::new().await;
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    let commands = crate::application_commands();
    let command = leaves(&commands)
        .into_iter()
        .find(|command| binding(command).key == CommandKey::AdminShow)
        .unwrap();
    assert!(!check(&fixture, command, &configured_data(), true, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(requests.iter().all(|request| request.method != "GET"));
}

#[tokio::test]
async fn registered_autocomplete_guards_skip_defer_and_rest_and_keep_suggestions_self_scoped() {
    let fixture = DiscordFixture::new().await;
    let data = Data {
        state: runtime::AppState::in_memory(),
        voice: None,
    };
    {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        for (guild, actor, fact) in [
            ("discord:123", "discord:789", "my guild fact"),
            ("discord:123", "discord:790", "other member secret"),
            ("discord:124", "discord:789", "other guild secret"),
            ("discord:dm:789", "discord:789", "my dm fact"),
            ("discord:dm:790", "discord:790", "other dm secret"),
        ] {
            assert!(stores.memory.remember(guild, actor, fact, 1));
            assert!(
                stores
                    .memory
                    .propose_supersession(guild, actor, "replacement", fact, 1)
            );
        }
    }
    let commands = crate::application_commands();
    let options = poise::FrameworkOptions::default();
    let mut checked = 0;
    for command in leaves(&commands) {
        for parameter in &command.parameters {
            let Some(callback) = parameter.autocomplete_callback else {
                continue;
            };
            checked += 1;
            for in_guild in [false, true] {
                let invocation = Invocation::new(command, in_guild, Some(OTHER));
                let context = invocation.context(
                    &fixture,
                    command,
                    &options,
                    &data,
                    poise::CommandInteractionType::Autocomplete,
                );
                assert!(
                    command.checks[0](poise::Context::Application(context))
                        .await
                        .unwrap()
                );
                let response = serde_json::to_value(callback(context, "").await.unwrap()).unwrap();
                let actual: Vec<_> = response["choices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|choice| choice["value"].as_str().unwrap())
                    .collect();
                if binding(command).key == CommandKey::PersonaAsk {
                    assert_eq!(actual, crate::brain::intent::suggest_completions(""));
                } else {
                    assert_eq!(
                        actual,
                        vec![if in_guild {
                            "my guild fact"
                        } else {
                            "my dm fact"
                        }],
                        "{}",
                        command.qualified_name
                    );
                }
                assert!(!invocation.sent.load(Ordering::SeqCst));
                assert!(fixture.take_requests().is_empty());
            }
        }
    }
    assert_eq!(
        checked, 5,
        "all existing autocomplete callbacks are exercised"
    );
}

#[tokio::test]
async fn registered_help_guard_defers_privately_before_its_adapter_loads_permissions() {
    let fixture = DiscordFixture::new().await;
    let commands = crate::application_commands();
    let command = leaves(&commands)
        .into_iter()
        .find(|command| binding(command).key == CommandKey::Help)
        .unwrap();
    assert!(check(&fixture, command, &configured_data(), true, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert_eq!(requests.len(), 1);
}
