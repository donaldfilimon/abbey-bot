use super::*;
use crate::engagement::{Introduction, IntroductionState, lifecycle::IntroductionReservation};
use serenity::all::{Guild, Member, Message, MessageId, Role, RoleId, User};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Network {
    adapter: DiscordEngagementDelivery,
    requests: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Network {
    async fn new(deny_second: bool, cross_guild: bool) -> Self {
        Self::with_member_failure(deny_second.then_some("403 Forbidden"), cross_guild).await
    }
    async fn with_member_failure(failure: Option<&'static str>, cross_guild: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 4096];
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, v)| v.trim().parse::<usize>().unwrap());
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 4096];
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 65536);
                }
                let first = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .collect::<Vec<_>>();
                let route = first[1].to_owned();
                let body = if length == 0 {
                    serde_json::Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                };
                recorded.lock().unwrap().push((route.clone(), body));
                let mut status = "200 OK";
                let response = if route.ends_with("/users/@me") {
                    let mut user = User::default();
                    user.id = UserId::new(99);
                    user.bot = true;
                    serde_json::to_vec(&user).unwrap()
                } else if route.contains("/members/") {
                    let id = route.rsplit('/').next().unwrap().parse::<u64>().unwrap();
                    if let Some(failure) = failure.filter(|_| id == 5) {
                        status = failure;
                        serde_json::to_vec(
                            &serde_json::json!({"code":50013,"message":"fixture denied"}),
                        )
                        .unwrap()
                    } else {
                        let mut member = Member::default();
                        member.user.id = UserId::new(id);
                        member.guild_id = GuildId::new(7);
                        serde_json::to_vec(&member).unwrap()
                    }
                } else if route.ends_with("/guilds/7") {
                    let mut guild = Guild::default();
                    guild.id = GuildId::new(7);
                    guild.owner_id = UserId::new(999);
                    let mut role = Role::default();
                    role.id = RoleId::new(7);
                    role.permissions = Permissions::VIEW_CHANNEL
                        | Permissions::READ_MESSAGE_HISTORY
                        | Permissions::SEND_MESSAGES;
                    guild.roles.insert(role.id, role);
                    serde_json::to_vec(&guild).unwrap()
                } else if route.ends_with("/channels/3") {
                    let mut channel = serenity::all::GuildChannel::default();
                    channel.id = ChannelId::new(3);
                    channel.guild_id = GuildId::new(if cross_guild { 8 } else { 7 });
                    channel.kind = ChannelType::Text;
                    serde_json::to_vec(&channel).unwrap()
                } else if route.ends_with("/channels/3/messages") {
                    let mut message = Message::default();
                    message.id = MessageId::new(123);
                    message.channel_id = ChannelId::new(3);
                    serde_json::to_vec(&message).unwrap()
                } else {
                    panic!("Unexpected fixture route {route}")
                };
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                stream.write_all(headers.as_bytes()).await.unwrap();
                stream.write_all(&response).await.unwrap();
            }
        });
        let http = serenity::http::HttpBuilder::new("synthetic-offline")
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
            adapter: DiscordEngagementDelivery(Arc::new(http)),
            requests,
            server,
        }
    }
    async fn finish(self) {
        self.server.abort();
        assert!(self.server.await.unwrap_err().is_cancelled());
    }
}
#[tokio::test]
async fn introductions_discord_unavailable_proof_is_distinct_from_explicit_denial() {
    for (status, expected) in [
        ("403 Forbidden", WorkError::Denied),
        ("404 Not Found", WorkError::Denied),
        ("401 Unauthorized", WorkError::Missing),
        ("429 Too Many Requests", WorkError::Missing),
        ("503 Service Unavailable", WorkError::Missing),
    ] {
        let fixture = Network::with_member_failure(Some(status), false).await;
        assert_eq!(
            fixture.adapter.authorize(&reservation()).await,
            Err(expected)
        );
        assert!(!fixture.requests.lock().unwrap().iter().any(|(route, _)| {
            route.ends_with("/messages")
                || route.contains("/users/2/channels")
                || route.contains("/users/5/channels")
        }));
        fixture.finish().await;
    }
}
fn reservation() -> EngagementReservation {
    let i = Introduction {
        id: 1,
        revision: 2,
        scope: EngagementScope::Guild {
            guild: 7,
            channel: 3,
        },
        members: [2, 5],
        approved_self_descriptions: [
            Some("I build compilers.".into()),
            Some("I build runtimes.".into()),
        ],
        approvals: [Some(2); 2],
        destination: 3,
        state: IntroductionState::Consumed,
    };
    EngagementReservation {
        candidate_id: 2,
        revision: 2,
        policy_revision: 1,
        scope: i.scope.clone(),
        member: None,
        destination: DestinationPreference::Origin,
        introduction: Some(IntroductionReservation {
            introduction: i,
            policy_revisions: [1; 2],
        }),
    }
}
#[tokio::test]
async fn introductions_actual_discord_authorizes_both_members_and_sends_only_approved_copy_without_mentions()
 {
    let f = Network::new(false, false).await;
    let r = reservation();
    let d = f.adapter.authorize(&r).await.unwrap_or_else(|error| {
        panic!(
            "Fixture authorization failed: {error:?}; loopback routes: {:?}",
            f.requests
                .lock()
                .unwrap()
                .iter()
                .map(|(route, _)| route.clone())
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(d.channel, 3);
    assert_eq!(d.member, None);
    let text = crate::engagement::introductions::publication(
        &r.introduction.as_ref().unwrap().introduction,
    )
    .unwrap();
    assert_eq!(f.adapter.send(3, &text).await.unwrap(), 123);
    {
        let seen = f.requests.lock().unwrap();
        assert!(seen.iter().any(|(p, _)| p.ends_with("/members/2")));
        assert!(seen.iter().any(|(p, _)| p.ends_with("/members/5")));
        assert!(
            !seen
                .iter()
                .any(|(p, _)| p.contains("/users/2/channels") || p.contains("/users/5/channels"))
        );
        let (_, body) = seen.iter().find(|(p, _)| p.ends_with("/messages")).unwrap();
        assert_eq!(body["content"], text);
        assert_eq!(body["allowed_mentions"]["parse"], serde_json::json!([]));
    }
    f.finish().await;
}
#[tokio::test]
async fn introductions_actual_discord_denies_one_member_or_cross_guild_without_reroute() {
    for (deny, cross) in [(true, false), (false, true)] {
        let f = Network::new(deny, cross).await;
        assert!(f.adapter.authorize(&reservation()).await.is_err());
        {
            let seen = f.requests.lock().unwrap();
            assert!(seen.iter().any(|(p, _)| p.ends_with("/channels/3")));
            if deny {
                assert!(seen.iter().any(|(p, _)| p.ends_with("/members/2")));
                assert!(seen.iter().any(|(p, _)| p.ends_with("/members/5")));
            } else {
                assert!(!seen.iter().any(|(p, _)| p.contains("/members/")));
            }
        }
        assert!(
            !f.requests
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _)| p.ends_with("/messages")
                    || p.contains("/users/2/channels")
                    || p.contains("/users/5/channels"))
        );
        f.finish().await;
    }
}
