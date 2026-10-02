use super::*;
use crate::community_ops::proposals::tests::{fixture, topic};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    data: PathBuf,
    policy: PathBuf,
    scope: AssessmentScope,
    proposal: PendingProposal,
    proofs: PublicProofs,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.data);
    }
}
fn setup() -> Fixture {
    let data = std::env::temp_dir().join(format!(
        "abbey-proposals-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&data).unwrap();
    let path = data.join("owner-policy.json");
    let (policy, scope, mut source, proofs) = fixture();
    publish_bytes(&path, &serde_json::to_vec_pretty(&policy).unwrap()).unwrap();
    let digest = load_policy(&path).unwrap().1;
    source.policy_digest = digest.clone();
    let reservation = begin_assessment(&data, &policy, &scope, &digest, 10).unwrap();
    source.assessment_id = reservation.id().into();
    let proposal = validate_drafts(
        &policy,
        &scope,
        &source,
        &proofs,
        ProposalInventory {
            execution: &crate::community_ops::Ledger::default(),
            pending: &ProposalStore::default(),
        },
        10,
        topic(),
    )
    .unwrap()
    .remove(0);
    finish_assessment(
        &data,
        &reservation,
        &policy,
        &digest,
        vec![proposal.clone()],
        AssessmentOutcome::PendingPublished,
        10,
    )
    .unwrap();
    Fixture {
        data,
        policy: path,
        scope,
        proposal,
        proofs,
    }
}
fn approval<'a>(f: &'a Fixture, digest: &'a str, revision: u64) -> ApprovalRequest<'a> {
    let policy = load_policy(&f.policy).unwrap().0;
    ApprovalRequest {
        policy_path: &f.policy,
        data: &f.data,
        scope: &f.scope,
        expected_policy_digest: digest,
        expected_store_revision: revision,
        proposal_id: &f.proposal.id,
        reviewed_hash: &f.proposal.hash,
        owner: FreshOwnerProof::verified(1, 2, 2, 7, 10).unwrap(),
        operation: FreshOperationProof::verified(&policy, &f.scope, &f.proposal, &f.proofs, 10)
            .unwrap(),
    }
}
#[test]
fn approval_appends_exact_action_preserves_mode_scope_and_blocks_duplicate() {
    let f = setup();
    let (before, digest) = load_policy(&f.policy).unwrap();
    let store = load(&f.data).unwrap();
    let action = approve(approval(&f, &digest, store.revision), || 10).unwrap();
    let after = load_policy(&f.policy).unwrap().0;
    assert_eq!(after.mode, before.mode);
    assert_eq!(
        scope_digest(&after, &f.scope).unwrap(),
        scope_digest(&before, &f.scope).unwrap()
    );
    assert_eq!(after.actions, vec![action.clone()]);
    let store = load(&f.data).unwrap();
    assert!(store.executable(&action));
    assert_eq!(
        store.proposals[&f.proposal.id].status,
        ProposalStatus::Approved
    );
    assert!(approve(approval(&f, &digest, store.revision), || 10).is_err());
}
#[test]
fn stale_digest_revision_proof_and_rejection_leave_policy_unchanged() {
    let f = setup();
    let bytes = fs::read(&f.policy).unwrap();
    let digest = load_policy(&f.policy).unwrap().1;
    let store = load(&f.data).unwrap();
    assert!(approve(approval(&f, "wrong", store.revision), || 10).is_err());
    assert!(approve(approval(&f, &digest, store.revision + 1), || 10).is_err());
    assert!(approve(approval(&f, &digest, store.revision), || 71).is_err());
    assert_eq!(fs::read(&f.policy).unwrap(), bytes);
    reject(
        &f.data,
        &f.policy,
        &f.scope,
        &digest,
        store.revision,
        &f.proposal.id,
        &f.proposal.hash,
        FreshOwnerProof::verified(1, 2, 2, 7, 10).unwrap(),
        || 10,
    )
    .unwrap();
    assert_eq!(
        load(&f.data).unwrap().proposals[&f.proposal.id].status,
        ProposalStatus::Rejected
    );
    assert_eq!(fs::read(&f.policy).unwrap(), bytes);
}
#[test]
fn crash_journal_reconciles_only_already_published_exact_policy_action() {
    for published in [false, true] {
        let f = setup();
        let (mut policy, digest) = load_policy(&f.policy).unwrap();
        let path = directory(&f.data).unwrap().join("proposals.json");
        let mut store = load_at(&path).unwrap();
        let p = store.proposals.get_mut(&f.proposal.id).unwrap();
        let action = p.action();
        policy.actions.push(action.clone());
        let bytes = serde_json::to_vec_pretty(&policy).unwrap();
        p.status = ProposalStatus::ApprovalPrepared;
        p.decision = Some(OwnerDecision {
            actor: 2,
            decided_at: 10,
            reviewed_hash: p.hash.clone(),
            previous_policy_digest: digest.clone(),
            published_policy_digest: None,
            approved_action_key: Some(action.key.clone()),
            journal: Some(ApprovalJournal {
                action: action.clone(),
                reviewed_hash: p.hash.clone(),
                scope_digest: p.source.scope_digest.clone(),
                previous_policy_digest: digest,
                planned_policy_digest: hash_bytes(&bytes).unwrap(),
                stage: ApprovalStage::Prepared,
            }),
            recovery_audit: None,
        });
        save_at(&path, &mut store).unwrap();
        assert!(!store.executable(&action));
        if published {
            publish_bytes(&f.policy, &bytes).unwrap();
        }
        assert_eq!(reconcile(&f.data, &f.policy, &f.scope).is_ok(), published);
        let store = load(&f.data).unwrap();
        assert_eq!(store.executable(&action), published);
        if !published {
            assert!(load_policy(&f.policy).unwrap().0.actions.is_empty());
            assert_eq!(
                store.proposals[&f.proposal.id].status,
                ProposalStatus::ReviewRequired
            );
        }
    }
}
#[test]
fn reservation_restart_charge_and_nonprivate_files_fail_closed() {
    let f = setup();
    let (policy, digest) = load_policy(&f.policy).unwrap();
    assert!(begin_assessment(&f.data, &policy, &f.scope, &digest, 11).is_err());
    assert!(begin_assessment(&f.data, &policy, &f.scope, &digest, 9).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = directory(&f.data).unwrap().join("proposals.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&f.data).is_err());
    }
}
#[test]
fn commit_time_expiry_retains_journal_and_never_publishes_policy() {
    let mut f = setup();
    let original = fs::read(&f.policy).unwrap();
    let (policy, digest) = load_policy(&f.policy).unwrap();
    let store = load(&f.data).unwrap();
    let expires = f.proposal.expires_at;
    f.proofs.checked_at = expires - 1;
    let request = ApprovalRequest {
        policy_path: &f.policy,
        data: &f.data,
        scope: &f.scope,
        expected_policy_digest: &digest,
        expected_store_revision: store.revision,
        proposal_id: &f.proposal.id,
        reviewed_hash: &f.proposal.hash,
        owner: FreshOwnerProof::verified(1, 2, 2, 7, expires - 1).unwrap(),
        operation: FreshOperationProof::verified(
            &policy,
            &f.scope,
            &f.proposal,
            &f.proofs,
            expires - 1,
        )
        .unwrap(),
    };
    let calls = std::cell::Cell::new(0);
    assert!(
        approve(request, || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                expires - 1
            } else {
                expires
            }
        })
        .is_err()
    );
    assert_eq!(fs::read(&f.policy).unwrap(), original);
    let p = load(&f.data)
        .unwrap()
        .proposals
        .remove(&f.proposal.id)
        .unwrap();
    assert_eq!(p.status, ProposalStatus::ReviewRequired);
    assert!(p.decision.unwrap().journal.is_some());
}
#[test]
fn stopped_assessment_can_record_cancellation_without_publishing_or_replay() {
    let f = setup();
    let (mut policy, digest) = load_policy(&f.policy).unwrap();
    let reservation = begin_assessment(&f.data, &policy, &f.scope, &digest, 86410).unwrap();
    policy.mode = Mode::Stopped;
    finish_assessment(
        &f.data,
        &reservation,
        &policy,
        "changed-by-stop",
        vec![],
        AssessmentOutcome::Cancelled,
        86411,
    )
    .unwrap();
    let store = load(&f.data).unwrap();
    assert_eq!(
        store.attempts.last().unwrap().outcome,
        AssessmentOutcome::Cancelled
    );
    policy.mode = Mode::Propose;
    assert!(begin_assessment(&f.data, &policy, &f.scope, &digest, 86412).is_err());
    assert_eq!(store.proposals.len(), 1);
}
#[test]
fn rejection_uses_commit_clock_and_overflow_never_publishes() {
    let f = setup();
    let original = fs::read(&f.policy).unwrap();
    let digest = load_policy(&f.policy).unwrap().1;
    let mut store = load(&f.data).unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(
        reject(
            &f.data,
            &f.policy,
            &f.scope,
            &digest,
            store.revision,
            &f.proposal.id,
            &f.proposal.hash,
            FreshOwnerProof::verified(1, 2, 2, 7, 10).unwrap(),
            || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 { 10 } else { 71 }
            }
        )
        .is_err()
    );
    assert_eq!(
        load(&f.data).unwrap().proposals[&f.proposal.id].status,
        ProposalStatus::Pending
    );
    store.proposals.get_mut(&f.proposal.id).unwrap().revision = u64::MAX;
    let path = directory(&f.data).unwrap().join("proposals.json");
    save_at(&path, &mut store).unwrap();
    assert!(approve(approval(&f, &digest, store.revision), || 10).is_err());
    assert_eq!(fs::read(&f.policy).unwrap(), original);
    assert_eq!(
        load(&f.data).unwrap().proposals[&f.proposal.id].status,
        ProposalStatus::Pending
    );
}
#[test]
fn inspection_persists_stale_pending_but_retains_finalized_approval_evidence() {
    for approved in [false, true] {
        let f = setup();
        let (policy, digest) = load_policy(&f.policy).unwrap();
        let store = load(&f.data).unwrap();
        if approved {
            approve(approval(&f, &digest, store.revision), || 10).unwrap();
        }
        let current = load_policy(&f.policy).unwrap().0;
        let inspected = inspect_pending(&f.data, &current, f.proposal.expires_at).unwrap();
        assert_eq!(
            inspected.proposals[&f.proposal.id].status,
            if approved {
                ProposalStatus::Approved
            } else {
                ProposalStatus::Stale
            }
        );
        assert_eq!(
            load(&f.data).unwrap().proposals[&f.proposal.id].status,
            inspected.proposals[&f.proposal.id].status
        );
        assert_eq!(
            scope_digest(&policy, &f.scope).unwrap(),
            scope_digest(&current, &f.scope).unwrap()
        );
    }
}

#[test]
fn proposal_real_lock_and_publish_io_failures_preserve_recovery_evidence() {
    use crate::persist::owned_file::{Fault, with_fault};
    let f = setup();
    let lock_path = f.data.join("community-operations/proposals.lock");
    assert_eq!(
        with_fault(&lock_path, Fault::Sync, || lock(lock_path.clone())).err(),
        Some("proposal lease sync failed")
    );
    assert!(!lock_path.exists());
    let path = f.data.join("fault-publication.json");
    let pending = path.with_extension("proposal-pending");
    for (phase, expected) in [
        (Fault::Write, "proposal write failed"),
        (Fault::Sync, "proposal sync failed"),
    ] {
        assert_eq!(
            with_fault(&pending, phase, || publish_bytes(&path, b"new")),
            Err(expected)
        );
        assert!(!pending.exists());
        assert!(!path.exists());
    }
    fs::create_dir(&path).unwrap();
    assert_eq!(publish_bytes(&path, b"new"), Err("proposal rename failed"));
    assert!(!pending.exists());
    fs::remove_dir(&path).unwrap();
    assert_eq!(
        with_fault(&f.data, Fault::DirectorySync, || publish_bytes(
            &path,
            b"published evidence"
        )),
        Err("proposal directory sync failed")
    );
    assert_eq!(fs::read(&path).unwrap(), b"published evidence");
    assert!(!pending.exists());
    fs::write(&pending, b"pending evidence").unwrap();
    assert!(publish_bytes(&path, b"new").is_err());
    assert_eq!(fs::read(&pending).unwrap(), b"pending evidence");
    fs::write(&lock_path, b"crash lock").unwrap();
    assert!(lock(lock_path.clone()).is_err());
    assert_eq!(fs::read(&lock_path).unwrap(), b"crash lock");
}
