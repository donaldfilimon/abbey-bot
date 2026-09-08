//! Exercise registered Poise checks with real contexts and loopback-only transports.
use super::*;
use catalog::{AccessId, ConditionId};
use serde_json::{Value, json};
use serenity::all::{
    ApplicationId, Cache, ChannelId, CommandInteraction, ComponentInteractionDataKind,
    GatewayIntents, Guild, GuildChannel, GuildCreateEvent, Member, Message, Role, RoleId, Shard,
    ShardId, ShardInfo, ShardManager, ShardManagerOptions, ShardMessenger, ShardRunner,
    ShardRunnerOptions, User,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const GUILD: u64 = 123;
const CHANNEL: u64 = 456;
const ACTOR: u64 = 789;
const OTHER: u64 = 790;

#[derive(Debug)]
struct Request {
    method: String,
    route: String,
    body: Value,
}

struct DiscordFixture {
    context: serenity::all::Context,
    manager: Arc<ShardManager>,
    requests: Arc<Mutex<Vec<Request>>>,
    permissions: Arc<AtomicU64>,
    fail_permissions: Arc<AtomicBool>,
    fail_acknowledgement: Arc<AtomicBool>,
    fail_next_edit: Arc<AtomicBool>,
    hold_acknowledgement: Arc<AtomicBool>,
    acknowledgement_entered: Arc<tokio::sync::Semaphore>,
    acknowledgement_release: Arc<tokio::sync::Semaphore>,
    server: tokio::task::JoinHandle<()>,
    address: std::net::SocketAddr,
}

struct ProviderFixture {
    address: std::net::SocketAddr,
    calls: Arc<AtomicU64>,
    server: tokio::task::JoinHandle<()>,
}

impl ProviderFixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let server_calls = Arc::clone(&calls);
        let server = tokio::spawn(async move {
            'connections: loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                server_calls.fetch_add(1, Ordering::SeqCst);
                // Consume the request before closing the response socket. An
                // unread request body can turn our intended schema failure
                // into a TCP reset and select transport recovery instead.
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        continue 'connections;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(
                        bytes.len() <= 1024 * 1024,
                        "bounded provider fixture request"
                    );
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let response = b"{}";
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.write_all(response).await;
            }
        });
        Self {
            address,
            calls,
            server,
        }
    }
}

impl Drop for ProviderFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn user(id: u64) -> User {
    let mut user = User::default();
    user.id = UserId::new(id);
    user.name = format!("fixture-{id}");
    user
}

fn guild(permissions: Permissions) -> Guild {
    let mut guild = Guild::default();
    guild.id = GuildId::new(GUILD);
    guild.owner_id = UserId::new(999);
    guild.name = "offline fixture".into();
    let mut role = Role::default();
    role.id = RoleId::new(GUILD);
    role.guild_id = guild.id;
    role.permissions = permissions;
    guild.roles.insert(role.id, role);
    guild
}

impl DiscordFixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let permissions = Arc::new(AtomicU64::new(Permissions::VIEW_CHANNEL.bits()));
        let fail_permissions = Arc::new(AtomicBool::new(false));
        let fail_acknowledgement = Arc::new(AtomicBool::new(false));
        let fail_next_edit = Arc::new(AtomicBool::new(false));
        let hold_acknowledgement = Arc::new(AtomicBool::new(false));
        let acknowledgement_entered = Arc::new(tokio::sync::Semaphore::new(0));
        let acknowledgement_release = Arc::new(tokio::sync::Semaphore::new(0));
        let server = {
            let requests = Arc::clone(&requests);
            let permissions = Arc::clone(&permissions);
            let fail_permissions = Arc::clone(&fail_permissions);
            let fail_acknowledgement = Arc::clone(&fail_acknowledgement);
            let fail_next_edit = Arc::clone(&fail_next_edit);
            let hold_acknowledgement = Arc::clone(&hold_acknowledgement);
            let acknowledgement_entered = Arc::clone(&acknowledgement_entered);
            let acknowledgement_release = Arc::clone(&acknowledgement_release);
            tokio::spawn(async move {
                'connections: loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let header_end = loop {
                        let mut buffer = [0; 4096];
                        let count = stream.read(&mut buffer).await.unwrap();
                        if count == 0 && bytes.is_empty() {
                            // A failed try_join can cancel a queued connection.
                            continue 'connections;
                        }
                        assert!(count > 0, "request ended before headers");
                        bytes.extend_from_slice(&buffer[..count]);
                        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                    let length = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
                    while bytes.len() < header_end + length {
                        let mut buffer = [0; 4096];
                        let count = stream.read(&mut buffer).await.unwrap();
                        assert!(count > 0, "request ended before body");
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    let mut first = headers.lines().next().unwrap().split_whitespace();
                    let method = first.next().unwrap().to_string();
                    let route = first.next().unwrap().to_string();
                    let multipart = headers
                        .to_ascii_lowercase()
                        .contains("content-type: multipart/form-data");
                    let body = if length == 0 {
                        Value::Null
                    } else if multipart {
                        json!({"multipart": String::from_utf8_lossy(&bytes[header_end..header_end + length])})
                    } else {
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                    };
                    let is_acknowledgement = matches!(body["type"].as_u64(), Some(5 | 6));
                    let is_permission_lookup = method == "GET";
                    let is_callback = route.ends_with("/callback");
                    let (content_type, response) = if method == "GET" && route == "/fixture.png" {
                        let mut encoded = std::io::Cursor::new(Vec::new());
                        ::image::DynamicImage::new_rgb8(2, 2)
                            .write_to(&mut encoded, ::image::ImageFormat::Png)
                            .unwrap();
                        ("image/png", encoded.into_inner())
                    } else if method == "POST" && route == "/v1/chat/completions" {
                        (
                            "application/json",
                            json!({"choices":[{"message":{"role":"assistant","content":"x".repeat(3_000)},"finish_reason":"stop"}]})
                                .to_string()
                                .into_bytes(),
                        )
                    } else if is_permission_lookup && route.contains("/members/") {
                        let mut member = Member::default();
                        member.user = user(ACTOR);
                        member.guild_id = GuildId::new(GUILD);
                        ("application/json", serde_json::to_vec(&member).unwrap())
                    } else if is_permission_lookup && route.contains("/guilds/") {
                        (
                            "application/json",
                            serde_json::to_vec(&guild(Permissions::from_bits_retain(
                                permissions.load(Ordering::SeqCst),
                            )))
                            .unwrap(),
                        )
                    } else if is_permission_lookup && route.contains("/channels/") {
                        let mut channel = GuildChannel::default();
                        channel.id = ChannelId::new(CHANNEL);
                        channel.guild_id = GuildId::new(GUILD);
                        ("application/json", serde_json::to_vec(&channel).unwrap())
                    } else if is_callback {
                        ("application/json", Vec::new())
                    } else {
                        assert!(route.contains("/webhooks/"), "unexpected route: {route}");
                        let mut message = Message::default();
                        message.content = body["content"].as_str().unwrap_or_default().into();
                        ("application/json", serde_json::to_vec(&message).unwrap())
                    };
                    let edit_failed =
                        method == "PATCH" && fail_next_edit.swap(false, Ordering::SeqCst);
                    requests.lock().unwrap().push(Request {
                        method,
                        route,
                        body,
                    });
                    let failed = edit_failed
                        || (is_permission_lookup && fail_permissions.load(Ordering::SeqCst))
                        || (is_acknowledgement && fail_acknowledgement.load(Ordering::SeqCst));
                    let (status, response) = if failed {
                        (
                            "403 Forbidden",
                            json!({"code": 50013, "message": "fixture denied"})
                                .to_string()
                                .into_bytes(),
                        )
                    } else if is_callback {
                        ("204 No Content", Vec::new())
                    } else {
                        ("200 OK", response)
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.len()
                    );
                    if is_acknowledgement && hold_acknowledgement.load(Ordering::SeqCst) {
                        // Continue serving other requests while withholding the ack,
                        // so premature permission I/O is observable by the test.
                        acknowledgement_entered.add_permits(1);
                        let release = Arc::clone(&acknowledgement_release);
                        tokio::spawn(async move {
                            release.acquire().await.unwrap().forget();
                            let _ = stream.write_all(reply.as_bytes()).await;
                            let _ = stream.write_all(&response).await;
                        });
                        continue 'connections;
                    }
                    // Failed permission joins may cancel other already-issued requests.
                    let _ = stream.write_all(reply.as_bytes()).await;
                    let _ = stream.write_all(&response).await;
                }
            })
        };
        let http = Arc::new(
            serenity::http::HttpBuilder::new("offline-fixture-token")
                .application_id(ApplicationId::new(321))
                .client(
                    reqwest::Client::builder()
                        .no_proxy()
                        .timeout(std::time::Duration::from_secs(5))
                        .build()
                        .unwrap(),
                )
                .proxy(format!("http://{address}"))
                .ratelimiter_disabled(true)
                .build(),
        );
        let cache = Arc::new(Cache::default());
        let data = Arc::new(tokio::sync::RwLock::new(Default::default()));
        // ShardMessenger has no public empty constructor. Build an inert runner
        // after a loopback WebSocket handshake; never start it or identify a bot.
        let gateway = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway_url = Arc::new(tokio::sync::Mutex::new(format!(
            "ws://{}",
            gateway.local_addr().unwrap()
        )));
        let (shard, socket) = tokio::join!(
            Shard::new(
                Arc::clone(&gateway_url),
                "offline-fixture-token",
                ShardInfo {
                    id: ShardId(0),
                    total: 1
                },
                GatewayIntents::empty(),
                None
            ),
            async {
                let (stream, _) = gateway.accept().await.unwrap();
                tokio_tungstenite::accept_async(stream).await.unwrap()
            }
        );
        drop(socket);
        let (manager, _) = ShardManager::new(ShardManagerOptions {
            data: Arc::clone(&data),
            event_handlers: vec![],
            raw_event_handlers: vec![],
            framework: Arc::new(std::sync::OnceLock::new()),
            shard_index: 0,
            shard_init: 0,
            shard_total: 1,
            voice_manager: None,
            ws_url: gateway_url,
            cache: Arc::clone(&cache),
            http: Arc::clone(&http),
            intents: GatewayIntents::empty(),
            presence: None,
        });
        let runner = ShardRunner::new(ShardRunnerOptions {
            data: Arc::clone(&data),
            event_handlers: vec![],
            raw_event_handlers: vec![],
            framework: None,
            manager: Arc::clone(&manager),
            shard: shard.unwrap(),
            voice_manager: None,
            cache: Arc::clone(&cache),
            http: Arc::clone(&http),
        });
        let context = serenity::all::Context {
            data,
            shard: ShardMessenger::new(&runner),
            shard_id: ShardId(0),
            http,
            cache,
        };
        Self {
            context,
            manager,
            requests,
            permissions,
            fail_permissions,
            fail_acknowledgement,
            fail_next_edit,
            hold_acknowledgement,
            acknowledgement_entered,
            acknowledgement_release,
            server,
            address,
        }
    }

    fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut self.requests.lock().unwrap())
    }

    fn actor_presence(&self, present: bool) {
        let mut guild = guild(Permissions::VIEW_CHANNEL);
        guild.voice_states.insert(
            UserId::new(ACTOR),
            serde_json::from_value(json!({
                "user_id": ACTOR.to_string(), "guild_id": GUILD.to_string(),
                "channel_id": present.then(|| CHANNEL.to_string()), "session_id": "fixture", "deaf": false,
                "mute": false, "self_deaf": false, "self_mute": false, "self_video": false,
                "suppress": false
            }))
            .unwrap(),
        );
        let mut event: GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(guild).unwrap()).unwrap();
        self.context.cache.update(&mut event);
    }
}

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

fn help_component(
    fixture: &DiscordFixture,
    session: help_center::HelpSession,
    select: bool,
    in_guild: bool,
) -> ComponentInteraction {
    let mut message = Message::default();
    message.id = serenity::all::MessageId::new(900);
    message.channel_id = ChannelId::new(CHANNEL);
    message.author = fixture.context.cache.current_user().clone().into();
    let kind = if select {
        json!({"custom_id": session.navigate(HelpSection::Start).custom_id(),
            "component_type": 3, "values": [session.section.slug()]})
    } else {
        json!({"custom_id": session.custom_id(), "component_type": 2})
    };
    serde_json::from_value(json!({
        "id": "901", "application_id": "321", "data": kind,
        "guild_id": in_guild.then(|| GUILD.to_string()), "channel_id": CHANNEL.to_string(),
        "message": message, "user": user(ACTOR), "token": "offline-component",
        "version": 1, "locale": "en-US", "entitlements": [], "attachment_size_limit": 1048576
    }))
    .unwrap()
}

fn assert_private_help_response(requests: &[Request]) -> &Value {
    assert_eq!(requests[0].body["type"], 5);
    assert_eq!(requests[0].body["data"]["flags"], 64);
    let reply = &requests
        .iter()
        .find(|request| request.method == "PATCH")
        .unwrap()
        .body;
    assert!(reply["content"].as_str().unwrap().chars().count() <= 2000);
    assert_eq!(reply["allowed_mentions"]["parse"], json!([]));
    assert_eq!(reply["allowed_mentions"]["replied_user"], false);
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
                    reply["content"].as_str().unwrap().contains("`/admin show`"),
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
            assert_eq!(reply["content"], expected);
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
            let body = reply["content"].as_str().unwrap();
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

impl Drop for DiscordFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn configured_data() -> Data {
    configured_data_at(None)
}

fn configured_data_at(address: Option<std::net::SocketAddr>) -> Data {
    let mut data = Data {
        state: runtime::AppState::in_memory(),
        voice: None,
    };
    let state = Arc::get_mut(&mut data.state).unwrap();
    let fixture_endpoint = address.map_or_else(
        || "http://127.0.0.1:1".into(),
        |address| format!("http://{address}"),
    );
    state
        .providers
        .set_primary(Some(crate::llm::Backend::OpenAiCompatible {
            endpoint: fixture_endpoint.clone(),
            model: "fixture".into(),
        }));
    if address.is_some() {
        state.attachments = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
    }
    state
        .providers
        .set_vision(crate::vision::ConfiguredVision::Remote(
            crate::vision::RemoteVision {
                config: crate::vision::VisionConfig {
                    base_url: address.map_or_else(
                        || "http://127.0.0.1:1/v1".into(),
                        |a| format!("http://{a}/v1"),
                    ),
                    model: "fixture".into(),
                    api_key: String::new(),
                },
                transport: runtime::HttpVisionTransport::default(),
            },
        ));
    data.voice = Some(Arc::new(crate::voice_session::VoiceRuntime::new(
        crate::voice::VoiceConfig::selected_only(
            GUILD,
            CHANNEL,
            crate::voice::VoiceBackendConfig::Local(
                crate::offline_voice::OfflineVoiceConfig::from_values(
                    Some(fixture_endpoint),
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap(),
            ),
            true,
        ),
    )));
    data
}

fn leaves(commands: &[poise::Command<Data, Error>]) -> Vec<&poise::Command<Data, Error>> {
    commands
        .iter()
        .flat_map(|command| {
            if command.subcommands.is_empty() {
                vec![command]
            } else {
                leaves(&command.subcommands)
            }
        })
        .collect()
}

fn binding(command: &poise::Command<Data, Error>) -> CatalogBinding {
    *command
        .custom_data
        .downcast_ref::<CatalogBinding>()
        .unwrap()
}

fn ordinary(command: &&poise::Command<Data, Error>) -> bool {
    !matches!(
        binding(command).key,
        CommandKey::Help | CommandKey::Modcall | CommandKey::VoiceLeave
    )
}

struct Invocation {
    interaction: CommandInteraction,
    sent: AtomicBool,
    invocation_data: tokio::sync::Mutex<Box<dyn std::any::Any + Send + Sync>>,
}

impl Invocation {
    fn new(command: &poise::Command<Data, Error>, in_guild: bool, subject: Option<u64>) -> Self {
        let options = subject.map_or_else(Vec::new, |subject| {
            vec![json!({"name":"user", "type":6, "value":subject.to_string()})]
        });
        let kind = match catalog::command(binding(command).key).kind {
            catalog::CommandKind::Slash => 1,
            catalog::CommandKind::UserContext => 2,
            catalog::CommandKind::MessageContext => 3,
        };
        let interaction = serde_json::from_value(json!({
            "id":"111", "application_id":"321", "data": {"id":"222", "name":command.name, "type":kind, "options":options},
            "guild_id":in_guild.then(|| GUILD.to_string()), "channel_id":CHANNEL.to_string(),
            "user":user(ACTOR), "token":"offline-interaction", "version":1, "locale":"en-US",
            "entitlements":[], "attachment_size_limit":1024
        })).unwrap();
        Self {
            interaction,
            sent: AtomicBool::new(false),
            invocation_data: tokio::sync::Mutex::new(Box::new(())),
        }
    }

    fn voice(command: &poise::Command<Data, Error>, consent: Option<bool>) -> Self {
        let mut invocation = Self::new(command, true, None);
        invocation.interaction.data.options = consent.map_or_else(Vec::new, |consent| {
            serde_json::from_value(json!([{
                "name": "consent", "type": 5, "value": consent
            }]))
            .unwrap()
        });
        invocation
    }

    fn context<'a>(
        &'a self,
        fixture: &'a DiscordFixture,
        command: &'a poise::Command<Data, Error>,
        options: &'a poise::FrameworkOptions<Data, Error>,
        data: &'a Data,
        interaction_type: poise::CommandInteractionType,
    ) -> poise::ApplicationContext<'a, Data, Error> {
        self.context_with_args(fixture, command, options, data, interaction_type, &[])
    }

    fn context_with_args<'a>(
        &'a self,
        fixture: &'a DiscordFixture,
        command: &'a poise::Command<Data, Error>,
        options: &'a poise::FrameworkOptions<Data, Error>,
        data: &'a Data,
        interaction_type: poise::CommandInteractionType,
        args: &'a [serenity::all::ResolvedOption<'a>],
    ) -> poise::ApplicationContext<'a, Data, Error> {
        poise::ApplicationContext {
            serenity_context: &fixture.context,
            interaction: &self.interaction,
            interaction_type,
            args,
            has_sent_initial_response: &self.sent,
            framework: poise::FrameworkContext {
                bot_id: UserId::new(321),
                options,
                user_data: data,
                shard_manager: &fixture.manager,
            },
            parent_commands: &[],
            command,
            data,
            invocation_data: &self.invocation_data,
            __non_exhaustive: (),
        }
    }
}

fn command_by_key(
    commands: &[poise::Command<Data, Error>],
    key: CommandKey,
) -> &poise::Command<Data, Error> {
    leaves(commands)
        .into_iter()
        .find(|command| binding(command).key == key)
        .unwrap()
}

async fn invoke_voice_slash_fails(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    consent: Option<bool>,
) -> bool {
    let invocation = Invocation::voice(command, consent);
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
        &args,
    );
    command.slash_action.unwrap()(context).await.is_err()
}

async fn check(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    in_guild: bool,
    subject: Option<u64>,
) -> bool {
    let invocation = Invocation::new(command, in_guild, subject);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    command.checks[0](poise::Context::Application(context))
        .await
        .unwrap()
}

async fn invoke_user_menu(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    target: User,
) {
    let invocation = Invocation::new(command, true, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    let Some(poise::ContextMenuCommandAction::User(action)) = command.context_menu_action else {
        panic!("expected user menu")
    };
    assert!(action(context, target).await.is_ok());
}

async fn invoke_message_menu(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    target: Message,
    in_guild: bool,
) {
    let invocation = Invocation::new(command, in_guild, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    let Some(poise::ContextMenuCommandAction::Message(action)) = command.context_menu_action else {
        panic!("expected message menu")
    };
    assert!(action(context, target).await.is_ok());
}

fn assert_deferred_first(requests: &[Request], command: &poise::Command<Data, Error>) {
    assert_eq!(requests[0].method, "POST", "{}", command.qualified_name);
    assert!(requests[0].route.ends_with("/callback"));
    assert_eq!(requests[0].body["type"], 5, "{}", command.qualified_name);
    assert_eq!(
        requests[0].body["data"]["flags"].as_u64().unwrap_or(0) & 64 != 0,
        command.ephemeral
    );
}

fn assert_private_no_mentions_reply(requests: &[Request]) -> &str {
    assert_eq!(requests[0].body["type"], 5);
    assert_ne!(
        requests[0].body["data"]["flags"].as_u64().unwrap_or(0) & 64,
        0
    );
    let reply = requests
        .iter()
        .find(|request| request.route.contains("/webhooks/"))
        .expect("private follow-up");
    assert_eq!(reply.body["allowed_mentions"]["parse"], json!([]));
    reply.body["content"].as_str().unwrap()
}

#[tokio::test]
async fn registered_memory_menu_shares_card_and_a1_denial_without_mutation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let guild = format!("discord:{GUILD}");
    let actor = format!("discord:{ACTOR}");
    data.state
        .memory_service()
        .remember(&guild, &actor, "likes Rust", 1)
        .unwrap();
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::MemoryMenu);
    let before = runtime::AppState::lock(&data.state.stores).clone();

    invoke_user_menu(&fixture, command, &data, user(ACTOR)).await;
    let requests = fixture.take_requests();
    let content = assert_private_no_mentions_reply(&requests);
    assert!(content.contains("likes Rust"));
    assert!(content.contains("standing 0.50"));
    let menu_body = content.to_string();
    let reply = requests
        .iter()
        .find(|request| request.route.contains("/webhooks/"))
        .unwrap();
    assert_eq!(
        reply.body["components"][0]["components"][0]["label"],
        "Browse facts"
    );
    let recall = command_by_key(&commands, CommandKey::Recall);
    let invocation = Invocation::new(recall, true, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        &fixture,
        recall,
        &options,
        &data,
        poise::CommandInteractionType::Command,
    );
    assert!(recall.slash_action.unwrap()(context).await.is_ok());
    let slash_requests = fixture.take_requests();
    assert_eq!(assert_private_no_mentions_reply(&slash_requests), menu_body);
    let slash = slash_requests
        .iter()
        .find(|request| request.route.contains("/webhooks/"))
        .unwrap();
    assert_eq!(
        slash.body["components"][0]["components"][0]["label"],
        "Browse facts"
    );
    assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
    assert_eq!(
        runtime::AppState::lock(&data.state.rewards).pending_len(),
        0
    );

    invoke_user_menu(&fixture, command, &data, user(OTHER)).await;
    let denied = fixture.take_requests();
    assert!(denied.iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|text| text.contains("only your own memory"))
    }));
    assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
}

#[tokio::test]
async fn registered_image_menus_invoke_provider_privately_without_state_mutation() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data_at(Some(fixture.address));
    let commands = crate::application_commands();
    let attachment: serenity::all::Attachment = serde_json::from_value(json!({
        "id":"1", "filename":"misleading.txt", "size":100,
        "url":format!("http://{}/fixture.png", fixture.address),
        "proxy_url":"https://ignored.invalid/proxy"
    }))
    .unwrap();
    let mut message = Message::default();
    message.content = "https://ignored.invalid/body.png".into();
    message.attachments.push(attachment);
    let before = runtime::AppState::lock(&data.state.stores).clone();

    for key in [CommandKey::DescribeImage, CommandKey::ReadImage] {
        let command = command_by_key(&commands, key);
        invoke_message_menu(&fixture, command, &data, message.clone(), false).await;
        let requests = fixture.take_requests();
        let content = assert_private_no_mentions_reply(&requests);
        assert!(content.contains('x'));
        assert!(content.chars().count() <= 2_000);
        assert!(
            requests
                .iter()
                .any(|request| request.route == "/fixture.png")
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.route.contains("ignored.invalid"))
        );
        assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
        assert_eq!(
            runtime::AppState::lock(&data.state.rewards).pending_len(),
            0
        );
    }

    let command = command_by_key(&commands, CommandKey::DescribeImage);
    invoke_message_menu(&fixture, command, &data, Message::default(), true).await;
    let requests = fixture.take_requests();
    let content = assert_private_no_mentions_reply(&requests);
    assert!(content.contains("no supported image attachment"));
    assert!(
        !requests
            .iter()
            .any(|request| request.route == "/v1/chat/completions")
    );
    assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
}

#[tokio::test]
async fn registered_ordinary_guards_deny_missing_access_after_acknowledgement() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(ordinary)
        .filter(|command| binding(command).eligibility.access != AccessId::A0)
    {
        assert!(
            !check(&fixture, command, &data, true, Some(OTHER)).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "GET")
                .count(),
            3,
            "{}",
            command.qualified_name
        );
        let body = requests.last().unwrap().body["content"].as_str().unwrap();
        assert!(
            [
                catalog::Blocker::Permission.message(),
                catalog::Blocker::VoicePresence.message()
            ]
            .contains(&body),
            "{body}"
        );
    }
}

#[tokio::test]
async fn registered_ordinary_guards_allow_current_access_and_capabilities() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands).into_iter().filter(ordinary) {
        assert!(
            check(&fixture, command, &data, true, Some(OTHER)).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "GET")
                .count(),
            if binding(command).eligibility.access == AccessId::A0 {
                0
            } else {
                3
            },
            "{}",
            command.qualified_name
        );
    }
}

#[tokio::test]
async fn registered_ordinary_guards_deny_missing_capabilities() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(ordinary)
        .filter(|command| binding(command).eligibility.condition != ConditionId::C0)
    {
        let mut data = configured_data();
        let state = Arc::get_mut(&mut data.state).unwrap();
        match binding(command).eligibility.condition {
            ConditionId::C1 | ConditionId::C8 | ConditionId::C5 | ConditionId::C6 => {
                state.providers.set_primary(None)
            }
            ConditionId::C2 | ConditionId::C9 | ConditionId::C3 => state.providers.clear_vision(),
            ConditionId::C4 => data.voice = None,
            other => panic!("uncovered condition: {other:?}"),
        }
        assert!(
            !check(&fixture, command, &data, true, None).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        let reason = match binding(command).eligibility.condition {
            ConditionId::C1 | ConditionId::C8 => catalog::Blocker::Generation,
            ConditionId::C2 | ConditionId::C3 => catalog::Blocker::Vision,
            ConditionId::C9 => catalog::Blocker::Ocr,
            ConditionId::C4 if binding(command).eligibility.access == AccessId::A5 => {
                catalog::Blocker::VoicePresence
            }
            ConditionId::C4 => catalog::Blocker::VoiceSetup,
            ConditionId::C5 | ConditionId::C6 => catalog::Blocker::VoiceMode,
            _ => unreachable!(),
        };
        assert_eq!(requests.last().unwrap().body["content"], reason.message());
    }
}

#[tokio::test]
async fn registered_voice_start_guards_deny_a_manager_absent_from_voice() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(false);
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A5)
    {
        assert!(
            !check(&fixture, command, &data, true, None).await,
            "{}",
            command.qualified_name
        );
    }
}

#[tokio::test]
async fn registered_voice_verification_guards_allow_the_application_owner() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let commands = crate::application_commands();
    let mut options = poise::FrameworkOptions::default();
    options.owners.insert(UserId::new(ACTOR));
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A7)
    {
        let invocation = Invocation::new(command, true, None);
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
                .unwrap(),
            "{}",
            command.qualified_name
        );
    }
}

#[tokio::test]
async fn registered_guild_only_guards_reject_dm_contexts() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(ordinary)
        .filter(|command| command.guild_only)
    {
        assert!(
            !check(&fixture, command, &data, false, None).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert!(requests.iter().all(|request| request.method != "GET"));
    }
}

#[tokio::test]
async fn registered_self_memory_guards_need_no_permission_rest() {
    let fixture = DiscordFixture::new().await;
    fixture.fail_permissions.store(true, Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A1)
    {
        let contexts: &[bool] = if command.guild_only {
            &[true]
        } else {
            &[false, true]
        };
        for &in_guild in contexts {
            for subject in [None, Some(ACTOR)] {
                assert!(
                    check(&fixture, command, &data, in_guild, subject).await,
                    "{} guild={in_guild} subject={subject:?}",
                    command.qualified_name
                );
                let requests = fixture.take_requests();
                assert_deferred_first(&requests, command);
                assert_eq!(requests.len(), 1, "{}", command.qualified_name);
            }
        }
    }
}

#[tokio::test]
async fn registered_memory_guards_reject_other_subjects_in_dms_without_permission_rest() {
    let fixture = DiscordFixture::new().await;
    fixture
        .permissions
        .store(Permissions::all().bits(), Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for command in leaves(&commands)
        .into_iter()
        .filter(|command| binding(command).eligibility.access == AccessId::A1)
    {
        assert!(
            !check(&fixture, command, &data, false, Some(OTHER)).await,
            "{}",
            command.qualified_name
        );
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert!(requests.iter().all(|request| request.method != "GET"));
    }
}

#[tokio::test]
async fn registered_guard_permission_failure_denies_privately_after_defer() {
    let fixture = DiscordFixture::new().await;
    fixture.fail_permissions.store(true, Ordering::SeqCst);
    let commands = crate::application_commands();
    let command = leaves(&commands)
        .into_iter()
        .find(|command| binding(command).key == CommandKey::AdminShow)
        .unwrap();
    assert!(!check(&fixture, command, &configured_data(), true, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    let denial = &requests.last().unwrap().body;
    assert_eq!(
        denial["content"],
        "Discord could not confirm the current permissions. Please try again."
    );
    assert!(denial["content"].as_str().unwrap().chars().count() <= 2000);
}

#[tokio::test]
async fn registered_guard_failed_acknowledgement_never_loads_permissions() {
    let fixture = DiscordFixture::new().await;
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    let commands = crate::application_commands();
    let command = leaves(&commands)
        .into_iter()
        .find(|command| binding(command).key == CommandKey::AdminShow)
        .unwrap();
    assert!(!check(&fixture, command, &configured_data(), true, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(requests.iter().all(|request| request.method != "GET"));
}

#[tokio::test]
async fn registered_autocomplete_guards_skip_defer_and_rest_and_keep_suggestions_self_scoped() {
    let fixture = DiscordFixture::new().await;
    let data = Data {
        state: runtime::AppState::in_memory(),
        voice: None,
    };
    {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        for (guild, actor, fact) in [
            ("discord:123", "discord:789", "my guild fact"),
            ("discord:123", "discord:790", "other member secret"),
            ("discord:124", "discord:789", "other guild secret"),
            ("discord:dm:789", "discord:789", "my dm fact"),
            ("discord:dm:790", "discord:790", "other dm secret"),
        ] {
            assert!(stores.memory.remember(guild, actor, fact, 1));
            assert!(
                stores
                    .memory
                    .propose_supersession(guild, actor, "replacement", fact, 1)
            );
        }
    }
    let commands = crate::application_commands();
    let options = poise::FrameworkOptions::default();
    let mut checked = 0;
    for command in leaves(&commands) {
        for parameter in &command.parameters {
            let Some(callback) = parameter.autocomplete_callback else {
                continue;
            };
            checked += 1;
            for in_guild in [false, true] {
                let invocation = Invocation::new(command, in_guild, Some(OTHER));
                let context = invocation.context(
                    &fixture,
                    command,
                    &options,
                    &data,
                    poise::CommandInteractionType::Autocomplete,
                );
                assert!(
                    command.checks[0](poise::Context::Application(context))
                        .await
                        .unwrap()
                );
                let response = serde_json::to_value(callback(context, "").await.unwrap()).unwrap();
                let actual: Vec<_> = response["choices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|choice| choice["value"].as_str().unwrap())
                    .collect();
                if binding(command).key == CommandKey::PersonaAsk {
                    assert_eq!(actual, crate::brain::intent::suggest_completions(""));
                } else {
                    assert_eq!(
                        actual,
                        vec![if in_guild {
                            "my guild fact"
                        } else {
                            "my dm fact"
                        }],
                        "{}",
                        command.qualified_name
                    );
                }
                assert!(!invocation.sent.load(Ordering::SeqCst));
                assert!(fixture.take_requests().is_empty());
            }
        }
    }
    assert_eq!(
        checked, 5,
        "all existing autocomplete callbacks are exercised"
    );
}

#[tokio::test]
async fn registered_help_guard_defers_privately_before_its_adapter_loads_permissions() {
    let fixture = DiscordFixture::new().await;
    let commands = crate::application_commands();
    let command = leaves(&commands)
        .into_iter()
        .find(|command| binding(command).key == CommandKey::Help)
        .unwrap();
    assert!(check(&fixture, command, &configured_data(), true, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn registered_voice_join_and_resume_stop_when_acknowledgement_fails() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for key in [CommandKey::VoiceJoin, CommandKey::VoiceResume] {
        let command = command_by_key(&commands, key);
        assert!(invoke_voice_slash_fails(&fixture, command, &data, Some(true)).await);
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert_eq!(requests.len(), 1, "{}", command.qualified_name);
    }
}

#[tokio::test]
async fn registered_voice_join_and_resume_do_not_mutate_lifecycle_while_acknowledgement_waits() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let data = configured_data();
    let runtime = data.voice.as_ref().unwrap();
    let commands = crate::application_commands();

    for key in [CommandKey::VoiceJoin, CommandKey::VoiceResume] {
        let command = command_by_key(&commands, key);
        let invocation = Invocation::voice(command, Some(true));
        let args = invocation.interaction.data.options();
        let options = poise::FrameworkOptions::default();
        let context = invocation.context_with_args(
            &fixture,
            command,
            &options,
            &data,
            poise::CommandInteractionType::Command,
            &args,
        );
        let before = runtime.snapshot().await;
        let action = command.slash_action.unwrap()(context);
        tokio::pin!(action);
        tokio::select! {
            permit = fixture.acknowledgement_entered.acquire() => permit.unwrap().forget(),
            _ = &mut action => panic!("{} completed before held acknowledgement", command.qualified_name),
        }
        let waiting = runtime.snapshot().await;
        assert_eq!(waiting.epoch, before.epoch, "{}", command.qualified_name);
        assert_eq!(waiting.phase, before.phase, "{}", command.qualified_name);
        assert_eq!(
            waiting.media_enabled, before.media_enabled,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            waiting.start_pending, before.start_pending,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            waiting.consent_epoch, before.consent_epoch,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            waiting.participant_count, before.participant_count,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            fixture.take_requests().len(),
            1,
            "{}",
            command.qualified_name
        );
        fixture.acknowledgement_release.add_permits(1);
        let _ = action.await;
        fixture.take_requests();
    }
}

#[tokio::test]
async fn registered_authorized_voice_leave_closes_pending_media_before_teardown_awaits() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    let data = configured_data();
    let runtime = data.voice.as_ref().unwrap();
    runtime.reserve_start();
    assert!(runtime.snapshot().await.start_pending);
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::VoiceLeave);

    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let transition = Arc::new(crate::commands_voice::VoiceLeaveTransitionProbe::new());
    fixture
        .context
        .data
        .write()
        .await
        .insert::<crate::commands_voice::VoiceLeaveTransitionProbeKey>(Arc::clone(&transition));
    let invocation = Invocation::voice(command, None);
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        &fixture,
        command,
        &options,
        &data,
        poise::CommandInteractionType::Command,
        &args,
    );
    let action = command.slash_action.unwrap()(context);
    tokio::pin!(action);
    let (acknowledgement, teardown) = tokio::select! {
        entered = async {
            tokio::join!(
                fixture.acknowledgement_entered.acquire(),
                transition.entered.acquire()
            )
        } => entered,
        _ = &mut action => panic!("voice leave completed before held branches entered"),
    };
    acknowledgement.unwrap().forget();
    teardown.unwrap().forget();

    // Both awaited branches have started and remain independently blocked.
    // The real adapter must already have performed the synchronous close.
    assert!(!runtime.snapshot().await.start_pending);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert_eq!(requests.len(), 1);
    fixture.acknowledgement_release.add_permits(1);
    transition.release.add_permits(1);
    assert!(action.await.is_err());
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

#[tokio::test]
async fn actual_member_voice_status_hides_channel_and_runs_no_provider_probe() {
    let fixture = DiscordFixture::new().await;
    let providers = ProviderFixture::new().await;
    fixture
        .permissions
        .store(Permissions::empty().bits(), Ordering::SeqCst);
    let data = configured_data_at(Some(providers.address));
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::VoiceStatus);
    assert!(!invoke_voice_slash_fails(&fixture, command, &data, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    let body = requests
        .iter()
        .find_map(|request| request.body["content"].as_str())
        .expect("member status response");
    assert!(body.contains("configured channel hidden"), "{body}");
    assert!(!body.contains(&CHANNEL.to_string()), "{body}");
    for forbidden in [
        "epoch",
        "model",
        "endpoint",
        "queue",
        "participant",
        "verifier",
    ] {
        assert!(!body.to_ascii_lowercase().contains(forbidden), "{body}");
    }
    assert!(
        !requests
            .iter()
            .any(|request| request.route.contains("health")
                || request.route.contains("models")
                || request.route.contains("chat/completions"))
    );
    assert_eq!(providers.calls.load(Ordering::SeqCst), 0);
}

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
        let body = assert_private_help_response(&requests);
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
    let body = assert_private_help_response(&requests);
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
        assert_private_help_response(&requests)["content"]
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
        let body = assert_private_help_response(&requests);
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
    assert_private_help_response(&requests);
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
        assert_private_help_response(&requests)["content"]
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
    let body = assert_private_help_response(&requests);
    assert!(body["content"].as_str().unwrap().contains("Page 1 of 1"));
    assert!(body["content"].as_str().unwrap().contains("fact 0"));
    assert_eq!(body["components"], json!([]));
}

async fn stats_output(fixture: &DiscordFixture, data: &Data, in_guild: bool) -> String {
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Stats);
    let invocation = Invocation::new(command, in_guild, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    assert!(command.slash_action.unwrap()(context).await.is_ok());
    let requests = fixture.take_requests();
    assert_private_no_mentions_reply(&requests).to_string()
}

#[tokio::test]
async fn registered_stats_ignores_other_guilds_and_dms_but_keeps_own_brain_and_budget() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let guild_before = stats_output(&fixture, &data, true).await;
    let dm_before = stats_output(&fixture, &data, false).await;
    assert!(guild_before.starts_with("This server"));
    assert!(dm_before.starts_with("Your DM"));
    {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        stores.memory.messages_seen += 900;
        stores
            .memory
            .interactions
            .record(crate::memory::InteractionEntry::new(
                "stats", true, None, 1, 1000,
            ));
        for scope in ["discord:other-guild", "discord:dm:790", "discord:dm:791"] {
            stores
                .memory
                .record_message(scope, "unrelated-user", "private unrelated activity", 1);
            runtime::AppState::lock(&data.state.rewards).register_reply(
                vec![0.5],
                1,
                scope,
                scope,
                1,
            );
            runtime::AppState::lock(&data.state.brains)
                .brain(scope, &*stores, runtime::now())
                .set_epsilon(0.8);
            runtime::AppState::lock(&data.state.budget).try_take(scope, 6, runtime::now());
        }
    }
    assert_eq!(stats_output(&fixture, &data, true).await, guild_before);
    assert_eq!(stats_output(&fixture, &data, false).await, dm_before);
    {
        let stores = runtime::AppState::lock(&data.state.stores);
        runtime::AppState::lock(&data.state.brains)
            .brain(&format!("discord:{GUILD}"), &*stores, runtime::now())
            .set_epsilon(0.123);
        assert!(runtime::AppState::lock(&data.state.budget).try_take(
            &format!("discord:{GUILD}"),
            6,
            runtime::now()
        ));
    }
    let changed = stats_output(&fixture, &data, true).await;
    assert_ne!(changed, guild_before);
    assert!(changed.contains("0.123"));
    assert!(changed.contains("5.0 of 6/h"));
    assert_eq!(stats_output(&fixture, &data, false).await, dm_before);
}

#[tokio::test]
async fn registered_image_failure_uses_typed_private_member_guidance() {
    let fixture = DiscordFixture::new().await;
    let provider = ProviderFixture::new().await;
    let data = configured_data_at(Some(provider.address));
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::DescribeImage);
    let mut message = Message::default();
    message.attachments.push(serde_json::from_value(json!({
        "id":"1", "filename":"image.png", "size":100,
        "url":format!("http://{}/fixture.png",fixture.address), "proxy_url":"https://private.invalid/canary"
    })).unwrap());
    invoke_message_menu(&fixture, command, &data, message, false).await;
    let requests = fixture.take_requests();
    let content = assert_private_no_mentions_reply(&requests);
    assert!(content.contains("Ask a server manager"), "{content}");
    for forbidden in [
        "logs",
        "credentials",
        "127.0.0.1",
        "fixture",
        "private.invalid",
        "ABBEY_",
    ] {
        assert!(!content.contains(forbidden));
    }
    assert!(provider.calls.load(Ordering::SeqCst) > 0);
}

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

#[tokio::test]
async fn actual_unconfigured_voice_status_reaches_explanatory_handler() {
    let fixture = DiscordFixture::new().await;
    let mut data = configured_data();
    data.voice = None;
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::VoiceStatus);
    assert!(!invoke_voice_slash_fails(&fixture, command, &data, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(requests.iter().any(|request| {
        request.body["content"].as_str().is_some_and(|body| {
            body.contains(
                "No voice session is prepared in this server. A manager in a voice channel can use /voice join first.",
            )
        })
    }));
}

#[path = "workflows/dispatch_tests.rs"]
mod workflow_dispatch_tests;
