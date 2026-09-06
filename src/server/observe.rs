//! What the engine knows about a live guild: a plain snapshot, serenity-free.
//!
//! `server::discord::snapshot` fills this from REST; the fake guild in
//! `server::apply`'s tests fills it by hand. Permission sets are carried as
//! the client-facing names serenity's `get_permission_names` returns, so the
//! pure `diff` can compare a plan (names) with the guild (names) without a
//! bit table of its own. A bit serenity has no name for is dropped here and,
//! if the plan mentions that target, overwritten on apply; that is what
//! "set this overwrite to the plan" means.

use std::collections::BTreeSet;

use super::ChannelKind;
use super::normalize_text_name;

/// Discord's `COMMUNITY` guild feature flag, required for forums,
/// announcement channels, and stages.
pub const COMMUNITY_FEATURE: &str = "COMMUNITY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildSnapshot {
    pub guild_id: u64,
    pub features: Vec<String>,
    pub roles: Vec<RoleState>,
    pub channels: Vec<ChannelState>,
    pub bot: BotState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleState {
    pub id: u64,
    pub name: String,
    pub position: u16,
    /// Owned by an integration; the engine never edits these.
    pub managed: bool,
    pub hoist: bool,
    pub mentionable: bool,
    /// Discord's raw colour; `0` is "no colour".
    pub colour: u32,
    pub permissions: Vec<String>,
}

/// How a live channel maps onto the plan vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelClass {
    Category,
    Kind(ChannelKind),
    /// Threads, DMs, directories, media channels: never matched by a plan.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelState {
    pub id: u64,
    pub name: String,
    pub class: ChannelClass,
    pub parent: Option<u64>,
    pub topic: Option<String>,
    pub overwrites: Vec<OverwriteState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteTarget {
    Role(u64),
    Member(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverwriteState {
    pub target: OverwriteTarget,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// The bot's own membership: which roles it holds decides what it may touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotState {
    pub user_id: u64,
    pub role_ids: Vec<u64>,
}

impl ChannelState {
    #[must_use]
    pub fn overwrite_for_role(&self, role_id: u64) -> Option<&OverwriteState> {
        self.overwrites
            .iter()
            .find(|o| o.target == OverwriteTarget::Role(role_id))
    }
}

impl GuildSnapshot {
    #[must_use]
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    /// The `@everyone` role shares the guild's id.
    #[must_use]
    pub fn everyone_role(&self) -> Option<&RoleState> {
        self.roles.iter().find(|r| r.id == self.guild_id)
    }

    #[must_use]
    pub fn role_by_id(&self, id: u64) -> Option<&RoleState> {
        self.roles.iter().find(|r| r.id == id)
    }

    /// Every role with this exact name. More than one is an ambiguity the
    /// engine refuses to resolve (divider roles routinely share names).
    #[must_use]
    pub fn roles_named(&self, name: &str) -> Vec<&RoleState> {
        self.roles.iter().filter(|r| r.name == name).collect()
    }

    #[must_use]
    pub fn categories_named(&self, name: &str) -> Vec<&ChannelState> {
        self.channels
            .iter()
            .filter(|c| {
                c.class == ChannelClass::Category && c.name.trim().eq_ignore_ascii_case(name.trim())
            })
            .collect()
    }

    /// Channels matching a plan channel: same kind, and the name Discord
    /// would store for that kind.
    #[must_use]
    pub fn channels_matching(&self, name: &str, kind: ChannelKind) -> Vec<&ChannelState> {
        let wanted = plan_channel_key(name, kind);
        self.channels
            .iter()
            .filter(|c| {
                c.class == ChannelClass::Kind(kind) && plan_channel_key(&c.name, kind) == wanted
            })
            .collect()
    }

    /// The union of the bot's role permissions, `@everyone` included.
    #[must_use]
    pub fn bot_permissions(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        if let Some(everyone) = self.everyone_role() {
            names.extend(everyone.permissions.iter().cloned());
        }
        for id in &self.bot.role_ids {
            if let Some(role) = self.role_by_id(*id) {
                names.extend(role.permissions.iter().cloned());
            }
        }
        names
    }

    #[must_use]
    pub fn bot_is_administrator(&self) -> bool {
        self.bot_permissions().contains("Administrator")
    }

    /// Highest position among the bot's roles; `@everyone` sits at 0.
    #[must_use]
    pub fn bot_top_position(&self) -> u16 {
        self.bot
            .role_ids
            .iter()
            .filter_map(|id| self.role_by_id(*id))
            .map(|r| r.position)
            .max()
            .unwrap_or(0)
    }
}

/// The comparison key for a channel name of a given kind.
#[must_use]
pub fn plan_channel_key(name: &str, kind: ChannelKind) -> String {
    if kind.normalizes_name() {
        normalize_text_name(name)
    } else {
        name.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> GuildSnapshot {
        GuildSnapshot {
            guild_id: 10,
            features: vec![COMMUNITY_FEATURE.into()],
            roles: vec![
                RoleState {
                    id: 10,
                    name: "@everyone".into(),
                    position: 0,
                    managed: false,
                    hoist: false,
                    mentionable: false,
                    colour: 0,
                    permissions: vec!["View Channel".into()],
                },
                RoleState {
                    id: 11,
                    name: "Abbey".into(),
                    position: 7,
                    managed: true,
                    hoist: false,
                    mentionable: false,
                    colour: 0,
                    permissions: vec!["Manage Channels".into()],
                },
                RoleState {
                    id: 12,
                    name: "Divider".into(),
                    position: 3,
                    managed: false,
                    hoist: false,
                    mentionable: false,
                    colour: 0,
                    permissions: vec![],
                },
                RoleState {
                    id: 13,
                    name: "Divider".into(),
                    position: 2,
                    managed: false,
                    hoist: false,
                    mentionable: false,
                    colour: 0,
                    permissions: vec![],
                },
            ],
            channels: vec![
                ChannelState {
                    id: 20,
                    name: "start here".into(),
                    class: ChannelClass::Category,
                    parent: None,
                    topic: None,
                    overwrites: vec![],
                },
                ChannelState {
                    id: 21,
                    name: "general".into(),
                    class: ChannelClass::Kind(ChannelKind::Text),
                    parent: Some(20),
                    topic: None,
                    overwrites: vec![],
                },
                ChannelState {
                    id: 22,
                    name: "Squad 1".into(),
                    class: ChannelClass::Kind(ChannelKind::Voice),
                    parent: None,
                    topic: None,
                    overwrites: vec![],
                },
            ],
            bot: BotState {
                user_id: 99,
                role_ids: vec![11],
            },
        }
    }

    #[test]
    fn bot_permissions_union_everyone_and_held_roles() {
        let snap = snapshot();
        let perms = snap.bot_permissions();
        assert!(perms.contains("View Channel") && perms.contains("Manage Channels"));
        assert!(!snap.bot_is_administrator());
        assert_eq!(snap.bot_top_position(), 7);
    }

    #[test]
    fn matching_follows_discords_naming_rules_per_kind() {
        let snap = snapshot();
        assert_eq!(
            snap.channels_matching("General", ChannelKind::Text).len(),
            1
        );
        assert!(
            snap.channels_matching("general", ChannelKind::Forum)
                .is_empty(),
            "kind is part of the key"
        );
        assert_eq!(
            snap.channels_matching("Squad 1", ChannelKind::Voice).len(),
            1
        );
        assert!(
            snap.channels_matching("squad-1", ChannelKind::Voice)
                .is_empty(),
            "voice names are not normalized"
        );
        assert_eq!(snap.categories_named("START HERE").len(), 1);
        assert_eq!(
            snap.roles_named("Divider").len(),
            2,
            "shared names surface as two matches"
        );
        assert_eq!(snap.everyone_role().map(|r| r.id), Some(10));
    }
}
