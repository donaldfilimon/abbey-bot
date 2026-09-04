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
pub(crate) mod tests {
    use super::*;
    use crate::persona::Persona;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FaultPoint {
        CreateDirectory,
        CreateTemporary,
        WriteTemporary,
        SyncTemporary,
        PublishRename,
        SyncDirectory,
    }

    #[derive(Default)]
    struct MemorySinkState {
        directories: std::collections::BTreeSet<PathBuf>,
        files: BTreeMap<PathBuf, Vec<u8>>,
        unsafe_paths: std::collections::BTreeSet<PathBuf>,
        operations: Vec<&'static str>,
    }

    #[derive(Clone, Default)]
    struct MemorySink {
        state: Arc<Mutex<MemorySinkState>>,
        fault: Option<FaultPoint>,
    }

    impl MemorySink {
        fn failing(fault: FaultPoint) -> Self {
            Self {
                fault: Some(fault),
                ..Self::default()
            }
        }

        fn put_directory(&self, path: &Path) {
            self.state
                .lock()
                .unwrap()
                .directories
                .insert(path.to_path_buf());
        }

        fn put_file(&self, path: &Path, bytes: &[u8]) {
            self.state
                .lock()
                .unwrap()
                .files
                .insert(path.to_path_buf(), bytes.to_vec());
        }

        fn put_unsafe(&self, path: &Path) {
            self.state
                .lock()
                .unwrap()
                .unsafe_paths
                .insert(path.to_path_buf());
        }

        fn bytes(&self, path: &Path) -> Option<Vec<u8>> {
            self.state.lock().unwrap().files.get(path).cloned()
        }
    }

    struct MemoryTemporary {
        path: PathBuf,
        state: Arc<Mutex<MemorySinkState>>,
        fault: Option<FaultPoint>,
    }

    impl io::Write for MemoryTemporary {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fault == Some(FaultPoint::WriteTemporary) {
                return Err(io::Error::other("injected write failure"));
            }
            self.state
                .lock()
                .unwrap()
                .files
                .get_mut(&self.path)
                .expect("temporary exists")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl PersistenceTemporaryFile for MemoryTemporary {
        fn sync_all(&mut self) -> io::Result<()> {
            if self.fault == Some(FaultPoint::SyncTemporary) {
                return Err(io::Error::other("injected file sync failure"));
            }
            Ok(())
        }
    }

    impl AtomicFileSink for MemorySink {
        fn path_kind(&self, path: &Path) -> io::Result<PersistPathKind> {
            let state = self.state.lock().unwrap();
            Ok(if state.unsafe_paths.contains(path) {
                PersistPathKind::Unsafe
            } else if state.directories.contains(path) {
                PersistPathKind::Directory
            } else if state.files.contains_key(path) {
                PersistPathKind::RegularFile
            } else {
                PersistPathKind::Missing
            })
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            if self.fault == Some(FaultPoint::CreateDirectory) {
                return Err(io::Error::other("injected directory creation failure"));
            }
            let mut state = self.state.lock().unwrap();
            state.operations.push("create_directory");
            state.directories.insert(path.to_path_buf());
            Ok(())
        }

        fn create_temporary(&self, path: &Path) -> io::Result<Box<dyn PersistenceTemporaryFile>> {
            if self.fault == Some(FaultPoint::CreateTemporary) {
                return Err(io::Error::other("injected temporary creation failure"));
            }
            let mut state = self.state.lock().unwrap();
            state.operations.push("create_temporary");
            if state.files.contains_key(path) {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "collision"));
            }
            state.files.insert(path.to_path_buf(), Vec::new());
            drop(state);
            Ok(Box::new(MemoryTemporary {
                path: path.to_path_buf(),
                state: Arc::clone(&self.state),
                fault: self.fault,
            }))
        }

        fn publish_rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            if self.fault == Some(FaultPoint::PublishRename) {
                return Err(io::Error::other("injected rename failure"));
            }
            let mut state = self.state.lock().unwrap();
            state.operations.push("publish_rename");
            let bytes = state.files.remove(from).expect("temporary exists");
            state.files.insert(to.to_path_buf(), bytes);
            Ok(())
        }

        fn sync_directory(&self, _path: &Path) -> io::Result<()> {
            if self.fault == Some(FaultPoint::SyncDirectory) {
                return Err(io::Error::other("injected directory sync failure"));
            }
            self.state.lock().unwrap().operations.push("sync_directory");
            Ok(())
        }

        fn remove_temporary(&self, path: &Path) {
            self.state.lock().unwrap().files.remove(path);
        }
    }

    impl PersistenceSink for MemorySink {
        fn publish(
            &self,
            directory: &Path,
            destination: &Path,
            bytes: &[u8],
        ) -> Result<(), PersistErrorCategory> {
            publish_bytes(self, directory, destination, bytes)
        }
    }

    #[derive(Clone)]
    pub(crate) struct RuntimeRecordingSink {
        attempts: Arc<Mutex<Vec<&'static str>>>,
        canonical_failure: Option<PersistErrorCategory>,
        projection_failure: Option<PersistErrorCategory>,
    }

    impl RuntimeRecordingSink {
        pub(crate) fn success() -> Self {
            Self {
                attempts: Arc::default(),
                canonical_failure: None,
                projection_failure: None,
            }
        }

        pub(crate) fn fail_canonical(category: PersistErrorCategory) -> Self {
            Self {
                attempts: Arc::default(),
                canonical_failure: Some(category),
                projection_failure: None,
            }
        }

        pub(crate) fn fail_projection(category: PersistErrorCategory) -> Self {
            Self {
                attempts: Arc::default(),
                canonical_failure: None,
                projection_failure: Some(category),
            }
        }

        pub(crate) fn attempts(&self) -> Vec<&'static str> {
            self.attempts.lock().unwrap().clone()
        }
    }

    impl PersistenceSink for RuntimeRecordingSink {
        fn publish(
            &self,
            _directory: &Path,
            destination: &Path,
            _bytes: &[u8],
        ) -> Result<(), PersistErrorCategory> {
            if destination
                .file_name()
                .is_some_and(|name| name == STATE_FILE)
            {
                self.attempts.lock().unwrap().push("canonical");
                self.canonical_failure.map_or(Ok(()), Err)
            } else {
                self.attempts.lock().unwrap().push("wdbx");
                self.projection_failure.map_or(Ok(()), Err)
            }
        }
    }

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

    #[test]
    fn persist_report_truth_table_is_exact() {
        assert_eq!(
            PersistReport::memory_only().overall,
            PersistOverall::MemoryOnly
        );
        assert_eq!(
            PersistReport::from_components(
                PersistComponentOutcome::Committed,
                PersistComponentOutcome::Committed,
            )
            .overall,
            PersistOverall::Complete
        );
        assert_eq!(
            PersistReport::from_components(
                PersistComponentOutcome::Committed,
                PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory),
            )
            .overall,
            PersistOverall::Partial
        );
        assert_eq!(
            PersistReport::from_components(
                PersistComponentOutcome::Failed(PersistErrorCategory::WriteTemporary),
                PersistComponentOutcome::SkippedCanonicalFailure,
            )
            .overall,
            PersistOverall::Failed
        );
    }

    #[test]
    fn prepublication_failures_preserve_the_old_destination() {
        let dir = Path::new("/injected/state");
        let destination = dir.join(STATE_FILE);
        let cases = [
            (
                FaultPoint::CreateTemporary,
                PersistErrorCategory::CreateTemporary,
            ),
            (
                FaultPoint::WriteTemporary,
                PersistErrorCategory::WriteTemporary,
            ),
            (
                FaultPoint::SyncTemporary,
                PersistErrorCategory::SyncTemporary,
            ),
            (
                FaultPoint::PublishRename,
                PersistErrorCategory::PublishRename,
            ),
        ];
        for (fault, expected) in cases {
            let sink = MemorySink::failing(fault);
            sink.put_directory(dir);
            sink.put_file(&destination, b"old-state");
            assert_eq!(
                sink.publish(dir, &destination, b"new-state"),
                Err(expected),
                "wrong category for {fault:?}"
            );
            assert_eq!(
                sink.bytes(&destination).as_deref(),
                Some(b"old-state".as_slice())
            );
        }
    }

    #[test]
    fn unsafe_destination_is_rejected_without_replacement() {
        let dir = Path::new("/injected/state");
        let destination = dir.join(STATE_FILE);
        let sink = MemorySink::default();
        sink.put_directory(dir);
        sink.put_unsafe(&destination);
        assert_eq!(
            sink.publish(dir, &destination, b"new-state"),
            Err(PersistErrorCategory::UnsafeFileType)
        );
        assert!(
            sink.state
                .lock()
                .unwrap()
                .unsafe_paths
                .contains(&destination)
        );
    }

    #[test]
    fn directory_creation_failure_is_categorized() {
        let dir = Path::new("/injected/state");
        let destination = dir.join(STATE_FILE);
        let sink = MemorySink::failing(FaultPoint::CreateDirectory);
        assert_eq!(
            sink.publish(dir, &destination, b"new-state"),
            Err(PersistErrorCategory::CreateDirectory)
        );
        assert!(sink.bytes(&destination).is_none());
    }

    #[test]
    fn directory_sync_failure_is_postpublication_and_reports_failure() {
        let dir = Path::new("/injected/state");
        let destination = dir.join(STATE_FILE);
        let sink = MemorySink::failing(FaultPoint::SyncDirectory);
        sink.put_directory(dir);
        sink.put_file(&destination, b"old-state");
        assert_eq!(
            sink.publish(dir, &destination, b"new-state"),
            Err(PersistErrorCategory::SyncDirectory)
        );
        assert_eq!(
            sink.bytes(&destination).as_deref(),
            Some(b"new-state".as_slice())
        );
    }

    #[test]
    fn non_finite_projection_is_categorized_before_any_output() {
        let mut store = crate::wdbx::WdbxStore::new();
        store.insert_vector(vec![f32::NAN]);
        let recall = crate::wdbx::Recall::from_store(store);
        assert_eq!(
            persist_projection(&FsPersistenceSink, Path::new("/never-touched"), &recall),
            Err(PersistErrorCategory::ProjectionEncode)
        );
    }

    #[test]
    fn non_roundtrippable_snapshot_is_categorized_before_any_output() {
        let mut stores = Stores::default();
        stores.reputations.insert(
            "g\u{1f}u".to_string(),
            ReputationRow {
                value: f64::NAN,
                interaction_count: 1,
            },
        );
        assert_eq!(
            persist_canonical(&FsPersistenceSink, Path::new("/never-touched"), &stores),
            Err(PersistErrorCategory::SnapshotEncode)
        );
    }
}
