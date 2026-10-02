//! Exact owner-matrix role/access operations, reserved before remote I/O.
use super::*;
use crate::community_ops::{Ledger, Policy};
use serenity::all::{
    EditRole, Member, PartialGuild, PermissionOverwrite, PermissionOverwriteType, RoleId, UserId,
};
use std::collections::HashMap;

pub(super) fn handles(operation: &Operation) -> bool {
    !matches!(
        operation,
        Operation::Topic { .. } | Operation::Move { .. } | Operation::CreateText { .. }
    )
}
fn posting() -> Permissions {
    Permissions::SEND_MESSAGES
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::CREATE_PRIVATE_THREADS
}
fn ordinary() -> Permissions {
    posting()
        | Permissions::VIEW_CHANNEL
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::ADD_REACTIONS
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::USE_VAD
        | Permissions::STREAM
        | Permissions::USE_APPLICATION_COMMANDS
        | Permissions::USE_EXTERNAL_EMOJIS
        | Permissions::USE_EXTERNAL_STICKERS
}
fn interest_permissions_safe(permissions: Permissions) -> bool {
    permissions.is_empty()
}
struct Snapshot {
    guild: PartialGuild,
    channels: HashMap<ChannelId, GuildChannel>,
    bot: Member,
    member: Option<Member>,
}
impl Snapshot {
    fn signature(&self) -> Result<serde_json::Value, &'static str> {
        serde_json::to_value((
            &self.guild.roles,
            &self.channels,
            &self.bot,
            &self.member,
            self.guild.owner_id,
        ))
        .map_err(|_| "invalid operational observation")
    }
    fn channel(&self, id: u64) -> Result<&GuildChannel, &'static str> {
        self.channels
            .get(&ChannelId::new(id))
            .ok_or("channel unavailable")
    }
    fn role(&self, id: u64) -> Result<&serenity::all::Role, &'static str> {
        self.guild
            .roles
            .get(&RoleId::new(id))
            .ok_or("role unavailable")
    }
    fn everyone(&self) -> Result<Permissions, &'static str> {
        Ok(self.role(self.guild.id.get())?.permissions)
    }
    fn bot_rights(&self) -> Permissions {
        self.bot
            .roles
            .iter()
            .chain(std::iter::once(&self.guild.id.everyone_role()))
            .filter_map(|id| self.guild.roles.get(id))
            .fold(Permissions::empty(), |p, r| p | r.permissions)
    }
    fn public_target(&self, policy: &Policy, id: u64) -> Result<&GuildChannel, &'static str> {
        let c = self.channel(id)?;
        let parent = c.parent_id.ok_or("channel has no public parent")?;
        if policy.protected_channels.contains(&id)
            || c.nsfw
            || !matches!(
                c.kind,
                ChannelType::Text | ChannelType::News | ChannelType::Forum
            )
            || !policy.public_categories.contains(&parent.get())
            || !public_category(self.channel(parent.get())?, self.everyone()?)
            || c.permission_overwrites
                .iter()
                .any(|o| o.deny.contains(Permissions::VIEW_CHANNEL))
        {
            return Err("channel is outside ordinary public scope");
        }
        let mut effective = self.everyone()?;
        for overwrite in &c.permission_overwrites {
            if overwrite.kind == PermissionOverwriteType::Role(c.guild_id.everyone_role()) {
                effective.remove(overwrite.deny);
                effective.insert(overwrite.allow);
            }
        }
        if !effective.contains(Permissions::VIEW_CHANNEL) {
            return Err("target is not visible to ordinary members");
        }
        Ok(c)
    }
    fn ordinary_role(
        &self,
        policy: &Policy,
        id: u64,
    ) -> Result<&serenity::all::Role, &'static str> {
        let r = self.role(id)?;
        let top = self
            .bot
            .roles
            .iter()
            .filter_map(|id| self.guild.roles.get(id))
            .map(|r| r.position)
            .max()
            .unwrap_or(0);
        if !policy.ordinary_roles.contains(&id)
            || id == self.guild.id.get()
            || r.managed
            || !interest_permissions_safe(r.permissions)
            || r.position >= top
        {
            return Err("role is privileged, managed, or outside hierarchy/matrix");
        }
        // An ordinary interest role must not be a key to a private/adult entitlement.
        let base_everyone = self.everyone()?;
        for c in self.channels.values() {
            let mut everyone = base_everyone;
            for o in &c.permission_overwrites {
                if o.kind == PermissionOverwriteType::Role(self.guild.id.everyone_role()) {
                    everyone.remove(o.deny);
                    everyone.insert(o.allow);
                }
            }
            let private = c.nsfw
                || policy.protected_channels.contains(&c.id.get())
                || !everyone.contains(Permissions::VIEW_CHANNEL)
                || c.parent_id.is_some_and(|parent| {
                    self.channels
                        .get(&parent)
                        .is_none_or(|p| !public_category(p, base_everyone))
                })
                || c.permission_overwrites
                    .iter()
                    .any(|o| o.deny.contains(Permissions::VIEW_CHANNEL));
            if private
                && c.permission_overwrites.iter().any(|o| {
                    o.kind == PermissionOverwriteType::Role(r.id)
                        && o.allow.contains(Permissions::VIEW_CHANNEL)
                })
            {
                return Err("role controls a protected entitlement");
            }
        }
        Ok(r)
    }
}
async fn snapshot(http: &Http, policy: &Policy, action: &Action) -> Result<Snapshot, &'static str> {
    let id = GuildId::new(policy.guild);
    let guild = id
        .to_partial_guild(http)
        .await
        .map_err(|_| "guild unavailable")?;
    if guild.owner_id.get() != policy.owner {
        return Err("owner authority changed");
    }
    let me = http
        .get_current_user()
        .await
        .map_err(|_| "bot identity unavailable")?;
    let bot = id
        .member(http, me.id)
        .await
        .map_err(|_| "bot permissions unavailable")?;
    let member = match action.operation {
        Operation::Membership { member, .. } => Some(
            id.member(http, UserId::new(member))
                .await
                .map_err(|_| "member unavailable")?,
        ),
        _ => None,
    };
    Ok(Snapshot {
        guild,
        channels: id
            .channels(http)
            .await
            .map_err(|_| "channels unavailable")?,
        bot,
        member,
    })
}
fn desired_overwrites(
    policy: &Policy,
    c: &GuildChannel,
) -> Result<Vec<PermissionOverwrite>, &'static str> {
    let mut result = c.permission_overwrites.clone();
    for o in &result {
        if o.allow.intersects(posting())
            && !matches!(o.kind, PermissionOverwriteType::Role(id) if id == c.guild_id.everyone_role() || policy.ordinary_roles.contains(&id.get()))
        {
            return Err("posting override outside approved ordinary matrix");
        }
    }
    for id in std::iter::once(c.guild_id.get()).chain(policy.ordinary_roles.iter().copied()) {
        let kind = PermissionOverwriteType::Role(RoleId::new(id));
        if let Some(o) = result.iter_mut().find(|o| o.kind == kind) {
            o.allow.remove(posting());
            o.deny.insert(posting());
        } else {
            result.push(PermissionOverwrite {
                kind,
                allow: Permissions::empty(),
                deny: posting(),
            });
        }
    }
    Ok(result)
}
fn restore_channel(ledger: &Ledger, channel: u64, key: &str) -> Result<GuildChannel, &'static str> {
    let r = ledger
        .receipts
        .get(key)
        .ok_or("archive receipt unavailable")?;
    if r.status != Status::Verified
        || !matches!(r.action.operation, Operation::Archive { channel: id, .. } if id == channel)
    {
        return Err("archive receipt is not verified authority");
    }
    serde_json::from_value(r.before["channel"].clone())
        .map_err(|_| "archive recovery snapshot invalid")
}
fn preflight(
    s: &Snapshot,
    p: &Policy,
    a: &Action,
    ledger: &Ledger,
) -> Result<serde_json::Value, &'static str> {
    let rights = s.bot_rights();
    let needs_roles = matches!(
        a.operation,
        Operation::CreateInterestRole { .. }
            | Operation::RetireInterestRole { .. }
            | Operation::Membership { .. }
            | Operation::Access { .. }
            | Operation::Archive { .. }
            | Operation::RestoreArchive { .. }
    );
    if !rights.contains(Permissions::ADMINISTRATOR)
        && (!rights.contains(Permissions::MANAGE_ROLES) && needs_roles)
    {
        return Err("bot lacks current Manage Roles");
    }
    match &a.operation {
        Operation::CreateInterestRole { name } => {
            if s.guild.roles.values().any(|r| r.name == *name) {
                return Err("interest role name collision");
            }
            Ok(serde_json::json!({"roles":s.guild.roles}))
        }
        Operation::RetireInterestRole { role, name } => {
            let r = s.ordinary_role(p, *role)?;
            if s.guild
                .roles
                .values()
                .any(|other| other.id != r.id && other.name == *name)
            {
                return Err("retired role name collision");
            }
            Ok(serde_json::json!({"role":r}))
        }
        Operation::Membership { role, .. } => {
            s.ordinary_role(p, *role)?;
            let m = s.member.as_ref().ok_or("member observation missing")?;
            if m.pending
                || m.user.bot
                || m.user.id == s.guild.owner_id
                || m.roles.iter().any(|id| {
                    s.guild
                        .roles
                        .get(id)
                        .is_none_or(|r| !ordinary().contains(r.permissions))
                })
            {
                return Err("rules acceptance or ordinary member authority missing");
            }
            Ok(serde_json::json!({"member":m}))
        }
        Operation::Access {
            channel,
            role,
            allow,
            deny,
        } => {
            let c = s.public_target(p, *channel)?;
            s.ordinary_role(p, *role)?;
            let requested =
                Permissions::from_bits(*allow | *deny).ok_or("unknown permission bits")?;
            if !ordinary().contains(requested) || requested.intersects(Permissions::VIEW_CHANNEL) {
                return Err("view or privileged access changes require human review");
            }
            Ok(serde_json::json!({"channel":c}))
        }
        Operation::Archive { channel, category } => {
            let c = s.public_target(p, *channel)?;
            if !public_category(s.channel(*category)?, s.everyone()?) {
                return Err("archive destination is not public");
            }
            for id in &p.ordinary_roles {
                s.ordinary_role(p, *id)?;
            }
            desired_overwrites(p, c)?;
            if !rights.intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_CHANNELS) {
                return Err("bot lacks Manage Channels");
            }
            Ok(serde_json::json!({"channel":c}))
        }
        Operation::RestoreArchive { channel, receipt } => {
            let c = s.public_target(p, *channel)?;
            let original = restore_channel(ledger, *channel, receipt)?;
            let archived = ledger
                .receipts
                .get(receipt)
                .and_then(|r| r.observed.as_ref())
                .ok_or("archive observation missing")?;
            if serde_json::to_value(c).map_err(|_| "channel observation invalid")?
                != archived["channel"]
            {
                return Err("archive state drift requires review");
            }
            let parent = original.parent_id.ok_or("archive origin missing")?;
            if !p.public_categories.contains(&parent.get())
                || !public_category(s.channel(parent.get())?, s.everyone()?)
            {
                return Err("restore destination unavailable or private");
            }
            if !rights.intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_CHANNELS) {
                return Err("bot lacks Manage Channels");
            }
            Ok(serde_json::json!({"channel":c}))
        }
        _ => Err("unsupported extended operation"),
    }
}
async fn execute(
    http: &Http,
    p: &Policy,
    a: &Action,
    s: &Snapshot,
    l: &Ledger,
) -> Result<Option<u64>, &'static str> {
    let g = GuildId::new(p.guild);
    let reason = "Owner-approved ordinary community operation";
    match &a.operation {
        Operation::CreateInterestRole { name } => g
            .create_role(
                http,
                EditRole::new()
                    .name(name)
                    .permissions(Permissions::empty())
                    .hoist(false)
                    .mentionable(false)
                    .audit_log_reason(reason),
            )
            .await
            .map(|r| Some(r.id.get()))
            .map_err(|_| "role creation uncertain"),
        Operation::RetireInterestRole { role, name } => g
            .edit_role(
                http,
                RoleId::new(*role),
                EditRole::new().name(name).audit_log_reason(reason),
            )
            .await
            .map(|_| None)
            .map_err(|_| "role retirement uncertain"),
        Operation::Membership { role, grant, .. } => {
            let m = s.member.as_ref().ok_or("member missing")?;
            if *grant {
                m.add_role(http, RoleId::new(*role)).await
            } else {
                m.remove_role(http, RoleId::new(*role)).await
            }
            .map_err(|_| "membership outcome uncertain")?;
            Ok(None)
        }
        Operation::Access {
            channel,
            role,
            allow,
            deny,
        } => {
            let mut overwrites = s.channel(*channel)?.permission_overwrites.clone();
            let kind = PermissionOverwriteType::Role(RoleId::new(*role));
            let ordinary_mask = ordinary() & !Permissions::VIEW_CHANNEL;
            let allow = Permissions::from_bits(*allow).ok_or("unknown allow bits")?;
            let deny = Permissions::from_bits(*deny).ok_or("unknown deny bits")?;
            if let Some(o) = overwrites.iter_mut().find(|o| o.kind == kind) {
                o.allow = (o.allow & !ordinary_mask) | allow;
                o.deny = (o.deny & !ordinary_mask) | deny;
            } else {
                overwrites.push(PermissionOverwrite { kind, allow, deny });
            }
            ChannelId::new(*channel)
                .edit(
                    http,
                    EditChannel::new()
                        .permissions(overwrites)
                        .audit_log_reason(reason),
                )
                .await
                .map_err(|_| "access outcome uncertain")?;
            Ok(None)
        }
        Operation::Archive { channel, category } => {
            let overwrites = desired_overwrites(p, s.channel(*channel)?)?;
            ChannelId::new(*channel)
                .edit(
                    http,
                    EditChannel::new()
                        .category(ChannelId::new(*category))
                        .permissions(overwrites)
                        .audit_log_reason(reason),
                )
                .await
                .map_err(|_| "archive outcome uncertain")?;
            Ok(None)
        }
        Operation::RestoreArchive { channel, receipt } => {
            let original = restore_channel(l, *channel, receipt)?;
            ChannelId::new(*channel)
                .edit(
                    http,
                    EditChannel::new()
                        .category(original.parent_id.ok_or("origin missing")?)
                        .permissions(original.permission_overwrites)
                        .audit_log_reason(reason),
                )
                .await
                .map_err(|_| "restore outcome uncertain")?;
            Ok(None)
        }
        _ => Err("unsupported extended operation"),
    }
}
fn same_roles_except(before: &Snapshot, after: &Snapshot, changed: Option<u64>) -> bool {
    let filtered = |s: &Snapshot| -> serde_json::Value {
        serde_json::to_value(
            s.guild
                .roles
                .iter()
                .filter(|(id, _)| Some(id.get()) != changed)
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap_or(serde_json::Value::Null)
    };
    before.guild.owner_id == after.guild.owner_id && filtered(before) == filtered(after)
}
fn non_posting(c: &GuildChannel) -> serde_json::Value {
    let mut c = c.clone();
    for o in &mut c.permission_overwrites {
        o.allow.remove(posting());
        o.deny.remove(posting());
    }
    c.permission_overwrites
        .retain(|o| !o.allow.is_empty() || !o.deny.is_empty());
    access(&c)
}
fn verify(
    p: &Policy,
    a: &Action,
    before: &Snapshot,
    after: &Snapshot,
    ledger: &Ledger,
    created: Option<u64>,
) -> Result<serde_json::Value, &'static str> {
    match &a.operation {
        Operation::CreateInterestRole { name } => {
            let id = created.ok_or("created role identity unavailable")?;
            let role = after.role(id)?;
            if role.name != *name
                || !role.permissions.is_empty()
                || role.hoist
                || role.mentionable
                || role.managed
                || !same_roles_except(before, after, Some(id))
                || before.guild.roles.contains_key(&role.id)
            {
                return Err("role creation readback differs");
            }
            Ok(serde_json::json!({"role":role}))
        }
        Operation::RetireInterestRole { role, name } => {
            let original = before.role(*role)?;
            let mut observed = after.role(*role)?.clone();
            if observed.name != *name {
                return Err("retirement name differs");
            }
            observed.name = original.name.clone();
            if observed != *original || !same_roles_except(before, after, Some(*role)) {
                return Err("retirement changed protected state");
            }
            Ok(serde_json::json!({"role":after.role(*role)?}))
        }
        Operation::Membership { role, grant, .. } => {
            let original = before.member.as_ref().ok_or("member missing")?;
            let observed = after.member.as_ref().ok_or("member missing")?;
            let mut expected = original.roles.clone();
            expected.retain(|id| id.get() != *role);
            if *grant {
                expected.push(RoleId::new(*role));
            }
            expected.sort();
            let mut actual = observed.roles.clone();
            actual.sort();
            if actual != expected || observed.pending || !same_roles_except(before, after, None) {
                return Err("membership readback differs");
            }
            Ok(serde_json::json!({"member":observed}))
        }
        Operation::Access {
            channel,
            role,
            allow,
            deny,
        } => {
            let original = before.channel(*channel)?;
            let observed = after.channel(*channel)?;
            let kind = PermissionOverwriteType::Role(RoleId::new(*role));
            let mask = ordinary() & !Permissions::VIEW_CHANNEL;
            let previous = original
                .permission_overwrites
                .iter()
                .find(|o| o.kind == kind);
            let expected_allow = previous
                .map_or(Permissions::empty(), |o| o.allow & !mask)
                .bits()
                | allow;
            let expected_deny = previous
                .map_or(Permissions::empty(), |o| o.deny & !mask)
                .bits()
                | deny;
            if !observed.permission_overwrites.iter().any(|o| {
                o.kind == kind && o.allow.bits() == expected_allow && o.deny.bits() == expected_deny
            }) {
                return Err("matrix readback differs");
            }
            let strip = |c: &GuildChannel| {
                let mut c = c.clone();
                c.permission_overwrites.retain(|o| o.kind != kind);
                record(&c)
            };
            if strip(original) != strip(observed) || !same_roles_except(before, after, None) {
                return Err("access changed unrelated state");
            }
            Ok(serde_json::json!({"channel":observed}))
        }
        Operation::Archive { channel, category } => {
            let original = before.channel(*channel)?;
            let observed = after.channel(*channel)?;
            if observed.parent_id != Some(ChannelId::new(*category))
                || observed.name != original.name
                || observed.topic != original.topic
                || non_posting(original) != non_posting(observed)
                || !same_roles_except(before, after, None)
            {
                return Err("archive changed protected state");
            }
            for id in std::iter::once(p.guild).chain(p.ordinary_roles.iter().copied()) {
                if !observed.permission_overwrites.iter().any(|o| {
                    o.kind == PermissionOverwriteType::Role(RoleId::new(id))
                        && o.deny.contains(posting())
                        && !o.allow.intersects(posting())
                }) {
                    return Err("archive posting closure not proven");
                }
            }
            if observed
                .permission_overwrites
                .iter()
                .any(|o| o.allow.intersects(posting()))
            {
                return Err("archive posting remains enabled");
            }
            Ok(serde_json::json!({"channel":observed}))
        }
        Operation::RestoreArchive { channel, receipt } => {
            let original = restore_channel(ledger, *channel, receipt)?;
            let observed = after.channel(*channel)?;
            if record(&original) != record(observed) || !same_roles_except(before, after, None) {
                return Err("archive restore differs");
            }
            Ok(serde_json::json!({"channel":observed}))
        }
        _ => Err("unsupported extended readback"),
    }
}
mod executor;
use executor::{Executor, run_one};
struct DiscordExecutor<'a> {
    http: &'a Http,
    policy: &'a Policy,
    action: &'a Action,
    ledger: Ledger,
}
impl Executor for DiscordExecutor<'_> {
    type State = Snapshot;
    async fn observe(&self) -> Result<Snapshot, &'static str> {
        snapshot(self.http, self.policy, self.action).await
    }
    fn before(&self, state: &Snapshot) -> Result<serde_json::Value, &'static str> {
        preflight(state, self.policy, self.action, &self.ledger)
    }
    fn signature(&self, state: &Snapshot) -> Result<serde_json::Value, &'static str> {
        state.signature()
    }
    async fn mutate(&self, state: &Snapshot) -> Result<Option<u64>, &'static str> {
        execute(self.http, self.policy, self.action, state, &self.ledger).await
    }
    fn verify(
        &self,
        before: &Snapshot,
        after: &Snapshot,
        created: Option<u64>,
    ) -> Result<serde_json::Value, &'static str> {
        verify(
            self.policy,
            self.action,
            before,
            after,
            &self.ledger,
            created,
        )
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    state: &crate::runtime::AppState,
    http: &Http,
    path: &Path,
    policy: &Policy,
    digest: &str,
    action: &Action,
    lease: &std::sync::Arc<Lease>,
    ledger: &mut Ledger,
    cancel: CancellationToken,
    now: u64,
) -> Result<(), &'static str> {
    let executor = DiscordExecutor {
        http,
        policy,
        action,
        ledger: ledger.clone(),
    };
    run_one(
        &executor,
        policy,
        digest,
        action,
        ledger,
        cancel,
        now,
        |ledger| {
            let ledger = ledger.clone();
            let lease = std::sync::Arc::clone(lease);
            async move { super::save_receipts(state, &lease, &ledger).await }
        },
        || state.community_policy(path.to_owned()),
    )
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interest_role_cannot_grant_base_view_to_hidden_channel_without_overwrites() {
        let everyone = Permissions::empty();
        let channel = channel(vec![]);
        assert!(channel.permission_overwrites.is_empty());
        assert!(!everyone.contains(Permissions::VIEW_CHANNEL));
        let interest_role = Permissions::VIEW_CHANNEL;
        assert!((everyone | interest_role).contains(Permissions::VIEW_CHANNEL));
        assert!(!interest_permissions_safe(interest_role));
        assert!(interest_permissions_safe(Permissions::empty()));
    }
    fn policy() -> Policy {
        serde_json::from_value(serde_json::json!({"version":1,"guild":1,"owner":2,"mode":"apply","daily_limit":5,"daily_creations":2,"public_categories":[3],"protected_channels":[],"ordinary_roles":[5],"actions":[]})).unwrap()
    }
    fn channel(overwrites: Vec<PermissionOverwrite>) -> GuildChannel {
        serde_json::from_value(serde_json::json!({"id":"4","guild_id":"1","type":0,"name":"old-project","position":0,"parent_id":"3","permission_overwrites":overwrites,"nsfw":false})).unwrap()
    }
    #[test]
    fn archive_preserves_view_and_closes_every_approved_ordinary_role_combination() {
        let p = policy();
        let original = channel(vec![PermissionOverwrite {
            kind: PermissionOverwriteType::Role(RoleId::new(5)),
            allow: Permissions::VIEW_CHANNEL | posting(),
            deny: Permissions::empty(),
        }]);
        let mut archived = original.clone();
        archived.permission_overwrites = desired_overwrites(&p, &original).unwrap();
        assert_eq!(non_posting(&original), non_posting(&archived));
        for roles in [vec![], vec![5]] {
            let mut effective = Permissions::VIEW_CHANNEL | posting();
            for id in std::iter::once(1).chain(roles) {
                let o = archived
                    .permission_overwrites
                    .iter()
                    .find(|o| o.kind == PermissionOverwriteType::Role(RoleId::new(id)))
                    .unwrap();
                effective.remove(o.deny);
                effective.insert(o.allow);
            }
            assert!(effective.contains(Permissions::VIEW_CHANNEL));
            assert!(!effective.intersects(posting()));
        }
    }
    #[test]
    fn archive_refuses_nonmatrix_member_or_role_posting_overrides() {
        let p = policy();
        for kind in [
            PermissionOverwriteType::Role(RoleId::new(99)),
            PermissionOverwriteType::Member(UserId::new(99)),
        ] {
            assert!(
                desired_overwrites(
                    &p,
                    &channel(vec![PermissionOverwrite {
                        kind,
                        allow: Permissions::SEND_MESSAGES,
                        deny: Permissions::empty()
                    }])
                )
                .is_err()
            );
        }
    }
}
