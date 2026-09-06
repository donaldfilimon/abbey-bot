//! The serenity-facing edge of the plan engine: read a guild into a
//! [`GuildSnapshot`], and perform resolved [`Op`]s against Discord.
//!
//! Permission names cross this boundary as serenity's `get_permission_names`
//! vocabulary. `permission_bits` is the only place a name becomes a bit, and
//! a test below checks every name the shipped plan and the archetypes use, so
//! a typo in a blueprint fails the gate instead of silently dropping a
//! permission at apply time.

use serenity::builder::{CreateChannel, CreateForumTag, EditChannel, EditRole};
use serenity::http::Http;
use serenity::model::prelude::*;

use super::ChannelKind;
use super::apply::{GuildWriter, Op};
use super::diff::TopicEdit;
use super::observe::{
    BotState, ChannelClass, ChannelState, GuildSnapshot, OverwriteState, OverwriteTarget, RoleState,
};

/// Resolve client-facing permission names to bits. Every name must be one
/// serenity itself emits; the error names the first that is not.
pub fn permission_bits(names: &[String]) -> Result<Permissions, String> {
    let mut bits = Permissions::empty();
    for name in names {
        bits |=
            single_permission(name).ok_or_else(|| format!("unknown permission name {name:?}"))?;
    }
    Ok(bits)
}

fn single_permission(name: &str) -> Option<Permissions> {
    (0..u64::BITS)
        .map(|bit| Permissions::from_bits_truncate(1u64 << bit))
        .find(|flag| flag.get_permission_names() == [name])
}

/// The names serenity gives a permission set. Bits it has no name for drop.
#[must_use]
pub fn permission_names(bits: Permissions) -> Vec<String> {
    bits.get_permission_names()
        .into_iter()
        .map(str::to_string)
        .collect()
}

const fn channel_class(kind: ChannelType) -> ChannelClass {
    match kind {
        ChannelType::Text => ChannelClass::Kind(ChannelKind::Text),
        ChannelType::Voice => ChannelClass::Kind(ChannelKind::Voice),
        ChannelType::Forum => ChannelClass::Kind(ChannelKind::Forum),
        ChannelType::News => ChannelClass::Kind(ChannelKind::Announcement),
        ChannelType::Stage => ChannelClass::Kind(ChannelKind::Stage),
        ChannelType::Category => ChannelClass::Category,
        _ => ChannelClass::Other,
    }
}

fn channel_type(class: ChannelClass) -> Result<ChannelType, String> {
    Ok(match class {
        ChannelClass::Category => ChannelType::Category,
        ChannelClass::Kind(ChannelKind::Text) => ChannelType::Text,
        ChannelClass::Kind(ChannelKind::Voice) => ChannelType::Voice,
        ChannelClass::Kind(ChannelKind::Forum) => ChannelType::Forum,
        ChannelClass::Kind(ChannelKind::Announcement) => ChannelType::News,
        ChannelClass::Kind(ChannelKind::Stage) => ChannelType::Stage,
        ChannelClass::Other => return Err("cannot create a channel of an unplanned kind".into()),
    })
}

/// `None` for an overwrite kind serenity does not name yet: the engine can
/// neither match nor safely rewrite it, so the snapshot leaves it out.
fn overwrite_state(overwrite: &PermissionOverwrite) -> Option<OverwriteState> {
    let target = match overwrite.kind {
        PermissionOverwriteType::Role(id) => OverwriteTarget::Role(id.get()),
        PermissionOverwriteType::Member(id) => OverwriteTarget::Member(id.get()),
        _ => return None,
    };
    Some(OverwriteState {
        target,
        allow: permission_names(overwrite.allow),
        deny: permission_names(overwrite.deny),
    })
}

fn nonzero(id: u64, what: &str) -> Result<u64, String> {
    if id == 0 {
        Err(format!("{what} id is zero"))
    } else {
        Ok(id)
    }
}

fn describe_error(what: &str, error: &serenity::Error) -> String {
    // The status and the request path are what an operator needs; the
    // Authorization header is never part of serenity's error, and the query
    // string is dropped in case a future route ever carries one.
    if let serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response)) = error {
        return format!(
            "{what}: HTTP {} on {}: {}",
            response.status_code,
            response.url.split('?').next().unwrap_or(""),
            response.error.message
        );
    }
    format!("{what}: {error}")
}

fn edit_channel_builder<'a>(
    parent: Option<u64>,
    topic: &TopicEdit,
    reason: &'a str,
) -> Result<EditChannel<'a>, String> {
    let mut edit = EditChannel::new().audit_log_reason(reason);
    if let Some(parent) = parent {
        edit = edit.category(ChannelId::new(nonzero(parent, "category")?));
    }
    match topic {
        TopicEdit::Unchanged => {}
        TopicEdit::Set(topic) => edit = edit.topic(topic.clone()),
    }
    Ok(edit)
}

/// Read everything `diff` needs, with REST only: the bot's own identity and
/// membership, the guild's roles and features, and every channel.
pub async fn snapshot(http: &Http, guild_id: GuildId) -> Result<GuildSnapshot, String> {
    let me = http
        .get_current_user()
        .await
        .map_err(|e| describe_error("Discord rejected the bot token", &e))?;
    // `GET /users/@me/guilds/{id}/member` is a user-token endpoint: Discord
    // answers a bot with "Bots cannot use this endpoint" (seen on the first
    // live dry run, 2026-09-06). Bots read their own membership by id.
    let member = guild_id.member(http, me.id).await.map_err(|e| {
        describe_error(
            "the bot is not a member of that guild, or cannot read it",
            &e,
        )
    })?;
    let guild = guild_id
        .to_partial_guild(http)
        .await
        .map_err(|e| describe_error("fetching the guild", &e))?;
    let channels = guild_id
        .channels(http)
        .await
        .map_err(|e| describe_error("fetching the guild's channels", &e))?;

    let mut roles: Vec<RoleState> = guild
        .roles
        .values()
        .map(|role| RoleState {
            id: role.id.get(),
            name: role.name.clone(),
            position: role.position,
            managed: role.managed,
            hoist: role.hoist,
            mentionable: role.mentionable,
            colour: role.colour.0,
            permissions: permission_names(role.permissions),
        })
        .collect();
    roles.sort_by_key(|r| r.id);
    let mut channels: Vec<ChannelState> = channels
        .values()
        .map(|channel| ChannelState {
            id: channel.id.get(),
            name: channel.name.clone(),
            class: channel_class(channel.kind),
            parent: channel.parent_id.map(ChannelId::get),
            topic: channel.topic.clone(),
            overwrites: channel
                .permission_overwrites
                .iter()
                .filter_map(overwrite_state)
                .collect(),
        })
        .collect();
    channels.sort_by_key(|c| c.id);
    Ok(GuildSnapshot {
        guild_id: guild_id.get(),
        features: guild.features.clone(),
        roles,
        channels,
        bot: BotState {
            user_id: me.id.get(),
            role_ids: member.roles.iter().map(|id| id.get()).collect(),
        },
    })
}

/// Performs ops against a live guild. Every write carries an audit-log reason.
pub struct DiscordWriter<'a> {
    pub http: &'a Http,
    pub guild_id: GuildId,
    pub reason: String,
}

impl GuildWriter for DiscordWriter<'_> {
    async fn perform(&mut self, op: &Op) -> Result<Option<u64>, String> {
        match op {
            Op::CreateRole { name, colour } => {
                let builder = EditRole::new()
                    .name(name.clone())
                    .permissions(Permissions::empty())
                    .hoist(false)
                    .mentionable(false)
                    .colour(colour.unwrap_or(0))
                    .audit_log_reason(&self.reason);
                let role = self
                    .guild_id
                    .create_role(self.http, builder)
                    .await
                    .map_err(|e| describe_error("creating the role", &e))?;
                Ok(Some(role.id.get()))
            }
            Op::EditRole {
                id,
                hoist,
                mentionable,
                colour,
            } => {
                let id = RoleId::new(nonzero(*id, "role")?);
                let builder = EditRole::new()
                    .hoist(*hoist)
                    .mentionable(*mentionable)
                    .colour(colour.unwrap_or(0))
                    .audit_log_reason(&self.reason);
                self.guild_id
                    .edit_role(self.http, id, builder)
                    .await
                    .map_err(|e| describe_error("editing the role", &e))?;
                Ok(None)
            }
            Op::CreateChannel {
                name,
                class,
                parent,
                topic,
                slowmode_secs,
                tags,
                overwrites,
            } => {
                let mut builder = CreateChannel::new(name.clone())
                    .kind(channel_type(*class)?)
                    .audit_log_reason(&self.reason);
                if let Some(parent) = parent {
                    builder = builder.category(ChannelId::new(nonzero(*parent, "category")?));
                }
                if let Some(topic) = topic {
                    builder = builder.topic(topic.clone());
                }
                if let Some(secs) = slowmode_secs {
                    builder = builder.rate_limit_per_user(*secs);
                }
                let mut resolved = Vec::with_capacity(overwrites.len());
                for overwrite in overwrites {
                    resolved.push(PermissionOverwrite {
                        allow: permission_bits(&overwrite.allow)?,
                        deny: permission_bits(&overwrite.deny)?,
                        kind: PermissionOverwriteType::Role(RoleId::new(nonzero(
                            overwrite.role_id,
                            "role",
                        )?)),
                    });
                }
                builder = builder.permissions(resolved);
                let mut payload = serde_json::to_value(builder)
                    .map_err(|e| format!("serializing the channel creation: {e}"))?;
                if !tags.is_empty() {
                    payload["available_tags"] = serde_json::to_value(
                        tags.iter()
                            .map(|tag| CreateForumTag::new(tag.clone()))
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|e| format!("serializing the forum tags: {e}"))?;
                }
                let channel = self
                    .http
                    .create_channel(self.guild_id, &payload, Some(&self.reason))
                    .await
                    .map_err(|e| describe_error("creating the channel", &e))?;
                Ok(Some(channel.id.get()))
            }
            Op::EditChannel { id, parent, topic } => {
                let edit = edit_channel_builder(*parent, topic, &self.reason)?;
                ChannelId::new(nonzero(*id, "channel")?)
                    .edit(self.http, edit)
                    .await
                    .map_err(|e| describe_error("editing the channel", &e))?;
                Ok(None)
            }
            Op::SetOverwrite {
                channel_id,
                role_id,
                allow,
                deny,
            } => {
                let overwrite = PermissionOverwrite {
                    allow: permission_bits(allow)?,
                    deny: permission_bits(deny)?,
                    kind: PermissionOverwriteType::Role(RoleId::new(nonzero(*role_id, "role")?)),
                };
                ChannelId::new(nonzero(*channel_id, "channel")?)
                    .create_permission(self.http, overwrite)
                    .await
                    .map_err(|e| describe_error("setting the overwrite", &e))?;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::plan::{MLAI_COMMUNITY, Overwrite, Plan};
    use crate::server::{Archetype, NEVER_FOR_EVERYONE, blueprint};
    use serenity::http::HttpBuilder;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn discord_fixture(
        status: &'static str,
        response_body: &'static str,
    ) -> (Http, tokio::sync::oneshot::Receiver<(String, usize)>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback Discord fixture");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (sent, received) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.expect("read request");
                assert!(read > 0, "request ended before its body arrived");
                request.extend_from_slice(&buffer[..read]);
                let Some(headers_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .expect("content length");
                if request.len() >= headers_end + 4 + length {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket.write_all(response.as_bytes()).await.expect("reply");
            socket.shutdown().await.expect("close reply");
            let follow_up = usize::from(
                tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                    .await
                    .is_ok(),
            );
            sent.send((
                String::from_utf8(request).expect("UTF-8 request"),
                follow_up,
            ))
            .ok();
        });
        (
            HttpBuilder::new("fixture-token")
                .proxy(endpoint)
                .ratelimiter_disabled(true)
                .build(),
            received,
        )
    }

    fn forum_op() -> Op {
        Op::CreateChannel {
            name: "field-notes".into(),
            class: ChannelClass::Kind(ChannelKind::Forum),
            parent: Some(77),
            topic: Some("Bring findings here.".into()),
            slowmode_secs: Some(12),
            tags: vec!["Question".into(), "Solved".into()],
            overwrites: vec![super::super::apply::ResolvedOverwrite {
                role_id: 42,
                allow: vec![],
                deny: vec!["View Channel".into()],
            }],
        }
    }

    #[test]
    fn every_permission_name_in_the_shipped_plan_resolves() {
        let plan = Plan::from_toml(MLAI_COMMUNITY).unwrap();
        for name in plan.permission_names() {
            assert!(
                single_permission(name).is_some(),
                "{name:?} is not a serenity permission name"
            );
        }
    }

    #[test]
    fn every_permission_name_in_the_archetypes_and_constants_resolves() {
        for archetype in Archetype::ALL {
            for name in Plan::from(&blueprint(archetype)).permission_names() {
                assert!(single_permission(name).is_some(), "{archetype:?}: {name:?}");
            }
        }
        for name in NEVER_FOR_EVERYONE {
            assert!(single_permission(name).is_some(), "{name:?}");
        }
        assert!(permission_bits(&Overwrite::hide_marker().deny).is_ok());
    }

    #[test]
    fn a_typo_is_an_error_not_a_dropped_permission() {
        let error = permission_bits(&["View Channel".into(), "Manage Server".into()]).unwrap_err();
        assert!(error.contains("Manage Server"), "{error}");
        assert_eq!(
            permission_bits(&["View Channel".into(), "Send Messages".into()]).unwrap(),
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES
        );
    }

    #[test]
    fn names_and_bits_round_trip() {
        let bits = Permissions::MANAGE_THREADS
            | Permissions::REQUEST_TO_SPEAK
            | Permissions::MODERATE_MEMBERS;
        let names = permission_names(bits);
        assert_eq!(permission_bits(&names).unwrap(), bits);
        assert!(names.contains(&"Moderate Members".to_string()));
    }

    #[test]
    fn topic_edits_serialize_to_exact_discord_payloads() {
        let payload = |topic| {
            serde_json::to_value(edit_channel_builder(Some(77), &topic, "test").unwrap()).unwrap()
        };

        let unchanged = payload(TopicEdit::Unchanged);
        assert_eq!(unchanged.get("parent_id"), Some(&serde_json::json!("77")));
        assert!(unchanged.get("topic").is_none(), "{unchanged}");

        let set = payload(TopicEdit::Set("exact topic".into()));
        assert_eq!(set.get("parent_id"), Some(&serde_json::json!("77")));
        assert_eq!(set.get("topic"), Some(&serde_json::json!("exact topic")));
    }

    #[test]
    fn channel_kinds_map_both_ways_and_unplanned_kinds_are_other() {
        for kind in [
            ChannelKind::Text,
            ChannelKind::Voice,
            ChannelKind::Forum,
            ChannelKind::Announcement,
            ChannelKind::Stage,
        ] {
            let class = ChannelClass::Kind(kind);
            assert_eq!(channel_class(channel_type(class).unwrap()), class);
        }
        assert_eq!(channel_class(ChannelType::Category), ChannelClass::Category);
        assert_eq!(
            channel_class(ChannelType::PublicThread),
            ChannelClass::Other
        );
        assert!(channel_type(ChannelClass::Other).is_err());
    }

    #[test]
    fn overwrite_state_keeps_the_target_kind() {
        let member = PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Member(UserId::new(5)),
        };
        let state = overwrite_state(&member).unwrap();
        assert_eq!(state.target, OverwriteTarget::Member(5));
        assert_eq!(state.allow, ["View Channel"]);
    }

    #[tokio::test]
    async fn forum_creation_sends_tags_and_all_channel_fields_in_one_post() {
        let response = r#"{"id":"99","type":15,"guild_id":"42","name":"field-notes","position":0,"permission_overwrites":[],"nsfw":false}"#;
        let (http, request) = discord_fixture("200 OK", response).await;
        let mut writer = DiscordWriter {
            http: &http,
            guild_id: GuildId::new(42),
            reason: "server plan test".into(),
        };

        assert_eq!(writer.perform(&forum_op()).await.unwrap(), Some(99));
        let (request, follow_up) = request.await.unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(
            headers.starts_with("POST /api/v10/guilds/42/channels HTTP/1.1"),
            "{headers}"
        );
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("x-audit-log-reason: server%20plan%20test"),
            "{headers}"
        );
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["name"], "field-notes");
        assert_eq!(payload["type"], 15);
        assert_eq!(payload["parent_id"], "77");
        assert_eq!(payload["topic"], "Bring findings here.");
        assert_eq!(payload["rate_limit_per_user"], 12);
        assert_eq!(payload["permission_overwrites"][0]["id"], "42");
        assert_eq!(
            payload["permission_overwrites"][0]["deny"],
            Permissions::VIEW_CHANNEL.bits().to_string()
        );
        assert_eq!(payload["available_tags"][0]["name"], "Question");
        assert_eq!(payload["available_tags"][1]["name"], "Solved");
        assert!(
            payload["available_tags"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tag| tag.get("id").is_none()),
            "{payload}"
        );
        assert_eq!(
            follow_up, 0,
            "forum creation must never PATCH tags afterward"
        );
    }

    #[tokio::test]
    async fn failed_forum_post_returns_failure_without_a_follow_up_request() {
        let (http, request) = discord_fixture(
            "400 Bad Request",
            r#"{"message":"invalid forum","code":50035}"#,
        )
        .await;
        let mut writer = DiscordWriter {
            http: &http,
            guild_id: GuildId::new(42),
            reason: "server plan test".into(),
        };

        let error = writer.perform(&forum_op()).await.unwrap_err();
        assert!(error.contains("creating the channel"), "{error}");
        assert!(error.contains("HTTP 400"), "{error}");
        let (request, follow_up) = request.await.unwrap();
        assert!(request.starts_with("POST "), "{request}");
        assert_eq!(follow_up, 0, "a failed POST must not trigger a PATCH");
    }

    #[tokio::test]
    async fn non_forum_creation_still_uses_the_same_single_post_path() {
        let response = r#"{"id":"100","type":0,"guild_id":"42","name":"general","position":0,"permission_overwrites":[],"nsfw":false}"#;
        let (http, request) = discord_fixture("200 OK", response).await;
        let mut writer = DiscordWriter {
            http: &http,
            guild_id: GuildId::new(42),
            reason: "server plan test".into(),
        };
        let op = Op::CreateChannel {
            name: "general".into(),
            class: ChannelClass::Kind(ChannelKind::Text),
            parent: Some(77),
            topic: None,
            slowmode_secs: None,
            tags: Vec::new(),
            overwrites: Vec::new(),
        };

        assert_eq!(writer.perform(&op).await.unwrap(), Some(100));
        let (request, follow_up) = request.await.unwrap();
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["name"], "general");
        assert_eq!(payload["type"], 0);
        assert!(payload.get("available_tags").is_none(), "{payload}");
        assert_eq!(follow_up, 0);
    }
}
