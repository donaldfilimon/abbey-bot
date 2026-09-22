//! `/help` navigation components: acknowledgement ordering, permission refresh, DM visibility.
use super::*;

fn private_help_body(reply: &Value) -> &str {
    reply["embeds"][0]["description"]
        .as_str()
        .expect("private help embed description")
}

fn assert_private_help_response(requests: &[Request]) -> &Value {
    let reply = private_ephemeral_patch(requests);
    let content = reply["content"].as_str().unwrap_or("");
    assert!(
        content.is_empty(),
        "help body lives in the Abbey embed, not content"
    );
    assert!(private_help_body(reply).chars().count() <= 2000);
    assert_eq!(reply["embeds"][0]["author"]["name"], "Abbey");
    reply
}

#[tokio::test]
async fn actual_help_navigation_waits_for_ack_and_refreshes_permissions_without_effects() {
    let fixture = DiscordFixture::new().await;
    let provider = ProviderFixture::new().await;
    let data = configured_data_at(Some(provider.address));
    runtime::AppState::lock(&data.state.stores).memory.remember(
        "discord:123",
        "discord:789",
        "navigation must preserve this fact",
        1,
    );
    runtime::AppState::lock(&data.state.rewards).register_reply(
        vec![0.5],
        1,
        "pending-help-canary",
        "discord:123",
        1,
    );
    let stores = runtime::AppState::lock(&data.state.stores).clone();
    let rewards = runtime::AppState::lock(&data.state.rewards).clone();
    runtime::AppState::lock(&data.state.engine).commit(
        "discord:456",
        "preserved help question",
        "preserved help reply",
        1,
    );
    let engine = format!("{:?}", *runtime::AppState::lock(&data.state.engine));
    let voice = data.voice.as_ref().unwrap();
    voice.reserve_start();
    let voice_before = format!("{:?}", voice.snapshot().await);
    let session =
        help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Administration).unwrap();
    for select in [false, true] {
        for section in HelpSection::ALL {
            let session = session.navigate(section);
            for manager in [true, false] {
                fixture.permissions.store(
                    if manager {
                        Permissions::MANAGE_GUILD.bits()
                    } else {
                        Permissions::VIEW_CHANNEL.bits()
                    },
                    Ordering::SeqCst,
                );
                fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
                let interaction = help_component(&fixture, session, select, true);
                let action = dispatch_component(&fixture.context, &interaction, &data, false);
                tokio::pin!(action);
                tokio::select! {
                    permit = fixture.acknowledgement_entered.acquire() => permit.unwrap().forget(),
                    _ = &mut action => panic!("help finished before acknowledgement"),
                }
                // Poll the action while the fake server remains free to record any GET.
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {},
                    _ = &mut action => panic!("help finished with acknowledgement withheld"),
                }
                assert_eq!(fixture.requests.lock().unwrap().len(), 1);
                fixture.acknowledgement_release.add_permits(1);
                assert!(action.await);
                let requests = fixture.take_requests();
                assert_eq!(
                    requests
                        .iter()
                        .filter(|request| request.method == "GET")
                        .count(),
                    3
                );
                let reply = assert_private_help_response(&requests);
                assert_eq!(
                    private_help_body(reply).contains("`/admin show`"),
                    manager && section == HelpSection::Administration
                );
                let id = reply["components"][0]["components"][0]["custom_id"]
                    .as_str()
                    .unwrap();
                assert_eq!(
                    help_center::validate(id, ACTOR, runtime::now())
                        .unwrap()
                        .expiry,
                    session.expiry
                );
                assert_eq!(*runtime::AppState::lock(&data.state.stores), stores);
                assert_eq!(*runtime::AppState::lock(&data.state.rewards), rewards);
                assert_eq!(
                    format!("{:?}", *runtime::AppState::lock(&data.state.engine)),
                    engine
                );
                assert_eq!(format!("{:?}", voice.snapshot().await), voice_before);
                assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
            }
        }
    }
}

#[tokio::test]
async fn actual_help_rejects_bad_controls_before_lookup_and_stops_on_ack_failure() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let session = help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Start).unwrap();
    for select in [false, true] {
        for case in 0..9 {
            let mut interaction = help_component(&fixture, session, select, true);
            let expected = match case {
                0 => {
                    interaction.data.custom_id = session.custom_id().replace(":v1:", ":v2:");
                    help_center::STALE
                }
                1 => {
                    interaction.user.id = UserId::new(OTHER);
                    help_center::NOT_OWNER
                }
                2 => {
                    interaction.data.custom_id = help_center::HelpSession {
                        expiry: runtime::now(),
                        ..session
                    }
                    .custom_id();
                    help_center::EXPIRED
                }
                3 => {
                    interaction.message.author.id = UserId::new(OTHER);
                    help_center::STALE
                }
                4 => {
                    interaction.user.bot = true;
                    help_center::STALE
                }
                5 => {
                    interaction.data.custom_id.push_str(":extra");
                    help_center::STALE
                }
                6 => {
                    interaction.data.kind =
                        ComponentInteractionDataKind::StringSelect { values: vec![] };
                    help_center::STALE
                }
                7 => {
                    interaction.data.kind = ComponentInteractionDataKind::StringSelect {
                        values: vec!["unknown".into()],
                    };
                    help_center::STALE
                }
                _ => {
                    interaction.data.kind = ComponentInteractionDataKind::StringSelect {
                        values: vec!["start".into(), "memory".into()],
                    };
                    help_center::STALE
                }
            };
            assert!(dispatch_component(&fixture.context, &interaction, &data, false).await);
            let requests = fixture.take_requests();
            assert_eq!(requests.len(), 2);
            assert!(!requests.iter().any(|request| request.method == "GET"));
            let reply = assert_private_help_response(&requests);
            assert_eq!(private_help_body(reply), expected);
            assert_eq!(reply["components"], json!([]));
        }
        fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
        let interaction = help_component(&fixture, session, select, true);
        assert!(dispatch_component(&fixture.context, &interaction, &data, false).await);
        let requests = fixture.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].body["type"], 5);
        fixture.fail_acknowledgement.store(false, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn actual_help_dm_navigation_has_private_controls_and_dm_specific_visibility() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let session = help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Start).unwrap();
    for select in [false, true] {
        for section in [
            HelpSection::Start,
            HelpSection::Conversation,
            HelpSection::Memory,
            HelpSection::Images,
        ] {
            let interaction = help_component(&fixture, session.navigate(section), select, false);
            assert!(dispatch_component(&fixture.context, &interaction, &data, false).await);
            let requests = fixture.take_requests();
            assert_eq!(requests.len(), 2);
            let reply = assert_private_help_response(&requests);
            let body = private_help_body(reply);
            assert!(!body.contains("channel-visible") && !body.contains("member menu;"));
            if section == HelpSection::Conversation || section == HelpSection::Images {
                assert!(body.contains("reply in this DM"));
            }
            if section == HelpSection::Start {
                assert_eq!(reply["components"].as_array().unwrap().len(), 2);
                assert_eq!(
                    reply["components"][1]["components"]
                        .as_array()
                        .unwrap()
                        .len(),
                    3
                );
            }
        }
    }
}
