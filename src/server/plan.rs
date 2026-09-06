//! Owned server plans: the TOML blueprint file and the invariants it must hold.
//!
//! `server.rs` keeps the four built-in archetypes as `&'static` tables. This
//! module is the owned, file-backed shape the plan engine consumes, and
//! `From<&Blueprint>` runs the archetypes through the same validator so one
//! property suite covers both. Nothing here imports serenity: permission names
//! are carried as strings and resolved against serenity's vocabulary in
//! `server::discord` (a unit test there checks every name in the shipped plan).
//!
//! What a plan deliberately cannot say:
//! - "delete": there is no field for it, and `diff::Change` has no such variant;
//! - an existing role's permissions: `roles[].permissions` is the target the
//!   engine reports as a manual step, never something it applies;
//! - "sync overwrites with the category": a channel either lists its own
//!   overwrites or inherits the category's at plan-read time (3.B.6).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::{
    Blueprint, ChannelKind, MAX_CHANNELS_PER_CATEGORY, NEVER_FOR_EVERYONE, normalize_text_name,
};

/// The overwrite target that means the guild's base role.
pub const EVERYONE: &str = "@everyone";
/// The permission whose denial to `@everyone` hides a channel (3.B.9).
pub const VIEW_CHANNEL: &str = "View Channel";
/// Discord's slowmode ceiling, in seconds.
pub const MAX_SLOWMODE_SECS: u16 = 21_600;
/// Discord's limit for role, category, and channel names.
pub const MAX_NAME_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub everyone: Everyone,
    #[serde(default)]
    pub roles: Vec<PlanRole>,
    #[serde(default)]
    pub categories: Vec<PlanCategory>,
}

/// The guild-level `@everyone` permission set. Reported, never applied.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Everyone {
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRole {
    pub name: String,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub mentionable: bool,
    /// `#rrggbb`; `None` renders as Discord's "no colour".
    #[serde(default, deserialize_with = "deserialize_colour")]
    pub colour: Option<u32>,
    /// Target guild permissions. A manual step, see the module doc.
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanCategory {
    pub name: String,
    #[serde(default)]
    pub overwrites: Vec<Overwrite>,
    #[serde(default)]
    pub channels: Vec<PlanChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanChannel {
    pub name: String,
    pub kind: ChannelKind,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub slowmode_secs: Option<u16>,
    /// Forum tags, applied at creation only.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Empty means "inherit the category's overwrites".
    #[serde(default)]
    pub overwrites: Vec<Overwrite>,
}

/// One permission overwrite: a role name (or [`EVERYONE`]) with the
/// permissions it is allowed and denied on that channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overwrite {
    pub role: String,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl Overwrite {
    /// The engine's own "hidden until revealed" marker: `@everyone` denied
    /// View Channel and nothing else. `diff` recognises exactly this shape.
    #[must_use]
    pub fn hide_marker() -> Self {
        Self {
            role: EVERYONE.into(),
            allow: Vec::new(),
            deny: vec![VIEW_CHANNEL.into()],
        }
    }

    /// Sorted, de-duplicated copy so two overwrites compare by content.
    #[must_use]
    pub fn normalized(&self) -> Self {
        Self {
            role: self.role.clone(),
            allow: sorted_unique(&self.allow),
            deny: sorted_unique(&self.deny),
        }
    }

    #[must_use]
    pub fn denies_view(&self) -> bool {
        self.deny.iter().any(|p| p == VIEW_CHANNEL)
    }

    #[must_use]
    pub fn is_hide_marker(&self) -> bool {
        self.normalized() == Self::hide_marker()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty()
    }
}

#[must_use]
pub fn sorted_unique(names: &[String]) -> Vec<String> {
    names
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Whether `@everyone` is denied View Channel anywhere in this overwrite set.
#[must_use]
pub fn denies_everyone_view(overwrites: &[Overwrite]) -> bool {
    overwrites
        .iter()
        .any(|o| o.role == EVERYONE && o.denies_view())
}

/// `#rrggbb` for a colour value.
#[must_use]
pub fn colour_hex(colour: u32) -> String {
    format!("#{colour:06x}")
}

/// Parse `#rrggbb` (case-insensitive). Empty means "no colour".
pub fn parse_colour(raw: &str) -> Result<Option<u32>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let hex = raw
        .strip_prefix('#')
        .ok_or_else(|| format!("colour {raw:?} must look like #rrggbb"))?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("colour {raw:?} must look like #rrggbb"));
    }
    u32::from_str_radix(hex, 16)
        .map(Some)
        .map_err(|_| format!("colour {raw:?} must look like #rrggbb"))
}

fn deserialize_colour<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<String> = Option::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(text) => parse_colour(&text).map_err(serde::de::Error::custom),
    }
}

impl Plan {
    /// Parse and validate a plan file. Every problem is reported, one per line.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let plan: Self = toml::from_str(text).map_err(|error| format!("plan file: {error}"))?;
        plan.validate().map_err(|problems| problems.join("\n"))?;
        Ok(plan)
    }

    #[must_use]
    pub fn role(&self, name: &str) -> Option<&PlanRole> {
        self.roles.iter().find(|r| r.name == name)
    }

    #[must_use]
    pub fn category(&self, name: &str) -> Option<&PlanCategory> {
        self.categories
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Every channel with its category, in plan order.
    pub fn channels(&self) -> impl Iterator<Item = (&PlanCategory, &PlanChannel)> {
        self.categories
            .iter()
            .flat_map(|c| c.channels.iter().map(move |ch| (c, ch)))
    }

    /// A channel's overwrites: its own, or the category's when it lists none.
    #[must_use]
    pub fn effective_overwrites<'a>(
        &'a self,
        category: &'a PlanCategory,
        channel: &'a PlanChannel,
    ) -> &'a [Overwrite] {
        if channel.overwrites.is_empty() {
            &category.overwrites
        } else {
            &channel.overwrites
        }
    }

    /// Every permission name the plan mentions anywhere, for vocabulary checks.
    #[must_use]
    pub fn permission_names(&self) -> BTreeSet<&str> {
        let mut names: BTreeSet<&str> = self
            .everyone
            .permissions
            .iter()
            .map(String::as_str)
            .collect();
        for role in &self.roles {
            names.extend(role.permissions.iter().map(String::as_str));
        }
        for category in &self.categories {
            for overwrite in category
                .overwrites
                .iter()
                .chain(category.channels.iter().flat_map(|ch| ch.overwrites.iter()))
            {
                names.extend(overwrite.allow.iter().map(String::as_str));
                names.extend(overwrite.deny.iter().map(String::as_str));
            }
        }
        names
    }

    /// Whether any channel needs Discord's `COMMUNITY` guild feature.
    #[must_use]
    pub fn needs_community(&self) -> bool {
        self.channels().any(|(_, ch)| ch.kind.needs_community())
    }

    /// Structural validation. Returns every problem found, not just the first.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push("plan name is empty".into());
        }
        self.validate_roles(&mut problems);
        self.validate_categories(&mut problems);
        self.validate_everyone(&mut problems);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    fn validate_roles(&self, problems: &mut Vec<String>) {
        let mut seen = BTreeSet::new();
        for role in &self.roles {
            let name = role.name.trim();
            if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
                problems.push(format!(
                    "role {:?}: name must be 1..={MAX_NAME_CHARS} characters",
                    role.name
                ));
            }
            if name == EVERYONE {
                problems.push("role @everyone cannot be declared; it always exists".into());
            }
            if !seen.insert(role.name.as_str()) {
                problems.push(format!("role {:?} is declared twice", role.name));
            }
            for dangerous in role
                .permissions
                .iter()
                .filter(|p| NEVER_FOR_EVERYONE.contains(&p.as_str()))
            {
                if role.permissions.iter().any(|p| p == "Administrator")
                    && dangerous != "Administrator"
                {
                    problems.push(format!(
                        "role {:?}: {dangerous:?} is redundant next to Administrator",
                        role.name
                    ));
                }
            }
        }
    }

    fn validate_categories(&self, problems: &mut Vec<String>) {
        let mut category_names = BTreeSet::new();
        let mut channel_keys: BTreeMap<(String, ChannelKind), &str> = BTreeMap::new();
        let mut somewhere_to_land = false;
        for category in &self.categories {
            let cname = category.name.trim();
            if cname.is_empty() || cname.chars().count() > MAX_NAME_CHARS {
                problems.push(format!(
                    "category {:?}: name must be 1..={MAX_NAME_CHARS} characters",
                    category.name
                ));
            }
            if !category_names.insert(cname.to_ascii_lowercase()) {
                problems.push(format!("category {:?} is declared twice", category.name));
            }
            if category.channels.len() > MAX_CHANNELS_PER_CATEGORY {
                problems.push(format!(
                    "category {:?} has {} channels; Discord allows {MAX_CHANNELS_PER_CATEGORY}",
                    category.name,
                    category.channels.len()
                ));
            }
            self.validate_overwrites(
                &format!("category {:?}", category.name),
                &category.overwrites,
                problems,
            );
            let category_hidden = denies_everyone_view(&category.overwrites);
            if category_hidden && !has_role_view_allow(&category.overwrites) {
                problems.push(format!(
                    "category {:?} hides @everyone but allows no role View Channel",
                    category.name
                ));
            }
            for channel in &category.channels {
                let label = format!("channel {:?} in {:?}", channel.name, category.name);
                let name = channel.name.trim();
                if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
                    problems.push(format!(
                        "{label}: name must be 1..={MAX_NAME_CHARS} characters"
                    ));
                }
                if channel.kind.normalizes_name()
                    && normalize_text_name(&channel.name) != channel.name
                {
                    problems.push(format!(
                        "{label}: Discord would store this {} channel as {:?}",
                        channel.kind.label(),
                        normalize_text_name(&channel.name)
                    ));
                }
                let key = (
                    if channel.kind.normalizes_name() {
                        normalize_text_name(&channel.name)
                    } else {
                        name.to_string()
                    },
                    channel.kind,
                );
                if let Some(previous) = channel_keys.insert(key, category.name.as_str()) {
                    problems.push(format!(
                        "{label}: a {} channel with this name already exists in {previous:?}",
                        channel.kind.label()
                    ));
                }
                if channel.topic.is_some() && !channel.kind.has_topic() {
                    problems.push(format!(
                        "{label}: {} channels carry no topic",
                        channel.kind.label()
                    ));
                }
                if let Some(topic) = &channel.topic
                    && topic.chars().count() > 1024
                {
                    problems.push(format!("{label}: topic exceeds 1024 characters"));
                }
                if !channel.tags.is_empty() && channel.kind != ChannelKind::Forum {
                    problems.push(format!("{label}: only forum channels carry tags"));
                }
                if channel.tags.len() > 20 {
                    problems.push(format!("{label}: Discord allows at most 20 forum tags"));
                }
                if let Some(secs) = channel.slowmode_secs {
                    if !channel.kind.has_topic() {
                        problems.push(format!(
                            "{label}: slowmode applies to text-like channels only"
                        ));
                    }
                    if secs > MAX_SLOWMODE_SECS {
                        problems.push(format!(
                            "{label}: slowmode exceeds {MAX_SLOWMODE_SECS} seconds"
                        ));
                    }
                }
                self.validate_overwrites(&label, &channel.overwrites, problems);
                let effective = self.effective_overwrites(category, channel);
                let hidden = denies_everyone_view(effective);
                if hidden && !has_role_view_allow(effective) {
                    problems.push(format!(
                        "{label}: hides @everyone but allows no role View Channel"
                    ));
                }
                if !hidden {
                    somewhere_to_land = true;
                }
            }
        }
        if !somewhere_to_land && self.categories.iter().any(|c| !c.channels.is_empty()) {
            problems.push(
                "every channel is hidden from @everyone: a new joiner would see an empty server"
                    .into(),
            );
        }
    }

    fn validate_overwrites(
        &self,
        label: &str,
        overwrites: &[Overwrite],
        problems: &mut Vec<String>,
    ) {
        let mut targets = BTreeSet::new();
        for overwrite in overwrites {
            if overwrite.role != EVERYONE && self.role(&overwrite.role).is_none() {
                problems.push(format!(
                    "{label}: overwrite targets role {:?}, which this plan never creates",
                    overwrite.role
                ));
            }
            if !targets.insert(overwrite.role.as_str()) {
                problems.push(format!(
                    "{label}: role {:?} has two overwrites",
                    overwrite.role
                ));
            }
            if overwrite.is_empty() {
                problems.push(format!(
                    "{label}: overwrite for {:?} allows and denies nothing",
                    overwrite.role
                ));
            }
            for both in overwrite
                .allow
                .iter()
                .filter(|p| overwrite.deny.contains(p))
            {
                problems.push(format!(
                    "{label}: {both:?} is both allowed and denied for {:?}",
                    overwrite.role
                ));
            }
            if overwrite.role == EVERYONE {
                for dangerous in overwrite
                    .allow
                    .iter()
                    .filter(|p| NEVER_FOR_EVERYONE.contains(&p.as_str()))
                {
                    problems.push(format!(
                        "{label}: @everyone must never be allowed {dangerous:?}"
                    ));
                }
            }
        }
    }

    fn validate_everyone(&self, problems: &mut Vec<String>) {
        for dangerous in self
            .everyone
            .permissions
            .iter()
            .filter(|p| NEVER_FOR_EVERYONE.contains(&p.as_str()))
        {
            problems.push(format!("@everyone must never hold {dangerous:?}"));
        }
    }
}

fn has_role_view_allow(overwrites: &[Overwrite]) -> bool {
    overwrites
        .iter()
        .any(|o| o.role != EVERYONE && o.allow.iter().any(|p| p == VIEW_CHANNEL))
}

impl From<&Blueprint> for Plan {
    /// The archetype tables, in owned form. `gated_to` becomes the pair of
    /// overwrites the `/server` steps describe; `deny_everyone` becomes an
    /// `@everyone` deny list. Roles start unhoisted and uncoloured, as the
    /// tables never said otherwise.
    fn from(bp: &Blueprint) -> Self {
        let roles = bp
            .roles
            .iter()
            .map(|r| PlanRole {
                name: r.name.into(),
                hoist: false,
                mentionable: false,
                colour: None,
                permissions: r.permissions.iter().map(|p| (*p).into()).collect(),
                note: r.note.into(),
            })
            .collect();
        let categories = bp
            .categories
            .iter()
            .map(|c| PlanCategory {
                name: c.name.into(),
                overwrites: Vec::new(),
                channels: c
                    .channels
                    .iter()
                    .map(|ch| {
                        let mut overwrites = Vec::new();
                        let mut everyone_deny: Vec<String> =
                            ch.deny_everyone.iter().map(|p| (*p).into()).collect();
                        if let Some(role) = ch.gated_to {
                            everyone_deny.push(VIEW_CHANNEL.into());
                            overwrites.push(Overwrite {
                                role: role.into(),
                                allow: vec![VIEW_CHANNEL.into()],
                                deny: Vec::new(),
                            });
                        }
                        if !everyone_deny.is_empty() {
                            overwrites.insert(
                                0,
                                Overwrite {
                                    role: EVERYONE.into(),
                                    allow: Vec::new(),
                                    deny: everyone_deny,
                                },
                            );
                        }
                        PlanChannel {
                            name: ch.name.into(),
                            kind: ch.kind,
                            topic: None,
                            slowmode_secs: None,
                            tags: Vec::new(),
                            overwrites,
                        }
                    })
                    .collect(),
            })
            .collect();
        Self {
            name: "archetype".into(),
            source: None,
            everyone: Everyone {
                permissions: bp.everyone.iter().map(|p| (*p).into()).collect(),
            },
            roles,
            categories,
        }
    }
}

/// The shipped MLAI plan, pinned by the tests below. `include_str!` rather than
/// a runtime path so the check holds on every platform the gate runs on.
#[cfg(test)]
pub const MLAI_COMMUNITY: &str = include_str!("../../blueprints/mlai-community.toml");

#[cfg(test)]
mod tests;
