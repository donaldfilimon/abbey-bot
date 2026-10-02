//! Independent operational receipts; never rewrites conversational state.
use super::owned_file::OwnedFile;
use crate::community_ops::{Ledger, Policy};
use std::{
    fs::{self},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 8 * 1024 * 1024;

fn private_file(path: &Path) -> Result<(), &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| "operations file unavailable")?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("unsafe operations file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 || meta.uid() != rustix::process::geteuid().as_raw() {
            return Err("operations file must be owner-only");
        }
    }
    Ok(())
}

pub fn load_policy(path: &Path) -> Result<(Policy, String), &'static str> {
    if !path.is_absolute() {
        return Err("policy path must be absolute");
    }
    private_file(path)?;
    let bytes = fs::read(path).map_err(|_| "policy read failed")?;
    let policy: Policy = serde_json::from_slice(&bytes).map_err(|_| "invalid operations policy")?;
    policy.validate()?;
    use sha2::{Digest, Sha256};
    Ok((
        policy,
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    ))
}

/// Change only execution mode after the caller verifies the live Discord owner.
/// A stale dashboard cannot overwrite a newer owner-authored policy.
pub fn set_mode(
    path: &Path,
    expected_digest: &str,
    guild: u64,
    owner: u64,
    mode: crate::community_ops::Mode,
) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("policy parent missing")?;
    // Policy publication has its own lock: Stop must remain available while
    // the execution lease is held by a running Discord operation.
    let lock = path.with_extension("mode-lock");
    let (file, control) = OwnedFile::create(&lock).map_err(|_| "policy control lease held")?;
    control
        .sync_all(&file)
        .map_err(|_| "policy control lease sync failed")?;
    let (mut policy, digest) = load_policy(path)?;
    if digest != expected_digest || policy.guild != guild || policy.owner != owner {
        return Err("policy changed or owner scope mismatched; refresh dashboard");
    }
    policy.mode = mode;
    policy.validate()?;
    let bytes = serde_json::to_vec_pretty(&policy).map_err(|_| "policy encode failed")?;
    let temporary = path.with_extension("mode-pending");
    let (mut file, mut unpublished) =
        OwnedFile::create(&temporary).map_err(|_| "mode publication requires review")?;
    unpublished
        .write_all(&mut file, &bytes)
        .map_err(|_| "policy write failed")?;
    unpublished
        .sync_all(&file)
        .map_err(|_| "policy sync failed")?;
    if load_policy(path)?.1 != expected_digest {
        return Err("policy changed during publication; owner review required");
    }
    fs::rename(&temporary, path).map_err(|_| "policy publication failed")?;
    unpublished.published();
    OwnedFile::sync_directory(parent).map_err(|_| "policy directory sync failed")?;
    let (observed, _) = load_policy(path)?;
    if observed != policy {
        return Err("policy readback mismatch");
    }
    Ok(())
}

pub struct Lease {
    _lock: OwnedFile,
    pub ledger: PathBuf,
}
impl Lease {
    pub fn acquire(data: &Path) -> Result<Self, &'static str> {
        let dir = data.join("community-operations");
        fs::create_dir_all(&dir).map_err(|_| "operations directory unavailable")?;
        if !fs::symlink_metadata(&dir)
            .map_err(|_| "operations directory unavailable")?
            .is_dir()
        {
            return Err("unsafe operations directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .map_err(|_| "operations permissions failed")?;
        }
        let lock = dir.join("execution.lock");
        let (mut file, ownership) = OwnedFile::create(&lock)
            .map_err(|_| "operations lease held; owner review required after crash")?;
        ownership
            .write_all(&mut file, std::process::id().to_string().as_bytes())
            .map_err(|_| "lease write failed")?;
        ownership.sync_all(&file).map_err(|_| "lease sync failed")?;
        Ok(Self {
            _lock: ownership,
            ledger: dir.join("receipts.json"),
        })
    }
    pub fn load(&self) -> Result<Ledger, &'static str> {
        match fs::symlink_metadata(&self.ledger) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            _ => {
                private_file(&self.ledger)?;
                serde_json::from_slice(&fs::read(&self.ledger).map_err(|_| "ledger read failed")?)
                    .map_err(|_| "invalid operations ledger")
            }
        }
    }
    pub fn save(&self, ledger: &Ledger) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec_pretty(ledger).map_err(|_| "ledger encode failed")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("operations ledger full; owner review required");
        }
        let temporary = self.ledger.with_extension("pending");
        let (mut file, mut unpublished) =
            OwnedFile::create(&temporary).map_err(|_| "ledger publication requires review")?;
        unpublished
            .write_all(&mut file, &bytes)
            .map_err(|_| "ledger write failed")?;
        unpublished
            .sync_all(&file)
            .map_err(|_| "ledger sync failed")?;
        fs::rename(&temporary, &self.ledger).map_err(|_| "ledger publication failed")?;
        unpublished.published();
        #[cfg(unix)]
        {
            OwnedFile::sync_directory(self.ledger.parent().ok_or("ledger parent missing")?)
                .map_err(|_| "ledger directory sync failed")?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_lock_and_publication_failures_cleanup_only_owned_paths() {
        use crate::persist::owned_file::{Fault, with_fault};
        let dir = std::env::temp_dir().join(format!("abbey-ops-io-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        let (policy, _, _, _) = crate::community_ops::proposals::tests::fixture();
        fs::write(&path, serde_json::to_vec(&policy).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let (_, digest) = load_policy(&path).unwrap();
        let control = path.with_extension("mode-lock");
        assert_eq!(
            with_fault(&control, Fault::Sync, || set_mode(
                &path,
                &digest,
                policy.guild,
                policy.owner,
                crate::community_ops::Mode::Stopped
            )),
            Err("policy control lease sync failed")
        );
        assert!(!control.exists());
        let execution = dir.join("community-operations/execution.lock");
        for (phase, expected) in [
            (Fault::Write, "lease write failed"),
            (Fault::Sync, "lease sync failed"),
        ] {
            assert_eq!(
                with_fault(&execution, phase, || Lease::acquire(&dir)).err(),
                Some(expected)
            );
            assert!(!execution.exists());
        }
        let mode_pending = path.with_extension("mode-pending");
        for (phase, expected) in [
            (Fault::Write, "policy write failed"),
            (Fault::Sync, "policy sync failed"),
        ] {
            assert_eq!(
                with_fault(&mode_pending, phase, || set_mode(
                    &path,
                    &digest,
                    policy.guild,
                    policy.owner,
                    crate::community_ops::Mode::Stopped
                )),
                Err(expected)
            );
            assert!(!mode_pending.exists());
            assert_eq!(load_policy(&path).unwrap().1, digest);
        }
        let lease = Lease::acquire(&dir).unwrap();
        let pending = lease.ledger.with_extension("pending");
        for (phase, expected) in [
            (Fault::Write, "ledger write failed"),
            (Fault::Sync, "ledger sync failed"),
        ] {
            assert_eq!(
                with_fault(&pending, phase, || lease.save(&Ledger::default())),
                Err(expected)
            );
            assert!(!pending.exists());
            assert!(!lease.ledger.exists());
        }
        fs::create_dir(&lease.ledger).unwrap();
        assert_eq!(
            lease.save(&Ledger::default()),
            Err("ledger publication failed")
        );
        assert!(!pending.exists());
        fs::remove_dir(&lease.ledger).unwrap();
        #[cfg(unix)]
        {
            let parent = lease.ledger.parent().unwrap();
            assert_eq!(
                with_fault(parent, Fault::DirectorySync, || lease
                    .save(&Ledger::default())),
                Err("ledger directory sync failed")
            );
            assert!(lease.ledger.exists());
            assert!(!pending.exists());
        }
        assert_eq!(
            with_fault(&dir, Fault::DirectorySync, || set_mode(
                &path,
                &digest,
                policy.guild,
                policy.owner,
                crate::community_ops::Mode::Stopped
            )),
            Err("policy directory sync failed")
        );
        assert_eq!(
            load_policy(&path).unwrap().0.mode,
            crate::community_ops::Mode::Stopped
        );
        assert!(!mode_pending.exists());
        fs::write(&pending, b"recovery evidence").unwrap();
        assert!(lease.save(&Ledger::default()).is_err());
        assert_eq!(fs::read(&pending).unwrap(), b"recovery evidence");
        fs::write(&mode_pending, b"policy recovery evidence").unwrap();
        assert!(
            set_mode(
                &path,
                &digest,
                policy.guild,
                policy.owner,
                crate::community_ops::Mode::Stopped
            )
            .is_err()
        );
        assert_eq!(
            fs::read(&mode_pending).unwrap(),
            b"policy recovery evidence"
        );
        drop(lease);
        fs::write(&execution, b"crash evidence").unwrap();
        assert!(Lease::acquire(&dir).is_err());
        assert_eq!(fs::read(&execution).unwrap(), b"crash evidence");
        fs::write(&control, b"control crash evidence").unwrap();
        assert!(
            set_mode(
                &path,
                &digest,
                policy.guild,
                policy.owner,
                crate::community_ops::Mode::Stopped
            )
            .is_err()
        );
        assert_eq!(fs::read(&control).unwrap(), b"control crash evidence");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reservation_survives_restart_and_lease_is_exclusive() {
        let dir = std::env::temp_dir().join(format!("abbey-ops-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let lease = Lease::acquire(&dir).unwrap();
        assert!(Lease::acquire(&dir).is_err());
        let ledger = Ledger {
            last_assessment: Some(42),
            ..Ledger::default()
        };
        lease.save(&ledger).unwrap();
        drop(lease);
        let lease = Lease::acquire(&dir).unwrap();
        assert_eq!(lease.load().unwrap().last_assessment, Some(42));
        drop(lease);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_policy_never_creates_a_lease() {
        assert!(load_policy(Path::new("relative.json")).is_err());
    }

    #[test]
    fn mode_change_rejects_stale_scope_and_preserves_policy() {
        let dir = std::env::temp_dir().join(format!("abbey-policy-mode-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        let path = dir.join("policy.json");
        let raw = serde_json::json!({"version":1,"guild":1,"owner":2,"mode":"propose",
            "daily_limit":5,"daily_creations":2,"public_categories":[],"protected_channels":[],"actions":[]});
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let (before, digest) = load_policy(&path).unwrap();
        let _execution = Lease::acquire(&dir).unwrap();
        for (expected, guild, owner) in [
            ("stale", 1, 2),
            (digest.as_str(), 3, 2),
            (digest.as_str(), 1, 4),
        ] {
            assert!(
                set_mode(
                    &path,
                    expected,
                    guild,
                    owner,
                    crate::community_ops::Mode::Apply
                )
                .is_err()
            );
            assert_eq!(load_policy(&path).unwrap().0, before);
        }
        set_mode(&path, &digest, 1, 2, crate::community_ops::Mode::Stopped).unwrap();
        let mut expected = before;
        expected.mode = crate::community_ops::Mode::Stopped;
        assert_eq!(load_policy(&path).unwrap().0, expected);
        fs::remove_dir_all(dir).unwrap();
    }
}
