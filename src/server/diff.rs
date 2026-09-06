//! The pure half of the plan engine: plan + snapshot + stage → changes.
//!
//! `diff` never touches Discord. It produces a [`Report`] whose `changes` are
//! exactly what `apply` would perform, plus the reasons it refuses
//! (`blockers`), the things it wants a human to know first (`warnings`), and
//! the work it leaves to a human on purpose (`manual`).
//!
//! The vocabulary is deliberately additive. [`Change`] has no variant that
//! deletes a role, a channel, or a category, and none that edits an existing
//! role's permissions (`@everyone`'s included): the proposal's stages 12 and
//! 13 are irreversible and stay human, and a role permission edit is a
//! hierarchy decision. A test enumerates the variants so a `Delete` cannot
//! slip in quietly.
//!
//! Stages mirror the proposal's rollout order (section 4):
//! - [`Stage::Additive`] creates what is missing and nothing else: roles with
//!   zero permissions, categories and channels hidden from `@everyone`
//!   ([`Overwrite::hide_marker`]). Nothing existing is touched.
//! - [`Stage::Reveal`] sets role cosmetics, moves plan channels into their
//!   categories, sets topics, and applies planned overwrites, with one
//!   guard: it changes what `@everyone` can *see* only from the engine's own
//!   hide marker to visible. Any other view-gate change on `@everyone` is
//!   deferred, because that is the 3.B.9 lockout.
//! - [`Stage::Overwrites`] applies every planned overwrite for one named
//!   category ("one category per day", stage 8), warning about each
//!   view-gate flip on a channel people can currently see.

use std::collections::BTreeSet;

use super::ChannelKind;
use super::plan::{Overwrite, colour_hex};

mod context;
pub use context::diff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Additive,
    Reveal,
    Overwrites,
}

impl Stage {
    /// Every stage, for the tests that must not silently skip one.
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::Additive, Self::Reveal, Self::Overwrites];

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "additive" => Some(Self::Additive),
            "reveal" => Some(Self::Reveal),
            "overwrites" => Some(Self::Overwrites),
            _ => None,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Additive => "additive",
            Self::Reveal => "reveal",
            Self::Overwrites => "overwrites",
        }
    }
}

/// What to diff: the stage, and for [`Stage::Overwrites`] the one category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub stage: Stage,
    pub category: Option<String>,
}

/// A channel-like thing an overwrite lands on, by plan name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Category(String),
    Channel { name: String, kind: ChannelKind },
}

impl Target {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Category(name) => format!("category {name:?}"),
            Self::Channel { name, kind } => describe_channel(*kind, name),
        }
    }
}

fn describe_channel(kind: ChannelKind, name: &str) -> String {
    match kind {
        ChannelKind::Text | ChannelKind::Forum | ChannelKind::Announcement => {
            format!("{} #{name}", kind.label())
        }
        ChannelKind::Voice | ChannelKind::Stage => format!("{} {name:?}", kind.label()),
    }
}

/// One thing `apply` may do. Additive by construction: see the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Zero permissions, unhoisted, not mentionable (proposal stage 1).
    CreateRole { name: String, colour: Option<u32> },
    /// Cosmetics only; permissions are never edited.
    EditRole {
        name: String,
        hoist: bool,
        mentionable: bool,
        colour: Option<u32>,
    },
    CreateCategory {
        name: String,
        overwrites: Vec<Overwrite>,
    },
    CreateChannel {
        name: String,
        kind: ChannelKind,
        category: String,
        topic: Option<String>,
        slowmode_secs: Option<u16>,
        tags: Vec<String>,
        overwrites: Vec<Overwrite>,
    },
    /// Parent category and topic; never the name, never the kind.
    EditChannel {
        name: String,
        kind: ChannelKind,
        category: String,
        topic: TopicEdit,
    },
    /// One `PUT` for one role on one channel. Other roles' and members'
    /// entries on that channel are untouched (this is what keeps 3.B.6 out).
    SetOverwrite {
        target: Target,
        overwrite: Overwrite,
    },
}

/// A channel edit must distinguish an omitted field from clearing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicEdit {
    Unchanged,
    Set(String),
}

impl Change {
    /// Every variant, spelled out. The exhaustiveness test walks this list.
    #[cfg(test)]
    pub const KIND_NAMES: [&'static str; 6] = [
        "CreateRole",
        "EditRole",
        "CreateCategory",
        "CreateChannel",
        "EditChannel",
        "SetOverwrite",
    ];

    #[cfg(test)]
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::CreateRole { .. } => "CreateRole",
            Self::EditRole { .. } => "EditRole",
            Self::CreateCategory { .. } => "CreateCategory",
            Self::CreateChannel { .. } => "CreateChannel",
            Self::EditChannel { .. } => "EditChannel",
            Self::SetOverwrite { .. } => "SetOverwrite",
        }
    }

    /// Editing overwrites and roles needs Manage Roles on top of Manage Channels.
    #[must_use]
    pub const fn needs_manage_roles(&self) -> bool {
        matches!(
            self,
            Self::CreateRole { .. }
                | Self::EditRole { .. }
                | Self::SetOverwrite { .. }
                | Self::CreateCategory { .. }
                | Self::CreateChannel { .. }
        )
    }

    /// Permission names this change would grant or deny through overwrites.
    #[must_use]
    pub fn overwrite_permission_names(&self) -> BTreeSet<&str> {
        let overwrites: &[Overwrite] = match self {
            Self::CreateCategory { overwrites, .. } | Self::CreateChannel { overwrites, .. } => {
                overwrites
            }
            Self::SetOverwrite { overwrite, .. } => std::slice::from_ref(overwrite),
            Self::CreateRole { .. } | Self::EditRole { .. } | Self::EditChannel { .. } => &[],
        };
        overwrites
            .iter()
            .flat_map(|o| o.allow.iter().chain(o.deny.iter()))
            .map(String::as_str)
            .collect()
    }

    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::CreateRole { name, colour } => format!(
                "create role {name:?} ({}; zero permissions, unhoisted, not mentionable)",
                colour.map_or_else(
                    || "no colour".to_string(),
                    |c| format!("colour {}", colour_hex(c))
                )
            ),
            Self::EditRole {
                name,
                hoist,
                mentionable,
                colour,
            } => format!(
                "edit role {name:?}: hoist {}, mentionable {}, {}",
                on_off(*hoist),
                on_off(*mentionable),
                colour.map_or_else(|| "no colour".to_string(), colour_hex)
            ),
            Self::CreateCategory { name, overwrites } => {
                format!(
                    "create category {name:?} ({})",
                    describe_overwrites(overwrites)
                )
            }
            Self::CreateChannel {
                name,
                kind,
                category,
                topic,
                slowmode_secs,
                tags,
                overwrites,
            } => {
                let mut extras = vec![describe_overwrites(overwrites)];
                if topic.is_some() {
                    extras.push("topic set".into());
                }
                if let Some(secs) = slowmode_secs {
                    extras.push(format!("slowmode {secs}s"));
                }
                if !tags.is_empty() {
                    extras.push(format!("{} tags", tags.len()));
                }
                format!(
                    "create {} in {category:?} ({})",
                    describe_channel(*kind, name),
                    extras.join("; ")
                )
            }
            Self::EditChannel {
                name,
                kind,
                category,
                topic,
            } => format!(
                "edit {}: parent {category:?}{}",
                describe_channel(*kind, name),
                match topic {
                    TopicEdit::Unchanged => "",
                    TopicEdit::Set(_) => ", topic set",
                }
            ),
            Self::SetOverwrite { target, overwrite } => {
                if overwrite.is_empty() {
                    format!(
                        "clear the {} overwrite on {}",
                        overwrite.role,
                        target.describe()
                    )
                } else {
                    format!(
                        "set the {} overwrite on {}: allow [{}], deny [{}]",
                        overwrite.role,
                        target.describe(),
                        overwrite.allow.join(", "),
                        overwrite.deny.join(", ")
                    )
                }
            }
        }
    }
}

const fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn describe_overwrites(overwrites: &[Overwrite]) -> String {
    if overwrites.len() == 1 && overwrites[0].is_hide_marker() {
        "hidden from @everyone until --stage reveal".into()
    } else if overwrites.is_empty() {
        "no overwrites".into()
    } else {
        format!("{} overwrites", overwrites.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub stage: Stage,
    /// Facts the decision rested on, for the operator's eyes.
    pub preflight: Vec<String>,
    pub changes: Vec<Change>,
    /// Any of these means `apply` is refused.
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    /// Work the engine leaves to a human on purpose.
    pub manual: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Operator-facing rendering. `heading` names the plan, guild, and mode.
    #[must_use]
    pub fn render(&self, heading: &str) -> String {
        let mut out = String::new();
        out.push_str(heading);
        out.push('\n');
        for line in &self.preflight {
            out.push_str(&format!("preflight: {line}\n"));
        }
        out.push_str(&format!("\nchanges ({})\n", self.changes.len()));
        for (index, change) in self.changes.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", index + 1, change.describe()));
        }
        if !self.blockers.is_empty() {
            out.push_str(&format!(
                "\nblockers ({}): nothing will be applied\n",
                self.blockers.len()
            ));
            for line in &self.blockers {
                out.push_str(&format!("  - {line}\n"));
            }
        }
        if !self.warnings.is_empty() {
            out.push_str(&format!("\nwarnings ({})\n", self.warnings.len()));
            for line in &self.warnings {
                out.push_str(&format!("  - {line}\n"));
            }
        }
        if !self.manual.is_empty() {
            out.push_str(&format!(
                "\nmanual steps ({}): the engine never does these\n",
                self.manual.len()
            ));
            for (index, line) in self.manual.iter().enumerate() {
                out.push_str(&format!("  {}. {line}\n", index + 1));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
