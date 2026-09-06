//! Exercise registered Poise checks with real contexts and loopback-only transports.
use super::*;
use catalog::{AccessId, ConditionId};
use serde_json::{Value, json};
use serenity::all::{
    ApplicationId, Cache, ChannelId, CommandInteraction, GatewayIntents, Guild, GuildChannel,
    GuildCreateEvent, Member, Message, Role, RoleId, Shard, ShardId, ShardInfo, ShardManager,
    ShardManagerOptions, ShardMessenger, ShardRunner, ShardRunnerOptions, User,
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
    hold_acknowledgement: Arc<AtomicBool>,
    acknowledgement_entered: Arc<tokio::sync::Semaphore>,
    acknowledgement_release: Arc<tokio::sync::Semaphore>,
    server: tokio::task::JoinHandle<()>,
    address: std::net::SocketAddr,
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
        let hold_acknowledgement = Arc::new(AtomicBool::new(false));
        let acknowledgement_entered = Arc::new(tokio::sync::Semaphore::new(0));
        let acknowledgement_release = Arc::new(tokio::sync::Semaphore::new(0));
        let server = {
            let requests = Arc::clone(&requests);
            let permissions = Arc::clone(&permissions);
            let fail_permissions = Arc::clone(&fail_permissions);
            let fail_acknowledgement = Arc::clone(&fail_acknowledgement);
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
                    let is_acknowledgement = body["type"] == 5;
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
                            json!({"choices":[{"message":{"content":"x".repeat(3_000)}}]})
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
                    requests.lock().unwrap().push(Request {
                        method,
                        route,
                        body,
                    });
                    if is_acknowledgement && hold_acknowledgement.load(Ordering::SeqCst) {
                        acknowledgement_entered.add_permits(1);
                        acknowledgement_release.acquire().await.unwrap().forget();
                    }
                    let failed = (is_permission_lookup && fail_permissions.load(Ordering::SeqCst))
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
    state.backend = Some(crate::llm::Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:1".into(),
        model: "fixture".into(),
    });
    if address.is_some() {
        state.attachments = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
    }
    state.vision = Some(crate::vision::ConfiguredVision::Remote(
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
                crate::offline_voice::OfflineVoiceConfig::from_values(None, None, None, None, None)
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
        assert!(
            requests.last().unwrap().body["content"]
                .as_str()
                .unwrap()
                .contains("unavailable")
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
            ConditionId::C1 | ConditionId::C5 | ConditionId::C6 => state.backend = None,
            ConditionId::C2 | ConditionId::C3 => state.vision = None,
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
        assert!(
            requests.last().unwrap().body["content"]
                .as_str()
                .unwrap()
                .contains("unavailable")
        );
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

    let current = crate::guild::scoped_channel_id("discord", &CHANNEL.to_string());
    let other = crate::guild::scoped_channel_id("discord", "457");
    crate::runtime::AppState::lock(&data.state.engine).commit(&current, "one", "reply", 1);
    crate::runtime::AppState::lock(&data.state.engine).commit(&other, "two", "reply", 1);
    let interaction = admin_component(&fixture, &session, AdminAction::RequestReset, CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    assert_eq!(
        crate::runtime::AppState::lock(&data.state.engine).session_len(&current),
        2
    );
    fixture.take_requests();
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
    fixture.take_requests();
    let interaction = admin_component(&fixture, &session, AdminAction::ConfirmReset, CHANNEL);
    crate::commands_brain::dispatch_admin_component(&fixture.context, &interaction, &data).await;
    assert!(fixture.take_requests().iter().any(|request| {
        request.body["content"]
            .as_str()
            .is_some_and(|body| body.contains("already clear"))
    }));

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
async fn actual_member_voice_status_hides_channel_and_runs_no_provider_probe() {
    let fixture = DiscordFixture::new().await;
    fixture
        .permissions
        .store(Permissions::empty().bits(), Ordering::SeqCst);
    let data = configured_data();
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
}
