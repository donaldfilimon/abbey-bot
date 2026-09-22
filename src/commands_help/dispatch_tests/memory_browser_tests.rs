//! Memory browser components: envelope checks, permission refresh, page recomputation.
use super::*;

fn memory_component(
    fixture: &DiscordFixture,
    session: crate::memory_browser::MemorySession,
) -> ComponentInteraction {
    let helper = help_center::HelpSession::new(ACTOR, runtime::now(), HelpSection::Memory).unwrap();
    let in_guild = matches!(session.scope, crate::memory_browser::MemoryScope::Guild(_));
    let mut component = help_component(fixture, helper, false, in_guild);
    component.context = Some(if in_guild {
        serenity::all::InteractionContext::Guild
    } else {
        serenity::all::InteractionContext::BotDm
    });
    component.data.custom_id = session.custom_id();
    component
}

#[tokio::test]
async fn memory_browser_dispatch_refreshes_full_facts_permissions_and_preserves_state() {
    use crate::memory_browser::{MemoryScope, MemorySession};
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    fixture.permissions.store(
        (Permissions::VIEW_CHANNEL | Permissions::MANAGE_MESSAGES).bits(),
        Ordering::SeqCst,
    );
    let guild = format!("discord:{GUILD}");
    let subject = format!("discord:{OTHER}");
    let facts: Vec<_> = (0..7)
        .map(|n| format!("fact {n} {}", "界".repeat(290)))
        .collect();
    for fact in &facts {
        data.state
            .memory_service()
            .remember(&guild, &subject, fact, 1)
            .unwrap();
    }
    let session =
        MemorySession::new(ACTOR, OTHER, MemoryScope::Guild(GUILD), runtime::now()).unwrap();
    let stores = runtime::AppState::lock(&data.state.stores).clone();
    let rewards = runtime::AppState::lock(&data.state.rewards).clone();
    let engine = format!("{:?}", *runtime::AppState::lock(&data.state.engine));
    let voice = format!("{:?}", data.voice.as_ref().unwrap().snapshot().await);
    for index in [0, 1] {
        let component = memory_component(&fixture, session.navigate(index).unwrap());
        assert!(dispatch_component(&fixture.context, &component, &data, false).await);
        let requests = fixture.take_requests();
        let body = assert_private_content_response(&requests);
        for fact in &facts[usize::from(index) * 4..(usize::from(index) * 4 + 4).min(facts.len())] {
            assert!(body["content"].as_str().unwrap().contains(fact));
        }
        assert!(requests.iter().any(|request| request.method == "GET"));
        let encoded = body["components"][0]["components"][0]["custom_id"]
            .as_str()
            .unwrap();
        let next = crate::memory_browser::validate(
            encoded,
            ACTOR,
            &MemoryScope::Guild(GUILD),
            runtime::now(),
        )
        .unwrap();
        assert_eq!(next.expiry, session.expiry);
    }
    fixture
        .permissions
        .store(Permissions::VIEW_CHANNEL.bits(), Ordering::SeqCst);
    let component = memory_component(&fixture, session.navigate(1).unwrap());
    dispatch_component(&fixture.context, &component, &data, false).await;
    let requests = fixture.take_requests();
    let body = assert_private_content_response(&requests);
    assert!(
        body["content"]
            .as_str()
            .unwrap()
            .contains("only while Discord grants")
    );
    assert!(!body["content"].as_str().unwrap().contains(&facts[4]));
    assert_eq!(*runtime::AppState::lock(&data.state.stores), stores);
    assert_eq!(*runtime::AppState::lock(&data.state.rewards), rewards);
    assert_eq!(
        format!("{:?}", *runtime::AppState::lock(&data.state.engine)),
        engine
    );
    assert_eq!(
        format!("{:?}", data.voice.as_ref().unwrap().snapshot().await),
        voice
    );
}

#[tokio::test]
async fn memory_browser_envelope_and_bot_dm_scope_fail_before_permission_lookup() {
    use crate::memory_browser::{MemoryScope, MemorySession};
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let session = MemorySession::new(ACTOR, ACTOR, MemoryScope::BotDm, runtime::now()).unwrap();
    data.state
        .memory_service()
        .remember(
            &format!("discord:dm:{ACTOR}"),
            &format!("discord:{ACTOR}"),
            "private DM fact",
            1,
        )
        .unwrap();
    let good = memory_component(&fixture, session);
    dispatch_component(&fixture.context, &good, &data, false).await;
    let requests = fixture.take_requests();
    assert!(
        assert_private_content_response(&requests)["content"]
            .as_str()
            .unwrap()
            .contains("private DM fact")
    );
    assert!(!requests.iter().any(|request| request.method == "GET"));
    for failure in 0..5 {
        let mut component = memory_component(&fixture, session);
        match failure {
            0 => component.context = None,
            1 => component.user.id = UserId::new(OTHER),
            2 => component.message.author = user(OTHER),
            3 => component.data.custom_id = "abbey:mem:v9:invalid".into(),
            _ => {
                component.data.custom_id = MemorySession {
                    expiry: runtime::now(),
                    ..session
                }
                .custom_id()
            }
        }
        dispatch_component(&fixture.context, &component, &data, false).await;
        let requests = fixture.take_requests();
        let body = assert_private_content_response(&requests);
        assert!(
            !body["content"]
                .as_str()
                .unwrap()
                .contains("private DM fact")
        );
        assert!(!requests.iter().any(|request| request.method == "GET"));
    }
}

#[tokio::test]
async fn memory_browser_acknowledgement_holds_all_permission_requests() {
    use crate::memory_browser::{MemoryScope, MemorySession};
    let fixture = DiscordFixture::new().await;
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let data = configured_data();
    let session =
        MemorySession::new(ACTOR, ACTOR, MemoryScope::Guild(GUILD), runtime::now()).unwrap();
    let component = memory_component(&fixture, session);
    let action = dispatch_component(&fixture.context, &component, &data, false);
    tokio::pin!(action);
    tokio::select! {
        permit = fixture.acknowledgement_entered.acquire() => permit.unwrap().forget(),
        _ = &mut action => panic!("browser completed before held acknowledgement"),
    }
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "GET")
    );
    fixture.acknowledgement_release.add_permits(1);
    assert!(action.await);
    let requests = fixture.take_requests();
    assert_private_content_response(&requests);
    assert!(requests.iter().any(|request| request.method == "GET"));
}

#[tokio::test]
async fn memory_browser_recomputes_pages_after_facts_are_removed_elsewhere() {
    use crate::memory_browser::{MemoryScope, MemorySession};
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let guild = format!("discord:{GUILD}");
    let actor = format!("discord:{ACTOR}");
    for index in 0..5 {
        data.state
            .memory_service()
            .remember(&guild, &actor, &format!("fact {index}"), 1)
            .unwrap();
    }
    let session = MemorySession::new(ACTOR, ACTOR, MemoryScope::Guild(GUILD), runtime::now())
        .unwrap()
        .navigate(1)
        .unwrap();
    let component = memory_component(&fixture, session);
    dispatch_component(&fixture.context, &component, &data, false).await;
    let requests = fixture.take_requests();
    assert!(
        assert_private_content_response(&requests)["content"]
            .as_str()
            .unwrap()
            .contains("Page 2 of 2")
    );
    for index in 1..5 {
        assert!(
            data.state
                .memory_service()
                .forget(&guild, &actor, &format!("fact {index}"))
        );
    }
    dispatch_component(&fixture.context, &component, &data, false).await;
    let requests = fixture.take_requests();
    let body = assert_private_content_response(&requests);
    assert!(body["content"].as_str().unwrap().contains("Page 1 of 1"));
    assert!(body["content"].as_str().unwrap().contains("fact 0"));
    assert_eq!(body["components"], json!([]));
}
