//! Existing actual registered /modcall source_message path, loopback only.
//! Add as a sibling commands_help/dispatch_tests module. These assertions are
//! behavioral RED before exact source/member/role and channel-staff repairs.
use super::*;
use serenity::all::{
    MessageId, PermissionOverwrite, PermissionOverwriteType, Timestamp, UserUpdateEvent,
};

const SOURCE: u64 = 1_234_567_890_123_456_789;
const MODERATOR_ROLE: u64 = 500;

#[derive(Clone, Default)]
struct Scenario {
    wrong_source: bool,
    wrong_target: bool,
    channel_staff: bool,
    missing_target_role: bool,
}
fn evidence_routes(fixture: &DiscordFixture, scenario: &Scenario) {
    let mut native_guild = guild(Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY);
    let mut moderator_role = Role::default();
    moderator_role.id = RoleId::new(MODERATOR_ROLE);
    moderator_role.guild_id = GuildId::new(GUILD);
    moderator_role.position = 10;
    moderator_role.permissions = Permissions::VIEW_CHANNEL
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::MANAGE_MESSAGES
        | Permissions::MODERATE_MEMBERS;
    native_guild.roles.insert(moderator_role.id, moderator_role);
    let mut moderator = Member::default();
    moderator.user = user(ACTOR);
    moderator.guild_id = GuildId::new(GUILD);
    moderator.roles = vec![RoleId::new(MODERATOR_ROLE)];
    let mut subject = Member::default();
    subject.user = user(if scenario.wrong_target {
        OTHER + 1
    } else {
        OTHER
    });
    subject.guild_id = GuildId::new(GUILD);
    if scenario.missing_target_role {
        subject.roles = vec![RoleId::new(600)];
    }
    let mut channel = GuildChannel::default();
    channel.id = ChannelId::new(CHANNEL);
    channel.guild_id = GuildId::new(GUILD);
    channel.kind = serenity::all::ChannelType::Text;
    if scenario.channel_staff {
        channel.permission_overwrites = vec![PermissionOverwrite {
            allow: Permissions::MANAGE_MESSAGES,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Member(UserId::new(OTHER)),
        }];
    }
    let mut message = Message::default();
    message.id = MessageId::new(if scenario.wrong_source {
        SOURCE + 1
    } else {
        SOURCE
    });
    message.guild_id = Some(GuildId::new(GUILD));
    message.channel_id = ChannelId::new(CHANNEL);
    message.author = user(OTHER);
    message.content =
        "Synthetic contextual source text, never an allegation about a real person".into();
    message.timestamp = Timestamp::from_unix_timestamp(1_700_000_000).unwrap();
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
            serde_json::to_value(subject).unwrap(),
        ),
        (
            format!("/channels/{CHANNEL}"),
            serde_json::to_value(channel).unwrap(),
        ),
        (
            format!("/channels/{CHANNEL}/messages/{SOURCE}"),
            serde_json::to_value(message).unwrap(),
        ),
    ]);
}
async fn contextual_reply(fixture: &DiscordFixture, data: &Data) -> (String, Vec<Request>) {
    // Match the actual application identity before exercising source failures.
    let mut bot = user(321);
    bot.bot = true;
    let mut event: UserUpdateEvent =
        serde_json::from_value(serde_json::to_value(bot).unwrap()).unwrap();
    fixture.context.cache.update(&mut event);
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
    // ChoiceParameter uses integer indexes in pinned Poise 0.6.2.
    invocation.interaction.data.options = serde_json::from_value(json!([
        {"name":"user", "type":6, "value":OTHER.to_string()},
        {"name":"severity", "type":4, "value":2},
        {"name":"source_message", "type":3, "value":SOURCE.to_string()},
        {"name":"context", "type":4, "value":0}
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
    assert!(command.slash_action.unwrap()(context).await.is_ok());
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    let text = requests
        .iter()
        .find(|r| r.route.contains("/webhooks/"))
        .and_then(|r| r.body["content"].as_str())
        .expect("private deferred contextual reply")
        .to_owned();
    // Mention parsing is a separate regression below, so source identity
    // assertions reach their intended old-production boundary first.
    assert!(
        requests
            .iter()
            .filter(|r| r.method != "GET")
            .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/"))
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.route.contains("/chat/completions"))
    );
    assert!(!requests.iter().any(|r| r.method == "DELETE"
        || r.method == "PUT"
        || r.route.ends_with(&format!("/members/{OTHER}")) && r.method == "PATCH"));
    (text, requests)
}

#[tokio::test]
async fn actual_contextual_proposal_requires_exact_native_source_target_and_role_coverage() {
    let mut qualification_mismatches = Vec::new();
    for (label, scenario, qualified) in [
        (
            "valid current message and moderator",
            Scenario::default(),
            true,
        ),
        (
            "source returned a different message ID",
            Scenario {
                wrong_source: true,
                ..Default::default()
            },
            false,
        ),
        (
            "target REST returned another ordinary member",
            Scenario {
                wrong_target: true,
                ..Default::default()
            },
            false,
        ),
        (
            "target holds current channel-only staff grant",
            Scenario {
                channel_staff: true,
                ..Default::default()
            },
            false,
        ),
        (
            "target assigned role facts are incomplete",
            Scenario {
                missing_target_role: true,
                ..Default::default()
            },
            false,
        ),
    ] {
        let fixture = DiscordFixture::new().await;
        evidence_routes(&fixture, &scenario);
        let data = configured_data();
        let before = runtime::AppState::lock(&data.state.stores).clone();
        let (text, _) = contextual_reply(&fixture, &data).await;
        let observed = text.contains("Proposed action: delete this offending message.");
        if qualified {
            // Keep the valid native control attributable and strict.
            assert!(observed, "valid native control must qualify: {text}");
        } else if observed {
            qualification_mismatches.push(format!("{label}: {text}"));
        }
        assert!(text.contains("No action taken") || text.contains("no action taken"));
        assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before));
        if qualified {
            assert!(text.contains(&format!(
                "https://discord.com/channels/{GUILD}/{CHANNEL}/{SOURCE}"
            )));
            assert!(text.contains("10 minutes maximum"));
            assert!(text.contains("not a saved case"));
        }
    }
    assert!(
        qualification_mismatches.is_empty(),
        "All attributable native qualification mismatches:\n{}",
        qualification_mismatches.join("\n\n")
    );
}

#[tokio::test]
async fn actual_contextual_private_reply_disables_mentions_independently() {
    let fixture = DiscordFixture::new().await;
    evidence_routes(&fixture, &Scenario::default());
    let data = configured_data();
    let (_, requests) = contextual_reply(&fixture, &data).await;
    assert_private_no_mentions_reply(&requests);
}
