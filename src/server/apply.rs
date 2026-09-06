//! Perform a diff's changes through a [`GuildWriter`], in order, stopping at
//! the first failure. No rollback: every change is additive, so what landed
//! is reported and the operator re-runs the dry run to see what remains.
//!
//! `apply` resolves plan names to ids (categories and roles may have been
//! created moments earlier in the same run) and hands the writer fully
//! resolved [`Op`]s. The writer is the only thing that talks to Discord; the
//! tests drive the same code through a fake guild that mutates a snapshot.

use std::collections::BTreeMap;

use super::ChannelKind;
use super::diff::{Change, Target, TopicEdit};
use super::observe::{ChannelClass, GuildSnapshot, plan_channel_key};
use super::plan::{EVERYONE, Overwrite};

/// An overwrite with its role resolved to an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOverwrite {
    pub role_id: u64,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// A change with every name resolved to a Discord id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    CreateRole {
        name: String,
        colour: Option<u32>,
    },
    EditRole {
        id: u64,
        hoist: bool,
        mentionable: bool,
        colour: Option<u32>,
    },
    CreateChannel {
        name: String,
        class: ChannelClass,
        parent: Option<u64>,
        topic: Option<String>,
        slowmode_secs: Option<u16>,
        tags: Vec<String>,
        overwrites: Vec<ResolvedOverwrite>,
    },
    EditChannel {
        id: u64,
        parent: Option<u64>,
        topic: TopicEdit,
    },
    SetOverwrite {
        channel_id: u64,
        role_id: u64,
        allow: Vec<String>,
        deny: Vec<String>,
    },
}

/// The one seam to Discord. Returns the created id for creations.
pub trait GuildWriter {
    fn perform(
        &mut self,
        op: &Op,
    ) -> impl std::future::Future<Output = Result<Option<u64>, String>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub applied: Vec<Change>,
    /// The change that failed and why; everything after it was not attempted.
    pub failed: Option<(Change, String)>,
    pub remaining: Vec<Change>,
}

impl Outcome {
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for change in &self.applied {
            out.push_str(&format!("  applied: {}\n", change.describe()));
        }
        if let Some((change, reason)) = &self.failed {
            out.push_str(&format!("  FAILED: {} ({reason})\n", change.describe()));
            out.push_str(&format!(
                "  not attempted: {} change(s); re-run the dry run to see what remains\n",
                self.remaining.len()
            ));
        }
        out
    }
}

/// Apply `changes` in order. Stops at the first failure.
pub async fn apply<W: GuildWriter>(
    changes: &[Change],
    snapshot: &GuildSnapshot,
    writer: &mut W,
) -> Outcome {
    let mut index = NameIndex::from_snapshot(snapshot);
    let mut outcome = Outcome {
        applied: Vec::new(),
        failed: None,
        remaining: Vec::new(),
    };
    for (position, change) in changes.iter().enumerate() {
        let op = match index.resolve(change) {
            Ok(op) => op,
            Err(reason) => {
                outcome.failed = Some((change.clone(), reason));
                outcome.remaining = changes[position + 1..].to_vec();
                return outcome;
            }
        };
        match writer.perform(&op).await {
            Ok(created) => {
                index.record(change, created);
                outcome.applied.push(change.clone());
            }
            Err(reason) => {
                outcome.failed = Some((change.clone(), reason));
                outcome.remaining = changes[position + 1..].to_vec();
                return outcome;
            }
        }
    }
    outcome
}

/// Plan names → ids, seeded from the snapshot and extended as things land.
/// Only unambiguous snapshot entries are indexed; `diff` already refused to
/// produce changes for ambiguous ones.
struct NameIndex {
    guild_id: u64,
    roles: BTreeMap<String, u64>,
    categories: BTreeMap<String, u64>,
    channels: BTreeMap<(String, ChannelKind), u64>,
}

impl NameIndex {
    fn from_snapshot(snapshot: &GuildSnapshot) -> Self {
        let mut roles: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for role in &snapshot.roles {
            roles.entry(role.name.clone()).or_default().push(role.id);
        }
        let mut categories: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        let mut channels: BTreeMap<(String, ChannelKind), Vec<u64>> = BTreeMap::new();
        for channel in &snapshot.channels {
            match channel.class {
                ChannelClass::Category => categories
                    .entry(channel.name.trim().to_ascii_lowercase())
                    .or_default()
                    .push(channel.id),
                ChannelClass::Kind(kind) => channels
                    .entry((plan_channel_key(&channel.name, kind), kind))
                    .or_default()
                    .push(channel.id),
                ChannelClass::Other => {}
            }
        }
        let unique = |map: BTreeMap<String, Vec<u64>>| -> BTreeMap<String, u64> {
            map.into_iter()
                .filter_map(|(k, ids)| (ids.len() == 1).then(|| (k, ids[0])))
                .collect()
        };
        Self {
            guild_id: snapshot.guild_id,
            roles: unique(roles),
            categories: unique(categories),
            channels: channels
                .into_iter()
                .filter_map(|(k, ids)| (ids.len() == 1).then(|| (k, ids[0])))
                .collect(),
        }
    }

    fn role_id(&self, name: &str) -> Result<u64, String> {
        if name == EVERYONE {
            return Ok(self.guild_id);
        }
        self.roles
            .get(name)
            .copied()
            .ok_or_else(|| format!("role {name:?} has no id yet"))
    }

    fn category_id(&self, name: &str) -> Result<u64, String> {
        self.categories
            .get(&name.trim().to_ascii_lowercase())
            .copied()
            .ok_or_else(|| format!("category {name:?} has no id yet"))
    }

    fn channel_id(&self, name: &str, kind: ChannelKind) -> Result<u64, String> {
        self.channels
            .get(&(plan_channel_key(name, kind), kind))
            .copied()
            .ok_or_else(|| format!("{} #{name} has no id yet", kind.label()))
    }

    fn resolve_overwrites(
        &self,
        overwrites: &[Overwrite],
    ) -> Result<Vec<ResolvedOverwrite>, String> {
        overwrites
            .iter()
            .map(|o| {
                Ok(ResolvedOverwrite {
                    role_id: self.role_id(&o.role)?,
                    allow: o.allow.clone(),
                    deny: o.deny.clone(),
                })
            })
            .collect()
    }

    fn resolve(&self, change: &Change) -> Result<Op, String> {
        Ok(match change {
            Change::CreateRole { name, colour } => Op::CreateRole {
                name: name.clone(),
                colour: *colour,
            },
            Change::EditRole {
                name,
                hoist,
                mentionable,
                colour,
            } => Op::EditRole {
                id: self.role_id(name)?,
                hoist: *hoist,
                mentionable: *mentionable,
                colour: *colour,
            },
            Change::CreateCategory { name, overwrites } => Op::CreateChannel {
                name: name.clone(),
                class: ChannelClass::Category,
                parent: None,
                topic: None,
                slowmode_secs: None,
                tags: Vec::new(),
                overwrites: self.resolve_overwrites(overwrites)?,
            },
            Change::CreateChannel {
                name,
                kind,
                category,
                topic,
                slowmode_secs,
                tags,
                overwrites,
            } => Op::CreateChannel {
                name: name.clone(),
                class: ChannelClass::Kind(*kind),
                parent: Some(self.category_id(category)?),
                topic: topic.clone(),
                slowmode_secs: *slowmode_secs,
                tags: tags.clone(),
                overwrites: self.resolve_overwrites(overwrites)?,
            },
            Change::EditChannel {
                name,
                kind,
                category,
                topic,
            } => Op::EditChannel {
                id: self.channel_id(name, *kind)?,
                parent: Some(self.category_id(category)?),
                topic: topic.clone(),
            },
            Change::SetOverwrite { target, overwrite } => Op::SetOverwrite {
                channel_id: match target {
                    Target::Category(name) => self.category_id(name)?,
                    Target::Channel { name, kind } => self.channel_id(name, *kind)?,
                },
                role_id: self.role_id(&overwrite.role)?,
                allow: overwrite.allow.clone(),
                deny: overwrite.deny.clone(),
            },
        })
    }

    fn record(&mut self, change: &Change, created: Option<u64>) {
        let Some(id) = created else { return };
        match change {
            Change::CreateRole { name, .. } => {
                self.roles.insert(name.clone(), id);
            }
            Change::CreateCategory { name, .. } => {
                self.categories.insert(name.trim().to_ascii_lowercase(), id);
            }
            Change::CreateChannel { name, kind, .. } => {
                self.channels
                    .insert((plan_channel_key(name, *kind), *kind), id);
            }
            Change::EditRole { .. } | Change::EditChannel { .. } | Change::SetOverwrite { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests;
