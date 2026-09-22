//! Pending-proposal buttons: permission refresh, acknowledgement ordering, stable indices.
use super::*;

fn pending_fixture_session(data: &Data) -> crate::commands_brain::PendingComponentSession {
    let mut stores = runtime::AppState::lock(&data.state.stores);
    for (old, new) in [("old one", "new one"), ("old two", "new two")] {
        assert!(stores.memory.remember("discord:123", "discord:790", old, 1));
        assert!(stores.memory.remember("discord:123", "discord:790", new, 1));
        assert!(
            stores
                .memory
                .propose_supersession("discord:123", "discord:790", new, old, 1)
        );
    }
    drop(stores);
    crate::commands_brain::PendingComponentSession {
        command_id: 77,
        owner: ACTOR,
        subject: OTHER,
        guild: Some(GUILD),
        channel: CHANNEL,
        version: 0,
        displayed: data
            .state
            .memory_service()
            .pending_supersessions("discord:123", "discord:790"),
    }
}

fn pending_press(fixture: &DiscordFixture, action: &str) -> ComponentInteraction {
    let help = help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Memory).unwrap();
    let mut press = help_component(fixture, help, false, true);
    press.context = Some(serenity::all::InteractionContext::Guild);
    press.data.custom_id = format!("77:p:{action}:{OTHER}:0:v:0");
    press
}

#[tokio::test]
async fn pending_buttons_refresh_revoked_permissions_before_any_mutation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    fixture.permissions.store(
        (Permissions::MANAGE_MESSAGES | Permissions::VIEW_CHANNEL).bits(),
        Ordering::SeqCst,
    );
    let mut session = pending_fixture_session(&data);
    let before = data
        .state
        .memory_service()
        .subject_snapshot("discord:123", "discord:790");
    fixture.permissions.store(0, Ordering::SeqCst);
    for action in ["c", "d"] {
        let press = pending_press(&fixture, action);
        session.version = 0;
        session.displayed = before.1.clone();
        assert!(
            crate::commands_brain::handle_pending_press(
                &fixture.context,
                &press,
                &data.state,
                &mut session
            )
            .await
            .unwrap()
        );
        assert_eq!(
            data.state
                .memory_service()
                .subject_snapshot("discord:123", "discord:790"),
            before
        );
        let requests = fixture.take_requests();
        assert_eq!(requests[0].body["type"], 6);
        assert!(requests.iter().any(|request| request.method == "GET"));
        let reply = &requests
            .iter()
            .find(|request| request.method == "PATCH")
            .unwrap()
            .body;
        assert_eq!(reply["allowed_mentions"]["parse"], json!([]));
        assert!(!reply["content"].as_str().unwrap().contains("old one"));
    }
}

#[tokio::test]
async fn pending_confirm_waits_for_ack_before_permissions_and_effects() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let mut session = pending_fixture_session(&data);
    fixture.permissions.store(
        (Permissions::MANAGE_MESSAGES | Permissions::VIEW_CHANNEL).bits(),
        Ordering::SeqCst,
    );
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let before = data
        .state
        .memory_service()
        .subject_snapshot("discord:123", "discord:790");
    let press = pending_press(&fixture, "c");
    let operation = crate::commands_brain::handle_pending_press(
        &fixture.context,
        &press,
        &data.state,
        &mut session,
    );
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => panic!("held acknowledgement completed: {result:?}"),
        permit = fixture.acknowledgement_entered.acquire() => permit.unwrap().forget(),
    }
    assert!(
        fixture
            .take_requests()
            .iter()
            .all(|request| request.method != "GET")
    );
    assert_eq!(
        data.state
            .memory_service()
            .subject_snapshot("discord:123", "discord:790"),
        before
    );
    fixture.acknowledgement_release.add_permits(1);
    operation.await.unwrap();
    assert_ne!(
        data.state
            .memory_service()
            .subject_snapshot("discord:123", "discord:790"),
        before
    );
}

#[tokio::test]
async fn pending_old_index_never_targets_a_shifted_proposal() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let mut session = pending_fixture_session(&data);
    fixture.permissions.store(
        (Permissions::MANAGE_MESSAGES | Permissions::VIEW_CHANNEL).bits(),
        Ordering::SeqCst,
    );
    let first = session.displayed[0].old_fact.clone();
    assert!(
        data.state
            .memory_service()
            .dismiss_supersession("discord:123", "discord:790", &first)
    );
    let before = data
        .state
        .memory_service()
        .subject_snapshot("discord:123", "discord:790");
    let press = pending_press(&fixture, "c");
    assert!(
        !crate::commands_brain::handle_pending_press(
            &fixture.context,
            &press,
            &data.state,
            &mut session
        )
        .await
        .unwrap()
    );
    assert_eq!(
        data.state
            .memory_service()
            .subject_snapshot("discord:123", "discord:790"),
        before
    );
    assert_eq!(session.version, 1);
    assert!(
        crate::commands_brain::handle_pending_press(
            &fixture.context,
            &press,
            &data.state,
            &mut session
        )
        .await
        .unwrap()
    );
    assert_eq!(
        data.state
            .memory_service()
            .subject_snapshot("discord:123", "discord:790"),
        before
    );
}
