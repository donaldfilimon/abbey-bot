use super::*;

fn example() -> ActionSpec {
    ActionSpec {
        target: ActionTarget::GitHubIssue {
            installation: 7,
            owner: "team".into(),
            repo: "repo".into(),
            issue: 3,
        },
        operation: ActionOperation::GitHubComment {
            body: "Status update".into(),
        },
        required: BTreeSet::from([ActionPermission::GitHubIssuesWrite]),
    }
}

fn facts<'a>(
    human: Option<u64>,
    permissions: &'a BTreeSet<ActionPermission>,
    target: &'a str,
    now: u64,
) -> ActionFacts<'a> {
    ActionFacts {
        human_principal: human,
        current_permissions: permissions,
        target_fingerprint: target,
        now,
    }
}

#[test]
fn human_confirmation_is_exact_and_single_use() {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let digest = store.proposal(id).unwrap().content_digest().to_owned();
    let perms = BTreeSet::from([ActionPermission::GitHubIssuesWrite]);
    assert_eq!(
        store.confirm(id, &digest, facts(None, &perms, "etag-1", 10)),
        Err(ActionError::Denied)
    );
    assert_eq!(
        store.confirm(id, &digest, facts(Some(2), &perms, "etag-2", 10)),
        Err(ActionError::Stale)
    );
    assert_eq!(
        store.confirm(id, &digest, facts(Some(2), &perms, "etag-1", 100)),
        Err(ActionError::Expired)
    );
    store
        .confirm(id, &digest, facts(Some(2), &perms, "etag-1", 10))
        .unwrap();
    assert_eq!(
        store.confirm(id, &digest, facts(Some(2), &perms, "etag-1", 11)),
        Err(ActionError::AlreadyHandled)
    );
    let proposal = store.proposal(id).unwrap();
    assert_eq!(proposal.approver(), Some(2));
    assert_eq!(proposal.spec(), &example());
    assert_eq!(proposal.requester(), 1);
    assert_eq!(proposal.expires_at(), 100);
    assert_eq!(proposal.target_fingerprint(), "etag-1");
    assert_eq!(proposal.result_reference(), None);
}

fn approved() -> (ActionStore, u64, BTreeSet<ActionPermission>) {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let digest = store.proposal(id).unwrap().content_digest().to_owned();
    let perms = BTreeSet::from([ActionPermission::GitHubIssuesWrite]);
    store
        .confirm(id, &digest, facts(Some(2), &perms, "etag-1", 10))
        .unwrap();
    (store, id, perms)
}

#[test]
fn authority_and_target_are_rechecked_before_execution() {
    for (target, now, permitted, reason) in [
        ("etag-2", 11, true, ActionError::Stale),
        ("etag-1", 100, true, ActionError::Expired),
        ("etag-1", 11, false, ActionError::Denied),
    ] {
        let (mut store, id, perms) = approved();
        let current = if permitted {
            perms.clone()
        } else {
            BTreeSet::new()
        };
        assert_eq!(
            store.begin(id, facts(Some(2), &current, target, now)),
            Ok(BeginOutcome::Invalidated(reason))
        );
        // A successful closure result can be committed; serialize that snapshot
        // and verify old facts never revive the approval after reload either.
        let mut restored: ActionStore =
            serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(
            restored.proposal(id).unwrap().state(),
            ActionState::Invalidated
        );
        assert_eq!(
            restored.begin(id, facts(Some(2), &perms, "etag-1", 11)),
            Err(ActionError::AlreadyHandled)
        );
    }
}

#[test]
fn different_human_cannot_execute_and_approved_attempt_is_single_use() {
    let (mut store, id, perms) = approved();
    assert_eq!(
        store.begin(id, facts(Some(3), &perms, "etag-1", 11)),
        Err(ActionError::Denied)
    );
    assert_eq!(
        store.begin(id, facts(Some(2), &perms, "etag-1", 11)),
        Ok(BeginOutcome::Ready(example()))
    );
    assert_eq!(
        store.begin(id, facts(Some(2), &perms, "etag-1", 12)),
        Err(ActionError::AlreadyHandled)
    );
    store
        .finish(id, ActionState::Verified, Some("comment-5".into()))
        .unwrap();
    assert_eq!(store.proposal(id).unwrap().state(), ActionState::Verified);
}

#[test]
fn changed_arguments_or_digest_invalidate_confirmation() {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let original = store.proposal(id).unwrap().content_digest().to_owned();
    let perms = BTreeSet::from([ActionPermission::GitHubIssuesWrite]);
    assert_eq!(
        store.confirm(id, "forged", facts(Some(2), &perms, "etag-1", 10)),
        Err(ActionError::Stale)
    );
    // Caller-owned specs can be edited without changing an existing proposal.
    let mut changed = example();
    changed.operation = ActionOperation::GitHubComment {
        body: "Different".into(),
    };
    let replacement = store.propose(changed, 1, 100, "etag-1").unwrap();
    assert_ne!(
        store.proposal(replacement).unwrap().content_digest(),
        original
    );
    assert_eq!(
        store.confirm(replacement, &original, facts(Some(2), &perms, "etag-1", 10)),
        Err(ActionError::Stale)
    );
    assert_eq!(store.proposal(id).unwrap().content_digest(), original);
}

#[test]
fn interrupted_execution_requires_review_after_reload() {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let digest = store.proposal(id).unwrap().content_digest().to_owned();
    let perms = BTreeSet::from([ActionPermission::GitHubIssuesWrite]);
    store
        .confirm(id, &digest, facts(Some(2), &perms, "etag-1", 10))
        .unwrap();
    store
        .begin(id, facts(Some(2), &perms, "etag-1", 11))
        .unwrap();
    let json = serde_json::to_string(&store).unwrap();
    let mut restored: ActionStore = serde_json::from_str(&json).unwrap();
    restored.mark_interrupted();
    assert_eq!(
        restored.proposal(id).unwrap().state(),
        ActionState::ReviewRequired
    );
    assert_eq!(
        restored.begin(id, facts(Some(2), &perms, "etag-1", 12)),
        Err(ActionError::AlreadyHandled)
    );
}

#[test]
fn mismatched_target_and_permission_cannot_be_proposed() {
    let mut store = ActionStore::default();
    let mut spec = example();
    spec.target = ActionTarget::DiscordChannel {
        guild: 1,
        channel: 2,
    };
    assert_eq!(
        store.propose(spec, 1, 100, "etag"),
        Err(ActionError::Invalid)
    );
    let mut spec = example();
    spec.required = BTreeSet::from([ActionPermission::DiscordSendMessages]);
    assert_eq!(
        store.propose(spec, 1, 100, "etag"),
        Err(ActionError::Invalid)
    );
    assert!(store.proposal(1).is_none());
}

#[test]
fn corrupted_persisted_arguments_invalidate_without_revival() {
    let (store, id, perms) = approved();
    let mut encoded = serde_json::to_value(&store).unwrap();
    encoded["proposals"][id.to_string()]["spec"]["operation"]["GitHubComment"]["body"] =
        serde_json::json!("Changed");
    let mut restored: ActionStore = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        restored.begin(id, facts(Some(2), &perms, "etag-1", 11)),
        Ok(BeginOutcome::Invalidated(ActionError::Stale))
    );
    let mut encoded = serde_json::to_value(&restored).unwrap();
    encoded["proposals"][id.to_string()]["spec"]["operation"]["GitHubComment"]["body"] =
        serde_json::json!("Status update");
    let mut restored: ActionStore = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        restored.begin(id, facts(Some(2), &perms, "etag-1", 12)),
        Err(ActionError::AlreadyHandled)
    );
}

#[test]
fn capacity_reclaims_terminal_records_and_reports_real_exhaustion() {
    let (mut store, id, perms) = approved();
    store
        .begin(id, facts(Some(2), &perms, "etag-1", 11))
        .unwrap();
    store.finish(id, ActionState::Verified, None).unwrap();
    for _ in 1..10_000 {
        store.propose(example(), 1, 100, "etag-1").unwrap();
    }
    let fresh = store.propose(example(), 1, 100, "etag-1").unwrap();
    assert!(fresh > 10_000);
    assert!(store.proposal(id).is_none());
    assert_eq!(
        store.propose(example(), 1, 100, "etag-1"),
        Err(ActionError::Full)
    );
    store.compact(100);
    assert!(store.propose(example(), 1, 200, "etag-1").unwrap() > fresh);
}

#[test]
fn compaction_preserves_every_unresolved_attempt() {
    let (mut store, executing, perms) = approved();
    store
        .begin(executing, facts(Some(2), &perms, "etag-1", 11))
        .unwrap();
    let mut ids = vec![executing];
    for state in [
        ActionState::ReviewRequired,
        ActionState::Failed,
        ActionState::Verified,
        ActionState::Approved,
        ActionState::Proposed,
    ] {
        let id = store.propose(example(), 1, 100, "etag-1").unwrap();
        ids.push(id);
        if state == ActionState::Proposed {
            continue;
        }
        let digest = store.proposal(id).unwrap().content_digest().to_owned();
        store
            .confirm(id, &digest, facts(Some(2), &perms, "etag-1", 10))
            .unwrap();
        if state == ActionState::Approved {
            continue;
        }
        store
            .begin(id, facts(Some(2), &perms, "etag-1", 11))
            .unwrap();
        store.finish(id, state, None).unwrap();
    }
    store.compact(100);
    for (index, id) in ids.into_iter().enumerate() {
        assert_eq!(store.proposal(id).is_some(), index < 2);
    }
}
