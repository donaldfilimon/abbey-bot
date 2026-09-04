//! Durable state: one JSON document plus one WDBX segment under `ABBEY_DATA_DIR`.
//!
//! The spec (`docs/spec/botarchitecture.md`) persists through Fluent +
//! PostgreSQL. This crate deliberately has no database: the host it deploys to
//! is a single process with a data directory, and every store the pure modules
//! need — guild config, per-guild brain snapshots, reputation, memory — is
//! small enough to hold in memory and write whole. `Stores` is that memory,
//! implementing the three store traits the registries speak. [`Stores::load`]
//! owns canonical loading; [`PersistenceSink`] owns canonical and projection
//! publication. Semantic memory lives beside it in the WDBX segment
//! ([`crate::wdbx::Recall`]) with its independent wire format.
//!
//! Writes are atomic (temp file + rename) so a crash mid-persist leaves the
//! previous document intact rather than a truncated one.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Content-free persistence failure categories safe for commands and
/// operational logs. Arbitrary paths, state, identifiers, and OS errors never
/// enter this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistErrorCategory {
    UnsafeFileType,
    CreateDirectory,
    SnapshotEncode,
    CreateTemporary,
    WriteTemporary,
    SyncTemporary,
    PublishRename,
    SyncDirectory,
    ProjectionEncode,
}

impl PersistErrorCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsafeFileType => "unsafe-file-type",
            Self::CreateDirectory => "create-directory",
            Self::SnapshotEncode => "snapshot-encode",
            Self::CreateTemporary => "create-temporary",
            Self::WriteTemporary => "write-temporary",
            Self::SyncTemporary => "sync-temporary",
            Self::PublishRename => "publish-rename",
            Self::SyncDirectory => "sync-directory",
            Self::ProjectionEncode => "projection-encode",
        }
    }
}

/// Result for one durable authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistComponentOutcome {
    NotConfigured,
    Committed,
    SkippedCanonicalFailure,
    Failed(PersistErrorCategory),
}

impl PersistComponentOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not-configured",
            Self::Committed => "committed",
            Self::SkippedCanonicalFailure => "skipped-canonical-failure",
            Self::Failed(_) => "failed",
        }
    }

    #[must_use]
    pub const fn error_category(self) -> Option<PersistErrorCategory> {
        match self {
            Self::Failed(category) => Some(category),
            _ => None,
        }
    }
}

/// Process-level persistence truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistOverall {
    MemoryOnly,
    Complete,
    Partial,
    Failed,
}

impl PersistOverall {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MemoryOnly => "memory-only",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

/// One complete persistence attempt, with canonical state and its rebuildable
/// WDBX projection reported independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistReport {
    pub overall: PersistOverall,
    pub canonical_state: PersistComponentOutcome,
    pub wdbx_projection: PersistComponentOutcome,
}

impl PersistReport {
    #[must_use]
    pub const fn memory_only() -> Self {
        Self {
            overall: PersistOverall::MemoryOnly,
            canonical_state: PersistComponentOutcome::NotConfigured,
            wdbx_projection: PersistComponentOutcome::NotConfigured,
        }
    }

    #[must_use]
    pub const fn from_components(
        canonical_state: PersistComponentOutcome,
        wdbx_projection: PersistComponentOutcome,
    ) -> Self {
        let overall = match (canonical_state, wdbx_projection) {
            (PersistComponentOutcome::NotConfigured, PersistComponentOutcome::NotConfigured) => {
                PersistOverall::MemoryOnly
            }
            (PersistComponentOutcome::Committed, PersistComponentOutcome::Committed) => {
                PersistOverall::Complete
            }
            (PersistComponentOutcome::Committed, PersistComponentOutcome::Failed(_)) => {
                PersistOverall::Partial
            }
            _ => PersistOverall::Failed,
        };
        Self {
            overall,
            canonical_state,
            wdbx_projection,
        }
    }
}

#[must_use]
pub fn render_component_outcome(outcome: PersistComponentOutcome) -> String {
    match outcome {
        PersistComponentOutcome::NotConfigured => "not configured".to_string(),
        PersistComponentOutcome::Committed => "committed".to_string(),
        PersistComponentOutcome::SkippedCanonicalFailure => {
            "skipped after canonical failure".to_string()
        }
        PersistComponentOutcome::Failed(category) => {
            format!("failed ({})", category.as_str())
        }
    }
}

/// Emit one content-free persistence result. `trigger` is supplied only by
/// fixed process call sites (`scheduled` and `shutdown`).
pub fn log_report(trigger: &'static str, report: &PersistReport) {
    let canonical_error = report
        .canonical_state
        .error_category()
        .map_or("none", PersistErrorCategory::as_str);
    let projection_error = report
        .wdbx_projection
        .error_category()
        .map_or("none", PersistErrorCategory::as_str);
    match report.overall {
        PersistOverall::MemoryOnly | PersistOverall::Complete => tracing::info!(
            trigger,
            overall = report.overall.as_str(),
            canonical = report.canonical_state.as_str(),
            canonical_error,
            wdbx = report.wdbx_projection.as_str(),
            wdbx_error = projection_error,
            "persistence attempt completed"
        ),
        PersistOverall::Partial => tracing::warn!(
            trigger,
            overall = report.overall.as_str(),
            canonical = report.canonical_state.as_str(),
            canonical_error,
            wdbx = report.wdbx_projection.as_str(),
            wdbx_error = projection_error,
            "persistence attempt completed"
        ),
        PersistOverall::Failed => tracing::error!(
            trigger,
            overall = report.overall.as_str(),
            canonical = report.canonical_state.as_str(),
            canonical_error,
            wdbx = report.wdbx_projection.as_str(),
            wdbx_error = projection_error,
            "persistence attempt completed"
        ),
    }
}

/// Injectable durable-output boundary. Tests can select exact component
/// failures without reading or mutating a real state directory.
pub trait PersistenceSink: Send + Sync {
    fn publish(
        &self,
        directory: &Path,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<(), PersistErrorCategory>;
}

#[derive(Debug, Default)]
pub struct FsPersistenceSink;

impl PersistenceSink for FsPersistenceSink {
    fn publish(
        &self,
        directory: &Path,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<(), PersistErrorCategory> {
        publish_bytes(&FsAtomicFileSink, directory, destination, bytes)
    }
}

pub fn persist_canonical(
    sink: &dyn PersistenceSink,
    directory: &Path,
    stores: &Stores,
) -> Result<(), PersistErrorCategory> {
    let bytes = serde_json::to_vec(stores).map_err(|_| PersistErrorCategory::SnapshotEncode)?;
    serde_json::from_slice::<Stores>(&bytes).map_err(|_| PersistErrorCategory::SnapshotEncode)?;
    sink.publish(directory, &Stores::state_path(directory), &bytes)
}

pub fn persist_projection(
    sink: &dyn PersistenceSink,
    directory: &Path,
    recall: &crate::wdbx::Recall,
) -> Result<(), PersistErrorCategory> {
    let bytes = recall
        .store()
        .try_render()
        .map_err(|_| PersistErrorCategory::ProjectionEncode)?
        .into_bytes();
    sink.publish(directory, &Stores::wdbx_path(directory), &bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistPathKind {
    Missing,
    Directory,
    RegularFile,
    Unsafe,
}

trait PersistenceTemporaryFile: Write + Send {
    fn sync_all(&mut self) -> io::Result<()>;
}

impl PersistenceTemporaryFile for File {
    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self)
    }
}

trait AtomicFileSink: Send + Sync {
    fn path_kind(&self, path: &Path) -> io::Result<PersistPathKind>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn create_temporary(&self, path: &Path) -> io::Result<Box<dyn PersistenceTemporaryFile>>;
    fn publish_rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn sync_directory(&self, path: &Path) -> io::Result<()>;
    fn remove_temporary(&self, path: &Path);
}

#[derive(Debug)]
struct FsAtomicFileSink;

impl AtomicFileSink for FsAtomicFileSink {
    fn path_kind(&self, path: &Path) -> io::Result<PersistPathKind> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                Ok(if file_type.is_symlink() {
                    PersistPathKind::Unsafe
                } else if file_type.is_dir() {
                    PersistPathKind::Directory
                } else if file_type.is_file() {
                    PersistPathKind::RegularFile
                } else {
                    PersistPathKind::Unsafe
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PersistPathKind::Missing),
            Err(error) => Err(error),
        }
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn create_temporary(&self, path: &Path) -> io::Result<Box<dyn PersistenceTemporaryFile>> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(path)
            .map(|file| Box::new(file) as Box<dyn PersistenceTemporaryFile>)
    }

    fn publish_rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            File::open(path)?.sync_all()
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(())
        }
    }

    fn remove_temporary(&self, path: &Path) {
        let _ = fs::remove_file(path);
    }
}

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const TEMPORARY_ATTEMPTS: u64 = 32;

fn publish_bytes(
    sink: &dyn AtomicFileSink,
    directory: &Path,
    destination: &Path,
    bytes: &[u8],
) -> Result<(), PersistErrorCategory> {
    match sink.path_kind(directory) {
        Ok(PersistPathKind::Missing) => {
            sink.create_dir_all(directory)
                .map_err(|_| PersistErrorCategory::CreateDirectory)?;
            if !matches!(sink.path_kind(directory), Ok(PersistPathKind::Directory)) {
                return Err(PersistErrorCategory::UnsafeFileType);
            }
        }
        Ok(PersistPathKind::Directory) => {}
        Ok(_) | Err(_) => return Err(PersistErrorCategory::UnsafeFileType),
    }

    match sink.path_kind(destination) {
        Ok(PersistPathKind::Missing | PersistPathKind::RegularFile) => {}
        Ok(_) | Err(_) => return Err(PersistErrorCategory::UnsafeFileType),
    }

    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PersistErrorCategory::UnsafeFileType)?;
    let mut opened = None;
    for _ in 0..TEMPORARY_ATTEMPTS {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(
            ".{file_name}.tmp-{}-{sequence}",
            std::process::id()
        ));
        match sink.path_kind(&temporary) {
            Ok(PersistPathKind::Missing) => match sink.create_temporary(&temporary) {
                Ok(file) => {
                    opened = Some((temporary, file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(PersistErrorCategory::CreateTemporary),
            },
            Ok(PersistPathKind::RegularFile) => continue,
            Ok(_) | Err(_) => return Err(PersistErrorCategory::UnsafeFileType),
        }
    }
    let (temporary, mut file) = opened.ok_or(PersistErrorCategory::CreateTemporary)?;

    let before_publish = (|| {
        file.write_all(bytes)
            .map_err(|_| PersistErrorCategory::WriteTemporary)?;
        file.flush()
            .map_err(|_| PersistErrorCategory::SyncTemporary)?;
        file.sync_all()
            .map_err(|_| PersistErrorCategory::SyncTemporary)?;
        drop(file);
        match sink.path_kind(destination) {
            Ok(PersistPathKind::Missing | PersistPathKind::RegularFile) => {}
            Ok(_) | Err(_) => return Err(PersistErrorCategory::UnsafeFileType),
        }
        sink.publish_rename(&temporary, destination)
            .map_err(|_| PersistErrorCategory::PublishRename)
    })();
    if let Err(category) = before_publish {
        sink.remove_temporary(&temporary);
        return Err(category);
    }
    sink.sync_directory(directory)
        .map_err(|_| PersistErrorCategory::SyncDirectory)
}

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
        let path = Self::state_path(dir);
        persist_canonical(&FsPersistenceSink, dir, self).map_err(|category| PersistError::Io {
            op: "persist",
            path,
            source: io::Error::other(category.as_str()),
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
pub(crate) mod tests;
