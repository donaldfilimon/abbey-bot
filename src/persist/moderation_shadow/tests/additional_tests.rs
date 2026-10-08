//! Intended as persist::moderation_shadow::tests::additional_tests.
use super::*;

fn ledger(f: &Fixture) -> PathBuf {
    f.data.join("community-operations/contextual-shadow.json")
}

fn assert_owner_paths_released(f: &Fixture) {
    assert!(!ledger(f).with_extension("shadow-pending").exists());
    assert!(!f.data.join("community-operations/shadow.lock").exists());
    assert!(!f.policy.with_extension("mode-lock").exists());
}

#[test]
fn actual_rename_failure_preserves_old_bytes_and_allows_explicit_retry() {
    let f = fixture();
    let created = write(&f, capture(100), 100).unwrap();
    let before = fs::read(ledger(&f)).unwrap();
    let before_store = load_existing(&f.data).unwrap().unwrap();
    let policy_before = fs::read(&f.policy).unwrap();
    let canonical = f.data.join(crate::persist::STATE_FILE);
    let projection = f.data.join(crate::persist::WDBX_FILE);
    fs::write(&canonical, b"canonical sentinel").unwrap();
    fs::write(&projection, b"projection sentinel").unwrap();
    let mutation = Mutation::Review {
        expected: ExpectedCase {
            id: created.case_id,
            revision: 1,
        },
        authority: authority(5, 101, Some(source()), true),
        decision: ReviewDecision::Agree,
    };
    let removed_own_pending = std::cell::Cell::new(false);
    let result = transact(&f.data, &f.policy, &f.digest, mutation.clone(), || {
        let pending = ledger(&f).with_extension("shadow-pending");
        if pending.exists() {
            // This is our exclusive, synced pending file. Removing it at the
            // existing final-clock seam causes the real fs::rename to fail.
            // Earlier authorization clocks may run before the file exists.
            assert!(!removed_own_pending.replace(true));
            assert!(pending.is_file());
            fs::remove_file(pending).unwrap();
        }
        101
    });
    assert_eq!(result, Err("contextual publication failed"));
    assert!(removed_own_pending.get());
    assert_eq!(fs::read(ledger(&f)).unwrap(), before);
    assert_eq!(load_existing(&f.data).unwrap().unwrap(), before_store);
    assert_eq!(fs::read(&f.policy).unwrap(), policy_before);
    assert_eq!(fs::read(canonical).unwrap(), b"canonical sentinel");
    assert_eq!(fs::read(projection).unwrap(), b"projection sentinel");
    assert_owner_paths_released(&f);
    let retry = write(&f, mutation, 102).unwrap();
    assert_eq!(retry.change, Change::Changed);
    assert_eq!((retry.case_revision, retry.store_revision), (2, 2));
    assert_owner_paths_released(&f);
}

#[test]
fn postrename_readback_failure_is_incomplete_then_stopped_explicit_retry_reconciles() {
    use crate::persist::owned_file::{Fault, with_fault};
    let f = fixture();
    let path = ledger(&f);
    let result = with_fault(&path, Fault::Readback, || write(&f, capture(100), 100));
    assert_eq!(result, Err("contextual readback failed"));
    // A failed acknowledgment is not rollback: the real rename already ran.
    let incomplete_bytes = fs::read(&path).unwrap();
    let incomplete = load_existing(&f.data).unwrap().unwrap();
    assert_eq!((incomplete.revision, incomplete.cases.len()), (1, 1));
    assert_owner_paths_released(&f);
    crate::persist::community_ops::set_mode(&f.policy, &f.digest, 1, 2, Mode::Stopped).unwrap();
    let digest = load_policy(&f.policy).unwrap().1;
    let retry = transact(&f.data, &f.policy, &digest, capture(101), || 101).unwrap();
    assert_eq!(retry.change, Change::AlreadyObserved);
    assert_eq!((retry.case_revision, retry.store_revision), (1, 1));
    assert_eq!(fs::read(path).unwrap(), incomplete_bytes);
    assert_eq!(load_existing(&f.data).unwrap().unwrap(), incomplete);
    assert_owner_paths_released(&f);
}

#[test]
fn existing_lookup_absence_does_not_create_directories_leases_or_state() {
    let f = fixture();
    let policy_before = fs::read(&f.policy).unwrap();
    let dir = f.data.join("community-operations");
    assert!(!dir.exists());
    assert_eq!(load_existing(&f.data).unwrap(), None);
    assert!(!dir.exists());
    assert!(!f.data.join(crate::persist::STATE_FILE).exists());
    assert!(!f.data.join(crate::persist::WDBX_FILE).exists());
    assert!(!f.policy.with_extension("mode-lock").exists());
    assert_eq!(fs::read(&f.policy).unwrap(), policy_before);
    fs::create_dir(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert_eq!(load_existing(&f.data).unwrap(), None);
    assert!(!ledger(&f).exists());
    assert_owner_paths_released(&f);
}

#[test]
fn existing_lookup_returns_exact_store_without_acquiring_other_owners_leases() {
    let f = fixture();
    write(&f, capture(100), 100).unwrap();
    let path = ledger(&f);
    let before = fs::read(&path).unwrap();
    let store = load_existing(&f.data).unwrap().unwrap();
    let policy_before = fs::read(&f.policy).unwrap();
    let shadow_lock = f.data.join("community-operations/shadow.lock");
    let mode_lock = f.policy.with_extension("mode-lock");
    let (_shadow_file, shadow_owner) = OwnedFile::create(&shadow_lock).unwrap();
    let (_mode_file, mode_owner) = OwnedFile::create(&mode_lock).unwrap();
    assert_eq!(load_existing(&f.data).unwrap(), Some(store));
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read(&f.policy).unwrap(), policy_before);
    assert!(shadow_lock.exists());
    assert!(mode_lock.exists());
    drop((shadow_owner, mode_owner));
    assert_owner_paths_released(&f);
}

#[test]
fn existing_lookup_refuses_missing_relative_and_non_directory_data() {
    let f = fixture();
    assert!(load_existing(Path::new("relative-shadow-data")).is_err());
    let missing = f.data.join("missing-data");
    assert!(load_existing(&missing).is_err());
    assert!(!missing.exists());
    assert!(load_existing(&f.policy).is_err());
    let dir = f.data.join("community-operations");
    fs::write(&dir, b"not a directory").unwrap();
    assert!(load_existing(&f.data).is_err());
    assert_eq!(fs::read(dir).unwrap(), b"not a directory");
}

#[test]
fn existing_lookup_malformed_oversized_and_nonfile_ledger_are_not_absence() {
    let f = fixture();
    let path = directory(&f.data).unwrap().join("contextual-shadow.json");
    fs::create_dir(&path).unwrap();
    assert!(load_existing(&f.data).is_err());
    assert!(path.is_dir());
    fs::remove_dir(&path).unwrap();
    for value in [
        b"{".as_slice(),
        b"{\"version\":99,\"revision\":0,\"cases\":{}}".as_slice(),
        b"{\"version\":1,\"revision\":1,\"cases\":{}}".as_slice(),
    ] {
        let (mut file, mut owner) = OwnedFile::create(&path).unwrap();
        owner.write_all(&mut file, value).unwrap();
        owner.published();
        assert!(load_existing(&f.data).is_err());
        assert_eq!(fs::read(&path).unwrap(), value);
        drop(file);
        fs::remove_file(&path).unwrap();
    }
    let (file, mut owner) = OwnedFile::create(&path).unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    owner.published();
    assert!(load_existing(&f.data).is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), MAX_BYTES + 1);
    assert_owner_paths_released(&f);
}

#[cfg(unix)]
#[test]
fn existing_lookup_never_chmods_private_or_refused_paths() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = fixture();
    let mode = |path: &Path| fs::symlink_metadata(path).unwrap().mode() & 0o777;
    fs::set_permissions(&f.data, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(load_existing(&f.data).is_err());
    assert_eq!(mode(&f.data), 0o755);
    assert!(!f.data.join("community-operations").exists());
    fs::set_permissions(&f.data, fs::Permissions::from_mode(0o700)).unwrap();
    let dir = f.data.join("community-operations");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(load_existing(&f.data).is_err());
    assert_eq!(mode(&dir), 0o755);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    write(&f, capture(100), 100).unwrap();
    let path = ledger(&f);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_existing(&f.data).is_err());
    assert_eq!(mode(&path), 0o644);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let before = [mode(&f.data), mode(&dir), mode(&path), mode(&f.policy)];
    assert!(load_existing(&f.data).unwrap().is_some());
    assert_eq!(
        [mode(&f.data), mode(&dir), mode(&path), mode(&f.policy)],
        before
    );
    assert_owner_paths_released(&f);
}

#[cfg(unix)]
#[test]
fn existing_lookup_refuses_symlink_data_directory_and_ledger_without_touching_targets() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let f = fixture();
    let foreign = fixture();
    write(&foreign, capture(100), 100).unwrap();
    let foreign_dir = foreign.data.join("community-operations");
    let foreign_path = ledger(&foreign);
    let before = fs::read(&foreign_path).unwrap();
    let before_modes = [
        fs::metadata(&foreign.data).unwrap().mode(),
        fs::metadata(&foreign_dir).unwrap().mode(),
        fs::metadata(&foreign_path).unwrap().mode(),
    ];
    let alias = f.data.join("data-alias");
    symlink(&foreign.data, &alias).unwrap();
    assert!(load_existing(&alias).is_err());
    fs::remove_file(alias).unwrap();
    let dir = f.data.join("community-operations");
    symlink(&foreign_dir, &dir).unwrap();
    assert!(load_existing(&f.data).is_err());
    fs::remove_file(&dir).unwrap();
    directory(&f.data).unwrap();
    symlink(&foreign_path, ledger(&f)).unwrap();
    assert!(load_existing(&f.data).is_err());
    assert_eq!(fs::read(&foreign_path).unwrap(), before);
    assert_eq!(
        [
            fs::metadata(&foreign.data).unwrap().mode(),
            fs::metadata(&foreign_dir).unwrap().mode(),
            fs::metadata(&foreign_path).unwrap().mode(),
        ],
        before_modes
    );
    assert_owner_paths_released(&f);
}
