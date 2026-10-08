//! Independent owner-only contextual records, separate from learning and facts.
//! Call through retained CommunityFilesystem; waiter cancellation cannot drop a writer.
use super::{community_ops::load_policy, owned_file::OwnedFile};
use crate::community_ops::Mode;
use crate::moderation::shadow::{CaseStore, Change, Mutation, ShadowPolicy};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 8 * 1024 * 1024;

fn private_file(path: &Path) -> Result<(), &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| "contextual file unavailable")?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("unsafe contextual file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 || meta.uid() != rustix::process::geteuid().as_raw() {
            return Err("contextual file must be owner-only");
        }
    }
    Ok(())
}
fn directory(data: &Path) -> Result<PathBuf, &'static str> {
    if !data.is_absolute() {
        return Err("contextual data path must be absolute");
    }
    let data_meta =
        fs::symlink_metadata(data).map_err(|_| "contextual data directory unavailable")?;
    if !data_meta.is_dir() {
        return Err("unsafe contextual data directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if data_meta.uid() != rustix::process::geteuid().as_raw() || data_meta.mode() & 0o077 != 0 {
            return Err("contextual data directory must be owner-only");
        }
    }
    let dir = data.join("community-operations");
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err("unsafe contextual directory"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&dir).map_err(|_| "contextual directory creation failed")?;
        }
        Err(_) => return Err("contextual directory unavailable"),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = fs::symlink_metadata(&dir).map_err(|_| "contextual directory unavailable")?;
        if meta.uid() != rustix::process::geteuid().as_raw() {
            return Err("contextual directory owner mismatch");
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "contextual directory privacy failed")?;
    }
    Ok(dir)
}
fn lease(path: &Path) -> Result<OwnedFile, &'static str> {
    let (file, guard) = OwnedFile::create(path)
        .map_err(|_| "contextual lease held; owner review required after crash")?;
    guard
        .sync_all(&file)
        .map_err(|_| "contextual lease sync failed")?;
    Ok(guard)
}
fn load_at(path: &Path) -> Result<CaseStore, &'static str> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CaseStore::default()),
        Err(_) => return Err("contextual store unavailable"),
        Ok(_) => {}
    }
    private_file(path)?;
    let bytes = fs::read(path).map_err(|_| "contextual store read failed")?;
    let store: CaseStore =
        serde_json::from_slice(&bytes).map_err(|_| "invalid contextual store")?;
    store.validate()?;
    Ok(store)
}
fn publish(
    path: &Path,
    store: &CaseStore,
    before_rename: impl FnOnce() -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    store.validate()?;
    let bytes = serde_json::to_vec_pretty(store).map_err(|_| "contextual store encode failed")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("contextual publication exceeds bound");
    }
    let pending = path.with_extension("shadow-pending");
    let (mut file, mut owned) =
        OwnedFile::create(&pending).map_err(|_| "contextual pending file requires owner review")?;
    owned
        .write_all(&mut file, &bytes)
        .map_err(|_| "contextual write failed")?;
    owned
        .sync_all(&file)
        .map_err(|_| "contextual sync failed")?;
    before_rename()?;
    fs::rename(&pending, path).map_err(|_| "contextual publication failed")?;
    owned.published();
    #[cfg(unix)]
    OwnedFile::sync_directory(path.parent().ok_or("contextual parent unavailable")?)
        .map_err(|_| "contextual directory sync failed")?;
    private_file(path)?;
    if OwnedFile::read_back(path).map_err(|_| "contextual readback failed")? != bytes
        || load_at(path)? != *store
    {
        return Err("contextual readback mismatch");
    }
    Ok(())
}

#[cfg(test)]
pub fn load(data: &Path) -> Result<CaseStore, &'static str> {
    load_at(&directory(data)?.join("contextual-shadow.json"))
}

/// Exact-existing preflight performs no creation, chmod, lease or publication.
pub fn load_existing(data: &Path) -> Result<Option<CaseStore>, &'static str> {
    if !data.is_absolute() {
        return Err("contextual data path must be absolute");
    }
    for path in [data.to_owned(), data.join("community-operations")] {
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path != data => return Ok(None),
            Err(_) => return Err("contextual directory unavailable"),
        };
        if !meta.is_dir() {
            return Err("unsafe contextual directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
                return Err("contextual directory must be owner-only");
            }
        }
    }
    let path = data.join("community-operations/contextual-shadow.json");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("contextual store unavailable"),
        Ok(_) => load_at(&path).map(Some),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationReceipt {
    pub case_id: String,
    pub case_revision: u64,
    pub store_revision: u64,
    pub change: Change,
}

/// No native network await occurs under either short lock. The shared mode
/// lock serializes the owner's Stop/control writer with this publication;
/// fresh policy is loaded inside it. Native proofs are checked against a fresh
/// infrastructure clock after both leases, rather than the queued request time.
/// External policy replacement must remain operator-coordinated, as elsewhere.
pub fn transact(
    data: &Path,
    policy_path: &Path,
    expected_policy_digest: &str,
    mutation: Mutation,
    clock: impl Fn() -> u64,
) -> Result<PublicationReceipt, &'static str> {
    if !policy_path.is_absolute() {
        return Err("contextual policy path must be absolute");
    }
    let _policy_control = lease(&policy_path.with_extension("mode-lock"))?;
    let (policy, digest) = load_policy(policy_path)?;
    if digest != expected_policy_digest {
        return Err("contextual policy changed; refresh native evidence");
    }
    let shadow = ShadowPolicy {
        guild: policy.guild,
        owner: policy.owner,
        stopped: policy.mode == Mode::Stopped,
        scope: policy.contextual_shadow,
    };
    mutation.authorize_snapshot(&shadow, clock())?;
    // Refuse stale/disabled new work before creating any operational paths.
    // This preliminary candidate is never published; repeat the full CAS from
    // the current store after acquiring the publication lease.
    let mut preliminary = load_existing(data)?.unwrap_or_default();
    preliminary.apply(mutation.clone(), &shadow, &digest, clock())?;
    let dir = directory(data)?;
    let _shadow_control = lease(&dir.join("shadow.lock"))?;
    if load_policy(policy_path)?.1 != digest {
        return Err("contextual policy changed before private snapshot");
    }
    mutation.authorize_snapshot(&shadow, clock())?;
    let path = dir.join("contextual-shadow.json");
    let mut store = load_at(&path)?;
    let last_proof = mutation.clone();
    let (case_id, change) = store.apply(mutation, &shadow, &digest, clock())?;
    if load_policy(policy_path)?.1 != digest {
        return Err("contextual policy changed before publication");
    }
    // Even AlreadyObserved is republished/read back: a previous rename may
    // have happened before directory-sync/readback failed. An explicit retry
    // re-establishes that durability evidence without adding an event/count.
    publish(&path, &store, || {
        if load_policy(policy_path)?.1 != digest {
            return Err("contextual policy changed during preparation");
        }
        // Validate the exact retained native proof again after disk preparation.
        // This copy observes an idempotent event and cannot add a second receipt.
        let mut candidate = store.clone();
        candidate.apply(last_proof, &shadow, &digest, clock())?;
        Ok(())
    })?;
    let observed = load_at(&path)?;
    let case_revision = observed
        .cases
        .get(&case_id)
        .ok_or("contextual case readback unavailable")?
        .revision;
    Ok(PublicationReceipt {
        case_id,
        case_revision,
        store_revision: observed.revision,
        change,
    })
}

#[cfg(test)]
pub(crate) mod tests;
