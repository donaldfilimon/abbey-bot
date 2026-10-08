// Apply as src/commands_help/dispatch_tests/native_follow_up_tests.rs and add
// mod native_follow_up_tests; to dispatch_tests.rs. Requires the optional exact
// REST route override map from native-dispatch-fixture.patch (test fixture only).
use super::*;
use crate::{
    engagement::{DestinationPreference, EngagementScope, MemberPolicy, SourceRef},
    persist::Stores,
    service::{ReapOutcome, ServiceSupervisor, ShutdownReason, persistence::PersistenceWriter},
    work::{WorkAccess, WorkContentRef, WorkStatus, WorkTask},
};
use serenity::all::{
    InteractionContext as NativeContext, MessageId, MessageReference, MessageReferenceKind,
    Timestamp, UserUpdateEvent,
};
use std::path::PathBuf;

const SOURCE_MESSAGE: u64 = 1_234_567_890_123_456_789;
const RESPONSE_MESSAGE: u64 = 1_234_567_890_123_456_790;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "abbey-task-follow-up-native-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn native_data(fixture: &DiscordFixture, directory: &Directory) -> Data {
    let mut bot = user(321);
    bot.bot = true;
    let mut event: UserUpdateEvent =
        serde_json::from_value(serde_json::to_value(bot).unwrap()).unwrap();
    fixture.context.cache.update(&mut event);
    let mut data = configured_data_at(Some(fixture.address));
    Arc::get_mut(&mut data.state).unwrap().data_dir = Some(directory.0.clone());
    data
}
fn scope() -> EngagementScope {
    EngagementScope::Guild {
        guild: GUILD,
        channel: CHANNEL,
    }
}
fn native_access() -> WorkAccess {
    WorkAccess {
        actor: ACTOR,
        guild: Some(GUILD),
        channel: CHANNEL,
        can_view: true,
        can_manage: true,
    }
}
fn seed(fixture: &DiscordFixture, data: &Data) -> (u64, WorkTask, SourceRef) {
    let at = runtime::now();
    let source = SourceRef {
        scope: scope(),
        message: SOURCE_MESSAGE,
        author: ACTOR,
        revision: 1,
        at: at - 86_400,
    };
    let (task, original) = {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        let w = &mut stores.work;
        let project = w
            .create_project(
                native_access(),
                "Native exact follow-up project",
                "native-follow-up-project",
            )
            .unwrap();
        let task = w
            .add_task(
                native_access(),
                WorkTask {
                    id: 0,
                    project_id: project,
                    title: "Native task must remain unchanged".into(),
                    owner: ACTOR,
                    assignee: None,
                    goal_id: None,
                    priority: 0,
                    status: WorkStatus::Open,
                    due_at: None,
                    remind_at: None,
                    reminder_revision: 0,
                    snoozed_until: None,
                    source: None,
                    github: None,
                    revision: 0,
                },
                "native-follow-up-task",
            )
            .unwrap();
        let e = &mut w.engagement;
        let mut policy = MemberPolicy {
            revision: 1,
            daily_limit: Some(1),
            timezone: Some("UTC".into()),
            quiet_start: 0,
            quiet_end: 0,
            ..Default::default()
        };
        policy
            .destinations
            .insert(scope(), DestinationPreference::Origin);
        e.member_policies.insert(ACTOR, policy);
        e.eligibility
            .entry(ACTOR)
            .or_default()
            .insert(source.clone());
        e.observations
            .entry(scope())
            .or_default()
            .insert(ACTOR, source.clone());
        e.responses.insert(SOURCE_MESSAGE, RESPONSE_MESSAGE);
        e.validate().unwrap();
        (task, w.tasks[&task].clone())
    };
    let mut native_guild = guild(
        Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY | Permissions::SEND_MESSAGES,
    );
    // The owner always sees the channel: it belongs to the exact Work audience.
    native_guild.owner_id = UserId::new(ACTOR);
    let mut native_channel = GuildChannel::default();
    native_channel.id = ChannelId::new(CHANNEL);
    native_channel.guild_id = GuildId::new(GUILD);
    native_channel.kind = serenity::all::ChannelType::Text;
    let mut actor = Member::default();
    actor.user = user(ACTOR);
    actor.guild_id = GuildId::new(GUILD);
    let mut bot = Member::default();
    bot.user = user(321);
    bot.user.bot = true;
    bot.guild_id = GuildId::new(GUILD);
    let mut message = Message::default();
    message.id = MessageId::new(SOURCE_MESSAGE);
    message.channel_id = ChannelId::new(CHANNEL);
    message.guild_id = Some(GuildId::new(GUILD));
    message.author = user(ACTOR);
    message.timestamp = Timestamp::from_unix_timestamp(source.at as i64).unwrap();
    message.content = "Synthetic human exchange text is transient".into();
    let mut response = Message::default();
    response.id = MessageId::new(RESPONSE_MESSAGE);
    response.channel_id = ChannelId::new(CHANNEL);
    response.guild_id = Some(GuildId::new(GUILD));
    response.author = bot.user.clone();
    response.timestamp = Timestamp::from_unix_timestamp((source.at + 1) as i64).unwrap();
    response.content = "Synthetic existing delivered response is transient".into();
    response.message_reference = Some(
        MessageReference::new(MessageReferenceKind::default(), ChannelId::new(CHANNEL))
            .message_id(MessageId::new(SOURCE_MESSAGE))
            .guild_id(GuildId::new(GUILD)),
    );
    let mut routes = fixture.native_responses.lock().unwrap();
    for (path, value) in [
        (
            format!("/guilds/{GUILD}"),
            serde_json::to_value(native_guild).unwrap(),
        ),
        (
            format!("/channels/{CHANNEL}"),
            serde_json::to_value(native_channel).unwrap(),
        ),
        (
            format!("/guilds/{GUILD}/members/{ACTOR}"),
            serde_json::to_value(&actor).unwrap(),
        ),
        (
            format!("/guilds/{GUILD}/members/321"),
            serde_json::to_value(&bot).unwrap(),
        ),
        (format!("/guilds/{GUILD}/members"), json!([actor, bot])),
        (
            "/users/@me".into(),
            serde_json::to_value(user_bot()).unwrap(),
        ),
        (
            format!("/channels/{CHANNEL}/messages/{SOURCE_MESSAGE}"),
            serde_json::to_value(message).unwrap(),
        ),
        (
            format!("/channels/{CHANNEL}/messages/{RESPONSE_MESSAGE}"),
            serde_json::to_value(&response).unwrap(),
        ),
        (format!("/channels/{CHANNEL}/messages"), json!([response])),
    ] {
        routes.insert(path, value);
    }
    (task, original, source)
}
fn user_bot() -> User {
    let mut bot = user(321);
    bot.bot = true;
    bot
}
fn options() -> poise::FrameworkOptions<Data, Error> {
    poise::FrameworkOptions {
        commands: crate::application_commands(),
        ..Default::default()
    }
}
fn interaction(task: u64) -> CommandInteraction {
    static NEXT: AtomicU64 = AtomicU64::new(4_000);
    serde_json::from_value(json!({
        "id": NEXT.fetch_add(1, Ordering::Relaxed).to_string(), "application_id": "321",
        "data": {"id": "222", "name": "engage", "type": 1, "options": [{"name": "follow_up", "type": 1, "options": [
            {"name": "task", "type": 4, "value": task}, {"name": "revision", "type": 4, "value": 0},
            {"name": "source_message", "type": 3, "value": SOURCE_MESSAGE.to_string()}, {"name": "expiry_seconds", "type": 4, "value": 3600}
        ]}]},
        "guild_id": GUILD.to_string(), "channel_id": CHANNEL.to_string(), "user": user(ACTOR),
        "token": "offline-interaction", "version": 1, "locale": "en-US", "context": 0,
        "entitlements": [], "attachment_size_limit": 1024
    })).unwrap()
}
async fn dispatch(
    fixture: &DiscordFixture,
    data: &Data,
    options: &poise::FrameworkOptions<Data, Error>,
    press: &CommandInteraction,
) -> Result<(), String> {
    let sent = AtomicBool::new(false);
    let invocation_data = tokio::sync::Mutex::new(Box::new(()) as _);
    let resolved = press.data.options();
    let mut parents = Vec::new();
    let result = poise::dispatch::dispatch_interaction(
        poise::FrameworkContext {
            bot_id: UserId::new(321),
            options,
            user_data: data,
            shard_manager: &fixture.manager,
        },
        &fixture.context,
        press,
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
        Err(_) => Err("The native framework refused this interaction.".into()),
    }
}
fn assert_private_receipt<'a>(
    requests: &'a [Request],
    options: &poise::FrameworkOptions<Data, Error>,
) -> &'a str {
    assert_deferred_first(
        requests,
        command_by_key(&options.commands, CommandKey::EngageFollowUp),
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.route.ends_with("/callback"))
            .count(),
        1
    );
    assert_eq!(requests[0].body["data"]["flags"].as_u64().unwrap() & 64, 64);
    let body = assert_private_no_mentions_reply(requests);
    assert!(body.chars().count() <= 2_000);
    assert!(
        !requests
            .iter()
            .any(|r| r.route.contains("/chat/completions")
                || (r.method == "POST"
                    && r.route.contains("/channels/")
                    && r.route.ends_with("/messages")))
    );
    body
}
async fn finish(mut supervisor: ServiceSupervisor, mut writer: PersistenceWriter) {
    let shutdown = supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    let report = supervisor
        .cancel_and_reap(shutdown.budget.stage(tokio::time::Instant::now()))
        .await;
    assert_eq!(report.outcome, ReapOutcome::Joined);
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn engage_follow_up_native_dispatch_saves_exact_metadata_after_private_ack_and_fresh_rest() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, &directory);
    let (task, original, source) = seed(&fixture, &data);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    fixture.take_requests();
    let options = options();
    dispatch(&fixture, &data, &options, &interaction(task))
        .await
        .unwrap();
    let requests = fixture.take_requests();
    let body = assert_private_receipt(&requests, &options);
    println!("Actual native private task follow-up receipt: {body}");
    assert!(body.contains("Saved task follow-up candidate 1:"));
    assert!(body.contains("No message was sent by this request."));
    assert!(
        requests
            .iter()
            .filter(|r| r.method == "GET" && r.route.ends_with("/users/@me"))
            .count()
            >= 3
    );
    assert!(
        requests
            .iter()
            .any(|r| r.route.ends_with(&format!("/messages/{SOURCE_MESSAGE}")))
    );
    assert!(
        requests
            .iter()
            .any(|r| r.route.ends_with(&format!("/messages/{RESPONSE_MESSAGE}")))
    );
    let disk = Stores::load(&directory.0).unwrap();
    let candidate = &disk.work.engagement.candidates[&1];
    assert_eq!(candidate.source, Some(source));
    assert_eq!(
        candidate.work_ref,
        Some(WorkContentRef::Task {
            project: original.project_id,
            id: task,
            revision: 0
        })
    );
    assert_eq!(disk.work.tasks[&task], original);
    assert!(disk.work.engagement.charges.is_empty());
    let bytes = std::fs::read_to_string(Stores::state_path(&directory.0)).unwrap();
    assert!(!bytes.contains("Synthetic human exchange text is transient"));
    assert!(!bytes.contains("Synthetic existing delivered response is transient"));
    dispatch(&fixture, &data, &options, &interaction(task))
        .await
        .unwrap();
    let repeated = fixture.take_requests();
    assert!(assert_private_receipt(&repeated, &options).contains("Task follow-up was not saved."));
    assert!(!repeated.iter().any(
        |r| r.route.ends_with(&format!("/messages/{SOURCE_MESSAGE}"))
            || r.route.ends_with(&format!("/messages/{RESPONSE_MESSAGE}"))
    ));
    assert_eq!(
        Stores::load(&directory.0)
            .unwrap()
            .work
            .engagement
            .candidates
            .len(),
        1
    );
    finish(supervisor, writer).await;
}

#[tokio::test]
async fn engage_follow_up_native_dispatch_waits_for_observed_ack_and_failed_ack_does_no_work() {
    let fixture = DiscordFixture::new().await;
    let directory = Directory::new();
    let data = native_data(&fixture, &directory);
    let (task, _, _) = seed(&fixture, &data);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    fixture.take_requests();
    let options = options();
    let press = interaction(task);
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let action = dispatch(&fixture, &data, &options, &press);
    tokio::pin!(action);
    tokio::select! {
        result = &mut action => panic!("request completed before ack was released: {result:?}"),
        permit = tokio::time::timeout(std::time::Duration::from_secs(2), fixture.acknowledgement_entered.acquire()) => { permit.unwrap().unwrap().forget(); }
    }
    let mut requests = fixture.take_requests();
    assert_eq!(
        requests.len(),
        1,
        "no fresh Work/source proof before observed defer"
    );
    assert_deferred_first(
        &requests,
        command_by_key(&options.commands, CommandKey::EngageFollowUp),
    );
    assert!(
        runtime::AppState::lock(&data.state.stores)
            .work
            .engagement
            .candidates
            .is_empty()
    );
    fixture.hold_acknowledgement.store(false, Ordering::SeqCst);
    fixture.acknowledgement_release.add_permits(1);
    action.await.unwrap();
    requests.extend(fixture.take_requests());
    assert_private_receipt(&requests, &options);
    let before = std::fs::read(Stores::state_path(&directory.0)).unwrap();
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    assert!(
        dispatch(&fixture, &data, &options, &interaction(task))
            .await
            .is_err()
    );
    let failed = fixture.take_requests();
    assert_deferred_first(
        &failed,
        command_by_key(&options.commands, CommandKey::EngageFollowUp),
    );
    assert!(
        failed.iter().all(|r| r.route.ends_with("/callback")),
        "HTTP retry/error callbacks do not admit private work"
    );
    assert_eq!(
        std::fs::read(Stores::state_path(&directory.0)).unwrap(),
        before
    );
    assert_eq!(
        Stores::load(&directory.0)
            .unwrap()
            .work
            .engagement
            .candidates
            .len(),
        1
    );
    finish(supervisor, writer).await;
}

#[tokio::test]
async fn engage_follow_up_native_dispatch_refuses_malformed_source_response_and_current_task_without_charge()
 {
    for failure in 0..4 {
        let fixture = DiscordFixture::new().await;
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let (task, original, _) = seed(&fixture, &data);
        match failure {
            0 => {
                fixture
                    .native_responses
                    .lock()
                    .unwrap()
                    .get_mut(&format!("/channels/{CHANNEL}/messages/{SOURCE_MESSAGE}"))
                    .unwrap()["id"] = json!((SOURCE_MESSAGE + 100).to_string());
            }
            1 => {
                fixture
                    .native_responses
                    .lock()
                    .unwrap()
                    .get_mut(&format!("/channels/{CHANNEL}/messages/{RESPONSE_MESSAGE}"))
                    .unwrap()["id"] = json!((SOURCE_MESSAGE + 100).to_string());
            }
            2 => {
                runtime::AppState::lock(&data.state.stores)
                    .work
                    .tasks
                    .get_mut(&task)
                    .unwrap()
                    .revision = 1;
            }
            _ => {
                fixture.fail_permissions.store(true, Ordering::SeqCst);
            }
        }
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        data.state.request_persistence().await.unwrap();
        let before = std::fs::read(Stores::state_path(&directory.0)).unwrap();
        fixture.take_requests();
        let options = options();
        dispatch(&fixture, &data, &options, &interaction(task))
            .await
            .unwrap();
        let requests = fixture.take_requests();
        let body = assert_private_receipt(&requests, &options);
        assert!(
            body.contains("Task follow-up was not saved."),
            "failure {failure}: {body}"
        );
        assert!(
            runtime::AppState::lock(&data.state.stores)
                .work
                .engagement
                .candidates
                .is_empty()
        );
        assert!(
            Stores::load(&directory.0)
                .unwrap()
                .work
                .engagement
                .charges
                .is_empty()
        );
        assert_eq!(
            std::fs::read(Stores::state_path(&directory.0)).unwrap(),
            before
        );
        if failure != 2 {
            assert_eq!(
                Stores::load(&directory.0).unwrap().work.tasks[&task],
                original
            );
        }
        finish(supervisor, writer).await;
    }
}

#[tokio::test]
async fn engage_follow_up_native_dispatch_human_application_and_origin_envelope_are_required() {
    for failure in 0..3 {
        let fixture = DiscordFixture::new().await;
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let (task, _, _) = seed(&fixture, &data);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        data.state.request_persistence().await.unwrap();
        let mut press = interaction(task);
        match failure {
            0 => press.user.bot = true,
            1 => press.application_id = ApplicationId::new(322),
            _ => press.context = Some(NativeContext::BotDm),
        }
        fixture.take_requests();
        let options = options();
        assert!(dispatch(&fixture, &data, &options, &press).await.is_err());
        let requests = fixture.take_requests();
        assert_deferred_first(
            &requests,
            command_by_key(&options.commands, CommandKey::EngageFollowUp),
        );
        assert!(!requests.iter().any(|r| r.method == "GET"
            || r.route.contains("/webhooks/")
            || r.route.contains("/chat/completions")));
        assert!(
            Stores::load(&directory.0)
                .unwrap()
                .work
                .engagement
                .candidates
                .is_empty()
        );
        assert!(
            Stores::load(&directory.0)
                .unwrap()
                .work
                .engagement
                .charges
                .is_empty()
        );
        finish(supervisor, writer).await;
    }
}

#[test]
fn engage_follow_up_registers_source_snowflake_as_string_without_integer_precision_limit() {
    let options = options();
    let root = options
        .commands
        .iter()
        .find(|command| command.name == "engage")
        .unwrap();
    let registered = serde_json::to_value(root.create_as_slash_command().unwrap()).unwrap();
    let leaf = registered["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["name"] == "follow_up")
        .unwrap();
    let argument = leaf["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["name"] == "source_message")
        .unwrap();
    assert_eq!(
        argument["type"],
        json!(3),
        "real message snowflakes exceed Discord INTEGER's 53-bit bound and must be exact decimal strings"
    );
    assert_eq!(argument["required"], json!(true));
}

#[tokio::test]
async fn engage_follow_up_native_dispatch_invalid_decimal_zero_and_overflow_refuse_before_source_rest()
 {
    for malformed in ["not-a-decimal", "0", "18446744073709551616", "-1"] {
        let fixture = DiscordFixture::new().await;
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let (task, original, _) = seed(&fixture, &data);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        data.state.request_persistence().await.unwrap();
        let before = std::fs::read(Stores::state_path(&directory.0)).unwrap();
        let mut wire = serde_json::to_value(interaction(task)).unwrap();
        wire["data"]["options"][0]["options"][2]["value"] = json!(malformed);
        let press: CommandInteraction = serde_json::from_value(wire).unwrap();
        fixture.take_requests();
        let options = options();
        let result = dispatch(&fixture, &data, &options, &press).await;
        let requests = fixture.take_requests();
        assert_deferred_first(
            &requests,
            command_by_key(&options.commands, CommandKey::EngageFollowUp),
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "GET" || r.route.contains("/chat/completions")),
            "invalid decimal must not read private facts: {malformed}"
        );
        if result.is_ok() {
            assert!(
                assert_private_receipt(&requests, &options)
                    .contains("Task follow-up was not saved.")
            );
        }
        let disk = Stores::load(&directory.0).unwrap();
        assert!(disk.work.engagement.candidates.is_empty());
        assert!(disk.work.engagement.charges.is_empty());
        assert_eq!(disk.work.tasks[&task], original);
        assert_eq!(
            std::fs::read(Stores::state_path(&directory.0)).unwrap(),
            before
        );
        finish(supervisor, writer).await;
    }
}

mod work_proof;
