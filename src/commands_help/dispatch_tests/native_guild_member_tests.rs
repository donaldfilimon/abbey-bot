//! Existing actual registered /modcall owner-proof regression, loopback only.
//! Apply as commands_help/dispatch_tests/native_guild_member_tests.rs and register
//! mod native_guild_member_tests; in dispatch_tests.rs. No source API is added.
use super::*;

const MODERATOR_ROLE: u64 = 500;
#[derive(Clone, Copy, Debug)]
enum NativeGuildFailure {
    None,
    ModeratorOwner,
    GuildIdentity,
    UnknownModeratorRole,
    MissingEveryone,
    TargetIdentity,
    UnknownTargetRole,
}
fn native_routes(fixture: &DiscordFixture, failure: NativeGuildFailure) {
    let mut native_guild = guild(Permissions::VIEW_CHANNEL);
    if matches!(failure, NativeGuildFailure::GuildIdentity) {
        native_guild.id = GuildId::new(GUILD + 1);
    }
    let mut role = Role::default();
    role.id = RoleId::new(MODERATOR_ROLE);
    role.guild_id = native_guild.id;
    role.position = 10;
    role.permissions =
        Permissions::VIEW_CHANNEL | Permissions::MODERATE_MEMBERS | Permissions::BAN_MEMBERS;
    native_guild.roles.insert(role.id, role);
    if matches!(failure, NativeGuildFailure::MissingEveryone) {
        native_guild.roles.remove(&RoleId::new(GUILD));
    }
    let mut moderator = Member::default();
    moderator.user = user(if matches!(failure, NativeGuildFailure::ModeratorOwner) {
        999
    } else {
        ACTOR
    });
    moderator.guild_id = GuildId::new(GUILD);
    moderator.roles.push(RoleId::new(MODERATOR_ROLE));
    if matches!(failure, NativeGuildFailure::ModeratorOwner) {
        moderator.roles.clear();
        // The genuine ACTOR has no role or moderation grant; only the wrong
        // returned owner object can satisfy the old body permission guard.
    }
    if matches!(failure, NativeGuildFailure::UnknownModeratorRole) {
        moderator.roles.push(RoleId::new(600));
    }
    let mut target = Member::default();
    target.user = user(if matches!(failure, NativeGuildFailure::TargetIdentity) {
        OTHER + 1
    } else {
        OTHER
    });
    target.guild_id = GuildId::new(GUILD);
    if matches!(failure, NativeGuildFailure::UnknownTargetRole) {
        target.roles.push(RoleId::new(600));
    }
    fixture.native_responses.lock().unwrap().extend([
        (
            format!("/guilds/{GUILD}"),
            serde_json::to_value(native_guild).unwrap(),
        ),
        (
            format!("/guilds/{GUILD}/members/{ACTOR}"),
            serde_json::to_value(moderator).unwrap(),
        ),
        (
            format!("/guilds/{GUILD}/members/{OTHER}"),
            serde_json::to_value(target).unwrap(),
        ),
    ]);
}
async fn actual_modcall(fixture: &DiscordFixture, data: &Data) -> (bool, Vec<Request>) {
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Modcall);
    let mut invocation = Invocation::new(command, true, Some(OTHER));
    invocation.interaction.context = Some(serenity::all::InteractionContext::Guild);
    invocation
        .interaction
        .data
        .resolved
        .users
        .insert(UserId::new(OTHER), user(OTHER));
    invocation.interaction.data.options = serde_json::from_value(json!([
        {"name":"user", "type":6, "value":OTHER.to_string()},
        {"name":"severity", "type":4, "value":2}
    ]))
    .unwrap();
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
        &args,
    );
    assert!(
        command.checks[0](poise::Context::Application(context))
            .await
            .unwrap()
    );
    // The adapter can truthfully refuse either as a command error or fixed
    // denial; neither may manufacture a qualified native hierarchy result.
    let _ = command.slash_action.unwrap()(context).await;
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(
        requests
            .iter()
            .filter(|r| r.method != "GET")
            .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/"))
    );
    let qualified = requests.iter().any(|r| {
        r.body["content"]
            .as_str()
            .is_some_and(|text| text.contains("— Ban.") && !text.contains('⚠'))
    });
    (qualified, requests)
}

#[tokio::test]
async fn native_guild_member_proof_actual_modcall_preserves_valid_moderator_control() {
    let fixture = DiscordFixture::new().await;
    native_routes(&fixture, NativeGuildFailure::None);
    let data = configured_data();
    let before = runtime::AppState::lock(&data.state.stores).clone();
    let (qualified, requests) = actual_modcall(&fixture, &data).await;
    assert!(
        qualified,
        "the registered native command must reach its existing real body"
    );
    assert_eq!(requests.iter().filter(|r| r.method == "GET").count(), 3);
    assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before));
}

#[tokio::test]
async fn native_guild_member_proof_actual_modcall_refuses_owner_scope_and_incomplete_role_substitution()
 {
    for failure in [
        NativeGuildFailure::ModeratorOwner,
        NativeGuildFailure::GuildIdentity,
        NativeGuildFailure::UnknownModeratorRole,
        NativeGuildFailure::MissingEveryone,
        NativeGuildFailure::TargetIdentity,
        NativeGuildFailure::UnknownTargetRole,
    ] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture, failure);
        let data = configured_data();
        let before = runtime::AppState::lock(&data.state.stores).clone();
        let (qualified, requests) = actual_modcall(&fixture, &data).await;
        assert!(
            !qualified,
            "{failure:?} cannot confer the real body moderator/hierarchy authority"
        );
        assert!(requests.iter().any(|r| r.method == "GET"));
        assert!(!requests.iter().any(|r| r.method == "DELETE"
            || r.method == "PUT"
            || r.route.contains("/chat/completions")
            || (r.method == "PATCH" && r.route.contains("/members/"))));
        assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before));
    }
}
