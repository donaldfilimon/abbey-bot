//! `/admin` dashboard dispatch: acknowledgement before reads, permission reload, page selects.
use super::*;

fn admin_component(
    fixture: &DiscordFixture,
    session: &crate::admin_dashboard::AdminSession,
    action: crate::admin_dashboard::AdminAction,
    channel: u64,
) -> serenity::all::ComponentInteraction {
    let mut message = Message::default();
    message.id = serenity::all::MessageId::new(900);
    message.channel_id = ChannelId::new(channel);
    message.author = fixture.context.cache.current_user().clone().into();
    serde_json::from_value(json!({
        "id": "901", "application_id": "321",
        "data": {"custom_id": session.custom_id(action), "component_type": 2},
        "guild_id": GUILD.to_string(), "channel_id": channel.to_string(),
        "message": message, "user": user(ACTOR), "token": "offline-component",
        "version": 1, "locale": "en-US", "entitlements": [], "attachment_size_limit": 1048576
    }))
    .unwrap()
}

fn admin_page_select(
    fixture: &DiscordFixture,
    session: &crate::admin_dashboard::AdminSession,
    page: crate::admin_dashboard::AdminPage,
    channel: u64,
) -> serenity::all::ComponentInteraction {
    use crate::admin_dashboard::AdminAction;
    let mut message = Message::default();
    message.id = serenity::all::MessageId::new(900);
    message.channel_id = ChannelId::new(channel);
    message.author = fixture.context.cache.current_user().clone().into();
    serde_json::from_value(json!({
        "id": "901", "application_id": "321",
        "data": {
            "custom_id": session.custom_id(AdminAction::SelectPage),
            "component_type": 3,
            "values": [AdminAction::View(page).slug()]
        },
        "guild_id": GUILD.to_string(), "channel_id": channel.to_string(),
        "message": message, "user": user(ACTOR), "token": "offline-component",
        "version": 1, "locale": "en-US", "entitlements": [], "attachment_size_limit": 1048576
    }))
    .unwrap()
}

#[tokio::test]
async fn actual_admin_dispatch_enforces_ack_permission_reload_reset_scope_and_private_export() {
    use crate::admin_dashboard::{AdminAction, AdminPage, AdminSession};
    let fixture = DiscordFixture::new().await;
    fixture
        .permissions
        .store(Permissions::MANAGE_GUILD.bits(), Ordering::SeqCst);
    let data = configured_data();
    let session = AdminSession {
        owner: ACTOR,
        guild: GUILD,
        expiry: runtime::now() + 900,
        page: AdminPage::Operations,
    };

    fixture.fail_permissions.store(true, Ordering::SeqCst);
    let interaction = admin_component(&fixture, &session, AdminAction::Flush, CHANNEL);
    assert!(
        crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data)
            .await
    );
    let requests = fixture.take_requests();
    assert_eq!(requests[0].body["type"], 5);
    assert_ne!(
        requests[0].body["data"]["flags"].as_u64().unwrap_or(0) & 64,
        0
    );
    assert!(requests.iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|body| body.contains("Nothing changed"))
    }));

    fixture.fail_permissions.store(false, Ordering::SeqCst);
    fixture
        .permissions
        .store(Permissions::empty().bits(), Ordering::SeqCst);
    let interaction = admin_component(&fixture, &session, AdminAction::SetLearning(true), CHANNEL);
    assert!(
        crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data)
            .await
    );
    assert!(
        !crate::runtime::AppState::lock(&data.state.stores)
            .guilds
            .get("discord:123")
            .is_some_and(|settings| settings.learning_enabled)
    );
    fixture.take_requests();

    fixture
        .permissions
        .store(Permissions::MANAGE_GUILD.bits(), Ordering::SeqCst);
    let authoritative = crate::guild::GuildSettings {
        learning_enabled: true,
        ..crate::guild::GuildSettings::default()
    };
    crate::runtime::AppState::lock(&data.state.stores)
        .guilds
        .insert("discord:123".into(), authoritative);
    let interaction = admin_component(&fixture, &session, AdminAction::SetLearning(true), CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    let requests = fixture.take_requests();
    assert!(requests.iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|body| body.contains("already has"))
    }));

    let interaction = admin_component(&fixture, &session, AdminAction::SetEpsilon(20), CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    fixture.take_requests();
    let epsilon = crate::guild::clamp_epsilon(0.2);
    assert_eq!(
        crate::runtime::AppState::lock(&data.state.stores).guilds["discord:123"].epsilon_override,
        Some(epsilon)
    );
    let brain_epsilon = {
        let stores = crate::runtime::AppState::lock(&data.state.stores);
        crate::runtime::AppState::lock(&data.state.brains)
            .brain("discord:123", &*stores, runtime::now())
            .epsilon()
    };
    assert_eq!(brain_epsilon, epsilon);
    let interaction = admin_component(&fixture, &session, AdminAction::SetEpsilon(20), CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    assert!(fixture.take_requests().iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|body| body.contains("already has"))
    }));

    use crate::brain::social::ReputationStore;
    let current_guild = "discord:123";
    let other_guild = "discord:999";
    let actor = "discord:789";
    data.state
        .memory_service()
        .remember(current_guild, actor, "current fact", 1)
        .unwrap();
    data.state
        .memory_service()
        .remember(other_guild, actor, "other guild fact", 1)
        .unwrap();
    let protected_current_settings =
        crate::runtime::AppState::lock(&data.state.stores).guilds[current_guild].clone();
    let protected_other_settings = crate::guild::GuildSettings {
        unsolicited: true,
        ..Default::default()
    };
    {
        let mut stores = crate::runtime::AppState::lock(&data.state.stores);
        stores
            .guilds
            .insert(other_guild.into(), protected_other_settings.clone());
        stores.store_reputation(current_guild, actor, 0.73, 1);
        stores.store_reputation(other_guild, actor, 0.41, 1);
    }
    let current = crate::guild::scoped_channel_id("discord", &CHANNEL.to_string());
    let other = crate::guild::scoped_channel_id("discord", "457");
    let other_guild_transcript = crate::guild::scoped_channel_id("discord", "9999");
    let dm_transcript = "discord:dm:790";
    crate::runtime::AppState::lock(&data.state.engine).commit(&current, "one", "reply", 1);
    crate::runtime::AppState::lock(&data.state.engine).commit(&other, "two", "reply", 1);
    crate::runtime::AppState::lock(&data.state.engine).commit(
        &other_guild_transcript,
        "three",
        "reply",
        1,
    );
    crate::runtime::AppState::lock(&data.state.engine).commit(dm_transcript, "four", "reply", 1);
    let assert_non_transcript_canaries = || {
        assert_eq!(
            data.state
                .memory_service()
                .subject_snapshot(current_guild, actor)
                .0,
            vec!["current fact"]
        );
        assert_eq!(
            data.state
                .memory_service()
                .subject_snapshot(other_guild, actor)
                .0,
            vec!["other guild fact"]
        );
        assert_eq!(data.state.reputation_snapshot(current_guild, actor), 0.73);
        assert_eq!(data.state.reputation_snapshot(other_guild, actor), 0.41);
        let stores = crate::runtime::AppState::lock(&data.state.stores);
        assert_eq!(stores.guilds[current_guild], protected_current_settings);
        assert_eq!(stores.guilds[other_guild], protected_other_settings);
        drop(stores);
        assert_eq!(
            crate::runtime::AppState::lock(&data.state.engine).session_len(&other),
            2
        );
        assert_eq!(
            crate::runtime::AppState::lock(&data.state.engine).session_len(&other_guild_transcript),
            2
        );
        assert_eq!(
            crate::runtime::AppState::lock(&data.state.engine).session_len(dm_transcript),
            2
        );
    };
    let interaction = admin_component(&fixture, &session, AdminAction::RequestReset, CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    assert_eq!(
        crate::runtime::AppState::lock(&data.state.engine).session_len(&current),
        2
    );
    assert_non_transcript_canaries();
    let requests = fixture.take_requests();
    let confirmation = requests
        .iter()
        .find_map(|request| request.body["content"].as_str())
        .expect("confirm reset view");
    assert!(confirmation.contains("Confirm reset"));
    assert!(requests.iter().any(|request| {
        request.body["components"]
            .to_string()
            .contains("confirm-reset")
    }));
    let interaction = admin_component(&fixture, &session, AdminAction::ConfirmReset, CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    assert_eq!(
        crate::runtime::AppState::lock(&data.state.engine).session_len(&current),
        0
    );
    assert_eq!(
        crate::runtime::AppState::lock(&data.state.engine).session_len(&other),
        2
    );
    assert_non_transcript_canaries();
    fixture.take_requests();
    let interaction = admin_component(&fixture, &session, AdminAction::ConfirmReset, CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    assert!(fixture.take_requests().iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|body| body.contains("already clear"))
    }));
    assert_non_transcript_canaries();

    let interaction = admin_component(&fixture, &session, AdminAction::Export, CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    let requests = fixture.take_requests();
    assert_eq!(requests[0].body["type"], 5);
    assert_ne!(
        requests[0].body["data"]["flags"].as_u64().unwrap_or(0) & 64,
        0
    );
    assert!(requests.iter().any(|request| {
        request.body["multipart"]
            .as_str()
            .is_some_and(|body| body.contains("brain.json") && body.contains("allowed_mentions"))
    }));
}

#[tokio::test]
async fn actual_admin_dispatch_never_reads_or_mutates_before_acknowledgement() {
    use crate::admin_dashboard::{AdminAction, AdminPage, AdminSession};
    let fixture = DiscordFixture::new().await;
    fixture
        .permissions
        .store(Permissions::MANAGE_GUILD.bits(), Ordering::SeqCst);
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let data = configured_data();
    let session = AdminSession {
        owner: ACTOR,
        guild: GUILD,
        expiry: runtime::now() + 900,
        page: AdminPage::Learning,
    };
    let interaction = admin_component(&fixture, &session, AdminAction::SetLearning(true), CHANNEL);
    let action =
        crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data);
    tokio::pin!(action);
    tokio::select! {
        entered = fixture.acknowledgement_entered.acquire() => entered.unwrap().forget(),
        _ = &mut action => panic!("admin mutation completed before held acknowledgement"),
    }
    let requests = fixture.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["type"], 5);
    assert!(
        !crate::runtime::AppState::lock(&data.state.stores)
            .guilds
            .contains_key("discord:123")
    );
    fixture.acknowledgement_release.add_permits(1);
    assert!(action.await);
    fixture.take_requests();

    fixture.hold_acknowledgement.store(false, Ordering::SeqCst);
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    let interaction = admin_component(&fixture, &session, AdminAction::SetVision(false), CHANNEL);
    assert!(
        crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data)
            .await
    );
    assert_eq!(fixture.take_requests().len(), 1);
    let settings = crate::runtime::AppState::lock(&data.state.stores).guilds["discord:123"].clone();
    assert!(settings.vision_enabled);
}

#[tokio::test]
async fn actual_admin_page_select_opens_dashboard_page_fail_closed() {
    use crate::admin_dashboard::{AdminAction, AdminPage, AdminSession};
    let fixture = DiscordFixture::new().await;
    fixture
        .permissions
        .store(Permissions::MANAGE_GUILD.bits(), Ordering::SeqCst);
    let data = configured_data();
    let session = AdminSession {
        owner: ACTOR,
        guild: GUILD,
        expiry: runtime::now() + 900,
        page: AdminPage::Overview,
    };

    let interaction = admin_page_select(&fixture, &session, AdminPage::Learning, CHANNEL);
    assert!(
        crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data)
            .await
    );
    let requests = fixture.take_requests();
    assert_eq!(requests[0].body["type"], 5);
    assert!(requests.iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|body| body.contains("Administration · Learning"))
    }));
    assert!(requests.iter().any(|request| {
        let rendered = request.body["components"].to_string();
        rendered.contains("page-select") && rendered.contains("learning-on")
    }));

    // Non-nav option value on the select sentinel fails closed (no mutation).
    let mut bad = admin_page_select(&fixture, &session, AdminPage::Conversation, CHANNEL);
    bad.data.custom_id = session.custom_id(AdminAction::SelectPage);
    if let ComponentInteractionDataKind::StringSelect { values } = &mut bad.data.kind {
        *values = vec!["confirm-reset".into()];
    } else {
        panic!("expected string select");
    }
    assert!(crate::commands_brain::dispatch_admin_component(&fixture.context, &bad, &data).await);
    assert!(fixture.take_requests().iter().any(|request| {
        request.body["content"].as_str().is_some_and(|body| {
            body.contains("stale")
                || body.contains("someone else")
                || body.contains("expired")
                || body.contains("Open `/admin dashboard`")
        })
    }));
}
