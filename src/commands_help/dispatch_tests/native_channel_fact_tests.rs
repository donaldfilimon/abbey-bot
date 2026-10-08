//! Remaining commands.rs channel evidence bindings through real registered leaves.
//! Apply as commands_help/dispatch_tests/native_channel_fact_tests.rs and add
//! mod native_channel_fact_tests; to dispatch_tests.rs. Loopback only, no provider.
use super::*;

async fn actual_roleplay_returned_channel(wrong_guild: bool, wrong_id: bool) -> (bool, String) {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        runtime::AppState::lock(&data.state.guilds).update(
            &format!("discord:{GUILD}"),
            &mut *stores,
            |settings| settings.nsfw_roleplay_enabled = true,
        );
    }
    let mut fetched_channel = GuildChannel::default();
    fetched_channel.id = ChannelId::new(if wrong_id { CHANNEL + 1 } else { CHANNEL });
    fetched_channel.guild_id = GuildId::new(if wrong_guild { GUILD + 1 } else { GUILD });
    fetched_channel.kind = serenity::all::ChannelType::Text;
    fetched_channel.nsfw = true;
    fixture.native_responses.lock().unwrap().insert(
        format!("/channels/{CHANNEL}"),
        serde_json::to_value(fetched_channel).unwrap(),
    );
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Roleplay);
    let invocation = Invocation::new(command, true, None);
    let options = poise::FrameworkOptions::default();
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
            .unwrap()
    );
    // With no prompt, this path records the selected persona and emits its gate
    // decision; it must never contact the configured synthetic provider.
    assert!(command.slash_action.unwrap()(context).await.is_ok());
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(
        requests
            .iter()
            .any(|r| r.method == "GET" && r.route.ends_with(&format!("/channels/{CHANNEL}")))
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.route.contains("/chat/completions"))
    );
    assert!(
        requests
            .iter()
            .filter(|r| r.method != "GET")
            .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/"))
    );
    let enabled = runtime::AppState::lock(&data.state.engine)
        .session_persona(&format!("discord:{CHANNEL}"))
        == Some(crate::persona::Persona::Aviva);
    let text = requests
        .iter()
        .filter_map(|r| r.body["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (enabled, text)
}

#[tokio::test]
async fn native_roleplay_channel_proof_preserves_exact_nsfw_control_without_provider_call() {
    let (enabled, text) = actual_roleplay_returned_channel(false, false).await;
    assert!(enabled);
    assert!(text.contains("Aviva roleplay is available here"));
}

#[tokio::test]
async fn native_roleplay_channel_proof_refuses_other_nsfw_channel_or_guild_before_persona_activation()
 {
    for (wrong_guild, wrong_id) in [(true, false), (false, true)] {
        let (enabled, text) = actual_roleplay_returned_channel(wrong_guild, wrong_id).await;
        assert!(
            !enabled,
            "another NSFW channel cannot activate Aviva in this origin; guild={wrong_guild}, id={wrong_id}"
        );
        assert!(text.contains("Abbey stays SFW here"), "{text}");
    }
}

const WEBHOOK_TARGET: u64 = CHANNEL + 10;
const HIDDEN_TARGET_NAME: &str = "synthetic-other-guild-webhook-name";
async fn actual_webhook_returned_channel(wrong_guild: bool, wrong_id: bool) -> bool {
    let fixture = DiscordFixture::new().await;
    fixture.permissions.store(
        (Permissions::VIEW_CHANNEL | Permissions::MANAGE_WEBHOOKS).bits(),
        Ordering::SeqCst,
    );
    let data = configured_data();
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Webhook);
    let mut invocation = Invocation::new(command, true, None);
    invocation.interaction.context = Some(serenity::all::InteractionContext::Guild);
    let mut selected = GuildChannel::default();
    selected.id = ChannelId::new(WEBHOOK_TARGET);
    selected.guild_id = GuildId::new(GUILD);
    selected.kind = serenity::all::ChannelType::Text;
    selected.name = "synthetic-selected-webhook-target".into();
    invocation.interaction.data.resolved.channels.insert(
        ChannelId::new(WEBHOOK_TARGET),
        serde_json::from_value(serde_json::to_value(&selected).unwrap()).unwrap(),
    );
    invocation.interaction.data.options = serde_json::from_value(json!([
        {"name":"channel", "type":7, "value":WEBHOOK_TARGET.to_string()}
    ]))
    .unwrap();
    let mut fetched = selected;
    fetched.name = HIDDEN_TARGET_NAME.into();
    if wrong_guild {
        fetched.guild_id = GuildId::new(GUILD + 1);
    }
    if wrong_id {
        fetched.id = ChannelId::new(WEBHOOK_TARGET + 1);
    }
    fixture.native_responses.lock().unwrap().insert(
        format!("/channels/{WEBHOOK_TARGET}"),
        serde_json::to_value(fetched).unwrap(),
    );
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        &fixture,
        command,
        &options,
        &data,
        poise::CommandInteractionType::Command,
        &args,
    );
    assert!(
        command.checks[0](poise::Context::Application(context))
            .await
            .unwrap(),
        "real invoking-channel webhook authority remains valid"
    );
    let before = runtime::AppState::lock(&data.state.stores).clone();
    let _ = command.slash_action.unwrap()(context).await;
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(requests.iter().any(|r| r.method == "GET" && r.route.ends_with(&format!("/channels/{WEBHOOK_TARGET}"))));
    assert!(
        requests
            .iter()
            .filter(|r| r.method != "GET")
            .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/"))
    );
    assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before));
    requests.iter().any(|r| {
        r.body["content"]
            .as_str()
            .is_some_and(|text| text.contains(HIDDEN_TARGET_NAME))
    })
}

#[tokio::test]
async fn native_webhook_channel_proof_preserves_valid_selected_target_control() {
    assert!(actual_webhook_returned_channel(false, false).await);
}

#[tokio::test]
async fn native_webhook_channel_proof_refuses_wrong_returned_guild_or_channel_before_private_guide()
{
    for (wrong_guild, wrong_id) in [(true, false), (false, true)] {
        assert!(
            !actual_webhook_returned_channel(wrong_guild, wrong_id).await,
            "wrongguild={wrong_guild} id={wrong_id} cannot supply the target label/nativeid of the private guide"
        );
    }
}
