//! /roleplay bot-DM admission through the actual registered Poise leaf.
//! Apply as commands_help/dispatch_tests/native_roleplay_dm_tests.rs and add
//! mod native_roleplay_dm_tests; to dispatch_tests.rs. Loopback only.
use super::*;
use serenity::all::{ChannelType, InteractionContext as NativeContext, PrivateChannel};

#[derive(Clone, Copy, Debug)]
enum DmFact {
    Valid,
    WrongRecipient,
    WrongChannel,
    BotRecipient,
    GroupDmEnvelope,
    GuildChannel,
    NativeUnavailable,
}

async fn actual_roleplay_dm(fact: DmFact) -> (bool, String) {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        runtime::AppState::lock(&data.state.guilds).update(
            &format!("discord:dm:{ACTOR}"),
            &mut *stores,
            |settings| settings.nsfw_roleplay_enabled = true,
        );
    }
    let mut fetched = PrivateChannel::default();
    fetched.id = ChannelId::new(if matches!(fact, DmFact::WrongChannel) {
        CHANNEL + 1
    } else {
        CHANNEL
    });
    fetched.kind = ChannelType::Private;
    fetched.recipient = user(if matches!(fact, DmFact::WrongRecipient) {
        OTHER
    } else {
        ACTOR
    });
    fetched.recipient.bot = matches!(fact, DmFact::BotRecipient);
    let mut response = serde_json::to_value(fetched).unwrap();
    if matches!(fact, DmFact::GroupDmEnvelope) {
        // This type is rejected by Serenity Channel deserialization if read;
        // the old shell never reads it and maps no-guild to BotDm unconditionally.
        response["type"] = json!(3);
    }
    if matches!(fact, DmFact::GuildChannel) {
        let mut channel = GuildChannel::default();
        channel.id = ChannelId::new(CHANNEL);
        channel.guild_id = GuildId::new(GUILD);
        channel.kind = ChannelType::Text;
        channel.nsfw = true;
        response = serde_json::to_value(channel).unwrap();
    }
    fixture
        .fail_permissions
        .store(matches!(fact, DmFact::NativeUnavailable), Ordering::SeqCst);
    fixture
        .native_responses
        .lock()
        .unwrap()
        .insert(format!("/channels/{CHANNEL}"), response);
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Roleplay);
    let mut invocation = Invocation::new(command, false, None);
    invocation.interaction.context = match fact {
        DmFact::GroupDmEnvelope => Some(NativeContext::PrivateChannel),
        _ => Some(NativeContext::BotDm),
    };
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        &fixture,
        command,
        &options,
        &data,
        poise::CommandInteractionType::Command,
    );
    let allowed = command.checks[0](poise::Context::Application(context))
        .await
        .unwrap();
    if allowed {
        // No prompt: the configured provider is never needed or invoked.
        let _ = command.slash_action.unwrap()(context).await;
    }
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
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
    let activated = runtime::AppState::lock(&data.state.engine)
        .session_persona(&format!("discord:{CHANNEL}"))
        == Some(crate::persona::Persona::Aviva);
    let text = requests
        .iter()
        .filter_map(|r| r.body["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (activated, text)
}

#[tokio::test]
async fn native_roleplay_dm_proof_preserves_exact_human_bot_dm_control_without_provider() {
    let (activated, text) = actual_roleplay_dm(DmFact::Valid).await;
    assert!(activated);
    assert!(text.contains("Aviva roleplay is available here"));
}

#[tokio::test]
async fn native_roleplay_dm_proof_refuses_wrong_recipient_channel_bot_or_non_bot_dm_envelope() {
    for fact in [
        DmFact::WrongRecipient,
        DmFact::WrongChannel,
        DmFact::BotRecipient,
        DmFact::GroupDmEnvelope,
        DmFact::GuildChannel,
        DmFact::NativeUnavailable,
    ] {
        let (activated, text) = actual_roleplay_dm(fact).await;
        if matches!(fact, DmFact::WrongRecipient) {
            println!("{text}");
        }
        assert!(
            !activated,
            "unconfirmed bot-DM context cannot activate Aviva: {fact:?}"
        );
        assert!(!text.contains("Aviva roleplay is available here"));
    }
}
