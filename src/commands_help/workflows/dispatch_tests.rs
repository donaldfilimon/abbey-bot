//! Actual task adapters over the existing loopback Discord fixture.
use super::*;
use serenity::all::{InteractionContext as DiscordContext, ModalInteraction};

fn task(fixture: &DiscordFixture, action: &str) -> ComponentInteraction {
    let session = help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Start).unwrap();
    let mut interaction = help_component(fixture, session, false, true);
    interaction.context = Some(DiscordContext::Guild);
    interaction.data.custom_id = format!(
        "abbey:task:v1:{ACTOR}:{GUILD}:{CHANNEL}:{}:{action}",
        session.expiry
    );
    interaction
}
fn modal(fixture: &DiscordFixture) -> ModalInteraction {
    let component = task(fixture, "ask");
    let mut value = serde_json::to_value(&component).unwrap();
    value["application_id"] = fixture.context.cache.current_user().id.to_string().into();
    value["data"] = json!({"custom_id":component.data.custom_id,"components":[{"type":1,"components":[{"type":4,"custom_id":"question","style":2,"label":"Question","value":"What is two plus two?"}]}]});
    serde_json::from_value(value).unwrap()
}
#[tokio::test]
async fn task_question_opens_modal_without_permission_or_provider_io() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    assert!(dispatch_component(&fixture.context, &task(&fixture, "ask"), &data, false).await);
    let requests = fixture.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["type"], 9);
    assert_eq!(requests[0].body["data"]["title"], "Ask Abbey privately");
}
#[tokio::test]
async fn task_buttons_reject_wrong_owner_context_expiry_and_unknown_action_before_io() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    for case in 0..5 {
        let mut interaction = task(&fixture, "memory");
        match case {
            0 => interaction.user.id = UserId::new(OTHER),
            1 => interaction.channel_id = ChannelId::new(CHANNEL + 1),
            2 => interaction.guild_id = Some(GuildId::new(GUILD + 1)),
            3 => {
                interaction.data.custom_id = format!(
                    "abbey:task:v1:{ACTOR}:{GUILD}:{CHANNEL}:{}:memory",
                    runtime::now()
                )
            }
            _ => {
                interaction.data.custom_id =
                    interaction.data.custom_id.replace(":memory", ":delete")
            }
        }
        assert!(dispatch_component(&fixture.context, &interaction, &data, false).await);
        let requests = fixture.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].body["type"], 4);
        assert_eq!(requests[0].body["data"]["flags"], 64);
    }
}
#[tokio::test]
async fn actual_memory_and_admin_tasks_deliver_existing_private_views() {
    let fixture = DiscordFixture::new().await;
    fixture.permissions.store(
        (Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD).bits(),
        Ordering::SeqCst,
    );
    let data = configured_data();
    for action in ["memory", "admin", "images", "voice"] {
        assert!(dispatch_component(&fixture.context, &task(&fixture, action), &data, false).await);
        let requests = fixture.take_requests();
        let body = assert_private_help_response(&requests)["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!body.contains("could not"), "{action}: {body}");
        match action {
            "memory" => assert!(body.to_lowercase().contains("fact")),
            "admin" => assert!(body.contains("Administration · Overview")),
            "images" => assert!(body.contains("/ocr image:")),
            "voice" => assert!(body.contains("Music")),
            _ => unreachable!(),
        }
    }
}
#[tokio::test]
async fn modal_waits_for_ack_and_rejects_changed_channel_access_before_generation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    fixture
        .permissions
        .store(Permissions::empty().bits(), Ordering::SeqCst);
    let interaction = modal(&fixture);
    let action = workflows::dispatch_modal(&fixture.context, &interaction, &data);
    tokio::pin!(action);
    tokio::select! {
        entered = fixture.acknowledgement_entered.acquire() => entered.unwrap().forget(),
        _ = &mut action => panic!("modal completed before acknowledgement"),
    }
    let requests = fixture.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["type"], 5);
    fixture.acknowledgement_release.add_permits(1);
    assert!(action.await);
    let requests = fixture.take_requests();
    assert!(!requests.iter().any(|r| r.route == "/v1/chat/completions"));
    assert!(requests.iter().any(|r| {
        r.body["content"]
            .as_str()
            .is_some_and(|s| s.contains("permissions"))
    }));
}
#[tokio::test]
async fn modal_uses_real_generation_privately_without_committing_the_transcript() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    let before = format!("{:?}", *runtime::AppState::lock(&data.state.engine));
    assert!(workflows::dispatch_modal(&fixture.context, &modal(&fixture), &data).await);
    let requests = fixture.take_requests();
    let body = assert_private_help_response(&requests)["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        requests.iter().any(|r| r.route == "/v1/chat/completions"),
        "{body}"
    );
    assert!(body.contains("xxx"), "{body}");
    assert_eq!(
        format!("{:?}", *runtime::AppState::lock(&data.state.engine)),
        before
    );
}
