//! Actual task adapters over the existing loopback Discord fixture.
use super::*;
use serenity::all::{InteractionContext as DiscordContext, ModalInteraction};

fn task(fixture: &DiscordFixture, action: &str) -> ComponentInteraction {
    let session = help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Start).unwrap();
    let mut interaction = help_component(fixture, session, false, true);
    interaction.context = Some(DiscordContext::Guild);
    interaction.application_id = ApplicationId::new(fixture.context.cache.current_user().id.get());
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

#[tokio::test]
async fn workflow_modal_rejects_foreign_context_and_malformed_inputs_before_io() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    for case in 0..14 {
        let mut interaction = modal(&fixture);
        match case {
            0 => interaction.message.as_mut().unwrap().channel_id = ChannelId::new(CHANNEL + 1),
            1 => interaction.user.bot = true,
            2 => interaction.user.id = UserId::new(OTHER),
            3 => interaction.data.custom_id = interaction.data.custom_id.replace(":ask", ":memory"),
            4 => interaction.data.custom_id = interaction.data.custom_id.replace(":v1:", ":v2:"),
            5 => interaction.data.components.clear(),
            6 => interaction
                .data
                .components
                .push(interaction.data.components[0].clone()),
            7 => interaction.application_id = ApplicationId::new(OTHER),
            8 => interaction.message = None,
            9 => {
                interaction.data.custom_id = format!(
                    "abbey:task:v1:{ACTOR}:{GUILD}:{CHANNEL}:{}:ask",
                    runtime::now()
                )
            }
            case => {
                let serenity::all::ActionRowComponent::InputText(field) =
                    &mut interaction.data.components[0].components[0]
                else {
                    panic!("question fixture");
                };
                match case {
                    10 => field.custom_id = "foreign".into(),
                    11 => field.value = Some(" \n ".into()),
                    12 => field.value = Some("x".repeat(2001)),
                    _ => field.value = None,
                }
            }
        }
        assert!(workflows::dispatch_modal(&fixture.context, &interaction, &data).await);
        let requests = fixture.take_requests();
        assert_eq!(
            requests.len(),
            2,
            "case {case}: only acknowledgement and rejection allowed"
        );
        let body = assert_private_help_response(&requests);
        assert!(body["content"].as_str().unwrap().chars().count() <= 2000);
    }
}

#[tokio::test]
async fn workflow_foreign_or_malformed_button_never_opens_input() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    for case in 0..6 {
        let mut interaction = task(&fixture, "ask");
        match case {
            0 => interaction.application_id = ApplicationId::new(OTHER),
            1 => interaction.message.channel_id = ChannelId::new(CHANNEL + 1),
            2 => interaction.message.author.id = UserId::new(OTHER),
            3 => {
                interaction.data.kind = ComponentInteractionDataKind::StringSelect {
                    values: vec!["ask".into()],
                }
            }
            4 => interaction.context = Some(DiscordContext::BotDm),
            _ => interaction.user.bot = true,
        }
        assert!(dispatch_component(&fixture.context, &interaction, &data, false).await);
        let requests = fixture.take_requests();
        assert_eq!(requests.len(), 1, "case {case}");
        assert_eq!(requests[0].body["type"], 4);
        assert_eq!(requests[0].body["data"]["flags"], 64);
    }
}

#[tokio::test]
async fn workflow_admin_rechecks_authority_after_acknowledgement() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    fixture.permissions.store(
        (Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD).bits(),
        Ordering::SeqCst,
    );
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let interaction = task(&fixture, "admin");
    let action = dispatch_component(&fixture.context, &interaction, &data, false);
    tokio::pin!(action);
    tokio::select! {
        entered = fixture.acknowledgement_entered.acquire() => entered.unwrap().forget(),
        _ = &mut action => panic!("completed before acknowledgement"),
    }
    fixture
        .permissions
        .store(Permissions::VIEW_CHANNEL.bits(), Ordering::SeqCst);
    fixture.acknowledgement_release.add_permits(1);
    assert!(action.await);
    let requests = fixture.take_requests();
    let body = assert_private_help_response(&requests);
    assert!(
        body["content"]
            .as_str()
            .unwrap()
            .contains("current Discord access")
    );
    assert_eq!(body["components"], json!([]));
}

#[tokio::test]
async fn workflow_modal_delivery_failure_explains_without_replaying_generation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    fixture.fail_next_edit.store(true, Ordering::SeqCst);
    assert!(workflows::dispatch_modal(&fixture.context, &modal(&fixture), &data).await);
    let requests = fixture.take_requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.route == "/v1/chat/completions")
            .count(),
        1
    );
    let edits: Vec<_> = requests.iter().filter(|r| r.method == "PATCH").collect();
    assert_eq!(edits.len(), 2);
    let body = edits[1].body["content"].as_str().unwrap();
    assert!(body.contains("delivering"));
    assert!(body.chars().count() <= 2000);
    assert_eq!(edits[1].body["allowed_mentions"]["parse"], json!([]));
}

#[tokio::test]
async fn workflow_failed_modal_ack_never_runs_permission_or_generation_work() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    assert!(workflows::dispatch_modal(&fixture.context, &modal(&fixture), &data).await);
    let requests = fixture.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["type"], 5);
}
