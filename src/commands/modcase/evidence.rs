//! Native GET-only facts. No serialized record or interaction permission hint is authority.
use super::Envelope;
use crate::{
    moderation::shadow::{FreshAuthority, NativeAuthorityFacts, SourceVersion},
    runtime,
};
use serenity::all::{
    ChannelId, ChannelType, GuildId, Http, MessageId, Permissions, RoleId, UserId,
};

const UNAVAILABLE: &str = "Current native case access is unproved.";
pub(super) struct Facts {
    pub(super) owner: u64,
    native: Envelope,
    at: u64,
    permissions: Permissions,
}
pub(super) async fn basic(http: &Http, native: Envelope) -> Result<Facts, &'static str> {
    // Bound the lease from the earliest request; response delays cannot renew
    // already fetched membership or permission evidence.
    let at = runtime::now();
    let (guild, member, channel) = tokio::try_join!(
        GuildId::new(native.guild).to_partial_guild(http),
        GuildId::new(native.guild).member(http, UserId::new(native.actor)),
        ChannelId::new(native.origin).to_channel(http),
    )
    .map_err(|_| UNAVAILABLE)?;
    let channel = channel.guild().ok_or(UNAVAILABLE)?;
    if guild.id.get() != native.guild
        || guild.owner_id.get() == 0
        || member.user.id.get() != native.actor
        || member.user.bot
        || member.guild_id != guild.id
        || channel.id.get() != native.origin
        || channel.guild_id != guild.id
        || !matches!(channel.kind, ChannelType::Text | ChannelType::News)
        || !guild.roles.contains_key(&RoleId::new(native.guild))
        || guild
            .roles
            .iter()
            .any(|(id, role)| id.get() == 0 || *id != role.id)
        || member
            .roles
            .iter()
            .any(|id| id.get() == 0 || !guild.roles.contains_key(id))
    {
        return Err(UNAVAILABLE);
    }
    let permissions = guild.user_permissions_in(&channel, &member);
    if !permissions.contains(Permissions::VIEW_CHANNEL) {
        return Err(UNAVAILABLE);
    }
    Ok(Facts {
        owner: guild.owner_id.get(),
        native,
        at,
        permissions,
    })
}
impl Facts {
    pub(super) fn authority(
        self,
        observed: Option<SourceVersion>,
        source_at: Option<u64>,
    ) -> Result<FreshAuthority, &'static str> {
        if observed.is_some() != source_at.is_some() {
            return Err(UNAVAILABLE);
        }
        FreshAuthority::verified(NativeAuthorityFacts {
            guild: self.native.guild,
            owner: self.owner,
            actor: self.native.actor,
            origin: self.native.origin,
            at: source_at.map_or(self.at, |at| at.min(self.at)),
            current_member: true,
            can_view_origin: true,
            can_view_source: observed.is_some(),
            complete_permissions: true,
            can_delete: self.permissions.contains(Permissions::MANAGE_MESSAGES),
            can_timeout: self.permissions.contains(Permissions::MODERATE_MEMBERS),
            observed_source: observed,
        })
    }
}
/// Unavailable/changed source cannot qualify confident Agree/Disagree/resolve;
/// the pure domain permits only an explicit NeedsContext receipt in that case.
pub(super) async fn source(
    http: &Http,
    native: Envelope,
    expected: &SourceVersion,
    owner: u64,
) -> Result<(Option<SourceVersion>, Option<u64>), &'static str> {
    if expected.guild != native.guild {
        return Err(UNAVAILABLE);
    }
    let source_origin = Envelope {
        origin: expected.channel,
        ..native
    };
    let Ok(access) = basic(http, source_origin).await else {
        return Ok((None, None));
    };
    if access.owner != owner {
        return Err(UNAVAILABLE);
    }
    if !access
        .permissions
        .contains(Permissions::READ_MESSAGE_HISTORY)
    {
        return Ok((None, None));
    }
    let Ok(message) = ChannelId::new(expected.channel)
        .message(http, MessageId::new(expected.message))
        .await
    else {
        return Ok((None, None));
    };
    if message.id.get() != expected.message
        || message.channel_id.get() != expected.channel
        || message.author.id.get() != expected.author
        || message.author.bot
        || message
            .guild_id
            .is_some_and(|guild| guild.get() != expected.guild)
    {
        return Ok((None, None));
    }
    let created = u64::try_from(message.timestamp.unix_timestamp()).map_err(|_| UNAVAILABLE)?;
    let edited = message
        .edited_timestamp
        .map(|at| u64::try_from(at.unix_timestamp()).map_err(|_| UNAVAILABLE))
        .transpose()?;
    let Ok(version) = SourceVersion::capture(
        expected.guild,
        expected.channel,
        expected.message,
        expected.author,
        created,
        edited,
        &message.content,
    ) else {
        return Ok((None, None));
    };
    if version != *expected {
        return Ok((None, None));
    }
    Ok((Some(version), Some(access.at)))
}
