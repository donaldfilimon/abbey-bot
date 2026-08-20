//! Durable state: one JSON document plus one WDBX segment under `ABBEY_DATA_DIR`.
//!
//! The spec (`docs/spec/botarchitecture.md`) persists through Fluent +
//! PostgreSQL. This crate deliberately has no database: the host it deploys to
//! is a single process with a data directory, and every store the pure modules
//! need — guild config, per-guild brain snapshots, reputation, memory — is
//! small enough to hold in memory and write whole. `Stores` is that memory,
//! implementing the three store traits the registries speak, and
//! [`Stores::save`] / [`Stores::load`] are the only I/O. Semantic memory lives
//! beside it in the WDBX segment ([`crate::wdbx::Recall`]), which has its own
//! format and its own save path.
//!
//! Writes are atomic (temp file + rename) so a crash mid-persist leaves the
//! previous document intact rather than a truncated one.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::brain::registry::BrainStore;
use crate::brain::social::{ReputationEvent, ReputationStore};
use crate::guild::{GuildConfigStore, GuildSettings};
use crate::memory::MemoryBank;

/// File name of the JSON document inside the data directory.
pub const STATE_FILE: &str = "abbey-state.json";
/// File name of the WDBX segment inside the data directory. The name follows
/// abi's own `wdbx.seg.N.jsonl` convention so the two tools' files are
/// recognisable side by side.
pub const WDBX_FILE: &str = "wdbx.seg.0.jsonl";
/// Audit rows kept in memory before the oldest are dropped. The trail is an
/// explanation aid, not a ledger of record; bounding it keeps the document
/// from growing without limit on a busy guild.
pub const MAX_EVENTS: usize = 10_000;

/// One persisted brain row (`brain_states` in the spec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainRow {
    pub snapshot_json: String,
    pub experience_count: u64,
}

/// One persisted reputation row (the `UserMemory.reputation` column in the spec).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReputationRow {
    pub value: f64,
    pub interaction_count: u32,
}

/// Everything the registries store, in one serialisable document.
///
/// Every field is `#[serde(default)]` so a document written by an older build
/// still loads — a missing section means "empty", never "refuse to start".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Stores {
    #[serde(default)]
    pub guilds: BTreeMap<String, GuildSettings>,
    #[serde(default)]
    pub brains: BTreeMap<String, BrainRow>,
    /// Keyed `"{scoped_guild_id}\u{1f}{scoped_user_id}"` — the unit separator
    /// cannot occur in a snowflake or a platform tag, unlike `:`, which every
    /// scoped id already contains.
    #[serde(default)]
    pub reputations: BTreeMap<String, ReputationRow>,
    #[serde(default)]
    pub events: Vec<ReputationEvent>,
    /// Replies still inside their 150 s settlement window at the last
    /// persist, so a restart does not drop their rewards.
    #[serde(default)]
    pub pending_rewards: Vec<(String, crate::brain::reward::Pending)>,
    #[serde(default)]
    pub memory: MemoryBank,
    /// `0` is the historical two-authority layout. Version `1` means
    /// `memory` is canonical and WDBX `mem:*` rows are a rebuildable
    /// projection. Kept in the canonical JSON so its atomic rename also
    /// publishes the migration decision.
    #[serde(default)]
    pub memory_projection_version: u32,
}

/// Why a load or save failed. Carries the path so the log line is actionable.
#[derive(Debug)]
pub enum PersistError {
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Decode {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { op, path, source } => {
                write!(f, "could not {op} {}: {source}", path.display())
            }
            Self::Decode { path, source } => {
                write!(f, "could not decode {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PersistError {}

fn rep_key(guild: &str, user: &str) -> String {
    format!("{guild}\u{1f}{user}")
}

impl Stores {
    /// Path of the JSON document inside `dir`.
    pub fn state_path(dir: &Path) -> PathBuf {
        dir.join(STATE_FILE)
    }

    /// Path of the WDBX segment inside `dir`.
    pub fn wdbx_path(dir: &Path) -> PathBuf {
        dir.join(WDBX_FILE)
    }

    /// Load the document from `dir`, or an empty one if the file does not
    /// exist yet (first run). A present-but-unreadable file is an error: a
    /// silently fresh start would discard every guild's learning.
    pub fn load(dir: &Path) -> Result<Self, PersistError> {
        let path = Self::state_path(dir);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(PersistError::Io {
                    op: "read",
                    path,
                    source,
                });
            }
        };
        serde_json::from_str(&text).map_err(|source| PersistError::Decode { path, source })
    }

    /// Write the document atomically into `dir`, creating the directory.
    pub fn save(&self, dir: &Path) -> Result<(), PersistError> {
        fs::create_dir_all(dir).map_err(|source| PersistError::Io {
            op: "create",
            path: dir.to_path_buf(),
            source,
        })?;
        let path = Self::state_path(dir);
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string(self).map_err(|source| PersistError::Decode {
            path: path.clone(),
            source,
        })?;
        fs::write(&tmp, text).map_err(|source| PersistError::Io {
            op: "write",
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, &path).map_err(|source| PersistError::Io {
            op: "rename",
            path,
            source,
        })
    }
}

impl GuildConfigStore for Stores {
    fn load(&self, id: &str) -> Option<GuildSettings> {
        self.guilds.get(id).cloned()
    }

    fn save(&mut self, id: &str, settings: &GuildSettings) {
        self.guilds.insert(id.to_string(), settings.clone());
    }
}

impl BrainStore for Stores {
    fn load(&self, scoped_guild_id: &str) -> Option<(String, u64)> {
        self.brains
            .get(scoped_guild_id)
            .map(|row| (row.snapshot_json.clone(), row.experience_count))
    }

    fn save(&mut self, scoped_guild_id: &str, snapshot_json: &str, experience_count: u64) {
        self.brains.insert(
            scoped_guild_id.to_string(),
            BrainRow {
                snapshot_json: snapshot_json.to_string(),
                experience_count,
            },
        );
    }
}

impl ReputationStore for Stores {
    fn load_reputation(&self, guild: &str, user: &str) -> Option<f64> {
        self.reputations.get(&rep_key(guild, user)).map(|r| r.value)
    }

    fn store_reputation(
        &mut self,
        guild: &str,
        user: &str,
        value: f64,
        interaction_count_delta: u32,
    ) {
        let row = self
            .reputations
            .entry(rep_key(guild, user))
            .or_insert(ReputationRow {
                value,
                interaction_count: 0,
            });
        row.value = value;
        row.interaction_count = row
            .interaction_count
            .saturating_add(interaction_count_delta);
    }

    fn append_event(&mut self, event: ReputationEvent) {
        self.events.push(event);
        if self.events.len() > MAX_EVENTS {
            let excess = self.events.len() - MAX_EVENTS;
            self.events.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::Persona;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("abbey-persist-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = temp_dir("missing");
        let stores = Stores::load(&dir).expect("missing file is not an error");
        assert_eq!(stores, Stores::default());
    }

    #[test]
    fn save_then_load_round_trips_every_section() {
        let dir = temp_dir("roundtrip");
        let mut stores = Stores::default();
        let settings = GuildSettings {
            default_persona: Persona::Aviva,
            reply_cooldown_seconds: 45,
            ..Default::default()
        };
        GuildConfigStore::save(&mut stores, "discord:1", &settings);
        BrainStore::save(&mut stores, "discord:1", "{\"snapshot\":true}", 7);
        stores.store_reputation("discord:1", "discord:9", 0.62, 2);
        stores.append_event(ReputationEvent {
            user_id: "discord:9".into(),
            guild_id: "discord:1".into(),
            delta: 0.12,
            reason: "interaction".into(),
            at: 5,
        });
        assert!(
            stores
                .memory
                .remember("discord:1", "discord:9", "likes rust", 5)
        );

        stores.save(&dir).expect("save");
        let loaded = Stores::load(&dir).expect("load");
        assert_eq!(loaded, stores);
        assert_eq!(
            GuildConfigStore::load(&loaded, "discord:1")
                .unwrap()
                .default_persona,
            Persona::Aviva
        );
        assert_eq!(
            BrainStore::load(&loaded, "discord:1"),
            Some(("{\"snapshot\":true}".to_string(), 7))
        );
        assert_eq!(loaded.load_reputation("discord:1", "discord:9"), Some(0.62));
        assert_eq!(
            loaded
                .reputations
                .values()
                .next()
                .unwrap()
                .interaction_count,
            2
        );
        assert_eq!(
            loaded.memory.facts("discord:1", "discord:9"),
            ["likes rust"]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reputation_key_survives_colons_in_scoped_ids() {
        let mut stores = Stores::default();
        stores.store_reputation("discord:1", "discord:2", 0.4, 1);
        // A naive "guild:user" join would make these two indistinguishable.
        stores.store_reputation("discord", "1:discord:2", 0.9, 1);
        assert_eq!(stores.load_reputation("discord:1", "discord:2"), Some(0.4));
        assert_eq!(stores.load_reputation("discord", "1:discord:2"), Some(0.9));
    }

    #[test]
    fn event_trail_is_bounded() {
        let mut stores = Stores::default();
        for i in 0..(MAX_EVENTS + 5) {
            stores.append_event(ReputationEvent {
                user_id: "u".into(),
                guild_id: "g".into(),
                delta: 0.0,
                reason: "interaction".into(),
                at: i as u64,
            });
        }
        assert_eq!(stores.events.len(), MAX_EVENTS);
        assert_eq!(stores.events[0].at, 5, "oldest rows are the ones dropped");
    }

    #[test]
    fn an_older_document_with_missing_sections_still_loads() {
        let stores: Stores = serde_json::from_str("{\"guilds\":{}}").expect("partial document");
        assert!(stores.brains.is_empty());
        assert!(stores.memory.users.is_empty());
    }

    #[test]
    fn a_corrupt_document_is_an_error_not_a_fresh_start() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(Stores::state_path(&dir), "{not json").unwrap();
        assert!(matches!(
            Stores::load(&dir),
            Err(PersistError::Decode { .. })
        ));
        fs::remove_dir_all(&dir).ok();
    }
}
