//! Actual recursive native Poise dispatch with private, current REST authority.
use super::*;
use crate::work::{WorkAccess, WorkContentRef, WorkScope, WorkStatus, WorkTask};
use serenity::all::{CommandType, InteractionContext as NativeContext, UserUpdateEvent};
use std::{collections::BTreeSet, path::PathBuf};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "abbey-continuity-native-dispatch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn native_data(fixture: &DiscordFixture, directory: Option<&Directory>) -> Data {
    let mut bot = user(321);
    bot.bot = true;
    let mut event: UserUpdateEvent =
        serde_json::from_value(serde_json::to_value(bot).unwrap()).unwrap();
    fixture.context.cache.update(&mut event);
    assert_eq!(fixture.context.cache.current_user().id.get(), 321);

    let mut data = configured_data();
    if let Some(directory) = directory {
        Arc::get_mut(&mut data.state).unwrap().data_dir = Some(directory.0.clone());
    }
    data.state
        .attach_continuity_access(Arc::new(
            crate::gateway::continuity_access::DiscordContinuityAccess(
                fixture.context.http.clone(),
            ),
        ))
        .unwrap();
    data
}

fn options() -> poise::FrameworkOptions<Data, Error> {
    poise::FrameworkOptions {
        commands: crate::application_commands(),
        ..Default::default()
    }
}

fn interaction(leaf: &str, arguments: Value) -> CommandInteraction {
    static NEXT: AtomicU64 = AtomicU64::new(1_100);
    serde_json::from_value(json!({
        "id": NEXT.fetch_add(1, Ordering::Relaxed).to_string(),
        "application_id": "321",
        "data": {
            "id": "222", "name": "work", "type": 1,
            "options": [{
                "name": "continuity", "type": 2,
                "options": [{"name": leaf, "type": 1, "options": arguments}]
            }]
        },
        "guild_id": GUILD.to_string(), "channel_id": CHANNEL.to_string(),
        "user": user(ACTOR), "token": "offline-interaction", "version": 1,
        "locale": "en-US", "context": 0, "entitlements": [],
        "attachment_size_limit": 1024
    }))
    .unwrap()
}

async fn dispatch_native(
    fixture: &DiscordFixture,
    data: &Data,
    options: &poise::FrameworkOptions<Data, Error>,
    interaction: &CommandInteraction,
) -> Result<(), String> {
    let sent = AtomicBool::new(false);
    let invocation_data = tokio::sync::Mutex::new(Box::new(()) as _);
    let resolved = interaction.data.options();
    let mut parents = Vec::new();
    let result = poise::dispatch::dispatch_interaction(
        poise::FrameworkContext {
            bot_id: UserId::new(321),
            options,
            user_data: data,
            shard_manager: &fixture.manager,
        },
        &fixture.context,
        interaction,
        &sent,
        &invocation_data,
        &resolved,
        &mut parents,
    )
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(poise::FrameworkError::Command { error, .. }) => Err(error.to_string()),
        Err(poise::FrameworkError::CommandCheckFailed {
            error: Some(error), ..
        }) => Err(error.to_string()),
        // Do not format complete contexts: they contain continuation tokens.
        Err(_) => Err("The native framework refused this interaction.".into()),
    }
}

fn access() -> WorkAccess {
    WorkAccess {
        actor: ACTOR,
        guild: Some(GUILD),
        channel: CHANNEL,
        can_view: true,
        can_manage: true,
    }
}
fn scope() -> WorkScope {
    access().scope()
}
fn seed_project(data: &Data, request: &str) -> u64 {
    runtime::AppState::lock(&data.state.stores)
        .work
        .create_project(access(), "Native private Work", request)
        .unwrap()
}
fn seed_task(data: &Data, project: u64) -> u64 {
    runtime::AppState::lock(&data.state.stores)
        .work
        .add_task(
            access(),
            WorkTask {
                id: 0,
                project_id: project,
                title: "Resume allocator qualification".into(),
                owner: ACTOR,
                assignee: None,
                goal_id: None,
                priority: 1,
                status: WorkStatus::Open,
                due_at: None,
                remind_at: None,
                reminder_revision: 0,
                snoozed_until: None,
                source: None,
                github: None,
                revision: 0,
            },
            "native-task",
        )
        .unwrap()
}
fn card(data: &Data) -> Option<crate::work::continuity::ContinuityCard> {
    runtime::AppState::lock(&data.state.stores)
        .continuity
        .card(&scope())
        .cloned()
}
fn preview_args(text: &str, sources: Option<&str>) -> Value {
    let mut arguments = vec![json!({"name": "text", "type": 3, "value": text})];
    if let Some(sources) = sources {
        arguments.push(json!({"name": "sources", "type": 3, "value": sources}));
    }
    Value::Array(arguments)
}
fn confirm_args(id: &str) -> Value {
    json!([{"name": "proposal", "type": 3, "value": id}])
}
fn preview_id(requests: &[Request]) -> String {
    let preview = requests
        .iter()
        .filter(|request| request.route.contains("/webhooks/"))
        .find(|request| request.body["embeds"][0]["title"] == "Continuity preview")
        .expect("actual native preview request");
    let field = preview.body["embeds"][0]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == "Proposal ID")
        .unwrap();
    field["value"]
        .as_str()
        .unwrap()
        .split('`')
        .nth(1)
        .unwrap()
        .into()
}
fn private_embed(requests: &[Request], title: &str) -> Value {
    let response = requests
        .iter()
        .filter(|request| request.route.contains("/webhooks/"))
        .find(|request| request.body["embeds"][0]["title"] == title)
        .expect("actual private embed");
    assert_eq!(response.body["allowed_mentions"]["parse"], json!([]));
    response.body["embeds"][0].clone()
}
fn assert_actual_defer(
    requests: &[Request],
    options: &poise::FrameworkOptions<Data, Error>,
    key: CommandKey,
) {
    assert_deferred_first(requests, command_by_key(&options.commands, key));
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.route.ends_with("/callback"))
            .count(),
        1,
        "one actual acknowledgement per private invocation"
    );
}

#[tokio::test]
async fn continuity_native_dispatch_private_cycle_preserves_exact_text_and_excludes_model() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, Some(&directory));
    let project = seed_project(&data, "cycle-project");
    let task = seed_task(&data, project);
    let decision = runtime::AppState::lock(&data.state.stores)
        .work
        .record_decision(
            project,
            access(),
            "Use checked allocation",
            runtime::now(),
            "native-decision",
        )
        .unwrap();
    // REST grants only VIEW_CHANNEL: the native manager is delegated via Work.
    fixture
        .permissions
        .store(Permissions::VIEW_CHANNEL.bits(), Ordering::SeqCst);
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    let options = options();
    let text = "Resume *allocator* 🦀\nKeep exact text.";
    let sources = format!("task:{task},decision:{decision}");
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("propose", preview_args(text, Some(&sources))),
    )
    .await
    .unwrap();
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityPropose);
    assert!(requests.iter().any(|request| request.method == "GET"));
    let embed = private_embed(&requests, "Continuity preview");
    assert_eq!(
        embed["description"],
        "Resume \\*allocator\\* 🦀\nKeep exact text."
    );
    println!("Actual private preview: {embed}");
    let id = preview_id(&requests);
    assert!(card(&data).is_none(), "preview must not persist a card");

    let mut host = runtime::ToolScope {
        memory_turn: None,
        state: &data.state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: format!("discord:{GUILD}"),
        scoped_user: format!("discord:{ACTOR}"),
        scoped_channel: format!("discord:{CHANNEL}"),
        now: runtime::now(),
        persona: crate::persona::Persona::Abbey,
    };
    for name in [
        "confirm_continuity",
        "continuity_confirm",
        "work continuity confirm",
    ] {
        assert!(
            crate::tools::production_tools()
                .iter()
                .all(|tool| tool.name != name)
        );
        let result = crate::tools::dispatch(
            &crate::tools::ToolCall {
                id: "native-proposal-model-attempt".into(),
                name: name.into(),
                arguments: json!({"proposal": id, "text": "replace exact text", "generation": 1}),
            },
            &mut host,
        );
        assert!(result.content.starts_with("Unknown tool"));
    }
    assert!(card(&data).is_none());
    assert!(fixture.take_requests().is_empty());

    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("confirm", confirm_args(&id)),
    )
    .await
    .unwrap();
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityConfirm);
    println!(
        "Actual confirm: {}",
        assert_private_no_mentions_reply(&requests)
    );
    let stored = card(&data).unwrap();
    assert_eq!(stored.confirmed_text, text);
    assert_eq!(stored.confirmed_by, ACTOR);
    assert_eq!(
        stored.source_refs,
        BTreeSet::from([
            WorkContentRef::Task {
                project,
                id: task,
                revision: 0
            },
            WorkContentRef::Decision {
                project,
                id: decision,
                revision: 1
            },
        ])
    );
    assert_eq!(
        crate::persist::Stores::load(&directory.0)
            .unwrap()
            .continuity
            .card(&scope()),
        Some(&stored)
    );

    dispatch_native(&fixture, &data, &options, &interaction("show", json!([])))
        .await
        .unwrap();
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityShow);
    assert_eq!(
        private_embed(&requests, "Confirmed continuity")["description"],
        embed["description"]
    );
    println!(
        "Actual show: {}",
        private_embed(&requests, "Confirmed continuity")
    );

    assert!(
        dispatch_native(
            &fixture,
            &data,
            &options,
            &interaction("confirm", confirm_args(&id))
        )
        .await
        .is_err()
    );
    assert_eq!(card(&data), Some(stored));
    fixture.take_requests();
    dispatch_native(&fixture, &data, &options, &interaction("clear", json!([])))
        .await
        .unwrap();
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityClear);
    println!(
        "Actual clear: {}",
        assert_private_no_mentions_reply(&requests)
    );
    assert!(card(&data).is_none());
    let disk = crate::persist::Stores::load(&directory.0).unwrap();
    assert!(disk.continuity.card(&scope()).is_none());
    assert!(disk.work.projects.contains_key(&project));
    assert!(disk.work.tasks.contains_key(&task));
    assert!(disk.work.decisions.contains_key(&decision));
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn continuity_native_dispatch_waits_for_ack_and_failed_ack_does_no_private_work() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, Some(&directory));
    seed_project(&data, "ack-project");
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    let options = options();
    let press = interaction(
        "propose",
        preview_args("Wait for observed acknowledgement", None),
    );
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let action = dispatch_native(&fixture, &data, &options, &press);
    tokio::pin!(action);
    tokio::select! {
        result = &mut action => panic!("native work finished before ack release: {result:?}"),
        permit = tokio::time::timeout(std::time::Duration::from_secs(2), fixture.acknowledgement_entered.acquire()) => {
            permit.unwrap().unwrap().forget();
        }
    }
    let mut requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityPropose);
    assert_eq!(
        requests.len(),
        1,
        "no REST or preview before acknowledged defer"
    );
    assert!(card(&data).is_none());
    fixture.hold_acknowledgement.store(false, Ordering::SeqCst);
    fixture.acknowledgement_release.add_permits(1);
    action.await.unwrap();
    requests.extend(fixture.take_requests());
    let id = preview_id(&requests);
    let before = std::fs::read(directory.0.join("abbey-state.json")).unwrap();
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    assert!(
        dispatch_native(
            &fixture,
            &data,
            &options,
            &interaction("confirm", confirm_args(&id))
        )
        .await
        .is_err()
    );
    let failed = fixture.take_requests();
    assert_deferred_first(
        &failed,
        command_by_key(&options.commands, CommandKey::WorkContinuityConfirm),
    );
    // The existing catalog guard also attempts a private content-free error
    // callback after the refused defer. Neither attempt admits the leaf body.
    assert_eq!(failed.len(), 2);
    assert!(
        failed
            .iter()
            .all(|r| r.method == "POST" && r.route.ends_with("/callback"))
    );
    assert_eq!(failed[1].body["type"], 4);
    assert_ne!(
        failed[1].body["data"]["flags"].as_u64().unwrap_or(0) & 64,
        0
    );
    assert_eq!(
        failed[1].body["data"]["content"],
        "Discord could not confirm the current permissions. Please try again."
    );
    assert!(card(&data).is_none());
    assert_eq!(
        std::fs::read(directory.0.join("abbey-state.json")).unwrap(),
        before
    );
    fixture.fail_acknowledgement.store(false, Ordering::SeqCst);
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("confirm", confirm_args(&id)),
    )
    .await
    .unwrap();
    assert_eq!(
        card(&data).unwrap().confirmed_text,
        "Wait for observed acknowledgement"
    );
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn continuity_native_dispatch_failed_preview_discards_its_exact_id() {
    let fixture = DiscordFixture::new().await;
    let data = native_data(&fixture, None);
    seed_project(&data, "failed-preview-project");
    let options = options();
    fixture.fail_next_followup.store(true, Ordering::SeqCst);
    assert!(
        dispatch_native(
            &fixture,
            &data,
            &options,
            &interaction(
                "propose",
                preview_args("Unseen text is never confirmable", None)
            )
        )
        .await
        .is_err()
    );
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityPropose);
    // Test-only wire observation sees the attempted body even though delivery failed.
    let failed_id = preview_id(&requests);
    assert!(
        dispatch_native(
            &fixture,
            &data,
            &options,
            &interaction("confirm", confirm_args(&failed_id))
        )
        .await
        .is_err()
    );
    assert!(card(&data).is_none());
    assert!(
        !fixture
            .take_requests()
            .iter()
            .any(|request| request.route.contains("/webhooks/"))
    );
}

#[tokio::test]
async fn continuity_native_dispatch_rechecks_every_project_manager_before_resolving() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, Some(&directory));
    seed_project(&data, "manager-first-project");
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    let options = options();
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("propose", preview_args("All projects must authorize", None)),
    )
    .await
    .unwrap();
    let id = preview_id(&fixture.take_requests());
    // Add a second same-scope project after preview and revoke manager membership
    // there. Discord administrator permission must not substitute for this grant.
    let second = seed_project(&data, "manager-second-project");
    runtime::AppState::lock(&data.state.stores)
        .work
        .projects
        .get_mut(&second)
        .unwrap()
        .managers
        .remove(&ACTOR);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    assert!(
        dispatch_native(
            &fixture,
            &data,
            &options,
            &interaction("confirm", confirm_args(&id))
        )
        .await
        .is_err()
    );
    assert!(card(&data).is_none());
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityConfirm);
    assert!(requests.iter().any(|request| request.method == "GET"));
    assert!(
        !requests
            .iter()
            .any(|request| request.route.contains("/webhooks/"))
    );
    runtime::AppState::lock(&data.state.stores)
        .work
        .projects
        .get_mut(&second)
        .unwrap()
        .managers
        .insert(ACTOR);
    fixture
        .permissions
        .store(Permissions::VIEW_CHANNEL.bits(), Ordering::SeqCst);
    // Same ID succeeds: the refused attempt did not consume the human's control.
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("confirm", confirm_args(&id)),
    )
    .await
    .unwrap();
    let before = card(&data).unwrap();
    fixture.take_requests();
    runtime::AppState::lock(&data.state.stores)
        .work
        .projects
        .get_mut(&second)
        .unwrap()
        .managers
        .remove(&ACTOR);
    assert!(
        dispatch_native(&fixture, &data, &options, &interaction("clear", json!([])))
            .await
            .is_err()
    );
    assert_eq!(card(&data), Some(before));
    runtime::AppState::lock(&data.state.stores)
        .work
        .projects
        .get_mut(&second)
        .unwrap()
        .managers
        .insert(ACTOR);
    dispatch_native(&fixture, &data, &options, &interaction("clear", json!([])))
        .await
        .unwrap();
    assert!(card(&data).is_none());
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn continuity_native_dispatch_changed_selected_task_is_not_published() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, Some(&directory));
    let project = seed_project(&data, "source-project");
    let task = seed_task(&data, project);
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    let options = options();
    let sources = format!("task:{task}");
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction(
            "propose",
            preview_args("Keep native source revision", Some(&sources)),
        ),
    )
    .await
    .unwrap();
    let id = preview_id(&fixture.take_requests());
    runtime::AppState::lock(&data.state.stores)
        .work
        .update_task(access(), task, 0, WorkStatus::Done, None)
        .unwrap();
    assert!(
        dispatch_native(
            &fixture,
            &data,
            &options,
            &interaction("confirm", confirm_args(&id))
        )
        .await
        .is_err()
    );
    assert!(card(&data).is_none());
    assert!(
        crate::persist::Stores::load(&directory.0)
            .unwrap()
            .continuity
            .card(&scope())
            .is_none()
    );
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn continuity_native_dispatch_rejects_bot_wrong_application_and_non_native_context() {
    let fixture = DiscordFixture::new().await;
    let data = native_data(&fixture, None);
    seed_project(&data, "native-envelope-project");
    let options = options();
    for case in 0..4 {
        let mut press = interaction("propose", preview_args("Denied private text", None));
        match case {
            0 => press.user.bot = true,
            1 => press.application_id = ApplicationId::new(322),
            2 => press.context = Some(NativeContext::PrivateChannel),
            _ => press.data.kind = CommandType::User,
        }
        assert!(
            dispatch_native(&fixture, &data, &options, &press)
                .await
                .is_err()
        );
        let requests = fixture.take_requests();
        assert_actual_defer(&requests, &options, CommandKey::WorkContinuityPropose);
        assert_eq!(
            requests.len(),
            1,
            "invalid native envelope must do no private REST/read/reply"
        );
        assert!(card(&data).is_none());
    }
}

#[tokio::test]
async fn continuity_native_dispatch_max_preview_is_intact_and_revoked_show_is_content_free() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, Some(&directory));
    let project = seed_project(&data, "max-preview-project");
    let sources: Vec<_> = (0..8)
        .map(|n| {
            let id = runtime::AppState::lock(&data.state.stores)
                .work
                .record_decision(
                    project,
                    access(),
                    "Checked source",
                    runtime::now(),
                    &format!("max-decision-{n}"),
                )
                .unwrap();
            format!("decision:{id}")
        })
        .collect();
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    let options = options();
    let text = "*".repeat(1600);
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("propose", preview_args(&text, Some(&sources.join(",")))),
    )
    .await
    .unwrap();
    let requests = fixture.take_requests();
    let embed = private_embed(&requests, "Continuity preview");
    assert_eq!(embed["description"], "\\*".repeat(1600));
    assert_eq!(embed["description"].as_str().unwrap().chars().count(), 3200);
    let fields = embed["fields"].as_array().unwrap();
    let source_field = fields
        .iter()
        .find(|field| field["name"] == "Source references")
        .unwrap();
    assert_eq!(source_field["value"].as_str().unwrap().lines().count(), 8);
    assert!(source_field["value"].as_str().unwrap().chars().count() <= 1024);
    let total = embed["title"].as_str().unwrap().chars().count()
        + embed["description"].as_str().unwrap().chars().count()
        + fields
            .iter()
            .map(|field| {
                field["name"].as_str().unwrap().chars().count()
                    + field["value"].as_str().unwrap().chars().count()
            })
            .sum::<usize>();
    assert!(total <= 6000);
    let id = preview_id(&requests);
    dispatch_native(
        &fixture,
        &data,
        &options,
        &interaction("confirm", confirm_args(&id)),
    )
    .await
    .unwrap();
    let before = card(&data).unwrap();
    assert_eq!(before.confirmed_text, text);
    fixture.take_requests();
    fixture.permissions.store(0, Ordering::SeqCst);
    dispatch_native(&fixture, &data, &options, &interaction("show", json!([])))
        .await
        .unwrap();
    let requests = fixture.take_requests();
    assert_actual_defer(&requests, &options, CommandKey::WorkContinuityShow);
    assert!(requests.iter().any(|request| request.method == "GET"));
    let content = assert_private_no_mentions_reply(&requests);
    println!("Actual revoked show: {content}");
    assert_eq!(content, "No confirmed continuity card is available here.");
    assert!(!requests.iter().any(|request| {
        request
            .body
            .get("embeds")
            .is_some_and(|v| !v.as_array().is_some_and(Vec::is_empty))
    }));
    assert_eq!(card(&data), Some(before));
    writer.stop();
    writer.joined().await.unwrap();
}
