//! Actual registered modcall producer and recursive modcase Poise dispatch.
//! Loopback REST only; no fabricated domain authority and no environment edits.
//! Apply after the retained shadow core, native producer and modcase adapter.
use super::*;
use crate::{
    moderation::shadow::{AppealDecision, CaseStore, ReviewDecision},
    service::{ReapOutcome, ServiceSupervisor, ShutdownReason, persistence::PersistenceWriter},
};
use serenity::all::{
    InteractionContext as NativeContext, MessageId, PermissionOverwrite, PermissionOverwriteType,
    Timestamp, UserUpdateEvent,
};
use std::path::PathBuf;

const SOURCE: u64 = 1_234_567_890_123_456_789;
const REVIEW: u64 = CHANNEL + 1;
const ASSESSOR: u64 = 700;
const RESOLVER: u64 = 791;
const UNRELATED: u64 = 792;
const STAFF_ROLE: u64 = 500;
const SOURCE_TEXT: &str =
    "Synthetic private source text is transient, never an allegation about a real person";

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "abbey-modcase-native-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let policy = json!({
            "version":1, "guild":GUILD, "owner":999, "mode":"propose",
            "daily_limit":5, "daily_creations":2, "public_categories":[],
            // The private staff review channel may be protected. Capture is
            // approved only in the ordinary original source channel.
            "protected_channels":[REVIEW], "actions":[],
            "contextual_shadow":{
                "enabled":true, "source_channels":[CHANNEL], "review_channel":REVIEW
            }
        });
        let path_policy = path.join("owner-policy.json");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path_policy).unwrap();
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&policy).unwrap()).unwrap();
        file.sync_all().unwrap();
        Self(path)
    }
    fn policy(&self) -> PathBuf {
        self.0.join("owner-policy.json")
    }
    fn ledger(&self) -> PathBuf {
        self.0.join("community-operations/contextual-shadow.json")
    }
    fn cases(&self) -> CaseStore {
        crate::persist::moderation_shadow::load(&self.0).unwrap()
    }
    fn stop(&self) {
        let (_, digest) = crate::persist::community_ops::load_policy(&self.policy()).unwrap();
        crate::persist::community_ops::set_mode(
            &self.policy(),
            &digest,
            GUILD,
            999,
            crate::community_ops::Mode::Stopped,
        )
        .unwrap();
    }
    fn disable_capture(&self) {
        let (mut policy, _) = crate::persist::community_ops::load_policy(&self.policy()).unwrap();
        policy.contextual_shadow.enabled = false;
        std::fs::write(self.policy(), serde_json::to_vec_pretty(&policy).unwrap()).unwrap();
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
    let mut data = configured_data();
    let state = Arc::get_mut(&mut data.state).unwrap();
    state.data_dir = Some(directory.0.clone());
    state.community_policy_path = Some(directory.policy());
    data
}
fn native_routes(fixture: &DiscordFixture) {
    let mut g = guild(Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY);
    let mut staff = Role::default();
    staff.id = RoleId::new(STAFF_ROLE);
    staff.guild_id = GuildId::new(GUILD);
    staff.position = 10;
    staff.permissions = Permissions::VIEW_CHANNEL
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::MANAGE_MESSAGES
        | Permissions::MODERATE_MEMBERS;
    g.roles.insert(staff.id, staff);
    let mut routes = fixture.native_responses.lock().unwrap();
    routes.insert(format!("/guilds/{GUILD}"), serde_json::to_value(g).unwrap());
    for actor in [ASSESSOR, ACTOR, RESOLVER, OTHER, UNRELATED] {
        let mut m = Member::default();
        m.user = user(actor);
        m.guild_id = GuildId::new(GUILD);
        if [ASSESSOR, ACTOR, RESOLVER].contains(&actor) {
            m.roles.push(RoleId::new(STAFF_ROLE));
        }
        routes.insert(
            format!("/guilds/{GUILD}/members/{actor}"),
            serde_json::to_value(m).unwrap(),
        );
    }
    for channel_id in [CHANNEL, REVIEW] {
        let mut c = GuildChannel::default();
        c.id = ChannelId::new(channel_id);
        c.guild_id = GuildId::new(GUILD);
        c.kind = serenity::all::ChannelType::Text;
        if channel_id == REVIEW {
            c.permission_overwrites = vec![
                PermissionOverwrite {
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                    kind: PermissionOverwriteType::Role(RoleId::new(GUILD)),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Role(RoleId::new(STAFF_ROLE)),
                },
            ];
        }
        routes.insert(
            format!("/channels/{channel_id}"),
            serde_json::to_value(c).unwrap(),
        );
    }
    let mut source = Message::default();
    source.id = MessageId::new(SOURCE);
    source.channel_id = ChannelId::new(CHANNEL);
    source.guild_id = Some(GuildId::new(GUILD));
    source.author = user(OTHER);
    source.timestamp = Timestamp::from_unix_timestamp(1_700_000_000).unwrap();
    source.content = SOURCE_TEXT.into();
    routes.insert(
        format!("/channels/{CHANNEL}/messages/{SOURCE}"),
        serde_json::to_value(source).unwrap(),
    );
}
fn options() -> poise::FrameworkOptions<Data, Error> {
    poise::FrameworkOptions {
        commands: crate::application_commands(),
        ..Default::default()
    }
}
fn interaction(
    top: &str,
    leaf: Option<&str>,
    actor: u64,
    origin: u64,
    args: Value,
) -> CommandInteraction {
    static NEXT: AtomicU64 = AtomicU64::new(8_000);
    let opts = leaf.map_or_else(
        || args.clone(),
        |leaf| json!([{"name":leaf,"type":1,"options":args}]),
    );
    let mut result: CommandInteraction = serde_json::from_value(json!({
        "id":NEXT.fetch_add(1,Ordering::Relaxed).to_string(), "application_id":"321",
        "data":{"id":"222","name":top,"type":1,"options":opts},
        "guild_id":GUILD.to_string(),"channel_id":origin.to_string(),"user":user(actor),
        "token":"offline-interaction","version":1,"locale":"en-US","context":0,
        "entitlements":[],"attachment_size_limit":1024
    }))
    .unwrap();
    if top == "modcall" {
        result
            .data
            .resolved
            .users
            .insert(UserId::new(OTHER), user(OTHER));
    }
    result
}
fn capture_interaction() -> CommandInteraction {
    interaction(
        "modcall",
        None,
        ASSESSOR,
        CHANNEL,
        json!([
            {"name":"user","type":6,"value":OTHER.to_string()},
            {"name":"severity","type":4,"value":2},
            {"name":"source_message","type":3,"value":SOURCE.to_string()},
            {"name":"context","type":4,"value":0}
        ]),
    )
}
fn case_interaction(
    leaf: &str,
    actor: u64,
    origin: u64,
    id: &str,
    revision: u64,
    choice: i64,
) -> CommandInteraction {
    let mut args = vec![json!({"name":"case","type":3,"value":id})];
    if leaf != "show" {
        args.push(json!({"name":"revision","type":4,"value":revision}));
        args.push(json!({"name":if leaf == "appeal" {"reason"} else {"decision"},"type":4,"value":choice}));
    }
    interaction("modcase", Some(leaf), actor, origin, json!(args))
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
    match poise::dispatch::dispatch_interaction(
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
    .await
    {
        Ok(()) => Ok(()),
        Err(poise::FrameworkError::Command { error, .. }) => Err(error.to_string()),
        Err(poise::FrameworkError::CommandCheckFailed {
            error: Some(error), ..
        }) => Err(error.to_string()),
        Err(_) => Err("The native framework refused this interaction.".into()),
    }
}
fn private_reply<'a>(
    requests: &'a [Request],
    options: &poise::FrameworkOptions<Data, Error>,
    key: CommandKey,
) -> &'a str {
    assert_deferred_first(requests, command_by_key(&options.commands, key));
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.route.ends_with("/callback"))
            .count(),
        1
    );
    let text = assert_private_no_mentions_reply(requests);
    assert!(text.chars().count() <= 2_000);
    assert!(
        requests
            .iter()
            .filter(|r| r.method != "GET")
            .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/"))
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.route.contains("/chat/completions"))
    );
    text
}
async fn seed(
    fixture: &DiscordFixture,
    data: &Data,
    options: &poise::FrameworkOptions<Data, Error>,
    directory: &Directory,
) -> String {
    dispatch(fixture, data, options, &capture_interaction())
        .await
        .unwrap();
    let requests = fixture.take_requests();
    let reply = private_reply(&requests, options, CommandKey::Modcall);
    println!("Actual private native shadow capture: {reply}");
    assert!(
        requests
            .iter()
            .any(|r| r.method == "GET" && r.route.ends_with(&format!("/messages/{SOURCE}")))
    );
    let cases = directory.cases();
    assert_eq!(
        cases.cases.len(),
        1,
        "registered producer must save a case only after actual publication"
    );
    assert_eq!(cases.revision, 1);
    let case = cases.cases.values().next().unwrap();
    assert_eq!(case.captured.actor, ASSESSOR);
    assert_eq!(case.source.author, OTHER);
    assert_eq!(case.source.message, SOURCE);
    assert_eq!(case.revision, 1);
    assert!(
        reply.contains(&case.id),
        "successful capture must give the actual readback case ID"
    );
    assert!(
        !String::from_utf8(std::fs::read(directory.ledger()).unwrap())
            .unwrap()
            .contains(SOURCE_TEXT)
    );
    case.id.clone()
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
async fn invoke(
    fixture: &DiscordFixture,
    data: &Data,
    options: &poise::FrameworkOptions<Data, Error>,
    leaf: &str,
    actor: u64,
    origin: u64,
    case: (&str, u64, i64),
) -> String {
    let (id, revision, choice) = case;
    dispatch(
        fixture,
        data,
        options,
        &case_interaction(leaf, actor, origin, id, revision, choice),
    )
    .await
    .unwrap();
    let requests = fixture.take_requests();
    let key = match leaf {
        "show" => CommandKey::ModcaseShow,
        "review" => CommandKey::ModcaseReview,
        "appeal" => CommandKey::ModcaseAppeal,
        "resolve_appeal" => CommandKey::ModcaseResolveAppeal,
        _ => panic!("unknown test leaf"),
    };
    private_reply(&requests, options, key).to_owned()
}

#[tokio::test]
async fn actual_shadow_producer_and_four_private_leaves_preserve_independence_and_no_action() {
    let fixture = DiscordFixture::new().await;
    native_routes(&fixture);
    let directory = Directory::new();
    let data = native_data(&fixture, &directory);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = data.state.attach_service(supervisor.operations());
    data.state.request_persistence().await.unwrap();
    let canonical = std::fs::read(directory.0.join(crate::persist::STATE_FILE)).unwrap();
    let projection = std::fs::read(directory.0.join(crate::persist::WDBX_FILE)).unwrap();
    let memory = runtime::AppState::lock(&data.state.stores).clone();
    let options = options();
    let id = seed(&fixture, &data, &options, &directory).await;
    for (actor, origin) in [(OTHER, CHANNEL), (ACTOR, REVIEW)] {
        let text = invoke(
            &fixture,
            &data,
            &options,
            "show",
            actor,
            origin,
            (&id, 0, 0),
        )
        .await;
        println!("Actual native private case view: {text}");
        assert!(text.contains(&id));
        assert!(text.contains("No action taken."));
        assert!(!text.contains(SOURCE_TEXT));
        assert!(text.contains("not independently classify"));
        assert!(text.contains("/forget_learning does not delete it"));
        assert_eq!(text.contains("Human assessment records:"), actor == ACTOR);
    }
    let before = std::fs::read(directory.ledger()).unwrap();
    let rejected = invoke(
        &fixture,
        &data,
        &options,
        "review",
        ASSESSOR,
        REVIEW,
        (&id, 1, 0),
    )
    .await;
    assert!(!rejected.contains("receipt saved"));
    assert!(!rejected.contains(&id));
    assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
    let reviewed = invoke(
        &fixture,
        &data,
        &options,
        "review",
        ACTOR,
        REVIEW,
        (&id, 1, 0),
    )
    .await;
    println!("Actual native independent review: {reviewed}");
    assert!(reviewed.contains("receipt saved"));
    assert_eq!(directory.cases().cases[&id].revision, 2);
    assert_eq!(
        directory.cases().cases[&id]
            .review
            .as_ref()
            .unwrap()
            .decision,
        ReviewDecision::Agree
    );
    let reviewed_bytes = std::fs::read(directory.ledger()).unwrap();
    let repeated = invoke(
        &fixture,
        &data,
        &options,
        "review",
        ACTOR,
        REVIEW,
        (&id, 1, 0),
    )
    .await;
    assert!(repeated.contains("Existing independent human review receipt saved"));
    assert_eq!(std::fs::read(directory.ledger()).unwrap(), reviewed_bytes);
    assert_eq!(directory.cases().revision, 2);
    let appealed = invoke(
        &fixture,
        &data,
        &options,
        "appeal",
        OTHER,
        CHANNEL,
        (&id, 2, 2),
    )
    .await;
    assert!(appealed.contains("receipt saved"));
    assert_eq!(directory.cases().cases[&id].revision, 3);
    let before = std::fs::read(directory.ledger()).unwrap();
    for actor in [ASSESSOR, ACTOR] {
        let rejected = invoke(
            &fixture,
            &data,
            &options,
            "resolve_appeal",
            actor,
            REVIEW,
            (&id, 3, 0),
        )
        .await;
        assert!(!rejected.contains("receipt saved"));
        assert!(!rejected.contains(&id));
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
    }
    let resolved = invoke(
        &fixture,
        &data,
        &options,
        "resolve_appeal",
        RESOLVER,
        REVIEW,
        (&id, 3, 0),
    )
    .await;
    assert!(resolved.contains("receipt saved"));
    let case = directory.cases().cases[&id].clone();
    assert_eq!(case.revision, 4);
    assert_eq!(
        case.appeal.unwrap().resolution.unwrap().decision,
        AppealDecision::Upheld
    );
    assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&memory));
    assert_eq!(
        std::fs::read(directory.0.join(crate::persist::STATE_FILE)).unwrap(),
        canonical
    );
    assert_eq!(
        std::fs::read(directory.0.join(crate::persist::WDBX_FILE)).unwrap(),
        projection
    );
    assert!(
        !String::from_utf8(std::fs::read(directory.ledger()).unwrap())
            .unwrap()
            .contains(SOURCE_TEXT)
    );
    finish(supervisor, writer).await;
}

#[tokio::test]
async fn stop_and_disabled_capture_keep_existing_subject_appeal_and_protected_staff_review_available()
 {
    for stopped in [true, false] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        let id = seed(&fixture, &data, &options, &directory).await;
        if stopped {
            directory.stop();
        } else {
            directory.disable_capture();
        }
        let shown = invoke(
            &fixture,
            &data,
            &options,
            "show",
            OTHER,
            CHANNEL,
            (&id, 0, 0),
        )
        .await;
        assert!(shown.contains(&id));
        let reviewed = invoke(
            &fixture,
            &data,
            &options,
            "review",
            ACTOR,
            REVIEW,
            (&id, 1, 1),
        )
        .await;
        assert!(reviewed.contains("receipt saved"));
        let appealed = invoke(
            &fixture,
            &data,
            &options,
            "appeal",
            OTHER,
            CHANNEL,
            (&id, 2, 1),
        )
        .await;
        assert!(appealed.contains("receipt saved"));
        let resolved = invoke(
            &fixture,
            &data,
            &options,
            "resolve_appeal",
            RESOLVER,
            REVIEW,
            (&id, 3, 1),
        )
        .await;
        assert!(resolved.contains("receipt saved"));
        let before = std::fs::read(directory.ledger()).unwrap();
        // New source version after Stop/disabled is neither a duplicate nor
        // a mutation of the old immutable case and must not be captured.
        let route = format!("/channels/{CHANNEL}/messages/{SOURCE}");
        fixture
            .native_responses
            .lock()
            .unwrap()
            .get_mut(&route)
            .unwrap()["content"] = json!("Another synthetic version");
        dispatch(&fixture, &data, &options, &capture_interaction())
            .await
            .unwrap();
        private_reply(&fixture.take_requests(), &options, CommandKey::Modcall);
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
        assert_eq!(directory.cases().cases.len(), 1);
        finish(supervisor, writer).await;
    }
}

#[tokio::test]
async fn unauthorized_subject_wrong_origin_and_stale_revision_never_reveal_or_change_a_case() {
    let fixture = DiscordFixture::new().await;
    native_routes(&fixture);
    let directory = Directory::new();
    let data = native_data(&fixture, &directory);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = data.state.attach_service(supervisor.operations());
    let options = options();
    let id = seed(&fixture, &data, &options, &directory).await;
    let before = std::fs::read(directory.ledger()).unwrap();
    for (leaf, actor, origin, revision) in [
        ("show", UNRELATED, CHANNEL, 0),
        ("show", ACTOR, CHANNEL, 0),
        ("appeal", UNRELATED, CHANNEL, 1),
        ("appeal", OTHER, REVIEW, 1),
        ("review", ACTOR, CHANNEL, 1),
        ("review", ACTOR, REVIEW, 2),
        ("resolve_appeal", RESOLVER, REVIEW, 1),
    ] {
        // Catalog denials may return a FrameworkError while the leaf's closed
        // denial returns Ok; assert observable privacy and unchanged disk.
        let _ = dispatch(
            &fixture,
            &data,
            &options,
            &case_interaction(leaf, actor, origin, &id, revision, 0),
        )
        .await;
        let requests = fixture.take_requests();
        let text = assert_private_no_mentions_reply(&requests);
        assert!(!text.contains(&id), "{leaf}/{actor}/{origin}: {text}");
        assert!(!text.contains("receipt saved"));
        assert!(!text.contains(SOURCE_TEXT));
        // Authorized current staff may read source before a stale revision or
        // missing-appeal state is rejected inside the actual transaction.
        if leaf != "resolve_appeal" && revision != 2 {
            assert!(
                !requests
                    .iter()
                    .any(|r| r.route.ends_with(&format!("/messages/{SOURCE}")))
            );
        }
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
    }
    finish(supervisor, writer).await;
}

#[tokio::test]
async fn wrong_native_member_guild_channel_missing_role_and_revoked_view_deny_private_case_access()
{
    for scenario in [
        "member",
        "guild",
        "channel",
        "missing-role",
        "revoked-view",
        "thread",
    ] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        let id = seed(&fixture, &data, &options, &directory).await;
        let before = std::fs::read(directory.ledger()).unwrap();
        {
            let mut routes = fixture.native_responses.lock().unwrap();
            match scenario {
                "member" => {
                    routes
                        .get_mut(&format!("/guilds/{GUILD}/members/{OTHER}"))
                        .unwrap()["user"]["id"] = json!(UNRELATED.to_string())
                }
                "guild" => {
                    routes.get_mut(&format!("/guilds/{GUILD}")).unwrap()["id"] =
                        json!((GUILD + 1).to_string())
                }
                "channel" => {
                    routes.get_mut(&format!("/channels/{CHANNEL}")).unwrap()["id"] =
                        json!((CHANNEL + 2).to_string())
                }
                "missing-role" => {
                    routes
                        .get_mut(&format!("/guilds/{GUILD}/members/{OTHER}"))
                        .unwrap()["roles"] = json!(["600"])
                }
                "revoked-view" => {
                    routes.get_mut(&format!("/channels/{CHANNEL}")).unwrap()["permission_overwrites"] = json!([{
                        "id":OTHER.to_string(),"type":1,"allow":"0","deny":Permissions::VIEW_CHANNEL.bits().to_string()
                    }])
                }
                "thread" => {
                    routes.get_mut(&format!("/channels/{CHANNEL}")).unwrap()["type"] = json!(11)
                }
                _ => unreachable!(),
            }
        }
        let _ = dispatch(
            &fixture,
            &data,
            &options,
            &case_interaction("show", OTHER, CHANNEL, &id, 0, 0),
        )
        .await;
        let requests = fixture.take_requests();
        let text = private_reply(&requests, &options, CommandKey::ModcaseShow);
        assert!(!text.contains(&id), "{scenario}: {text}");
        assert!(!text.contains(SOURCE_TEXT));
        assert!(
            !requests
                .iter()
                .any(|r| r.route.ends_with(&format!("/messages/{SOURCE}")))
        );
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
        finish(supervisor, writer).await;
    }
}

#[tokio::test]
async fn changed_source_identity_or_content_refuses_confident_review_but_allows_needs_context() {
    for change in ["wrong-message", "wrong-author", "edited-content"] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        let id = seed(&fixture, &data, &options, &directory).await;
        let before = std::fs::read(directory.ledger()).unwrap();
        {
            let route = format!("/channels/{CHANNEL}/messages/{SOURCE}");
            let mut responses = fixture.native_responses.lock().unwrap();
            let source = responses.get_mut(&route).unwrap();
            match change {
                "wrong-message" => source["id"] = json!((SOURCE + 1).to_string()),
                "wrong-author" => source["author"] = serde_json::to_value(user(UNRELATED)).unwrap(),
                "edited-content" => source["content"] = json!("Synthetic changed source text"),
                _ => unreachable!(),
            }
        }
        let rejected = invoke(
            &fixture,
            &data,
            &options,
            "review",
            ACTOR,
            REVIEW,
            (&id, 1, 0),
        )
        .await;
        assert!(!rejected.contains("receipt saved"), "{change}: {rejected}");
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
        let uncertain = invoke(
            &fixture,
            &data,
            &options,
            "review",
            ACTOR,
            REVIEW,
            (&id, 1, 2),
        )
        .await;
        assert!(uncertain.contains("receipt saved"));
        let reviewed = directory.cases().cases[&id].clone();
        assert_eq!(
            reviewed.review.unwrap().decision,
            ReviewDecision::NeedsContext
        );
        assert_eq!(reviewed.revision, 2);
        assert_eq!(directory.cases().cases.len(), 1);
        finish(supervisor, writer).await;
    }
}

#[tokio::test]
async fn private_acknowledgement_is_observed_before_native_case_reads_and_failed_ack_stops_work() {
    let fixture = DiscordFixture::new().await;
    native_routes(&fixture);
    let directory = Directory::new();
    let data = native_data(&fixture, &directory);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = data.state.attach_service(supervisor.operations());
    let options = options();
    let id = seed(&fixture, &data, &options, &directory).await;
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let press = case_interaction("show", OTHER, CHANNEL, &id, 0, 0);
    let work = dispatch(&fixture, &data, &options, &press);
    tokio::pin!(work);
    tokio::select! {
        permit=fixture.acknowledgement_entered.acquire()=>permit.unwrap().forget(),
        result=&mut work=>panic!("command ended before held acknowledgement: {result:?}"),
    }
    let held = fixture.take_requests();
    assert_eq!(held.len(), 1);
    assert!(held[0].route.ends_with("/callback"));
    assert_eq!(held[0].body["type"], 5);
    assert_eq!(held[0].body["data"]["flags"].as_u64().unwrap() & 64, 64);
    assert!(!held.iter().any(|r| r.method == "GET"));
    fixture.acknowledgement_release.add_permits(1);
    work.await.unwrap();
    let completed = fixture.take_requests();
    assert!(completed.iter().any(|r| r.method == "GET"));
    fixture.hold_acknowledgement.store(false, Ordering::SeqCst);
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    let before = std::fs::read(directory.ledger()).unwrap();
    assert!(
        dispatch(
            &fixture,
            &data,
            &options,
            &case_interaction("review", ACTOR, REVIEW, &id, 1, 0)
        )
        .await
        .is_err()
    );
    let failed = fixture.take_requests();
    assert!(failed.iter().all(|r| r.route.ends_with("/callback")));
    assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
    finish(supervisor, writer).await;
}

#[tokio::test]
async fn native_envelope_rejects_dm_bot_and_wrong_application_without_case_or_source_snapshot() {
    for changed in ["dm", "bot", "application"] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        let id = seed(&fixture, &data, &options, &directory).await;
        let before = std::fs::read(directory.ledger()).unwrap();
        let mut press = case_interaction("show", OTHER, CHANNEL, &id, 0, 0);
        match changed {
            "dm" => press.context = Some(NativeContext::BotDm),
            "bot" => press.user.bot = true,
            "application" => press.application_id = ApplicationId::new(322),
            _ => unreachable!(),
        }
        let _ = dispatch(&fixture, &data, &options, &press).await;
        let requests = fixture.take_requests();
        assert!(
            requests
                .iter()
                .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/")),
            "no GET after invalid native envelope"
        );
        if requests.iter().any(|r| r.route.contains("/webhooks/")) {
            let text = assert_private_no_mentions_reply(&requests);
            assert!(!text.contains(&id));
            assert!(!text.contains(SOURCE_TEXT));
        }
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
        finish(supervisor, writer).await;
    }
}

#[tokio::test]
async fn revoked_staff_delete_timeout_or_current_role_denies_before_case_source_hydration() {
    for revoked in ["delete", "timeout", "role"] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        let id = seed(&fixture, &data, &options, &directory).await;
        let before = std::fs::read(directory.ledger()).unwrap();
        {
            let mut routes = fixture.native_responses.lock().unwrap();
            if revoked == "role" {
                routes
                    .get_mut(&format!("/guilds/{GUILD}/members/{ACTOR}"))
                    .unwrap()["roles"] = json!([]);
            } else {
                let deny = if revoked == "delete" {
                    Permissions::MANAGE_MESSAGES
                } else {
                    Permissions::MODERATE_MEMBERS
                };
                routes.get_mut(&format!("/channels/{REVIEW}")).unwrap()["permission_overwrites"] = json!([
                    {"id":GUILD.to_string(),"type":0,"allow":"0","deny":Permissions::VIEW_CHANNEL.bits().to_string()},
                    {"id":STAFF_ROLE.to_string(),"type":0,"allow":Permissions::VIEW_CHANNEL.bits().to_string(),"deny":"0"},
                    {"id":ACTOR.to_string(),"type":1,"allow":"0","deny":deny.bits().to_string()}
                ]);
            }
        }
        let _ = dispatch(
            &fixture,
            &data,
            &options,
            &case_interaction("review", ACTOR, REVIEW, &id, 1, 0),
        )
        .await;
        let requests = fixture.take_requests();
        let text = assert_private_no_mentions_reply(&requests);
        assert!(!text.contains(&id), "{revoked}: {text}");
        assert!(!text.contains("receipt saved"));
        assert!(
            !requests
                .iter()
                .any(|r| r.route.ends_with(&format!("/messages/{SOURCE}")))
        );
        assert_eq!(std::fs::read(directory.ledger()).unwrap(), before);
        finish(supervisor, writer).await;
    }
}

mod retry_tests;

mod read_only_tests;

mod late_proof_tests;
