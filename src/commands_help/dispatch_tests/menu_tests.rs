//! Registered memory and image context menus: private replies, provider calls, no state mutation.
use super::*;

async fn invoke_user_menu(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    target: User,
) {
    let invocation = Invocation::new(command, true, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    let Some(poise::ContextMenuCommandAction::User(action)) = command.context_menu_action else {
        panic!("expected user menu")
    };
    assert!(action(context, target).await.is_ok());
}

async fn invoke_message_menu(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    target: Message,
    in_guild: bool,
) {
    let invocation = Invocation::new(command, in_guild, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    let Some(poise::ContextMenuCommandAction::Message(action)) = command.context_menu_action else {
        panic!("expected message menu")
    };
    assert!(action(context, target).await.is_ok());
}

#[tokio::test]
async fn registered_memory_menu_shares_card_and_a1_denial_without_mutation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let guild = format!("discord:{GUILD}");
    let actor = format!("discord:{ACTOR}");
    data.state
        .memory_service()
        .remember(&guild, &actor, "likes Rust", 1)
        .unwrap();
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::MemoryMenu);
    let before = runtime::AppState::lock(&data.state.stores).clone();

    invoke_user_menu(&fixture, command, &data, user(ACTOR)).await;
    let requests = fixture.take_requests();
    let content = assert_private_no_mentions_reply(&requests);
    assert!(content.contains("likes Rust"));
    assert!(content.contains("standing 0.50"));
    let menu_body = content.to_string();
    let reply = requests
        .iter()
        .find(|request| request.route.contains("/webhooks/"))
        .unwrap();
    assert_eq!(
        reply.body["components"][0]["components"][0]["label"],
        "Browse facts"
    );
    let recall = command_by_key(&commands, CommandKey::Recall);
    let invocation = Invocation::new(recall, true, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        &fixture,
        recall,
        &options,
        &data,
        poise::CommandInteractionType::Command,
    );
    assert!(recall.slash_action.unwrap()(context).await.is_ok());
    let slash_requests = fixture.take_requests();
    assert_eq!(assert_private_no_mentions_reply(&slash_requests), menu_body);
    let slash = slash_requests
        .iter()
        .find(|request| request.route.contains("/webhooks/"))
        .unwrap();
    assert_eq!(
        slash.body["components"][0]["components"][0]["label"],
        "Browse facts"
    );
    assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
    assert_eq!(
        runtime::AppState::lock(&data.state.rewards).pending_len(),
        0
    );

    invoke_user_menu(&fixture, command, &data, user(OTHER)).await;
    let denied = fixture.take_requests();
    assert!(denied.iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|text| text.contains("only your own memory"))
    }));
    assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
}

#[tokio::test]
async fn registered_image_menus_invoke_provider_privately_without_state_mutation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    let commands = crate::application_commands();
    let attachment: serenity::all::Attachment = serde_json::from_value(json!({
        "id":"1", "filename":"misleading.txt", "size":100,
        "url":format!("http://{}/fixture.png", fixture.address),
        "proxy_url":"https://ignored.invalid/proxy"
    }))
    .unwrap();
    let mut message = Message::default();
    message.content = "https://ignored.invalid/body.png".into();
    message.attachments.push(attachment);
    let before = runtime::AppState::lock(&data.state.stores).clone();

    for key in [CommandKey::DescribeImage, CommandKey::ReadImage] {
        let command = command_by_key(&commands, key);
        invoke_message_menu(&fixture, command, &data, message.clone(), false).await;
        let requests = fixture.take_requests();
        let content = assert_private_no_mentions_reply(&requests);
        assert!(content.contains('x'));
        assert!(content.chars().count() <= 2_000);
        assert!(
            requests
                .iter()
                .any(|request| request.route == "/fixture.png")
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.route.contains("ignored.invalid"))
        );
        assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
        assert_eq!(
            runtime::AppState::lock(&data.state.rewards).pending_len(),
            0
        );
    }

    let command = command_by_key(&commands, CommandKey::DescribeImage);
    invoke_message_menu(&fixture, command, &data, Message::default(), true).await;
    let requests = fixture.take_requests();
    let content = assert_private_no_mentions_reply(&requests);
    assert!(content.contains("no supported image attachment"));
    assert!(
        !requests
            .iter()
            .any(|request| request.route == "/v1/chat/completions")
    );
    assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
}

#[tokio::test]
async fn registered_image_failure_uses_typed_private_member_guidance() {
    let fixture = DiscordFixture::new().await;
    let provider = ProviderFixture::new().await;
    let data = configured_data_at(Some(provider.address));
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::DescribeImage);
    let mut message = Message::default();
    message.attachments.push(serde_json::from_value(json!({
        "id":"1", "filename":"image.png", "size":100,
        "url":format!("http://{}/fixture.png",fixture.address), "proxy_url":"https://private.invalid/canary"
    })).unwrap());
    invoke_message_menu(&fixture, command, &data, message, false).await;
    let requests = fixture.take_requests();
    let content = assert_private_no_mentions_reply(&requests);
    assert!(content.contains("Ask a server manager"), "{content}");
    for forbidden in [
        "logs",
        "credentials",
        "127.0.0.1",
        "fixture",
        "private.invalid",
        "ABBEY_",
    ] {
        assert!(!content.contains(forbidden));
    }
    assert!(provider.calls.load(Ordering::SeqCst) > 0);
}
