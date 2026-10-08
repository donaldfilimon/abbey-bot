use super::*;
use serde_json::{Value, json};
use serenity::all::{
    Guild, Member, PermissionOverwrite, PermissionOverwriteType, Role, RoleId, User,
};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct Scenario {
    owner: u64,
    actor_can_manage: bool,
    actor_can_view: bool,
    bot_can_manage: bool,
    thread_guild: u64,
    parent_guild: u64,
    archived: bool,
    locked: bool,
    status_tags: Vec<(u64, &'static str)>,
    moderated_tags: Vec<u64>,
    initial_tags: Vec<u64>,
    current_tags: Vec<u64>,
    final_owner: Option<u64>,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            owner: 7,
            actor_can_manage: false,
            actor_can_view: true,
            bot_can_manage: true,
            thread_guild: 1,
            parent_guild: 1,
            archived: false,
            locked: false,
            status_tags: vec![(10, "Solved"), (11, "Unresolved")],
            moderated_tags: Vec::new(),
            initial_tags: vec![20, 11],
            current_tags: vec![20, 11, 21],
            final_owner: None,
        }
    }
}

#[derive(Debug)]
struct Request {
    method: String,
    route: String,
    body: Value,
}

struct Fixture {
    http: Http,
    requests: Arc<Mutex<Vec<Request>>>,
    server: tokio::task::JoinHandle<()>,
}

fn thread(scenario: &Scenario, final_fetch: bool) -> GuildChannel {
    let mut thread = GuildChannel::default();
    thread.id = ChannelId::new(3);
    thread.guild_id = GuildId::new(scenario.thread_guild);
    thread.kind = ChannelType::PublicThread;
    thread.owner_id = Some(UserId::new(if final_fetch {
        scenario.final_owner.unwrap_or(scenario.owner)
    } else {
        scenario.owner
    }));
    thread.parent_id = Some(ChannelId::new(2));
    thread.thread_metadata = Some(
        serde_json::from_value(json!({
            "archived": scenario.archived,
            "locked": scenario.locked,
            "auto_archive_duration": 1440,
            "archive_timestamp": null,
            "create_timestamp": null
        }))
        .unwrap(),
    );
    thread.applied_tags = if final_fetch {
        &scenario.current_tags
    } else {
        &scenario.initial_tags
    }
    .iter()
    .copied()
    .map(ForumTagId::new)
    .collect();
    thread
}

fn parent(scenario: &Scenario) -> GuildChannel {
    let mut parent = GuildChannel::default();
    parent.id = ChannelId::new(2);
    parent.guild_id = GuildId::new(scenario.parent_guild);
    parent.kind = ChannelType::Forum;
    parent.available_tags = scenario
        .status_tags
        .iter()
        .map(|(id, name)| {
            serde_json::from_value(json!({
                "id": id.to_string(), "name": name,
                "moderated": scenario.moderated_tags.contains(id),
                "emoji_id": null, "emoji_name": null
            }))
            .unwrap()
        })
        .collect();
    if !scenario.actor_can_view {
        parent.permission_overwrites.push(PermissionOverwrite {
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL,
            kind: PermissionOverwriteType::Member(UserId::new(7)),
        });
    }
    parent
}

fn guild() -> Guild {
    let mut guild = Guild::default();
    guild.id = GuildId::new(1);
    guild.owner_id = UserId::new(999);
    for (id, permissions) in [
        (1, Permissions::VIEW_CHANNEL),
        (50, Permissions::MANAGE_THREADS),
    ] {
        let mut role = Role::default();
        role.id = RoleId::new(id);
        role.permissions = permissions;
        guild.roles.insert(role.id, role);
    }
    guild
}

impl Fixture {
    async fn new(scenario: Scenario) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            let mut thread_fetches = 0;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
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
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                }
                let mut first = headers.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_owned();
                let route = first.next().unwrap().to_owned();
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                };
                let response = if route.ends_with("/channels/3") && method == "GET" {
                    thread_fetches += 1;
                    serde_json::to_value(thread(&scenario, thread_fetches > 1)).unwrap()
                } else if route.ends_with("/channels/3") && method == "PATCH" {
                    let mut updated = thread(&scenario, true);
                    updated.applied_tags =
                        serde_json::from_value(body["applied_tags"].clone()).unwrap();
                    serde_json::to_value(updated).unwrap()
                } else if route.ends_with("/channels/2") {
                    serde_json::to_value(parent(&scenario)).unwrap()
                } else if route.ends_with("/guilds/1") {
                    serde_json::to_value(guild()).unwrap()
                } else if route.ends_with("/users/@me") {
                    let mut bot = User::default();
                    bot.id = UserId::new(99);
                    bot.bot = true;
                    serde_json::to_value(bot).unwrap()
                } else if route.contains("/guilds/1/members/") {
                    let id = route.rsplit('/').next().unwrap().parse::<u64>().unwrap();
                    let mut member = Member::default();
                    member.user.id = UserId::new(id);
                    member.user.name = "same-display-name".into();
                    if (id == 7 && scenario.actor_can_manage)
                        || (id == 99 && scenario.bot_can_manage)
                    {
                        member.roles.push(RoleId::new(50));
                    }
                    serde_json::to_value(member).unwrap()
                } else {
                    panic!("unexpected fixture route {method} {route}");
                };
                recorded.lock().unwrap().push(Request {
                    method,
                    route,
                    body,
                });
                let response = serde_json::to_vec(&response).unwrap();
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(&response).await.unwrap();
            }
        });
        let http = serenity::http::HttpBuilder::new("offline-forum-fixture")
            .client(
                reqwest::Client::builder()
                    .no_proxy()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .unwrap(),
            )
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build();
        Self {
            http,
            requests,
            server,
        }
    }

    async fn resolve(&self, status: ResolutionState) -> Result<bool, ResolveError> {
        resolve_thread(
            &self.http,
            GuildId::new(1),
            ChannelId::new(3),
            UserId::new(7),
            status,
        )
        .await
    }

    fn mutations(&self) -> Vec<Value> {
        let requests = self.requests.lock().unwrap();
        assert!(requests.iter().all(|request| {
            request.method == "GET"
                || (request.method == "PATCH" && request.route.ends_with("/channels/3"))
        }));
        requests
            .iter()
            .filter(|request| request.method == "PATCH")
            .map(|request| request.body.clone())
            .collect()
    }

    async fn finish(self) {
        self.server.abort();
        assert!(self.server.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn forum_resolution_rest_preserves_latest_unrelated_tags_and_only_patches_tags() {
    let fixture = Fixture::new(Scenario::default()).await;
    assert_eq!(fixture.resolve(ResolutionState::Solved).await, Ok(true));
    assert_eq!(
        fixture.mutations(),
        vec![json!({"applied_tags": ["20", "21", "10"]})]
    );
    fixture.finish().await;
}

#[tokio::test]
async fn forum_resolution_same_display_name_without_author_snowflake_is_denied() {
    let fixture = Fixture::new(Scenario {
        owner: 8,
        ..Scenario::default()
    })
    .await;
    assert_eq!(
        fixture.resolve(ResolutionState::Solved).await,
        Err(ResolveError::Denied)
    );
    assert!(fixture.mutations().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn forum_resolution_current_manager_can_change_other_authors_post() {
    let fixture = Fixture::new(Scenario {
        owner: 8,
        actor_can_manage: true,
        ..Scenario::default()
    })
    .await;
    assert_eq!(
        fixture.resolve(ResolutionState::Unresolved).await,
        Ok(false)
    );
    assert!(fixture.mutations().is_empty());
    assert_eq!(fixture.resolve(ResolutionState::Solved).await, Ok(true));
    assert_eq!(fixture.mutations().len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn forum_resolution_current_permissions_and_final_author_recheck_refuse() {
    for scenario in [
        Scenario {
            actor_can_view: false,
            ..Scenario::default()
        },
        Scenario {
            bot_can_manage: false,
            ..Scenario::default()
        },
        Scenario {
            final_owner: Some(8),
            ..Scenario::default()
        },
    ] {
        let fixture = Fixture::new(scenario).await;
        assert_eq!(
            fixture.resolve(ResolutionState::Solved).await,
            Err(ResolveError::Denied)
        );
        assert!(fixture.mutations().is_empty());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn forum_resolution_cross_guild_or_inactive_thread_never_mutates() {
    for (scenario, expected) in [
        (
            Scenario {
                thread_guild: 8,
                ..Scenario::default()
            },
            ResolveError::InvalidThread,
        ),
        (
            Scenario {
                parent_guild: 8,
                ..Scenario::default()
            },
            ResolveError::InvalidThread,
        ),
        (
            Scenario {
                archived: true,
                ..Scenario::default()
            },
            ResolveError::InactiveThread,
        ),
        (
            Scenario {
                locked: true,
                ..Scenario::default()
            },
            ResolveError::InactiveThread,
        ),
    ] {
        let fixture = Fixture::new(scenario).await;
        assert_eq!(
            fixture.resolve(ResolutionState::Solved).await,
            Err(expected)
        );
        assert!(fixture.mutations().is_empty());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn forum_resolution_missing_duplicate_tags_and_capacity_refuse_without_loss() {
    for scenario in [
        Scenario {
            status_tags: vec![(10, "Solved")],
            ..Scenario::default()
        },
        Scenario {
            status_tags: vec![(10, "Solved"), (12, "SOLVED"), (11, "Unresolved")],
            ..Scenario::default()
        },
        Scenario {
            current_tags: vec![20, 21, 22, 23, 24],
            ..Scenario::default()
        },
    ] {
        let fixture = Fixture::new(scenario).await;
        assert_eq!(
            fixture.resolve(ResolutionState::Solved).await,
            Err(ResolveError::InvalidTags)
        );
        assert!(fixture.mutations().is_empty());
        fixture.finish().await;
    }
}

#[test]
fn forum_resolution_replies_are_static_bounded_and_readable() {
    for status in [ResolutionState::Solved, ResolutionState::Unresolved] {
        for result in [
            Ok(true),
            Ok(false),
            Err(ResolveError::InvalidThread),
            Err(ResolveError::InactiveThread),
            Err(ResolveError::Denied),
            Err(ResolveError::InvalidTags),
            Err(ResolveError::Unavailable),
        ] {
            let text = reply(result, status);
            assert_eq!(crate::commands::clamp_message(text.to_string()), text);
            println!("{text}");
        }
    }
}

#[tokio::test]
async fn forum_resolution_moderated_status_add_requires_current_manager() {
    let fixture = Fixture::new(Scenario {
        moderated_tags: vec![10],
        ..Scenario::default()
    })
    .await;
    assert_eq!(
        fixture.resolve(ResolutionState::Solved).await,
        Err(ResolveError::Denied)
    );
    assert!(fixture.mutations().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn forum_resolution_moderated_status_removal_requires_current_manager() {
    let fixture = Fixture::new(Scenario {
        moderated_tags: vec![11],
        ..Scenario::default()
    })
    .await;
    assert_eq!(
        fixture.resolve(ResolutionState::Solved).await,
        Err(ResolveError::Denied)
    );
    assert!(fixture.mutations().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn forum_resolution_moderated_current_manager_can_change_status() {
    let fixture = Fixture::new(Scenario {
        owner: 8,
        actor_can_manage: true,
        moderated_tags: vec![10, 11],
        ..Scenario::default()
    })
    .await;
    assert_eq!(fixture.resolve(ResolutionState::Solved).await, Ok(true));
    assert_eq!(
        fixture.mutations(),
        vec![json!({"applied_tags": ["20", "21", "10"]})]
    );
    fixture.finish().await;
}

#[tokio::test]
async fn forum_resolution_moderated_unchanged_status_and_unrelated_tags_are_preserved() {
    for (scenario, expected) in [
        (
            Scenario {
                current_tags: vec![20, 10, 21],
                moderated_tags: vec![10],
                ..Scenario::default()
            },
            false,
        ),
        (
            Scenario {
                status_tags: vec![(10, "Solved"), (11, "Unresolved"), (20, "Rust")],
                moderated_tags: vec![20],
                ..Scenario::default()
            },
            true,
        ),
    ] {
        let fixture = Fixture::new(scenario).await;
        assert_eq!(fixture.resolve(ResolutionState::Solved).await, Ok(expected));
        if expected {
            assert_eq!(
                fixture.mutations(),
                vec![json!({"applied_tags": ["20", "21", "10"]})]
            );
        } else {
            assert!(fixture.mutations().is_empty());
        }
        fixture.finish().await;
    }
}
