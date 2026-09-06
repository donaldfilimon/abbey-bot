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
mod tests {
    use super::*;
    use crate::server::{Archetype, blueprint};

    fn mlai() -> Plan {
        Plan::from_toml(MLAI_COMMUNITY).expect("shipped plan validates")
    }

    fn minimal(extra: &str) -> String {
        format!(
            r#"
name = "t"
[[roles]]
name = "Member"
[[categories]]
name = "Main"
[[categories.channels]]
name = "general"
kind = "text"
{extra}
"#
        )
    }

    #[test]
    fn every_archetype_converts_and_validates() {
        for archetype in Archetype::ALL {
            let plan = Plan::from(&blueprint(archetype));
            plan.validate()
                .unwrap_or_else(|problems| panic!("{archetype:?}: {problems:?}"));
        }
    }

    #[test]
    fn archetype_gating_becomes_the_overwrite_pair_the_steps_describe() {
        let plan = Plan::from(&blueprint(Archetype::Community));
        let (category, channel) = plan
            .channels()
            .find(|(_, ch)| ch.name == "mod-log")
            .expect("community has mod-log");
        let overwrites = plan.effective_overwrites(category, channel);
        assert!(denies_everyone_view(overwrites));
        assert!(
            overwrites
                .iter()
                .any(|o| o.role == "Moderator" && o.allow == [VIEW_CHANNEL])
        );
        let (category, rules) = plan.channels().find(|(_, ch)| ch.name == "rules").unwrap();
        let rules = plan.effective_overwrites(category, rules);
        assert!(!denies_everyone_view(rules), "newcomers must see rules");
        assert!(
            rules
                .iter()
                .any(|o| o.role == EVERYONE && o.deny.iter().any(|p| p == "Send Messages"))
        );
    }

    #[test]
    fn the_shipped_mlai_plan_parses_and_validates() {
        let plan = mlai();
        assert_eq!(plan.name, "MLAI");
        assert_eq!(
            plan.categories.len(),
            8,
            "START HERE, COMMONS, THE STACK, BUILD LOG, PRODUCTS, VOICE, STAFF, ARCHIVE"
        );
        assert_eq!(plan.roles.len(), 12);
        assert!(
            plan.needs_community(),
            "forums, announcements, and a stage need COMMUNITY"
        );
    }

    #[test]
    fn mlai_hides_exactly_the_channels_the_proposal_hides() {
        let plan = mlai();
        let hidden: Vec<&str> = plan
            .channels()
            .filter(|(c, ch)| denies_everyone_view(plan.effective_overwrites(c, ch)))
            .map(|(_, ch)| ch.name.as_str())
            .collect();
        assert_eq!(
            hidden,
            [
                "ci-and-deploys",
                "ops-console",
                "mod-chat",
                "mod-log",
                "staging",
                "server-updates"
            ]
        );
        let staff = plan.category("STAFF").unwrap();
        assert!(
            staff.channels.iter().all(|ch| ch.overwrites.is_empty()),
            "STAFF children inherit (sync left on)"
        );
        assert!(denies_everyone_view(&staff.overwrites));
    }

    #[test]
    fn mlai_read_only_channels_block_every_posting_path() {
        let plan = mlai();
        for name in ["welcome", "rules", "announcements", "releases"] {
            let (category, channel) = plan.channels().find(|(_, ch)| ch.name == name).unwrap();
            let everyone = plan
                .effective_overwrites(category, channel)
                .iter()
                .find(|o| o.role == EVERYONE)
                .unwrap_or_else(|| panic!("{name} has no @everyone overwrite"));
            for permission in [
                "Send Messages",
                "Create Public Threads",
                "Create Private Threads",
                "Send Messages in Threads",
            ] {
                assert!(
                    everyone.deny.iter().any(|p| p == permission),
                    "{name} leaves {permission} open"
                );
            }
            assert!(!everyone.denies_view(), "{name} must stay visible");
        }
    }

    #[test]
    fn mlai_console_and_stage_overwrites_match_the_proposal() {
        let plan = mlai();
        let (category, console) = plan
            .channels()
            .find(|(_, ch)| ch.name == "ops-console")
            .unwrap();
        let overwrites = plan.effective_overwrites(category, console);
        assert!(
            overwrites
                .iter()
                .any(|o| o.role == "Console" && o.allow.contains(&"Send Messages".to_string()))
        );
        assert!(
            overwrites
                .iter()
                .any(|o| o.role == "Team" && o.allow == [VIEW_CHANNEL])
        );

        let (category, stage) = plan
            .channels()
            .find(|(_, ch)| ch.name == "Office Hours")
            .unwrap();
        assert_eq!(stage.kind, ChannelKind::Stage);
        let overwrites = plan.effective_overwrites(category, stage);
        let everyone = overwrites.iter().find(|o| o.role == EVERYONE).unwrap();
        assert!(everyone.allow.contains(&"Request to Speak".to_string()));
        assert_eq!(everyone.deny, ["Speak"]);

        let (_, help) = plan.channels().find(|(_, ch)| ch.name == "help").unwrap();
        assert_eq!(help.kind, ChannelKind::Forum);
        assert_eq!(help.tags.len(), 7);
    }

    #[test]
    fn mlai_interest_roles_carry_no_permissions_and_are_not_hoisted() {
        let plan = mlai();
        for name in [
            "WDBX",
            "ABI",
            "Personas",
            "Apple Silicon",
            "Site Builder",
            "Announcements",
        ] {
            let role = plan.role(name).unwrap_or_else(|| panic!("missing {name}"));
            assert!(role.permissions.is_empty(), "{name}");
            assert!(!role.hoist && !role.mentionable, "{name}");
        }
        assert_eq!(plan.role("Team").unwrap().colour, Some(0x22d3ee));
        assert!(plan.role("Team").unwrap().hoist);
    }

    #[test]
    fn mlai_text_names_are_in_discords_final_form() {
        // The validator already checks this; the test states it as a property
        // the shipped file must keep even if the validator is loosened.
        for (_, channel) in mlai().channels() {
            if channel.kind.normalizes_name() {
                assert_eq!(normalize_text_name(&channel.name), channel.name);
            }
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = minimal("colour = \"#ffffff\"");
        let error = Plan::from_toml(&text).unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
        assert!(Plan::from_toml("name = \"x\"\n[[roles]]\nname = \"a\"\nperms = []\n").is_err());
    }

    #[test]
    fn overwrites_must_target_a_declared_role() {
        let text = minimal(r#"overwrites = [{ role = "Ghost", allow = ["View Channel"] }]"#);
        let error = Plan::from_toml(&text).unwrap_err();
        assert!(error.contains("never creates"), "{error}");
    }

    #[test]
    fn a_text_channel_name_discord_would_rewrite_is_rejected() {
        let text = minimal("").replace("name = \"general\"", "name = \"General Chat\"");
        let error = Plan::from_toml(&text).unwrap_err();
        assert!(error.contains("general-chat"), "{error}");
        // Voice names are exempt.
        let voice = minimal("")
            .replace("kind = \"text\"", "kind = \"voice\"")
            .replace("name = \"general\"", "name = \"Squad 1\"");
        Plan::from_toml(&voice).expect("voice names are free-form");
    }

    #[test]
    fn a_plan_that_hides_everything_is_rejected() {
        let text = minimal(
            r#"overwrites = [{ role = "@everyone", deny = ["View Channel"] }, { role = "Member", allow = ["View Channel"] }]"#,
        );
        let error = Plan::from_toml(&text).unwrap_err();
        assert!(error.contains("empty server"), "{error}");
    }

    #[test]
    fn a_hidden_channel_must_let_some_role_in() {
        let text = format!(
            "{}\n[[categories.channels]]\nname = \"secret\"\nkind = \"text\"\noverwrites = [{{ role = \"@everyone\", deny = [\"View Channel\"] }}]\n",
            minimal("")
        );
        let error = Plan::from_toml(&text).unwrap_err();
        assert!(error.contains("allows no role View Channel"), "{error}");
    }

    #[test]
    fn everyone_is_never_allowed_a_dangerous_permission() {
        let text = minimal(r#"overwrites = [{ role = "@everyone", allow = ["Administrator"] }]"#);
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("never be allowed")
        );
        let text = minimal("").replace(
            "name = \"t\"",
            "name = \"t\"\n[everyone]\npermissions = [\"Manage Guild\"]",
        );
        assert!(Plan::from_toml(&text).unwrap_err().contains("never hold"));
    }

    #[test]
    fn contradictory_duplicate_and_empty_overwrites_are_rejected() {
        let text =
            minimal(r#"overwrites = [{ role = "Member", allow = ["Speak"], deny = ["Speak"] }]"#);
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("both allowed and denied")
        );
        let text = minimal(
            r#"overwrites = [{ role = "Member", allow = ["Speak"] }, { role = "Member", deny = ["Connect"] }]"#,
        );
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("two overwrites")
        );
        let text = minimal(r#"overwrites = [{ role = "Member" }]"#);
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("allows and denies nothing")
        );
    }

    #[test]
    fn tags_slowmode_and_topics_are_kind_checked() {
        assert!(
            Plan::from_toml(&minimal("tags = [\"a\"]"))
                .unwrap_err()
                .contains("only forum")
        );
        assert!(
            Plan::from_toml(&minimal("slowmode_secs = 30000"))
                .unwrap_err()
                .contains("exceeds")
        );
        let voice_topic = minimal("topic = \"x\"")
            .replace("kind = \"text\"", "kind = \"voice\"")
            .replace("name = \"general\"", "name = \"Lounge\"");
        assert!(
            Plan::from_toml(&voice_topic)
                .unwrap_err()
                .contains("carry no topic")
        );
    }

    #[test]
    fn duplicate_roles_categories_and_channels_are_rejected() {
        let text = format!("{}\n[[roles]]\nname = \"Member\"\n", minimal(""));
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("declared twice")
        );
        let text = format!("{}\n[[categories]]\nname = \"main\"\n", minimal(""));
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("declared twice")
        );
        let text = format!(
            "{}\n[[categories.channels]]\nname = \"general\"\nkind = \"text\"\n",
            minimal("")
        );
        assert!(
            Plan::from_toml(&text)
                .unwrap_err()
                .contains("already exists")
        );
    }

    #[test]
    fn colours_parse_and_render_round_trip() {
        assert_eq!(parse_colour("#22D3EE").unwrap(), Some(0x22d3ee));
        assert_eq!(parse_colour("").unwrap(), None);
        assert!(parse_colour("22d3ee").is_err());
        assert!(parse_colour("#22d3e").is_err());
        assert!(parse_colour("#zzzzzz").is_err());
        assert_eq!(colour_hex(0x22d3ee), "#22d3ee");
        assert_eq!(colour_hex(0x000005), "#000005");
        let bad = minimal("").replace("name = \"Member\"", "name = \"Member\"\ncolour = \"red\"");
        assert!(Plan::from_toml(&bad).unwrap_err().contains("#rrggbb"));
    }

    #[test]
    fn hide_marker_is_recognised_by_content_not_order() {
        let marker = Overwrite {
            role: EVERYONE.into(),
            allow: vec![],
            deny: vec![VIEW_CHANNEL.into(), VIEW_CHANNEL.into()],
        };
        assert!(marker.is_hide_marker());
        let mut wider = marker.clone();
        wider.deny.push("Send Messages".into());
        assert!(!wider.is_hide_marker());
        assert!(wider.denies_view());
        let role_marker = Overwrite {
            role: "Member".into(),
            ..marker
        };
        assert!(!role_marker.is_hide_marker());
    }

    #[test]
    fn permission_names_collect_every_mention() {
        let plan = mlai();
        let names = plan.permission_names();
        for expected in [
            "Administrator",
            "Request to Speak",
            "Manage Threads",
            "Create Invites",
        ] {
            assert!(
                names.contains(expected),
                "{expected} missing from {names:?}"
            );
        }
    }
}
