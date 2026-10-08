//! Actual offline final Engagement recipient proof, including existing bot-only work.
//! Apply as gateway/engagement_delivery/tests/identity/recipient.rs and register
//! #[path = "identity/recipient.rs"] mod recipient; in identity.rs.
use super::*;
use serde_json::{Value, json};
use serenity::all::{Guild, GuildChannel, Member, Role, RoleId, ThreadMetadata};

#[derive(Clone, Copy, Debug)]
enum Broken {
    None,
    RecipientOwner,
    RecipientBot,
    PairedGuild,
    ChannelId,
    UnknownRecipientRole,
    MissingEveryone,
    BotMemberId,
    BotMemberHuman,
    CurrentBotHuman,
    ParentId,
    ParentGuild,
    ThreadId,
    ThreadUser,
    ThreadGuild,
    ThreadEmbeddedMember,
    RestThreadGuildOmitted,
    DmRecipientBot,
    DmKind,
}

fn proof_member(id: u64, bot: bool, role: u64) -> Member {
    let mut member = Member::default();
    member.user = native_user(id, bot);
    member.guild_id = GuildId::new(7);
    member.roles.push(RoleId::new(role));
    member
}
fn proof_channel(id: u64) -> GuildChannel {
    let mut channel = GuildChannel::default();
    channel.id = ChannelId::new(id);
    channel.guild_id = GuildId::new(7);
    channel.kind = ChannelType::Text;
    channel
}
fn response(broken: Broken, thread: bool, dm: bool, route: &str) -> Value {
    if route.ends_with("/users/@me") {
        return serde_json::to_value(native_user(99, !matches!(broken, Broken::CurrentBotHuman)))
            .unwrap();
    }
    if route.ends_with("/users/@me/channels") {
        let mut channel = PrivateChannel::default();
        channel.id = ChannelId::new(if dm { 3 } else { 6 });
        channel.kind = if matches!(broken, Broken::DmKind) {
            ChannelType::GroupDm
        } else {
            ChannelType::Private
        };
        channel.recipient = native_user(2, matches!(broken, Broken::DmRecipientBot));
        return serde_json::to_value(channel).unwrap();
    }
    if route.ends_with("/guilds/7") {
        let mut guild = Guild::default();
        guild.id = GuildId::new(if matches!(broken, Broken::PairedGuild) {
            8
        } else {
            7
        });
        guild.owner_id = UserId::new(42);
        guild.name = "synthetic final proof".into();
        for (id, permissions) in [
            (7, Permissions::empty()),
            (
                10,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            ),
            (
                20,
                Permissions::VIEW_CHANNEL
                    | Permissions::READ_MESSAGE_HISTORY
                    | Permissions::SEND_MESSAGES
                    | Permissions::SEND_MESSAGES_IN_THREADS,
            ),
        ] {
            if id == 7 && matches!(broken, Broken::MissingEveryone) {
                continue;
            }
            let mut role = Role::default();
            role.id = RoleId::new(id);
            role.guild_id = guild.id;
            role.permissions = permissions;
            guild.roles.insert(role.id, role);
        }
        return serde_json::to_value(guild).unwrap();
    }
    if route.ends_with("/channels/3") {
        let mut channel = proof_channel(if matches!(broken, Broken::ChannelId) {
            88
        } else {
            3
        });
        if matches!(broken, Broken::PairedGuild) {
            channel.guild_id = GuildId::new(8);
        }
        if thread {
            channel.kind = ChannelType::PrivateThread;
            channel.parent_id = Some(ChannelId::new(4));
            channel.thread_metadata = Some(
                serde_json::from_value::<ThreadMetadata>(json!({
                    "archived": false, "auto_archive_duration": 60,
                    "archive_timestamp": null, "locked": false,
                    "create_timestamp": null, "invitable": false
                }))
                .unwrap(),
            );
        }
        return serde_json::to_value(channel).unwrap();
    }
    if route.ends_with("/channels/4") {
        let mut channel = proof_channel(if matches!(broken, Broken::ParentId) {
            88
        } else {
            4
        });
        if matches!(broken, Broken::ParentGuild) {
            channel.guild_id = GuildId::new(8);
        }
        return serde_json::to_value(channel).unwrap();
    }
    if let Some(user) = route
        .split("/channels/3/thread-members/")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        let embedded = proof_member(
            if user == 2 && matches!(broken, Broken::ThreadEmbeddedMember) {
                42
            } else {
                user
            },
            user == 99,
            if user == 2 { 10 } else { 20 },
        );
        let mut result = json!({
            "id": if matches!(broken, Broken::ThreadId) { "88" } else { "3" },
            "user_id": if user == 2 && matches!(broken, Broken::ThreadUser) { 42 } else { user }.to_string(),
            "join_timestamp": "2026-10-01T12:00:00+00:00", "flags": 0,
            "member": embedded,
            "guild_id": if matches!(broken, Broken::ThreadGuild) { "8" } else { "7" }
        });
        if matches!(broken, Broken::RestThreadGuildOmitted) {
            result.as_object_mut().unwrap().remove("guild_id");
            result["member"].as_object_mut().unwrap().remove("guild_id");
        }
        return result;
    }
    if route.contains("/members/") {
        let requested = route.rsplit('/').next().unwrap().parse::<u64>().unwrap();
        let mut member = proof_member(
            match (requested, broken) {
                (2, Broken::RecipientOwner) | (99, Broken::BotMemberId) => 42,
                _ => requested,
            },
            match requested {
                2 => matches!(broken, Broken::RecipientBot),
                99 => !matches!(broken, Broken::BotMemberHuman),
                _ => false,
            },
            if requested == 2 { 10 } else { 20 },
        );
        if requested == 2 && matches!(broken, Broken::UnknownRecipientRole) {
            member.roles.push(RoleId::new(999));
        }
        // An owner substitution must not inherit the real recipient's roles.
        if requested == 2 && matches!(broken, Broken::RecipientOwner) {
            member.roles.clear();
        }
        return serde_json::to_value(member).unwrap();
    }
    panic!("Unexpected synthetic proof route: {route}");
}

struct RecipientNetwork {
    adapter: DiscordEngagementDelivery,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    server: Option<tokio::task::JoinHandle<()>>,
}
impl RecipientNetwork {
    async fn new(broken: Broken, thread: bool, dm: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let server = tokio::spawn(async move {
            'connections: loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 4096];
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 && bytes.is_empty() {
                        continue 'connections;
                    }
                    assert!(n > 0, "request ended before headers");
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() <= 65_536);
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
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() <= 65_536);
                }
                let mut first = headers.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_owned();
                let route = first.next().unwrap().split('?').next().unwrap().to_owned();
                assert!(
                    method == "GET" || (method == "POST" && route.ends_with("/users/@me/channels")),
                    "authorization cannot send or moderate: {method} {route}"
                );
                recorded.lock().unwrap().push((method, route.clone()));
                let synthetic_reply =
                    serde_json::to_vec(&response(broken, thread, dm, &route)).unwrap();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    synthetic_reply.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.write_all(&synthetic_reply).await;
            }
        });
        let http = serenity::http::HttpBuilder::new("synthetic-offline-recipient")
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
            server: Some(server),
        }
    }
    async fn finish(mut self) {
        let server = self.server.take().unwrap();
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
impl Drop for RecipientNetwork {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}
fn reservation(dm: bool, member: Option<u64>, private: bool) -> EngagementReservation {
    EngagementReservation {
        introduction: None,
        candidate_id: 1,
        revision: 1,
        policy_revision: 1,
        scope: if dm {
            EngagementScope::Dm {
                member: 2,
                channel: 3,
            }
        } else {
            EngagementScope::Guild {
                guild: 7,
                channel: 3,
            }
        },
        member,
        destination: if private {
            DestinationPreference::Private
        } else {
            DestinationPreference::Origin
        },
    }
}

#[tokio::test]
async fn engagement_final_recipient_native_proof_preserves_valid_human_bot_only_dm_and_thread_controls()
 {
    for (thread, dm, member, private, expected) in [
        (false, false, Some(2), false, 3),
        (false, false, None, false, 3),
        (true, false, Some(2), false, 3),
        (false, true, Some(2), false, 3),
        (false, false, Some(2), true, 6),
    ] {
        let fixture = RecipientNetwork::new(Broken::None, thread, dm).await;
        let request = reservation(dm, member, private);
        let actual = fixture.adapter.authorize(&request).await.unwrap();
        assert_eq!(actual.channel, expected);
        assert_eq!(actual.member, member);
        assert_eq!(actual.scope, request.scope);
        assert!(!fixture.requests.lock().unwrap().is_empty());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn engagement_final_recipient_native_proof_rejects_identity_role_and_bot_substitution() {
    for broken in [
        Broken::RecipientOwner,
        Broken::RecipientBot,
        Broken::PairedGuild,
        Broken::ChannelId,
        Broken::UnknownRecipientRole,
        Broken::MissingEveryone,
        Broken::BotMemberId,
        Broken::BotMemberHuman,
        Broken::CurrentBotHuman,
    ] {
        let fixture = RecipientNetwork::new(broken, false, false).await;
        assert_eq!(
            fixture
                .adapter
                .authorize(&reservation(false, Some(2), false))
                .await,
            Err(WorkError::Denied),
            "{broken:?}"
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn engagement_final_recipient_native_thread_proof_rejects_parent_and_membership_substitution()
{
    for broken in [
        Broken::ParentId,
        Broken::ParentGuild,
        Broken::ThreadId,
        Broken::ThreadUser,
        Broken::ThreadGuild,
        Broken::ThreadEmbeddedMember,
    ] {
        let fixture = RecipientNetwork::new(broken, true, false).await;
        assert_eq!(
            fixture
                .adapter
                .authorize(&reservation(false, Some(2), false))
                .await,
            Err(WorkError::Denied),
            "{broken:?}"
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn engagement_final_recipient_native_dm_proof_rejects_bot_recipient_and_nonprivate_type() {
    for dm in [true, false] {
        for broken in [Broken::DmRecipientBot, Broken::DmKind] {
            let fixture = RecipientNetwork::new(broken, false, dm).await;
            assert_eq!(
                fixture
                    .adapter
                    .authorize(&reservation(dm, Some(2), !dm))
                    .await,
                Err(WorkError::Denied),
                "dm={dm} {broken:?}"
            );
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn engagement_final_recipient_native_thread_proof_accepts_rest_member_without_guild_fields() {
    let fixture = RecipientNetwork::new(Broken::RestThreadGuildOmitted, true, false).await;
    let result = fixture
        .adapter
        .authorize(&reservation(false, Some(2), false))
        .await;
    assert!(
        result.is_ok(),
        "REST thread membership omits guild fields: {result:?}"
    );
    fixture.finish().await;
}
