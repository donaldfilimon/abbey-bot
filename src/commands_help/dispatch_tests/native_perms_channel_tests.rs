//! Existing registered /perms native channel-scope binding, loopback only.
//! Apply as commands_help/dispatch_tests/native_perms_channel_tests.rs and add
//! mod native_perms_channel_tests; to dispatch_tests.rs.
use super::*;

const HIDDEN_NAME: &str = "synthetic-other-guild-private-name";
async fn actual_perms_returned_channel(wrong_guild: bool, wrong_id: bool) -> bool {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Perms);
    let mut invocation = Invocation::new(command, true, Some(ACTOR));
    invocation.interaction.context = Some(serenity::all::InteractionContext::Guild);
    invocation
        .interaction
        .data
        .resolved
        .users
        .insert(UserId::new(ACTOR), user(ACTOR));
    let mut resolved_channel = GuildChannel::default();
    resolved_channel.id = ChannelId::new(CHANNEL);
    resolved_channel.guild_id = GuildId::new(GUILD);
    resolved_channel.kind = serenity::all::ChannelType::Text;
    resolved_channel.name = "synthetic-selected-channel".into();
    invocation.interaction.data.resolved.channels.insert(
        ChannelId::new(CHANNEL),
        serde_json::from_value(serde_json::to_value(&resolved_channel).unwrap()).unwrap(),
    );
    invocation.interaction.data.options = serde_json::from_value(json!([
        {"name":"channel", "type":7, "value":CHANNEL.to_string()},
        {"name":"user", "type":6, "value":ACTOR.to_string()}
    ]))
    .unwrap();
    let mut fetched_channel = resolved_channel;
    fetched_channel.name = HIDDEN_NAME.into();
    if wrong_guild {
        fetched_channel.guild_id = GuildId::new(GUILD + 1);
    }
    if wrong_id {
        fetched_channel.id = ChannelId::new(CHANNEL + 1);
    }
    fixture.native_responses.lock().unwrap().insert(
        format!("/channels/{CHANNEL}"),
        serde_json::to_value(fetched_channel).unwrap(),
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
            .unwrap()
    );
    let before = runtime::AppState::lock(&data.state.stores).clone();
    let _ = command.slash_action.unwrap()(context).await;
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(
        requests
            .iter()
            .any(|r| r.method == "GET" && r.route.ends_with(&format!("/channels/{CHANNEL}")))
    );
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
            .is_some_and(|text| text.contains(HIDDEN_NAME))
    })
}

#[tokio::test]
async fn native_perms_channel_proof_preserves_valid_requested_channel_control() {
    assert!(actual_perms_returned_channel(false, false).await);
}

#[tokio::test]
async fn native_perms_channel_proof_refuses_wrong_returned_guild_or_channel_before_public_copy() {
    for (wrong_guild, wrong_id) in [(true, false), (false, true)] {
        assert!(
            !actual_perms_returned_channel(wrong_guild, wrong_id).await,
            "wrong guild={wrong_guild} id={wrong_id} must not reveal mismatched name/overwrites in current guild"
        );
    }
}
