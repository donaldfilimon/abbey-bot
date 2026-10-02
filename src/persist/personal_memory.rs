//! One process-death-safe owner for canonical and consent journal publication.
use crate::{
    persist::{PersistenceSink, Stores, persist_canonical_owned},
    personal_memory::*,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Withdrawal {
    pub guild: String,
    pub user: String,
    pub epoch: u64,
    pub minimum_revision: u64,
    pub exposure_epoch: u64,
    pub cutoff: u64,
    pub request_id: String,
    pub payload_digest: String,
    pub at: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub schema: u32,
    pub revision: u64,
    pub store_identity: String,
    pub withdrawals: BTreeMap<String, Withdrawal>,
    #[serde(default)]
    pub activations: BTreeMap<String, Withdrawal>,
    pub checksum: String,
}
const NAME: &str = "personal-memory-withdrawals.json";
pub struct Lease {
    file: File,
    dir: PathBuf,
}
impl Lease {
    pub(crate) fn owns(&self, dir: &Path) -> bool {
        fs::canonicalize(dir).is_ok_and(|p| p == self.dir)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
pub fn lease(dir: &Path) -> Result<Lease, MemoryConsentError> {
    fs::create_dir_all(dir).map_err(|_| MemoryConsentError::Persistence)?;
    let path = dir.join("personal-memory-consent.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(MemoryConsentError::Persistence);
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| MemoryConsentError::Persistence)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file
            .metadata()
            .map_err(|_| MemoryConsentError::Persistence)?
            .mode()
            & 0o077
            != 0
        {
            return Err(MemoryConsentError::Persistence);
        }
    }
    file.try_lock()
        .map_err(|_| MemoryConsentError::Persistence)?;
    Ok(Lease {
        file,
        dir: fs::canonicalize(dir).map_err(|_| MemoryConsentError::Persistence)?,
    })
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn identity(dir: &Path) -> Result<String, MemoryConsentError> {
    Ok(digest(
        fs::canonicalize(dir)
            .map_err(|_| MemoryConsentError::Persistence)?
            .to_string_lossy()
            .as_bytes(),
    ))
}
fn checksum(m: &Marker) -> Result<String, MemoryConsentError> {
    let mut unsigned = m.clone();
    unsigned.checksum.clear();
    serde_json::to_vec(&("abbey-withdrawal-v1", unsigned))
        .map(|b| digest(&b))
        .map_err(|_| MemoryConsentError::Persistence)
}
fn validate(marker: &Marker, dir: &Path) -> Result<(), MemoryConsentError> {
    if marker.schema != 1
        || marker.store_identity != identity(dir)?
        || marker.withdrawals.len() + marker.activations.len() > MAX_SUBJECTS
    {
        return Err(MemoryConsentError::Bounds);
    }
    for (key, w) in marker.withdrawals.iter().chain(&marker.activations) {
        if !valid_scope(&w.guild)
            || !valid_scope(&w.user)
            || subject_key(&w.guild, &w.user) != *key
            || w.request_id.is_empty()
            || w.request_id.len() > 64
            || !valid_digest(&w.payload_digest)
            || w.epoch == 0
            || w.epoch == u64::MAX
            || w.minimum_revision == 0
            || w.minimum_revision == u64::MAX
            || w.exposure_epoch == 0
            || w.exposure_epoch == u64::MAX
            || w.cutoff == 0
            || w.at == 0
            || w.at > w.cutoff
        {
            return Err(MemoryConsentError::InvalidProof);
        }
    }
    Ok(())
}
pub fn load(dir: &Path) -> Result<Marker, MemoryConsentError> {
    let path = dir.join(NAME);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Marker {
                schema: 1,
                store_identity: identity(dir)?,
                ..Default::default()
            });
        }
        Err(_) => return Err(MemoryConsentError::Persistence),
        Ok(m) => {
            if !m.is_file() {
                return Err(MemoryConsentError::Persistence);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if m.mode() & 0o077 != 0 {
                    return Err(MemoryConsentError::Persistence);
                }
            }
        }
    }
    let bytes = fs::read(path).map_err(|_| MemoryConsentError::Persistence)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(MemoryConsentError::Bounds);
    }
    let m: Marker = serde_json::from_slice(&bytes).map_err(|_| MemoryConsentError::Persistence)?;
    validate(&m, dir)?;
    if m.checksum != checksum(&m)? {
        return Err(MemoryConsentError::Persistence);
    }
    Ok(m)
}
#[cfg(test)]
pub fn publish(dir: &Path, expected: u64, marker: Marker) -> Result<Marker, MemoryConsentError> {
    let owner = lease(dir)?;
    publish_owned(&owner, dir, expected, marker)
}
pub fn publish_owned(
    owner: &Lease,
    dir: &Path,
    expected: u64,
    mut marker: Marker,
) -> Result<Marker, MemoryConsentError> {
    if !owner.owns(dir) || load(dir)?.revision != expected {
        return Err(MemoryConsentError::Stale);
    }
    validate(&marker, dir)?;
    marker.revision = expected.checked_add(1).ok_or(MemoryConsentError::Bounds)?;
    marker.checksum = checksum(&marker)?;
    let bytes = serde_json::to_vec(&marker).map_err(|_| MemoryConsentError::Bounds)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(MemoryConsentError::Bounds);
    }
    let temp = dir.join(format!(
        ".{NAME}.{}.{}",
        std::process::id(),
        marker.revision
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|_| MemoryConsentError::Persistence)?;
    #[cfg(test)]
    let phase = if marker.withdrawals.is_empty() && marker.activations.is_empty() {
        "removal"
    } else {
        "marker"
    };
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|_| MemoryConsentError::Persistence)?;
        file.sync_all()
            .map_err(|_| MemoryConsentError::Persistence)?;
        #[cfg(test)]
        crash_checkpoint(&format!("{phase}-temporary-fsync"));
        fs::rename(&temp, dir.join(NAME)).map_err(|_| MemoryConsentError::Persistence)?;
        #[cfg(test)]
        crash_checkpoint(&format!("{phase}-rename"));
        File::open(dir)
            .and_then(|f| f.sync_all())
            .map_err(|_| MemoryConsentError::Persistence)?;
        #[cfg(test)]
        crash_checkpoint(&format!("{phase}-directory-fsync"));
        let read = load(dir)?;
        if read != marker {
            return Err(MemoryConsentError::Persistence);
        }
        Ok(read)
    })();
    let _ = fs::remove_file(temp);
    result
}
pub fn add_withdrawal(
    marker: &mut Marker,
    guild: &str,
    user: &str,
    mut w: Withdrawal,
) -> Result<(), MemoryConsentError> {
    if w.guild != guild || w.user != user {
        return Err(MemoryConsentError::InvalidProof);
    }
    let key = subject_key(guild, user);
    if !marker.withdrawals.contains_key(&key) && marker.withdrawals.len() >= MAX_SUBJECTS {
        return Err(MemoryConsentError::Bounds);
    }
    if let Some(old) = marker.withdrawals.get(&key) {
        w.epoch = w.epoch.max(old.epoch);
        w.minimum_revision = w.minimum_revision.max(old.minimum_revision);
        w.exposure_epoch = w.exposure_epoch.max(old.exposure_epoch);
        w.cutoff = w.cutoff.max(old.cutoff);
    }
    marker.withdrawals.insert(key, w);
    Ok(())
}
pub fn fold_denials(stores: &mut Stores, marker: &Marker) -> Result<bool, MemoryConsentError> {
    let mut changed = false;
    for (key, w) in marker.withdrawals.iter().chain(&marker.activations) {
        if !stores.personal_memory.contains_key(key) && stores.personal_memory.len() >= MAX_SUBJECTS
        {
            return Err(MemoryConsentError::Bounds);
        }
        let s = stores.personal_memory.entry(key.clone()).or_default();
        let changes_choice = s.choice != UseChoice::Off || s.activation_pending;
        s.choice = UseChoice::Off;
        s.activation_pending = false;
        if s.outcomes
            .get(&w.request_id)
            .is_some_and(|r| r.result.choice == UseChoice::On)
        {
            s.outcomes.remove(&w.request_id);
        }
        s.schema = 1;
        s.consent_epoch = s.consent_epoch.max(w.epoch);
        s.revision = s.revision.max(w.minimum_revision);
        if changes_choice {
            s.advance()?;
        }
        stores.personal_memory_exposure.epoch =
            stores.personal_memory_exposure.epoch.max(w.exposure_epoch);
        stores.personal_memory_exposure.cutoff =
            stores.personal_memory_exposure.cutoff.max(w.cutoff);
        changed = true;
    }
    validate_metadata(&stores.personal_memory, &stores.personal_memory_exposure)?;
    Ok(changed)
}
pub fn verify_minima(stores: &Stores, marker: &Marker) -> Result<(), MemoryConsentError> {
    for (key, w) in marker.withdrawals.iter().chain(&marker.activations) {
        let s = stores
            .personal_memory
            .get(key)
            .ok_or(MemoryConsentError::Persistence)?;
        if s.choice != UseChoice::Off
            || s.activation_pending
            || s.consent_epoch < w.epoch
            || s.revision < w.minimum_revision
            || stores.personal_memory_exposure.epoch < w.exposure_epoch
            || stores.personal_memory_exposure.cutoff < w.cutoff
        {
            return Err(MemoryConsentError::Persistence);
        }
    }
    Ok(())
}
pub fn recover(stores: &mut Stores, dir: &Path) -> Result<(), MemoryConsentError> {
    let owner = lease(dir)?;
    recover_owned(&owner, stores, dir, &crate::persist::FsPersistenceSink)
}
pub fn recover_owned(
    owner: &Lease,
    stores: &mut Stores,
    dir: &Path,
    sink: &dyn PersistenceSink,
) -> Result<(), MemoryConsentError> {
    if !owner.owns(dir) {
        return Err(MemoryConsentError::Persistence);
    }
    let mut current = Stores::load(dir).map_err(|_| MemoryConsentError::Persistence)?;
    validate_metadata(&current.personal_memory, &current.personal_memory_exposure)?;
    let mut marker = load(dir)?;
    let mut changed = fold_denials(&mut current, &marker)?;
    let mut unresolved = false;
    for s in current.personal_memory.values_mut() {
        if s.activation_pending {
            let previous = s.revision;
            s.outcomes.retain(|_, r| {
                !(r.result.stamp.revision == previous && r.result.choice == UseChoice::On)
            });
            s.choice = UseChoice::Off;
            s.activation_pending = false;
            s.advance()?;
            unresolved = true;
            changed = true;
        }
    }
    if unresolved {
        current.personal_memory_exposure.epoch = current
            .personal_memory_exposure
            .epoch
            .checked_add(1)
            .ok_or(MemoryConsentError::Bounds)?;
        current.personal_memory_exposure.cutoff = current
            .personal_memory_exposure
            .cutoff
            .max(crate::runtime::now().max(1));
    }
    if current.personal_memory_exposure.schema == 0 {
        current.personal_memory_exposure.schema = 1;
        current.personal_memory_exposure.epoch = current
            .personal_memory_exposure
            .epoch
            .checked_add(1)
            .ok_or(MemoryConsentError::Bounds)?;
        current.personal_memory_exposure.cutoff = crate::runtime::now().max(1);
        let epoch = current.personal_memory_exposure.epoch;
        let cutoff = current.personal_memory_exposure.cutoff;
        current
            .personal_memory_exposure
            .receipts
            .push(ExposureReceipt {
                epoch,
                cutoff,
                request_digest: digest(b"initial-exposure-v1"),
            });
        changed = true;
    }
    // An accepted canonical image alone is not a completed retained transaction.
    if current
        .personal_memory
        .values()
        .any(|s| s.outcomes.values().any(|r| !r.completed))
    {
        let projection_path = Stores::wdbx_path(dir);
        let mut projection = if projection_path.exists() {
            crate::wdbx::Recall::load(&projection_path)
                .map_err(|_| MemoryConsentError::Persistence)?
        } else {
            crate::wdbx::Recall::default()
        };
        projection.reconcile_memory_facts(
            current
                .memory
                .fact_records()
                .into_iter()
                .map(|f| (f.guild, f.user, f.text, f.at)),
        );
        crate::persist::persist_projection(sink, dir, &projection)
            .map_err(|_| MemoryConsentError::Persistence)?;
        let read = crate::wdbx::Recall::load(&Stores::wdbx_path(dir))
            .map_err(|_| MemoryConsentError::Persistence)?;
        if read.all_memory_facts() != projection.all_memory_facts() {
            return Err(MemoryConsentError::Persistence);
        }
        for s in current.personal_memory.values_mut() {
            for r in s.outcomes.values_mut() {
                r.completed = true;
            }
        }
        changed = true;
    }
    validate_metadata(&current.personal_memory, &current.personal_memory_exposure)?;
    if changed {
        persist_canonical_owned(owner, sink, dir, &current)
            .map_err(|_| MemoryConsentError::Persistence)?;
        let disk = Stores::load(dir).map_err(|_| MemoryConsentError::Persistence)?;
        let expected = serde_json::to_vec(&current).map_err(|_| MemoryConsentError::Persistence)?;
        let actual =
            fs::read(Stores::state_path(dir)).map_err(|_| MemoryConsentError::Persistence)?;
        if expected != actual {
            return Err(MemoryConsentError::Persistence);
        }
        current.canonical_base.set(disk.canonical_base.get());
        if disk.personal_memory != current.personal_memory
            || disk.personal_memory_exposure != current.personal_memory_exposure
        {
            return Err(MemoryConsentError::Persistence);
        }
        verify_minima(&disk, &marker)?;
        if !marker.withdrawals.is_empty() || !marker.activations.is_empty() {
            marker.withdrawals.clear();
            marker.activations.clear();
            publish_owned(owner, dir, marker.revision, marker)?;
        }
    }
    *stores = current;
    Ok(())
}
#[cfg(test)]
fn crash_checkpoint(phase: &str) {
    if std::env::var("ABBEY_CONSENT_TEST_CRASH_AT").is_ok_and(|value| value == phase) {
        std::process::exit(91)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct ChildOwner(std::process::Child);
    impl Drop for ChildOwner {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn directory() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "abbey-consent-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    panic!("create personal-memory persistence fixture directory: {error}")
                }
            }
        }
    }
    fn withdrawal(id: &str) -> Withdrawal {
        Withdrawal {
            guild: "g".into(),
            user: "u".into(),
            epoch: 7,
            minimum_revision: 8,
            exposure_epoch: 9,
            cutoff: 10,
            request_id: id.into(),
            payload_digest: digest(id.as_bytes()),
            at: 1,
        }
    }
    #[test]
    fn stale_marker_cas_refuses() {
        let dir = directory();
        let m = load(&dir).unwrap();
        let first = publish(&dir, 0, m.clone()).unwrap();
        assert!(publish(&dir, 0, m).is_err());
        assert_eq!(load(&dir).unwrap(), first);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn marker_validation_and_maxima() {
        let dir = directory();
        let mut m = load(&dir).unwrap();
        add_withdrawal(&mut m, "g", "u", withdrawal("first")).unwrap();
        let mut lesser = withdrawal("second");
        lesser.epoch = 1;
        lesser.minimum_revision = 1;
        lesser.exposure_epoch = 1;
        lesser.cutoff = 1;
        add_withdrawal(&mut m, "g", "u", lesser).unwrap();
        let w = &m.withdrawals[&subject_key("g", "u")];
        assert_eq!(
            (w.epoch, w.minimum_revision, w.exposure_epoch, w.cutoff),
            (7, 8, 9, 10)
        );
        publish(&dir, 0, m.clone()).unwrap();
        let mut bad = load(&dir).unwrap();
        bad.withdrawals
            .get_mut(&subject_key("g", "u"))
            .unwrap()
            .request_id = "x".repeat(65);
        bad.checksum = checksum(&bad).unwrap();
        fs::write(dir.join(NAME), serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(load(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn lease_never_removes_live_or_released_sentinel() {
        let dir = directory();
        let first = lease(&dir).unwrap();
        assert!(lease(&dir).is_err());
        drop(first);
        assert!(dir.join("personal-memory-consent.lock").exists());
        let second = lease(&dir).unwrap();
        drop(second);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn recovery_reload_preserves_newer_facts() {
        let dir = directory();
        let mut old = Stores::default();
        old.memory.remember("g", "u", "old", 1);
        old.save(&dir).unwrap();
        let mut newer = Stores::load(&dir).unwrap();
        newer.memory.remember("g", "u", "newer", 2);
        newer.save(&dir).unwrap();
        let mut marker = load(&dir).unwrap();
        add_withdrawal(&mut marker, "g", "u", withdrawal("off")).unwrap();
        publish(&dir, 0, marker).unwrap();
        recover(&mut old, &dir).unwrap();
        let disk = Stores::load(&dir).unwrap();
        assert_eq!(disk.memory.facts("g", "u"), ["old", "newer"]);
        assert_eq!(
            disk.personal_memory[&subject_key("g", "u")].choice,
            UseChoice::Off
        );
        assert!(disk.personal_memory_exposure.cutoff >= 10);
        assert!(load(&dir).unwrap().withdrawals.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn initial_cutoff_is_durable_and_idempotent() {
        let dir = directory();
        let mut stores = Stores::default();
        recover(&mut stores, &dir).unwrap();
        let first = stores.personal_memory_exposure.clone();
        assert_eq!(first.schema, 1);
        assert!(first.epoch > 0 && first.cutoff > 0);
        recover(&mut stores, &dir).unwrap();
        assert_eq!(stores.personal_memory_exposure, first);
        assert_eq!(Stores::load(&dir).unwrap().personal_memory_exposure, first);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn future_metadata_is_not_overwritten() {
        let dir = directory();
        let bytes=br#"{"personal_memory":{"g\u001fu":{"schema":99,"opaque":{"future":"keep"},"revision":1,"consent_epoch":1,"choice":"Off","policy_version":1,"proofs":{},"outcomes":{}}}}"#;
        fs::write(Stores::state_path(&dir), bytes).unwrap();
        assert!(Stores::load(&dir).is_err());
        assert!(Stores::default().save(&dir).is_err());
        assert_eq!(fs::read(Stores::state_path(&dir)).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ordinary_writer_cannot_erase_newer_consent() {
        let dir = directory();
        let stale = Stores::default();
        let mut current = stale.clone();
        current.personal_memory_exposure.schema = 1;
        current.personal_memory_exposure.epoch = 2;
        current.personal_memory_exposure.cutoff = 1;
        current.save(&dir).unwrap();
        assert!(stale.save(&dir).is_err());
        assert_eq!(
            Stores::load(&dir).unwrap().personal_memory_exposure.epoch,
            2
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn preexisting_temp_preserved() {
        let dir = directory();
        let temp = dir.join(format!(".{NAME}.{}.1", std::process::id()));
        fs::write(&temp, "existing").unwrap();
        assert!(publish(&dir, 0, load(&dir).unwrap()).is_err());
        assert_eq!(fs::read_to_string(temp).unwrap(), "existing");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore]
    fn lease_child_helper() {
        let Some(dir) = std::env::var_os("ABBEY_CONSENT_TEST_CHILD_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let _owner = lease(&dir).unwrap();
        fs::write(dir.join("child-ready"), "ready").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    fn killed_owner_releases_lock_without_removing_sentinel() {
        let dir = directory();
        let mut child = ChildOwner(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persist::personal_memory::tests::lease_child_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("ABBEY_CONSENT_TEST_CHILD_DIR", &dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        for _ in 0..2000 {
            if dir.join("child-ready").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(dir.join("child-ready").exists());
        assert!(lease(&dir).is_err());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let owner = lease(&dir).unwrap();
        drop(owner);
        assert!(dir.join("personal-memory-consent.lock").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore]
    fn withdrawal_crash_child() {
        let Some(raw) = std::env::var_os("ABBEY_CONSENT_TEST_CHILD_DIR") else {
            return;
        };
        let dir = PathBuf::from(raw);
        let mut stores = Stores::default();
        stores.personal_memory.insert(
            subject_key("g", "u"),
            PersonalMemorySubject {
                schema: 1,
                revision: 1,
                consent_epoch: 1,
                choice: UseChoice::On,
                policy_version: 1,
                ..Default::default()
            },
        );
        stores.personal_memory_exposure = ExposureState {
            schema: 1,
            epoch: 1,
            cutoff: 1,
            ..Default::default()
        };
        stores.save(&dir).unwrap();
        stores = Stores::load(&dir).unwrap();
        let owner = lease(&dir).unwrap();
        let mut marker = load(&dir).unwrap();
        add_withdrawal(&mut marker, "g", "u", withdrawal("off")).unwrap();
        marker = publish_owned(&owner, &dir, 0, marker).unwrap();
        let subject = stores
            .personal_memory
            .get_mut(&subject_key("g", "u"))
            .unwrap();
        subject.choice = UseChoice::Off;
        subject.revision = 8;
        subject.consent_epoch = 7;
        stores.personal_memory_exposure.epoch = 9;
        stores.personal_memory_exposure.cutoff = 10;
        persist_canonical_owned(&owner, &crate::persist::FsPersistenceSink, &dir, &stores).unwrap();
        crash_checkpoint("canonical-off");
        let disk = Stores::load(&dir).unwrap();
        assert_eq!(disk.personal_memory, stores.personal_memory);
        crash_checkpoint("canonical-readback");
        marker.withdrawals.clear();
        publish_owned(&owner, &dir, marker.revision, marker).unwrap();
    }
    #[test]
    fn durable_withdrawal_process_crash_windows_recover_off() {
        for phase in [
            "marker-rename",
            "marker-directory-fsync",
            "canonical-off",
            "canonical-readback",
            "removal-temporary-fsync",
            "removal-rename",
            "removal-directory-fsync",
        ] {
            let dir = directory();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persist::personal_memory::tests::withdrawal_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("ABBEY_CONSENT_TEST_CHILD_DIR", &dir)
                .env("ABBEY_CONSENT_TEST_CRASH_AT", phase)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(91), "{phase}");
            let mut stores = Stores::load(&dir).unwrap();
            recover(&mut stores, &dir).unwrap();
            assert_eq!(
                stores.personal_memory[&subject_key("g", "u")].choice,
                UseChoice::Off,
                "{phase}"
            );
            recover(&mut stores, &dir).unwrap();
            assert_eq!(
                Stores::load(&dir).unwrap().personal_memory[&subject_key("g", "u")].choice,
                UseChoice::Off
            );
            fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    #[ignore]
    fn publisher_child_helper() {
        let Some(raw) = std::env::var_os("ABBEY_CONSENT_TEST_CHILD_DIR") else {
            return;
        };
        let dir = PathBuf::from(raw);
        let id = std::env::var("ABBEY_CONSENT_TEST_WRITER_ID").unwrap();
        let owner = match lease(&dir) {
            Ok(owner) => owner,
            Err(_) => std::process::exit(23),
        };
        if id == "first" {
            fs::write(dir.join("publisher-ready"), "ready").unwrap();
            while !dir.join("publisher-release").exists() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let mut marker = load(&dir).unwrap();
        let mut w = withdrawal(&id);
        w.user = id.clone();
        add_withdrawal(&mut marker, "g", &id, w).unwrap();
        publish_owned(&owner, &dir, marker.revision, marker).unwrap();
    }
    #[test]
    fn concurrent_process_publishers_reject_then_preserve_both() {
        let dir = directory();
        let command = |id: &str| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "persist::personal_memory::tests::publisher_child_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("ABBEY_CONSENT_TEST_CHILD_DIR", &dir)
                .env("ABBEY_CONSENT_TEST_WRITER_ID", id)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            command
        };
        let mut first = ChildOwner(command("first").spawn().unwrap());
        for _ in 0..2000 {
            if dir.join("publisher-ready").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(dir.join("publisher-ready").exists());
        assert_eq!(command("second").status().unwrap().code(), Some(23));
        fs::write(dir.join("publisher-release"), "release").unwrap();
        assert!(first.0.wait().unwrap().success());
        assert!(command("second").status().unwrap().success());
        assert_eq!(load(&dir).unwrap().withdrawals.len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
