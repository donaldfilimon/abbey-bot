//! Real loopback Serenity routes; fixture owns and joins every socket handler.
use super::*;
use serenity::all::{ChannelId, ChannelType, PrivateChannel, UserId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

async fn dm_access(body: serde_json::Value) -> (Result<WorkAccess, WorkError>, Vec<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let server = tokio::spawn(async move {
        let mut routes = Vec::new();
        loop {
            let mut socket = tokio::select! {
                biased;
                _ = stopped.cancelled() => break,
                accepted = listener.accept() => accepted.unwrap().0,
            };
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0 && request.len() < 65536);
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            routes.push(request.lines().next().unwrap().to_owned());
            let response = serde_json::to_vec(&body).unwrap();
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len(),
            ).as_bytes()).await.unwrap();
            socket.write_all(&response).await.unwrap();
        }
        routes
    });
    let http = serenity::http::HttpBuilder::new("offline-continuity-fixture")
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
    let result = DiscordContinuityAccess(Arc::new(http))
        .authorize(&WorkScope::Personal { owner: 7 }, 7, 70)
        .await;
    stop.cancel();
    (result, server.await.unwrap())
}
fn dm(owner: u64, channel: u64) -> serde_json::Value {
    let mut dm = PrivateChannel::default();
    dm.id = ChannelId::new(channel);
    dm.kind = ChannelType::Private;
    dm.recipient.id = UserId::new(owner);
    serde_json::to_value(dm).unwrap()
}
#[tokio::test]
async fn continuity_native_owner_proof_is_one_get_without_dm_creation() {
    let (access, routes) = dm_access(dm(7, 70)).await;
    let access = access.unwrap();
    assert_eq!(access.actor, 7);
    assert_eq!(access.channel, 70);
    assert!(access.guild.is_none() && access.can_view);
    assert_eq!(routes.len(), 1);
    assert!(routes[0].starts_with("GET ") && routes[0].contains("/channels/70 "));
}
#[tokio::test]
async fn continuity_native_mismatched_dm_identity_refuses() {
    for body in [dm(8, 70), dm(7, 80)] {
        let (access, routes) = dm_access(body).await;
        assert!(access.is_err());
        assert_eq!(routes.len(), 1);
    }
}

mod team_rest {
    use crate::{
        gateway::continuity_access::DiscordContinuityAccess,
        runtime::continuity_context::ContinuityAccessProvider,
        work::{WorkAccess, WorkError, WorkScope},
    };
    use serenity::all::{
        Channel, ChannelId, ChannelType, Guild, GuildChannel, GuildId, Member, PartialGuild,
        PermissionOverwrite, PermissionOverwriteType, Permissions, Role, RoleId, UserId,
    };
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        task::JoinSet,
    };
    use tokio_util::sync::CancellationToken;

    const GUILD: u64 = 1;
    const ACTOR: u64 = 7;
    const CHANNEL: u64 = 20;
    const MEMBER_ROLE: u64 = 50;

    #[derive(Clone)]
    struct Scenario {
        member_id: u64,
        member_is_bot: bool,
        returned_guild_id: u64,
        channel_id: u64,
        channel_guild: u64,
        channel_kind: ChannelType,
        everyone_exists: bool,
        member_role_exists: bool,
        member_roles: Vec<u64>,
        everyone_permissions: Permissions,
        role_permissions: Permissions,
        overwrites: Vec<PermissionOverwrite>,
    }
    impl Default for Scenario {
        fn default() -> Self {
            Self {
                member_id: ACTOR,
                member_is_bot: false,
                returned_guild_id: GUILD,
                channel_id: CHANNEL,
                channel_guild: GUILD,
                channel_kind: ChannelType::Text,
                everyone_exists: true,
                member_role_exists: true,
                member_roles: vec![MEMBER_ROLE],
                // VIEW comes from the currently returned assigned role, not
                // from @everyone or a supplied interaction permission hint.
                everyone_permissions: Permissions::empty(),
                role_permissions: Permissions::VIEW_CHANNEL,
                overwrites: Vec::new(),
            }
        }
    }
    #[derive(Clone)]
    struct Responses {
        member: serde_json::Value,
        guild: serde_json::Value,
        channel: serde_json::Value,
    }
    fn responses(scenario: &Scenario) -> Responses {
        let mut member = Member::default();
        member.user.id = UserId::new(scenario.member_id);
        member.user.name = "same-display-name".into();
        member.user.bot = scenario.member_is_bot;
        member.guild_id = GuildId::new(GUILD);
        member.roles = scenario
            .member_roles
            .iter()
            .copied()
            .map(RoleId::new)
            .collect();
        // REST computation must ignore this stale/interaction-only hint.
        member.permissions = Some(Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD);

        let mut guild = Guild::default();
        guild.id = GuildId::new(scenario.returned_guild_id);
        guild.owner_id = UserId::new(999);
        for (id, exists, permissions) in [
            (
                GUILD,
                scenario.everyone_exists,
                scenario.everyone_permissions,
            ),
            (
                MEMBER_ROLE,
                scenario.member_role_exists,
                scenario.role_permissions,
            ),
        ] {
            if exists {
                let mut role = Role::default();
                role.id = RoleId::new(id);
                role.guild_id = GuildId::new(GUILD);
                role.name = "same-display-name".into();
                role.permissions = permissions;
                guild.roles.insert(role.id, role);
            }
        }

        let mut channel = GuildChannel::default();
        channel.id = ChannelId::new(scenario.channel_id);
        channel.guild_id = GuildId::new(scenario.channel_guild);
        channel.kind = scenario.channel_kind;
        channel.name = "continuity-fixture".into();
        channel.permission_overwrites = scenario.overwrites.clone();
        if matches!(
            scenario.channel_kind,
            ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread
        ) {
            channel.parent_id = Some(ChannelId::new(30));
            channel.owner_id = Some(UserId::new(ACTOR));
            channel.thread_metadata = Some(
                serde_json::from_value(serde_json::json!({
                    "archived": false, "locked": false, "auto_archive_duration": 1440,
                    "archive_timestamp": null, "create_timestamp": null
                }))
                .unwrap(),
            );
        }
        let values = Responses {
            member: serde_json::to_value(member).unwrap(),
            guild: serde_json::to_value(PartialGuild::from(guild)).unwrap(),
            channel: serde_json::to_value(channel).unwrap(),
        };
        // Characterize malformed-fixture failures before starting any sockets.
        let _: Member = serde_json::from_value(values.member.clone()).unwrap();
        let _: PartialGuild = serde_json::from_value(values.guild.clone()).unwrap();
        let _: Channel = serde_json::from_value(values.channel.clone()).unwrap();
        values
    }
    #[derive(Clone, Debug)]
    struct RequestLine {
        method: String,
        path: String,
    }
    async fn handle(
        mut socket: tokio::net::TcpStream,
        response_state: Arc<Mutex<Responses>>,
        requests: Arc<Mutex<Vec<RequestLine>>>,
        wave: Arc<tokio::sync::Barrier>,
    ) -> Result<(), String> {
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if count == 0 || bytes.len() + count > 65536 {
                return Err("fixture request closed or exceeded header bound".into());
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end]).map_err(|e| e.to_string())?;
        let first = headers.lines().next().ok_or("missing request line")?;
        let mut words = first.split_whitespace();
        let method = words.next().ok_or("missing method")?.to_owned();
        let path = words.next().ok_or("missing path")?.to_owned();
        let content_length = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map_or(Ok(0), |(_, value)| value.trim().parse::<usize>())
            .map_err(|e| e.to_string())?;
        requests.lock().unwrap().push(RequestLine {
            method: method.clone(),
            path: path.clone(),
        });
        if method != "GET" || content_length != 0 {
            return Err(format!(
                "unexpected non-read-only fixture request {method} {path}"
            ));
        }
        let response = {
            let state = response_state.lock().unwrap();
            if path.ends_with("/guilds/1/members/7") {
                state.member.clone()
            } else if path.ends_with("/guilds/1") {
                state.guild.clone()
            } else if path.ends_with("/channels/20") {
                state.channel.clone()
            } else {
                return Err(format!("unexpected fixture route {method} {path}"));
            }
        };
        // No handler replies before all three requests arrive. A serial caller
        // therefore fails its bound, while the actual try_join caller succeeds.
        wave.wait().await;
        let response = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        socket
            .write_all(headers.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        socket
            .write_all(&response)
            .await
            .map_err(|e| e.to_string())?;
        socket.shutdown().await.map_err(|e| e.to_string())?;
        Ok(())
    }
    fn observe_handler(
        result: Result<Result<(), String>, tokio::task::JoinError>,
        errors: &mut Vec<String>,
    ) {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(error),
            Err(error) => errors.push(format!("fixture handler join failed: {error}")),
        }
    }
    async fn access_sequence(
        scenarios: &[Scenario],
    ) -> (Vec<Result<WorkAccess, WorkError>>, Vec<RequestLine>) {
        // Validate every fixture before creating the retained socket owner.
        let fixture_values: Vec<_> = scenarios.iter().map(responses).collect();
        let initial = fixture_values
            .first()
            .expect("one scenario required")
            .clone();
        let response_state = Arc::new(Mutex::new(initial));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let server_stop = stop.clone();
        let server_responses = response_state.clone();
        let server_requests = requests.clone();
        let server = tokio::spawn(async move {
            let wave = Arc::new(tokio::sync::Barrier::new(3));
            let mut handlers = JoinSet::new();
            let mut errors = Vec::new();
            loop {
                tokio::select! {
                    biased;
                    _ = server_stop.cancelled() => break,
                    result = handlers.join_next(), if !handlers.is_empty() => {
                        observe_handler(result.unwrap(), &mut errors);
                    },
                    accepted = listener.accept() => {
                        let socket = match accepted {
                            Ok((socket, _)) => socket,
                            Err(error) => { errors.push(error.to_string()); server_stop.cancel(); break; },
                        };
                        let (state, recorded, barrier, cancelled) = (
                            server_responses.clone(), server_requests.clone(),
                            wave.clone(), server_stop.clone(),
                        );
                        handlers.spawn(async move {
                            tokio::select! {
                                biased;
                                _ = cancelled.cancelled() => Ok(()),
                                outcome = tokio::time::timeout(Duration::from_secs(5), handle(socket, state, recorded, barrier)) => {
                                    outcome.map_err(|_| "fixture handler timed out".to_owned())?
                                },
                            }
                        });
                    },
                }
            }
            // Cancelling a waiter never substitutes for observing socket owners.
            while let Some(result) = handlers.join_next().await {
                observe_handler(result, &mut errors);
            }
            errors
        });
        let http = serenity::http::HttpBuilder::new("offline-continuity-team-fixture")
            .client(
                reqwest::Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .unwrap(),
            )
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build();
        let native = DiscordContinuityAccess(Arc::new(http));
        let scope = WorkScope::Team {
            guild: GUILD,
            channel: CHANNEL,
        };
        let mut outcomes = Vec::new();
        for fixture in fixture_values {
            *response_state.lock().unwrap() = fixture;
            let outcome = tokio::time::timeout(
                Duration::from_secs(10),
                native.authorize(&scope, ACTOR, CHANNEL),
            )
            .await;
            let timed_out = outcome.is_err();
            outcomes.push(outcome);
            if timed_out {
                break;
            }
        }
        stop.cancel();
        let errors = server.await.expect("fixture listener owner joined");
        assert!(errors.is_empty(), "fixture handler errors: {errors:?}");
        let outcomes = outcomes
            .into_iter()
            .map(|outcome| {
                outcome.expect("native proof timed out; fixture owners joined before assertion")
            })
            .collect();
        let recorded = requests.lock().unwrap().clone();
        (outcomes, recorded)
    }
    fn assert_three_gets_per_call(requests: &[RequestLine], calls: usize) {
        assert_eq!(requests.len(), calls * 3, "{requests:?}");
        let mut counts = BTreeMap::new();
        for request in requests {
            assert_eq!(request.method, "GET", "{request:?}");
            let suffix = ["/guilds/1/members/7", "/guilds/1", "/channels/20"]
                .into_iter()
                .find(|suffix| request.path.ends_with(*suffix))
                .expect("only the three native proof routes are allowed");
            *counts.entry(suffix).or_insert(0) += 1;
        }
        assert_eq!(counts.len(), 3);
        assert!(counts.values().all(|&count| count == calls), "{counts:?}");
    }
    fn assert_actor_access(access: WorkAccess, manager: bool) {
        assert_eq!(access.actor, ACTOR);
        assert_eq!(access.guild, Some(GUILD));
        assert_eq!(access.channel, CHANNEL);
        assert!(access.can_view);
        assert_eq!(access.can_manage, manager);
    }
    #[tokio::test]
    async fn continuity_native_team_proof_joins_three_gets_and_current_view_role() {
        let (mut outcomes, requests) = access_sequence(&[Scenario::default()]).await;
        assert_actor_access(outcomes.remove(0).unwrap(), false);
        assert_three_gets_per_call(&requests, 1);
    }
    #[tokio::test]
    async fn continuity_native_team_manager_bit_comes_from_current_guild_roles() {
        let scenario = Scenario {
            role_permissions: Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD,
            ..Scenario::default()
        };
        let (mut outcomes, requests) = access_sequence(&[scenario]).await;
        assert_actor_access(outcomes.remove(0).unwrap(), true);
        assert_three_gets_per_call(&requests, 1);
    }
    #[tokio::test]
    async fn continuity_native_team_view_revocation_is_observed_on_the_next_get() {
        let revoked = Scenario {
            overwrites: vec![PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: PermissionOverwriteType::Member(UserId::new(ACTOR)),
            }],
            ..Scenario::default()
        };
        let (mut outcomes, requests) = access_sequence(&[Scenario::default(), revoked]).await;
        assert_actor_access(outcomes.remove(0).unwrap(), false);
        assert!(matches!(outcomes.remove(0), Err(WorkError::Denied)));
        assert_three_gets_per_call(&requests, 2);
    }
    #[tokio::test]
    async fn continuity_native_team_overwrites_match_snowflakes_not_display_names() {
        let other_member = Scenario {
            overwrites: vec![PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: PermissionOverwriteType::Member(UserId::new(8)),
            }],
            ..Scenario::default()
        };
        let (mut outcomes, requests) = access_sequence(&[other_member]).await;
        assert_actor_access(outcomes.remove(0).unwrap(), false);
        assert_three_gets_per_call(&requests, 1);
    }
    #[tokio::test]
    async fn continuity_native_team_member_guild_channel_identity_and_bot_mismatches_refuse() {
        for scenario in [
            Scenario {
                member_id: 8,
                ..Scenario::default()
            },
            Scenario {
                member_is_bot: true,
                ..Scenario::default()
            },
            Scenario {
                returned_guild_id: 2,
                ..Scenario::default()
            },
            Scenario {
                channel_guild: 2,
                ..Scenario::default()
            },
            Scenario {
                channel_id: 21,
                ..Scenario::default()
            },
        ] {
            let (mut outcomes, requests) = access_sequence(&[scenario]).await;
            assert!(matches!(outcomes.remove(0), Err(WorkError::Denied)));
            assert_three_gets_per_call(&requests, 1);
        }
    }
    #[tokio::test]
    async fn continuity_native_team_missing_everyone_or_assigned_role_refuses() {
        for scenario in [
            Scenario {
                everyone_exists: false,
                ..Scenario::default()
            },
            Scenario {
                member_role_exists: false,
                everyone_permissions: Permissions::VIEW_CHANNEL,
                ..Scenario::default()
            },
            Scenario {
                member_roles: vec![MEMBER_ROLE, 99],
                ..Scenario::default()
            },
        ] {
            let (mut outcomes, requests) = access_sequence(&[scenario]).await;
            assert!(matches!(outcomes.remove(0), Err(WorkError::Denied)));
            assert_three_gets_per_call(&requests, 1);
        }
    }
    #[tokio::test]
    async fn continuity_native_team_role_view_overwrite_refuses() {
        let scenario = Scenario {
            overwrites: vec![PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: PermissionOverwriteType::Role(RoleId::new(MEMBER_ROLE)),
            }],
            ..Scenario::default()
        };
        let (mut outcomes, requests) = access_sequence(&[scenario]).await;
        assert!(matches!(outcomes.remove(0), Err(WorkError::Denied)));
        assert_three_gets_per_call(&requests, 1);
    }
    #[tokio::test]
    async fn continuity_native_team_threads_refuse_without_parent_or_membership_probes() {
        for channel_kind in [
            ChannelType::PublicThread,
            ChannelType::PrivateThread,
            ChannelType::NewsThread,
        ] {
            let (mut outcomes, requests) = access_sequence(&[Scenario {
                channel_kind,
                ..Scenario::default()
            }])
            .await;
            assert!(matches!(outcomes.remove(0), Err(WorkError::Denied)));
            assert_three_gets_per_call(&requests, 1);
        }
    }
}
