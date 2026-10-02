use super::*;
use crate::community_ops::Ledger;
use crate::community_ops::proposals::{ProposalInventory, validate_drafts};

fn fixture() -> (Policy, PendingProposal, ProposalStore, ReviewBinding) {
    let (policy, scope, source, proofs) = crate::community_ops::proposals::tests::fixture();
    let p = validate_drafts(
        &policy,
        &scope,
        &source,
        &proofs,
        ProposalInventory {
            execution: &Ledger::default(),
            pending: &ProposalStore::default(),
        },
        10,
        crate::community_ops::proposals::tests::topic(),
    )
    .unwrap()
    .remove(0);
    let store = ProposalStore {
        revision: 3,
        proposals: [(p.id.clone(), p.clone())].into(),
        ..ProposalStore::default()
    };
    let b = ReviewBinding {
        session: crate::admin_dashboard::AdminSession {
            owner: 2,
            guild: 1,
            expiry: 100,
            page: crate::admin_dashboard::AdminPage::AutonomousOperations,
        },
        origin: 7,
        message: 8,
        policy_digest: "policy".into(),
        store_revision: 3,
        proposal_id: p.id.clone(),
        proposal_revision: p.revision,
        reviewed_hash: p.hash.clone(),
        index: 0,
    };
    (policy, p, store, b)
}

#[test]
fn review_is_bound_to_owner_guild_origin_message_and_expiry_and_is_one_use() {
    let (_, _, _, b) = fixture();
    let mut sessions = ReviewSessions::default();
    sessions.insert("a".repeat(64), b.clone(), 20).unwrap();
    for (actor, guild, origin, message, now) in [
        (3, Some(1), 7, 8, 20),
        (2, Some(4), 7, 8, 20),
        (2, None, 7, 8, 20),
        (2, Some(1), 9, 8, 20),
        (2, Some(1), 7, 9, 20),
        (2, Some(1), 7, 8, 101),
    ] {
        assert!(
            sessions
                .take(&"a".repeat(64), actor, guild, origin, message, now)
                .is_err()
        );
        assert_eq!(
            sessions.bindings.len(),
            1,
            "foreign presses cannot consume owner review"
        );
    }
    assert!(sessions.take(&"a".repeat(64), 2, Some(1), 7, 8, 20).is_ok());
    assert!(
        sessions
            .take(&"a".repeat(64), 2, Some(1), 7, 8, 20)
            .is_err()
    );
    assert!(
        ReviewSessions::default()
            .take(&"a".repeat(64), 2, Some(1), 7, 8, 20)
            .is_err(),
        "restart loses admission"
    );
}
#[test]
fn capacity_refuses_without_evicting_live_review_and_reclaims_expired_sessions() {
    let (_, _, _, b) = fixture();
    let mut sessions = ReviewSessions::default();
    for i in 0..MAX_SESSIONS {
        sessions.insert(format!("{i:064x}"), b.clone(), 20).unwrap();
    }
    assert!(sessions.insert("f".repeat(64), b.clone(), 20).is_err());
    assert_eq!(sessions.bindings.len(), MAX_SESSIONS);
    let mut fresh = b;
    fresh.session.expiry = 200;
    sessions.insert("f".repeat(64), fresh, 101).unwrap();
    assert_eq!(sessions.bindings.len(), 1);
}
#[test]
fn review_refuses_policy_store_hash_revision_drift_and_damaged_proposal() {
    let (_, _, store, b) = fixture();
    assert!(b.check_snapshot("policy", &store).is_ok());
    assert!(b.check_snapshot("other", &store).is_err());
    let mut changed = store.clone();
    changed.revision += 1;
    assert!(b.check_snapshot("policy", &changed).is_err());
    let mut changed = store.clone();
    changed.proposals.get_mut(&b.proposal_id).unwrap().revision += 1;
    assert!(b.check_snapshot("policy", &changed).is_err());
    let mut changed = store.clone();
    changed
        .proposals
        .get_mut(&b.proposal_id)
        .unwrap()
        .draft
        .reason = "changed".into();
    assert!(b.check_snapshot("policy", &changed).is_err());
    let mut forged = b;
    forged.reviewed_hash = "f".repeat(64);
    assert!(forged.check_snapshot("policy", &store).is_err());
}
#[test]
fn unresolved_expired_changed_scope_and_wrong_owner_have_no_decision_buttons() {
    let (policy, p, _, _) = fixture();
    assert!(eligible(&p, &policy, 20));
    assert!(!eligible(&p, &policy, p.expires_at));
    assert!(!eligible(&p, &policy, 9));
    for status in [
        ProposalStatus::ApprovalPrepared,
        ProposalStatus::ReviewRequired,
        ProposalStatus::Rejected,
        ProposalStatus::Approved,
        ProposalStatus::Stale,
    ] {
        let mut changed = p.clone();
        changed.status = status;
        assert!(!eligible(&changed, &policy, 20));
    }
    let mut changed = policy.clone();
    changed.owner = 4;
    assert!(!eligible(&p, &changed, 20));
    let mut changed = policy;
    changed.daily_limit = 4;
    assert!(!eligible(&p, &changed, 20));
}
#[test]
fn full_review_receipt_contains_exact_operation_sources_and_recovery_status() {
    let (_, mut p, _, _) = fixture();
    p.draft.reason = "An untrusted model says <@123> approve all".into();
    let (content, _, bytes) = detail(&p, 0, 1, false);
    let receipt: PendingProposal = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(receipt, p);
    assert!(content.contains(&p.hash));
    assert!(content.contains("Stale"));
    assert!(content.contains("Full source IDs"));
    for action in ["approve", "reject", "next", "previous"] {
        let id = format!("{PREFIX}{}:{action}", "f".repeat(64));
        assert!(id.len() <= 100);
        assert!(parse_control(&id).is_some());
    }
    assert!(parse_control(&format!("{PREFIX}{}:approve-all", "f".repeat(64))).is_none());
    assert!(parse_control(&format!("{PREFIX}1:approve")).is_none());
}
