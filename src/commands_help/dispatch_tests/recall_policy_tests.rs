//! Actual registered recall inspection: acknowledge, REST, canonical scope,
//! private no-mention reply. No live Discord connection or credentials.
use super::*;

#[tokio::test]
async fn registered_recall_policy_show_defers_and_rechecks_fresh_permissions() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let access = crate::work::WorkAccess {
        actor: ACTOR,
        guild: Some(GUILD),
        channel: CHANNEL,
        can_view: true,
        can_manage: true,
    };
    runtime::AppState::lock(&data.state.stores)
        .work
        .create_project(access, "Private scope", "one")
        .unwrap();
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::WorkRecallShow);
    fixture
        .permissions
        .store(Permissions::VIEW_CHANNEL.bits(), Ordering::SeqCst);
    for visible in [true, false] {
        if !visible {
            fixture.permissions.store(0, Ordering::SeqCst);
        }
        let invocation = Invocation::new(command, true, None);
        let options = poise::FrameworkOptions::default();
        let context = invocation.context(
            &fixture,
            command,
            &options,
            &data,
            poise::CommandInteractionType::Command,
        );
        assert!(
            command.checks[0](poise::Context::Application(context))
                .await
                .unwrap()
        );
        let result = command.slash_action.unwrap()(context).await;
        assert_eq!(result.is_ok(), visible);
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert!(requests.iter().any(|r| r.method == "GET"));
        if visible {
            let body = assert_private_no_mentions_reply(&requests);
            println!("Actual recall show: {body}");
            assert!(body.contains("Scope opt-in: disabled"));
            assert!(body.contains("Operator rollout: unavailable"));
        } else {
            assert!(
                !requests.iter().any(|r| r.route.contains("/webhooks/")),
                "revocation must suppress inspection reply"
            );
        }
    }
}
