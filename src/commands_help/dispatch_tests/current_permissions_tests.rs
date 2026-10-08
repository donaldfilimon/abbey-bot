//! Add as commands_help/dispatch_tests/native_permission_identity_tests.rs.
//! Uses existing APIs and the existing optional exact native_responses map.
//! The owner-member impersonation case is RED against current_permissions
//! before its requested-identity and complete-role repair; no new API required.
use super::*;

fn bind_permission_routes(fixture: &DiscordFixture, member: Value, guild: Value, channel: Value) {
    fixture.native_responses.lock().unwrap().extend([
        (format!("/guilds/{GUILD}/members/{ACTOR}"), member),
        (format!("/guilds/{GUILD}"), guild),
        (format!("/channels/{CHANNEL}"), channel),
    ]);
}
fn valid_permissions() -> (Value, Value, Value) {
    let mut member = Member::default();
    member.user = user(ACTOR);
    member.guild_id = GuildId::new(GUILD);
    let mut channel = GuildChannel::default();
    channel.id = ChannelId::new(CHANNEL);
    channel.guild_id = GuildId::new(GUILD);
    (
        serde_json::to_value(member).unwrap(),
        serde_json::to_value(guild(Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD)).unwrap(),
        serde_json::to_value(channel).unwrap(),
    )
}

#[tokio::test]
async fn current_permission_proof_requires_requested_ids_and_complete_native_roles() {
    let fixture = DiscordFixture::new().await;
    let (member, native_guild, channel) = valid_permissions();
    bind_permission_routes(
        &fixture,
        member.clone(),
        native_guild.clone(),
        channel.clone(),
    );
    let valid = current_permissions(
        &fixture.context,
        GuildId::new(GUILD),
        ChannelId::new(CHANNEL),
        UserId::new(ACTOR),
    )
    .await
    .unwrap();
    assert!(valid.contains(Permissions::MANAGE_GUILD));
    fixture.take_requests();
    let mut impersonated_owner = member.clone();
    impersonated_owner["user"]["id"] = json!("999");
    let mut substituted_guild = native_guild.clone();
    substituted_guild["id"] = json!((GUILD + 1).to_string());
    let mut substituted_guild_channel = channel.clone();
    substituted_guild_channel["guild_id"] = json!((GUILD + 1).to_string());
    let mut substituted_channel = channel.clone();
    substituted_channel["id"] = json!((CHANNEL + 1).to_string());
    let mut missing_role = member.clone();
    missing_role["roles"] = json!(["987654"]);
    let mut missing_everyone = native_guild.clone();
    missing_everyone["roles"] = json!([]);
    for (label, member, guild, channel) in [
        (
            "returned member impersonates owner",
            impersonated_owner,
            native_guild.clone(),
            channel.clone(),
        ),
        (
            "both returned guild and channel switch scope",
            member.clone(),
            substituted_guild,
            substituted_guild_channel,
        ),
        (
            "returned channel substitutes ID",
            member.clone(),
            native_guild.clone(),
            substituted_channel,
        ),
        (
            "assigned member role is absent",
            missing_role,
            native_guild.clone(),
            channel.clone(),
        ),
        ("everyone role is absent", member, missing_everyone, channel),
    ] {
        bind_permission_routes(&fixture, member, guild, channel);
        let result = current_permissions(
            &fixture.context,
            GuildId::new(GUILD),
            ChannelId::new(CHANNEL),
            UserId::new(ACTOR),
        )
        .await;
        assert!(
            result.is_err(),
            "{label} must refuse a proof, even if its remaining permissions appear sufficient"
        );
        let requests = fixture.take_requests();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.method == "GET"));
    }
}

#[tokio::test]
async fn registered_catalog_guard_cannot_authorize_actor_with_returned_owner_identity() {
    let fixture = DiscordFixture::new().await;
    // Native guild has only VIEW; the requested actor is not a manager. The old
    // helper attributes the returned owner Member to ACTOR and grants all bits.
    let (mut member, _, channel) = valid_permissions();
    member["user"]["id"] = json!("999");
    bind_permission_routes(
        &fixture,
        member,
        serde_json::to_value(guild(Permissions::VIEW_CHANNEL)).unwrap(),
        channel,
    );
    let data = configured_data();
    let before = runtime::AppState::lock(&data.state.stores).clone();
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::AdminBrain);
    let invocation = Invocation::new(command, true, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        &fixture,
        command,
        &options,
        &data,
        poise::CommandInteractionType::Command,
    );
    let approved = command.checks[0](poise::Context::Application(context))
        .await
        .unwrap();
    if approved {
        // Exercise what dispatch would really do; then the negative assertion
        // reports attributable authorization failure, not an uncalled body.
        let _ = command.slash_action.unwrap()(context).await;
    }
    assert!(
        !approved,
        "a substituted native owner must not pass the real catalog guard"
    );
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert_eq!(requests.iter().filter(|r| r.method == "GET").count(), 3);
    assert_eq!(
        assert_private_no_mentions_reply(&requests),
        "Discord could not confirm the current permissions. Please try again."
    );
    assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before));
    assert!(
        runtime::AppState::lock(&data.state.brains)
            .stats("discord:123")
            .is_none()
    );
    assert!(!requests.iter().any(|r| {
        r.body["content"]
            .as_str()
            .is_some_and(|text| text.contains("Feedback since process start"))
    }));
}
