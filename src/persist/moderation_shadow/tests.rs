use super::*;
use crate::moderation::shadow::tests::{authority, capture, policy, source};
use crate::moderation::shadow::{AppealReason, ExpectedCase, ReviewDecision};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
#[test]
fn unsafe_parent_refusal_precedes_operational_directory_creation() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    fs::set_permissions(&f.data, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(directory(&f.data).is_err());
    assert!(
        !f.data.join("community-operations").exists(),
        "refused parent must not receive a newly created operational directory"
    );
}

pub(crate) struct Fixture {
    pub(crate) data: PathBuf,
    pub(crate) policy: PathBuf,
    pub(crate) digest: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.data);
    }
}
pub(crate) fn fixture() -> Fixture {
    let data = std::env::temp_dir().join(format!(
        "abbey-shadow-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&data).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path = data.join("owner-policy.json");
    let value = serde_json::json!({
        "version": 1, "guild": 1, "owner": 2, "mode": "propose",
        "daily_limit": 5, "daily_creations": 2,
        "public_categories": [9], "protected_channels": [], "actions": [],
        "contextual_shadow": {"enabled": true, "source_channels": [7], "review_channel": null}
    });
    let (mut file, mut owned) = OwnedFile::create(&path).unwrap();
    owned
        .write_all(&mut file, &serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
    owned.sync_all(&file).unwrap();
    owned.published();
    let digest = load_policy(&path).unwrap().1;
    Fixture {
        data,
        policy: path,
        digest,
    }
}
fn write(f: &Fixture, mutation: Mutation, now: u64) -> Result<PublicationReceipt, &'static str> {
    transact(&f.data, &f.policy, &f.digest, mutation, || now)
}
#[test]
fn publication_is_independent_content_free_and_exactly_read_back() {
    let f = fixture();
    let before_policy = fs::read(&f.policy).unwrap();
    let canonical = f.data.join(crate::persist::STATE_FILE);
    let projection = f.data.join(crate::persist::WDBX_FILE);
    fs::write(&canonical, b"canonical sentinel").unwrap();
    fs::write(&projection, b"projection sentinel").unwrap();
    let first = write(&f, capture(100), 100).unwrap();
    assert_eq!(first.change, Change::Changed);
    assert_eq!((first.case_revision, first.store_revision), (1, 1));
    let bytes = fs::read(f.data.join("community-operations/contextual-shadow.json")).unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("synthetic secret source text")
    );
    assert_eq!(load(&f.data).unwrap().cases[&first.case_id].revision, 1);
    let duplicate = write(&f, capture(101), 101).unwrap();
    assert_eq!(duplicate.change, Change::AlreadyObserved);
    assert_eq!((duplicate.case_revision, duplicate.store_revision), (1, 1));
    assert_eq!(
        fs::read(f.data.join("community-operations/contextual-shadow.json")).unwrap(),
        bytes
    );
    assert_eq!(fs::read(&f.policy).unwrap(), before_policy);
    assert_eq!(fs::read(canonical).unwrap(), b"canonical sentinel");
    assert_eq!(fs::read(projection).unwrap(), b"projection sentinel");
    assert!(!f.data.join("community-operations/shadow.lock").exists());
    assert!(!f.policy.with_extension("mode-lock").exists());
}
#[test]
fn wrong_policy_expired_proof_and_stopped_new_capture_do_not_publish() {
    let f = fixture();
    assert!(transact(&f.data, &f.policy, "wrong", capture(100), || 100).is_err());
    assert!(write(&f, capture(100), 161).is_err());
    crate::persist::community_ops::set_mode(&f.policy, &f.digest, 1, 2, Mode::Stopped).unwrap();
    let changed_digest = load_policy(&f.policy).unwrap().1;
    assert!(transact(&f.data, &f.policy, &changed_digest, capture(100), || 100).is_err());
    assert!(load(&f.data).unwrap().cases.is_empty());
}
#[test]
fn stopped_policy_preserves_existing_subject_appeal() {
    let f = fixture();
    let created = write(&f, capture(100), 100).unwrap();
    crate::persist::community_ops::set_mode(&f.policy, &f.digest, 1, 2, Mode::Stopped).unwrap();
    let digest = load_policy(&f.policy).unwrap().1;
    let mutation = Mutation::Appeal {
        expected: ExpectedCase {
            id: created.case_id.clone(),
            revision: 1,
        },
        authority: authority(4, 101, None, false),
        reason: AppealReason::AssessmentDisputed,
    };
    let observed = transact(&f.data, &f.policy, &digest, mutation, || 101).unwrap();
    assert_eq!(observed.case_revision, 2);
    assert_eq!(
        load(&f.data).unwrap().cases[&created.case_id]
            .appeal
            .as_ref()
            .unwrap()
            .provenance
            .actor,
        4
    );
}
#[test]
fn stale_revision_and_conflicting_receipt_preserve_previous_bytes() {
    let f = fixture();
    let created = write(&f, capture(100), 100).unwrap();
    let review = |revision, actor, decision| Mutation::Review {
        expected: ExpectedCase {
            id: created.case_id.clone(),
            revision,
        },
        authority: authority(actor, 101, Some(source()), true),
        decision,
    };
    let before = fs::read(f.data.join("community-operations/contextual-shadow.json")).unwrap();
    assert!(write(&f, review(2, 5, ReviewDecision::Agree), 101).is_err());
    assert_eq!(
        fs::read(f.data.join("community-operations/contextual-shadow.json")).unwrap(),
        before
    );
    write(&f, review(1, 5, ReviewDecision::Agree), 101).unwrap();
    let after = fs::read(f.data.join("community-operations/contextual-shadow.json")).unwrap();
    assert!(write(&f, review(1, 6, ReviewDecision::Disagree), 102).is_err());
    assert_eq!(
        fs::read(f.data.join("community-operations/contextual-shadow.json")).unwrap(),
        after
    );
}
#[test]
fn held_control_leases_refuse_and_never_unlink_other_owner() {
    let f = fixture();
    let dir = directory(&f.data).unwrap();
    for path in [
        dir.join("shadow.lock"),
        f.policy.with_extension("mode-lock"),
    ] {
        let (_file, guard) = OwnedFile::create(&path).unwrap();
        assert!(write(&f, capture(100), 100).is_err());
        assert!(path.exists());
        drop(guard);
        assert!(!path.exists());
        assert!(load(&f.data).unwrap().cases.is_empty());
    }
}
#[test]
fn write_and_file_sync_failure_preserve_previous_state_and_release_own_paths() {
    use crate::persist::owned_file::{Fault, with_fault};
    for phase in [Fault::Write, Fault::Sync] {
        let f = fixture();
        let dir = directory(&f.data).unwrap();
        let pending = dir
            .join("contextual-shadow.json")
            .with_extension("shadow-pending");
        assert!(with_fault(&pending, phase, || write(&f, capture(100), 100)).is_err());
        assert!(load(&f.data).unwrap().cases.is_empty());
        assert!(!pending.exists());
        assert!(!dir.join("shadow.lock").exists());
        assert!(!f.policy.with_extension("mode-lock").exists());
        write(&f, capture(101), 101).unwrap();
    }
}
#[cfg(unix)]
#[test]
fn postrename_directory_sync_failure_is_incomplete_then_explicit_retry_reconciles() {
    use crate::persist::owned_file::{Fault, with_fault};
    let f = fixture();
    let dir = directory(&f.data).unwrap();
    assert!(with_fault(&dir, Fault::DirectorySync, || write(&f, capture(100), 100)).is_err());
    let incomplete = load(&f.data).unwrap();
    assert_eq!((incomplete.revision, incomplete.cases.len()), (1, 1));
    let retry = write(&f, capture(101), 101).unwrap();
    assert_eq!(retry.change, Change::AlreadyObserved);
    assert_eq!(retry.store_revision, 1);
    assert_eq!(load(&f.data).unwrap(), incomplete);
}
#[test]
fn proof_expiring_during_preparation_never_crosses_rename() {
    let f = fixture();
    let pending = f
        .data
        .join("community-operations/contextual-shadow.shadow-pending");
    let saw_prepared_pending = std::cell::Cell::new(false);
    let result = transact(&f.data, &f.policy, &f.digest, capture(100), || {
        if pending.exists() {
            saw_prepared_pending.set(true);
            161
        } else {
            100
        }
    });
    assert!(result.is_err());
    assert!(saw_prepared_pending.get());
    assert!(load(&f.data).unwrap().cases.is_empty());
    assert!(!pending.exists());
}

#[test]
fn malformed_oversized_and_nonfile_stores_fail_closed() {
    let f = fixture();
    let path = directory(&f.data).unwrap().join("contextual-shadow.json");
    fs::create_dir(&path).unwrap();
    assert!(load(&f.data).is_err());
    fs::remove_dir(&path).unwrap();
    let (mut file, mut owned) = OwnedFile::create(&path).unwrap();
    owned
        .write_all(&mut file, b"{\"version\":99,\"revision\":0,\"cases\":{}}")
        .unwrap();
    owned.published();
    assert!(load(&f.data).is_err());
    file.set_len(MAX_BYTES + 1).unwrap();
    assert!(load(&f.data).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_and_nonprivate_store_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = fixture();
    let path = directory(&f.data).unwrap().join("contextual-shadow.json");
    symlink(&f.policy, &path).unwrap();
    assert!(load(&f.data).is_err());
    fs::remove_file(&path).unwrap();
    write(&f, capture(100), 100).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load(&f.data).is_err());
}
#[test]
fn stored_receipt_cannot_substitute_for_fresh_native_subject_access() {
    let f = fixture();
    let created = write(&f, capture(100), 100).unwrap();
    let loaded = load(&f.data).unwrap();
    assert!(
        loaded
            .inspect(
                &created.case_id,
                &authority(9, 101, None, false),
                &policy(),
                101
            )
            .is_err()
    );
}

mod additional_tests;

mod admission_order_tests;
