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

fn private_ephemeral_patch(requests: &[Request]) -> &Value {
    assert_eq!(requests[0].body["type"], 5);
    assert_eq!(requests[0].body["data"]["flags"], 64);
    let reply = &requests
        .iter()
        .find(|request| request.method == "PATCH")
        .unwrap()
        .body;
    assert_eq!(reply["allowed_mentions"]["parse"], json!([]));
    assert_eq!(reply["allowed_mentions"]["replied_user"], false);
    reply
}

/// Memory-browser and similar private patches still use plain content.
fn assert_private_content_response(requests: &[Request]) -> &Value {
    let reply = private_ephemeral_patch(requests);
    assert!(reply["content"].as_str().unwrap().chars().count() <= 2000);
    reply
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

mod admin_tests;
mod guard_tests;
mod help_tests;
mod memory_browser_tests;
mod menu_tests;
mod pending_tests;
mod stats_tests;
mod voice_tests;

#[path = "workflows/dispatch_tests.rs"]
mod workflow_dispatch_tests;
