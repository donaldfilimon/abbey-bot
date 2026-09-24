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
    let digest = store.proposals[&id].content_digest.clone();
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
    assert_eq!(store.proposals[&id].approver, Some(2));
}

#[test]
fn authority_and_target_are_rechecked_before_execution() {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let digest = store.proposals[&id].content_digest.clone();
    let perms = BTreeSet::from([ActionPermission::GitHubIssuesWrite]);
    store
        .confirm(id, &digest, facts(Some(2), &perms, "etag-1", 10))
        .unwrap();
    assert_eq!(
        store.begin(id, facts(Some(2), &BTreeSet::new(), "etag-1", 11)),
        Err(ActionError::Denied)
    );
    assert_eq!(
        store.begin(id, facts(Some(2), &perms, "etag-2", 11)),
        Err(ActionError::Stale)
    );
    assert_eq!(
        store.begin(id, facts(Some(3), &perms, "etag-1", 11)),
        Err(ActionError::Denied)
    );
    assert_eq!(
        store.begin(id, facts(Some(2), &perms, "etag-1", 11)),
        Ok(example())
    );
    assert_eq!(
        store.begin(id, facts(Some(2), &perms, "etag-1", 12)),
        Err(ActionError::AlreadyHandled)
    );
    store
        .finish(id, ActionState::Verified, Some("comment-5".into()))
        .unwrap();
    assert_eq!(store.proposals[&id].state, ActionState::Verified);
}

#[test]
fn changed_arguments_or_digest_invalidate_confirmation() {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let original = store.proposals[&id].content_digest.clone();
    let perms = BTreeSet::from([ActionPermission::GitHubIssuesWrite]);
    assert_eq!(
        store.confirm(id, "forged", facts(Some(2), &perms, "etag-1", 10)),
        Err(ActionError::Stale)
    );
    store.proposals.get_mut(&id).unwrap().spec.operation = ActionOperation::GitHubComment {
        body: "Different".into(),
    };
    assert_eq!(
        store.confirm(id, &original, facts(Some(2), &perms, "etag-1", 10)),
        Err(ActionError::Stale)
    );
}

#[test]
fn interrupted_execution_requires_review_after_reload() {
    let mut store = ActionStore::default();
    let id = store.propose(example(), 1, 100, "etag-1").unwrap();
    let digest = store.proposals[&id].content_digest.clone();
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
    assert_eq!(restored.proposals[&id].state, ActionState::ReviewRequired);
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
    assert!(store.proposals.is_empty());
}
