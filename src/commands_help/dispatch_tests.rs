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
    server: tokio::task::JoinHandle<()>,
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
        let server = {
            let requests = Arc::clone(&requests);
            let permissions = Arc::clone(&permissions);
            let fail_permissions = Arc::clone(&fail_permissions);
            let fail_acknowledgement = Arc::clone(&fail_acknowledgement);
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
                    let body = if length == 0 {
                        Value::Null
                    } else {
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                    };
                    let is_acknowledgement = body["type"] == 5;
                    let is_permission_lookup = method == "GET";
                    let is_callback = route.ends_with("/callback");
                    let response = if is_permission_lookup && route.contains("/members/") {
                        let mut member = Member::default();
                        member.user = user(ACTOR);
                        member.guild_id = GuildId::new(GUILD);
                        serde_json::to_value(member).unwrap()
                    } else if is_permission_lookup && route.contains("/guilds/") {
                        serde_json::to_value(guild(Permissions::from_bits_retain(
                            permissions.load(Ordering::SeqCst),
                        )))
                        .unwrap()
                    } else if is_permission_lookup && route.contains("/channels/") {
                        let mut channel = GuildChannel::default();
                        channel.id = ChannelId::new(CHANNEL);
                        channel.guild_id = GuildId::new(GUILD);
                        serde_json::to_value(channel).unwrap()
                    } else if is_callback {
                        Value::Null
                    } else {
                        assert!(route.contains("/webhooks/"), "unexpected route: {route}");
                        let mut message = Message::default();
                        message.content = body["content"].as_str().unwrap_or_default().into();
                        serde_json::to_value(message).unwrap()
                    };
                    requests.lock().unwrap().push(Request {
                        method,
                        route,
                        body,
                    });
                    let failed = (is_permission_lookup && fail_permissions.load(Ordering::SeqCst))
                        || (is_acknowledgement && fail_acknowledgement.load(Ordering::SeqCst));
                    let (status, response) = if failed {
                        (
                            "403 Forbidden",
                            json!({"code": 50013, "message": "fixture denied"}).to_string(),
                        )
                    } else if is_callback {
                        ("204 No Content", String::new())
                    } else {
                        ("200 OK", response.to_string())
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    );
                    // Failed permission joins may cancel other already-issued requests.
                    let _ = stream.write_all(reply.as_bytes()).await;
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
            server,
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

impl Drop for DiscordFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn configured_data() -> Data {
    let mut data = Data {
        state: runtime::AppState::in_memory(),
        voice: None,
    };
    let state = Arc::get_mut(&mut data.state).unwrap();
    state.backend = Some(crate::llm::Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:1".into(),
        model: "fixture".into(),
    });
    state.vision = Some(crate::vision::ConfiguredVision::Remote(
        crate::vision::RemoteVision {
            config: crate::vision::VisionConfig {
                base_url: "http://127.0.0.1:1/v1".into(),
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

    fn context<'a>(
        &'a self,
        fixture: &'a DiscordFixture,
        command: &'a poise::Command<Data, Error>,
        options: &'a poise::FrameworkOptions<Data, Error>,
        data: &'a Data,
        interaction_type: poise::CommandInteractionType,
    ) -> poise::ApplicationContext<'a, Data, Error> {
        poise::ApplicationContext {
            serenity_context: &fixture.context,
            interaction: &self.interaction,
            interaction_type,
            args: &[],
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

fn assert_deferred_first(requests: &[Request], command: &poise::Command<Data, Error>) {
    assert_eq!(requests[0].method, "POST", "{}", command.qualified_name);
    assert!(requests[0].route.ends_with("/callback"));
    assert_eq!(requests[0].body["type"], 5, "{}", command.qualified_name);
    assert_eq!(
        requests[0].body["data"]["flags"].as_u64().unwrap_or(0) & 64 != 0,
        command.ephemeral
    );
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
        for in_guild in [false, true] {
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
