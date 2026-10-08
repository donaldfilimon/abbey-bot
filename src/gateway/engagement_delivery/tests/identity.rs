// Apply as src/gateway/engagement_delivery/tests/identity.rs. Register in
// engagement_delivery/tests.rs with #[path = "tests/identity.rs"] mod identity;
// Calls the real DiscordEngagementDelivery source and hydrate_exchange methods.
use super::*;
use serenity::all::{Message, MessageId, PrivateChannel, Timestamp, User};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const AT: u64 = 1_790_683_200;
const SOURCE_TEXT: &str = "Synthetic exact human source";
const REPLY_TEXT: &str = "Synthetic exact Abbey reply";

#[derive(Clone, Copy, Debug)]
enum Malformed {
    None,
    SourceIdentity,
    SourceBot,
    SourceAuthor,
    SourceChannel,
    SourceTimestamp,
    SourceEdited,
    SourceGuild,
    DmRecipient,
    ReplyIdentity,
    ReplyHuman,
    ReplyAuthor,
    ReplyChannel,
    ReplyEdited,
    ReplyGuild,
    ReplyReferenceMessage,
    ReplyReferenceChannel,
    CurrentBotIdentity,
    CurrentBotHuman,
}
fn native_user(id: u64, bot: bool) -> User {
    let mut user = User::default();
    user.id = UserId::new(id);
    user.bot = bot;
    user.name = format!("synthetic-{id}");
    user
}
fn native_source(malformed: Malformed) -> Message {
    let mut message = Message::default();
    message.id = MessageId::new(4);
    message.channel_id = ChannelId::new(3);
    message.author = native_user(2, false);
    message.timestamp = Timestamp::from_unix_timestamp(AT as i64).unwrap();
    message.content = SOURCE_TEXT.into();
    match malformed {
        Malformed::SourceIdentity => message.id = MessageId::new(400),
        Malformed::SourceBot => message.author.bot = true,
        Malformed::SourceAuthor => message.author.id = UserId::new(8),
        Malformed::SourceChannel => message.channel_id = ChannelId::new(8),
        Malformed::SourceTimestamp => {
            message.timestamp = Timestamp::from_unix_timestamp((AT + 1) as i64).unwrap()
        }
        Malformed::SourceEdited => {
            message.edited_timestamp =
                Some(Timestamp::from_unix_timestamp((AT + 1) as i64).unwrap())
        }
        Malformed::SourceGuild => message.guild_id = Some(GuildId::new(8)),
        _ => {}
    }
    message
}
fn native_reply(malformed: Malformed) -> Message {
    let mut message = Message::default();
    message.id = MessageId::new(5);
    message.channel_id = ChannelId::new(3);
    message.author = native_user(99, true);
    message.timestamp = Timestamp::from_unix_timestamp((AT + 1) as i64).unwrap();
    message.content = REPLY_TEXT.into();
    message.message_reference = Some(
        serenity::all::MessageReference::new(
            serenity::all::MessageReferenceKind::default(),
            ChannelId::new(3),
        )
        .message_id(MessageId::new(4)),
    );
    match malformed {
        Malformed::ReplyIdentity => message.id = MessageId::new(500),
        Malformed::ReplyHuman => message.author.bot = false,
        Malformed::ReplyAuthor => message.author.id = UserId::new(8),
        Malformed::ReplyChannel => message.channel_id = ChannelId::new(8),
        Malformed::ReplyEdited => {
            message.edited_timestamp =
                Some(Timestamp::from_unix_timestamp((AT + 2) as i64).unwrap())
        }
        Malformed::ReplyGuild => message.guild_id = Some(GuildId::new(8)),
        Malformed::ReplyReferenceMessage => {
            message.message_reference.as_mut().unwrap().message_id = Some(MessageId::new(8))
        }
        Malformed::ReplyReferenceChannel => {
            message.message_reference.as_mut().unwrap().channel_id = ChannelId::new(8)
        }
        _ => {}
    }
    message
}
fn source_ref() -> SourceRef {
    SourceRef {
        scope: EngagementScope::Dm {
            member: 2,
            channel: 3,
        },
        message: 4,
        author: 2,
        revision: 1,
        at: AT,
    }
}
fn linked_candidate() -> Candidate {
    Candidate {
        id: 1,
        kind: EngagementKind::FollowUp,
        source: Some(source_ref()),
        member: Some(2),
        scope: source_ref().scope,
        due_at: AT + 86_400,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: "synthetic-task-identity".into(),
        policy_revision: 1,
        destination: DestinationPreference::Origin,
        message_id: None,
        introduction_id: None,
        work_ref: Some(crate::work::WorkContentRef::Task {
            project: 10,
            id: 11,
            revision: 0,
        }),
        expires_at: Some(AT + 90_000),
        follow_up_reason: None,
    }
}
struct IdentityNetwork {
    adapter: DiscordEngagementDelivery,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    server: Option<tokio::task::JoinHandle<()>>,
}
impl IdentityNetwork {
    async fn new(malformed: Malformed) -> Self {
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
                let route = first.next().unwrap().to_owned();
                assert_eq!(method, "GET", "proof cannot mutate Discord");
                recorded.lock().unwrap().push((method, route.clone()));
                let response = if route.ends_with("/channels/3/messages/4") {
                    serde_json::to_vec(&native_source(malformed)).unwrap()
                } else if route.ends_with("/channels/3/messages/5") {
                    serde_json::to_vec(&native_reply(malformed)).unwrap()
                } else if route.ends_with("/channels/3") {
                    let mut channel = PrivateChannel::default();
                    channel.id = ChannelId::new(3);
                    channel.kind = ChannelType::Private;
                    channel.recipient = native_user(
                        if matches!(malformed, Malformed::DmRecipient) {
                            8
                        } else {
                            2
                        },
                        false,
                    );
                    serde_json::to_vec(&channel).unwrap()
                } else if route.ends_with("/users/@me") {
                    serde_json::to_vec(&native_user(
                        if matches!(malformed, Malformed::CurrentBotIdentity) {
                            8
                        } else {
                            99
                        },
                        !matches!(malformed, Malformed::CurrentBotHuman),
                    ))
                    .unwrap()
                } else {
                    panic!("Unexpected loopback route: {route}")
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.write_all(&response).await;
            }
        });
        let http = serenity::http::HttpBuilder::new("synthetic-offline-identity")
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
impl Drop for IdentityNetwork {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

#[tokio::test]
async fn engagement_discord_source_and_exchange_identity_have_valid_http_controls() {
    let fixture = IdentityNetwork::new(Malformed::None).await;
    assert_eq!(
        fixture.adapter.source(&source_ref()).await.unwrap(),
        SOURCE_TEXT
    );
    assert_eq!(
        fixture
            .adapter
            .hydrate_exchange(&linked_candidate(), 5)
            .await
            .unwrap(),
        format!("Human: {SOURCE_TEXT}\nAbbey's delivered response: {REPLY_TEXT}")
    );
    {
        let requests = fixture.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|(_, route)| route.ends_with("/channels/3/messages/4"))
        );
        assert!(
            requests
                .iter()
                .any(|(_, route)| route.ends_with("/channels/3/messages/5"))
        );
        assert!(
            requests
                .iter()
                .any(|(_, route)| route.ends_with("/users/@me"))
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn engagement_discord_source_rejects_wrong_returned_message_identity() {
    let fixture = IdentityNetwork::new(Malformed::SourceIdentity).await;
    assert_eq!(
        fixture.adapter.source(&source_ref()).await,
        Err(WorkError::Denied),
        "correct author/channel/timestamp do not prove the requested message ID"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn engagement_discord_exchange_rejects_wrong_returned_response_identity() {
    let fixture = IdentityNetwork::new(Malformed::ReplyIdentity).await;
    assert_eq!(
        fixture
            .adapter
            .hydrate_exchange(&linked_candidate(), 5)
            .await,
        Err(WorkError::Denied),
        "correct bot/channel/reference do not prove the recorded delivered-response ID"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn engagement_discord_source_refuses_bot_author_scope_timestamp_edit_and_recipient_mismatches()
 {
    for malformed in [
        Malformed::SourceBot,
        Malformed::SourceAuthor,
        Malformed::SourceChannel,
        Malformed::SourceTimestamp,
        Malformed::SourceEdited,
        Malformed::SourceGuild,
        Malformed::DmRecipient,
    ] {
        let fixture = IdentityNetwork::new(malformed).await;
        assert_eq!(
            fixture.adapter.source(&source_ref()).await,
            Err(WorkError::Denied),
            "{malformed:?}"
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn engagement_discord_exchange_refuses_bot_channel_edit_guild_reference_and_current_bot_mismatches()
 {
    for malformed in [
        Malformed::ReplyHuman,
        Malformed::ReplyAuthor,
        Malformed::ReplyChannel,
        Malformed::ReplyEdited,
        Malformed::ReplyGuild,
        Malformed::ReplyReferenceMessage,
        Malformed::ReplyReferenceChannel,
        Malformed::CurrentBotIdentity,
        Malformed::CurrentBotHuman,
    ] {
        let fixture = IdentityNetwork::new(malformed).await;
        assert_eq!(
            fixture
                .adapter
                .hydrate_exchange(&linked_candidate(), 5)
                .await,
            Err(WorkError::Denied),
            "{malformed:?}"
        );
        fixture.finish().await;
    }
}

#[path = "identity/recipient.rs"]
mod recipient;
