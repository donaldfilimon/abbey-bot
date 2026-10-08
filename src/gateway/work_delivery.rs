//! Discord work delivery. No cached member or permission facts authorize a send.
use crate::{
    runtime::work_delivery::WorkDeliveryTransport,
    work::{WorkAccess, WorkDestination, WorkError, WorkScope},
};
use serenity::all::{
    ChannelId, ChannelType, CreateMessage, GuildId, Http, Permissions, RoleId, UserId,
};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) struct DiscordWorkDelivery(pub Arc<Http>);

impl WorkDeliveryTransport for DiscordWorkDelivery {
    async fn authorize(
        &self,
        scope: &WorkScope,
        actor: u64,
        origin: u64,
        target: &WorkDestination,
        audience: &BTreeSet<u64>,
    ) -> Result<(WorkAccess, u64), WorkError> {
        if actor == 0 || origin == 0 || matches!(scope, WorkScope::Team { guild: 0, .. }) {
            return Err(WorkError::Invalid);
        }
        match scope {
            WorkScope::Personal { owner } => {
                if *owner != actor || target != &(WorkDestination::Personal { principal: actor }) {
                    return Err(WorkError::Denied);
                }
                let dm = UserId::new(actor)
                    .create_dm_channel(&self.0)
                    .await
                    .map_err(native_failure)?;
                if dm.id.get() != origin
                    || dm.kind != ChannelType::Private
                    || dm.recipient.id.get() != actor
                    || dm.recipient.bot
                {
                    return Err(WorkError::Denied);
                }
                Ok((
                    WorkAccess {
                        actor,
                        guild: None,
                        channel: origin,
                        can_view: true,
                        can_manage: false,
                    },
                    origin,
                ))
            }
            WorkScope::Team { guild, channel } => {
                if *channel != origin {
                    return Err(WorkError::Denied);
                }
                let expected_guild = *guild;
                let guild = GuildId::new(expected_guild)
                    .to_partial_guild(&self.0)
                    .await
                    .map_err(native_failure)?;
                let channel = ChannelId::new(origin)
                    .to_channel(&self.0)
                    .await
                    .map_err(native_failure)?;
                let channel = channel.guild().ok_or(WorkError::Denied)?;
                // Thread membership requires a separate proof; never infer it
                // from parent overwrites or silently fall back to another channel.
                if guild.id.get() != expected_guild
                    || guild.owner_id.get() == 0
                    || channel.id.get() != origin
                    || channel.guild_id != guild.id
                    || channel.kind != ChannelType::Text
                    || !guild.roles.contains_key(&RoleId::new(expected_guild))
                {
                    return Err(WorkError::Denied);
                }
                let actor_member = guild
                    .member(&self.0, UserId::new(actor))
                    .await
                    .map_err(native_failure)?;
                if actor_member.user.id.get() != actor
                    || actor_member.user.bot
                    || actor_member
                        .roles
                        .iter()
                        .any(|role| !guild.roles.contains_key(role))
                {
                    return Err(WorkError::Denied);
                }
                let perms = guild.user_permissions_in(&channel, &actor_member);
                if !perms.contains(Permissions::VIEW_CHANNEL) {
                    return Err(WorkError::Denied);
                }
                let access = WorkAccess {
                    actor,
                    guild: Some(guild.id.get()),
                    channel: origin,
                    can_view: true,
                    can_manage: perms.contains(Permissions::MANAGE_GUILD),
                };
                match target {
                    WorkDestination::TeamPrivate { principal } if *principal == actor => {
                        let dm = UserId::new(actor)
                            .create_dm_channel(&self.0)
                            .await
                            .map_err(native_failure)?;
                        if dm.recipient.id.get() != actor
                            || dm.recipient.bot
                            || dm.id.get() == 0
                            || dm.id.get() == origin
                            || dm.kind != ChannelType::Private
                        {
                            return Err(WorkError::Denied);
                        }
                        Ok((access, dm.id.get()))
                    }
                    WorkDestination::TeamChannel {
                        channel: destination,
                    } if *destination == origin => {
                        let current_bot =
                            self.0.get_current_user().await.map_err(native_failure)?;
                        if current_bot.id.get() == 0 || !current_bot.bot {
                            return Err(WorkError::Denied);
                        }
                        let bot = current_bot.id;
                        let member = guild.member(&self.0, bot).await.map_err(native_failure)?;
                        if member.user.id != bot
                            || !member.user.bot
                            || member
                                .roles
                                .iter()
                                .any(|role| !guild.roles.contains_key(role))
                        {
                            return Err(WorkError::Denied);
                        }
                        if !guild
                            .user_permissions_in(&channel, &member)
                            .contains(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES)
                        {
                            return Err(WorkError::Denied);
                        }
                        let mut after = None;
                        let mut seen = BTreeSet::new();
                        let mut viewers = BTreeSet::new();
                        for _ in 0..10 {
                            let page = guild
                                .members(&self.0, Some(1000), after)
                                .await
                                .map_err(native_failure)?;
                            let complete = page.len() < 1000;
                            for member in &page {
                                if member.user.id.get() == 0
                                    || member
                                        .roles
                                        .iter()
                                        .any(|role| !guild.roles.contains_key(role))
                                    || !seen.insert(member.user.id.get())
                                {
                                    return Err(WorkError::Denied);
                                }
                                if guild
                                    .user_permissions_in(&channel, member)
                                    .contains(Permissions::VIEW_CHANNEL)
                                {
                                    viewers.insert(member.user.id.get());
                                }
                            }
                            if complete {
                                // The owner always has access, including if an
                                // incomplete API response accidentally omitted it.
                                viewers.insert(guild.owner_id.get());
                                check_audience(&viewers, audience, actor, bot.get())?;
                                return Ok((access, origin));
                            }
                            let next = page
                                .iter()
                                .map(|m| m.user.id)
                                .max()
                                .ok_or(WorkError::Denied)?;
                            if after.is_some_and(|old| next <= old) {
                                return Err(WorkError::Denied);
                            }
                            after = Some(next);
                        }
                        Err(WorkError::Denied) // Bounded enumeration failed closed.
                    }
                    _ => Err(WorkError::Denied),
                }
            }
        }
    }

    async fn send(&self, channel: u64, body: &str) -> Result<u64, WorkError> {
        if channel == 0 || body.is_empty() || body.chars().count() > 1900 {
            return Err(WorkError::Invalid);
        }
        ChannelId::new(channel)
            .send_message(
                &self.0,
                CreateMessage::new()
                    .content(body)
                    .allowed_mentions(super::no_mentions()),
            )
            .await
            .map(|m| m.id.get())
            .map_err(|_| WorkError::Denied)
    }
}

fn native_failure(error: serenity::Error) -> WorkError {
    match error {
        serenity::Error::Http(ref error)
            if error
                .status_code()
                .is_some_and(|status| matches!(status.as_u16(), 403 | 404)) =>
        {
            WorkError::Denied
        }
        _ => WorkError::Missing,
    }
}

fn check_audience(
    viewers: &BTreeSet<u64>,
    allowed: &BTreeSet<u64>,
    actor: u64,
    bot: u64,
) -> Result<(), WorkError> {
    if !viewers.contains(&actor) || viewers.iter().any(|id| *id != bot && !allowed.contains(id)) {
        Err(WorkError::Denied)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn malformed_restored_ids_fail_before_snowflake_construction_or_network() {
        let transport = DiscordWorkDelivery(Arc::new(Http::new("synthetic-fixture")));
        let audience = BTreeSet::from([1]);
        for (scope, actor, origin) in [
            (WorkScope::Personal { owner: 1 }, 0, 2),
            (WorkScope::Personal { owner: 1 }, 1, 0),
            (
                WorkScope::Team {
                    guild: 0,
                    channel: 2,
                },
                1,
                2,
            ),
        ] {
            assert_eq!(
                transport
                    .authorize(
                        &scope,
                        actor,
                        origin,
                        &WorkDestination::Personal { principal: 1 },
                        &audience
                    )
                    .await
                    .unwrap_err(),
                WorkError::Invalid
            );
        }
        assert_eq!(
            transport.send(0, "synthetic").await,
            Err(WorkError::Invalid)
        );
    }

    #[test]
    fn shared_channel_requires_every_viewer_and_includes_owner_and_other_bots() {
        let allowed = BTreeSet::from([1, 2]);
        assert_eq!(
            check_audience(&BTreeSet::from([1, 2, 9]), &allowed, 1, 9),
            Ok(())
        );
        for viewers in [
            BTreeSet::from([1, 2, 3, 9]),
            BTreeSet::from([1, 2, 8, 9]),
            BTreeSet::from([2, 9]),
        ] {
            assert_eq!(
                check_audience(&viewers, &allowed, 1, 9),
                Err(WorkError::Denied)
            );
        }
    }
}
