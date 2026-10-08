//! Read-only fresh native proof for private Work continuity. Threads require
//! separate parent/membership proof and are deliberately unavailable here.
use crate::{
    runtime::continuity_context::ContinuityAccessProvider,
    work::{WorkAccess, WorkError, WorkScope},
};
use serenity::all::{ChannelId, ChannelType, GuildId, Http, Permissions, UserId};
use std::{future::Future, pin::Pin, sync::Arc};

pub(crate) struct DiscordContinuityAccess(pub Arc<Http>);
impl ContinuityAccessProvider for DiscordContinuityAccess {
    fn authorize<'a>(
        &'a self,
        scope: &'a WorkScope,
        actor: u64,
        channel: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
        Box::pin(async move {
            if actor == 0 || channel == 0 {
                return Err(WorkError::Invalid);
            }
            match scope {
                WorkScope::Personal { owner } => {
                    if *owner != actor {
                        return Err(WorkError::Denied);
                    }
                    let native = ChannelId::new(channel)
                        .to_channel(&self.0)
                        .await
                        .map_err(|_| WorkError::Denied)?;
                    let serenity::all::Channel::Private(native) = native else {
                        return Err(WorkError::Denied);
                    };
                    if native.id.get() != channel
                        || native.kind != ChannelType::Private
                        || native.recipient.id.get() != actor
                        || native.recipient.bot
                    {
                        return Err(WorkError::Denied);
                    }
                    Ok(WorkAccess {
                        actor,
                        guild: None,
                        channel,
                        can_view: true,
                        can_manage: false,
                    })
                }
                WorkScope::Team {
                    guild,
                    channel: origin,
                } => {
                    if *guild == 0 || *origin != channel {
                        return Err(WorkError::Denied);
                    }
                    let (member, native_guild, native_channel) = tokio::try_join!(
                        GuildId::new(*guild).member(&self.0, UserId::new(actor)),
                        GuildId::new(*guild).to_partial_guild(&self.0),
                        ChannelId::new(channel).to_channel(&self.0),
                    )
                    .map_err(|_| WorkError::Denied)?;
                    let native = native_channel.guild().ok_or(WorkError::Denied)?;
                    if member.user.id.get() != actor
                        || member.user.bot
                        || native_guild.id.get() != *guild
                        || native.guild_id.get() != *guild
                        || native.id.get() != channel
                        || native.kind != ChannelType::Text
                        || !native_guild
                            .roles
                            .contains_key(&serenity::all::RoleId::new(*guild))
                        || member
                            .roles
                            .iter()
                            .any(|role| !native_guild.roles.contains_key(role))
                    {
                        return Err(WorkError::Denied);
                    }
                    let permissions = native_guild.user_permissions_in(&native, &member);
                    if !permissions.contains(Permissions::VIEW_CHANNEL) {
                        return Err(WorkError::Denied);
                    }
                    Ok(WorkAccess {
                        actor,
                        guild: Some(*guild),
                        channel,
                        can_view: true,
                        can_manage: permissions.contains(Permissions::MANAGE_GUILD),
                    })
                }
            }
        })
    }
}
#[cfg(test)]
mod tests;
