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
