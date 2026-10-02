use super::*;
pub(crate) fn fixture() -> (Policy, AssessmentScope, AssessmentSource, PublicProofs) {
    let mut policy: Policy = serde_json::from_value(serde_json::json!({"version":1,"guild":1,"owner":2,"mode":"propose","daily_limit":5,"daily_creations":2,"public_categories":[3],"protected_channels":[9],"ordinary_roles":[5],"membership_matrix":[],"actions":[]})).unwrap();
    let scope = AssessmentScope {
        enabled: true,
        allowed_kinds: [
            AssessmentKind::Topic,
            AssessmentKind::CreateText,
            AssessmentKind::CreateInterestRole,
            AssessmentKind::Archive,
        ]
        .into(),
        source_channels: [4].into(),
        review_channel: Some(7),
    };
    policy.assessment = scope.clone();
    let channel = PublicChannelMetadata {
        id: 4,
        name: "general".into(),
        kind: "text".into(),
        parent: 3,
        topic: Some("before".into()),
    };
    let category = NamedMetadata {
        id: 3,
        name: "Commons".into(),
    };
    let role = NamedMetadata {
        id: 5,
        name: "Research".into(),
    };
    let proofs = PublicProofs {
        guild: 1,
        checked_at: 10,
        channels: [(
            4,
            ChannelProof {
                metadata: channel.clone(),
                public: true,
                archive_safe: true,
                permissions_digest: "access".into(),
                before_digest: "before".into(),
            },
        )]
        .into(),
        categories: [(3, "category".into())].into(),
        roles: [(
            5,
            RoleProof {
                metadata: role.clone(),
                permissions: 0,
                entitlement_free: true,
                before_digest: "role".into(),
            },
        )]
        .into(),
    };
    let source = AssessmentSource {
        version: 1,
        assessment_id: "assessment".into(),
        guild: 1,
        scope_digest: scope_digest(&policy, &scope).unwrap(),
        policy_digest: "policy".into(),
        captured_at: 10,
        inventory_digest: digest(&(
            &vec![channel.clone()],
            &vec![category.clone()],
            &vec![role.clone()],
        ))
        .unwrap(),
        channels: vec![channel],
        categories: vec![category],
        roles: vec![role],
    };
    (policy, scope, source, proofs)
}
pub(crate) fn topic() -> ModelProposalBatch {
    ModelProposalBatch {
        version: 1,
        proposals: vec![ModelProposal {
            operation: Operation::Topic {
                channel: 4,
                topic: "after".into(),
            },
            reason: "Improve the public topic".into(),
        }],
    }
}
#[test]
fn strict_json_rejects_authority_unknown_variants_and_excess_bounds() {
    assert!(ModelProposalBatch::parse(br#"{"version":1,"proposals":[],"approved":true}"#).is_err());
    assert!(ModelProposalBatch::parse(br#"{"version":1,"proposals":[{"operation":{"kind":"delete","channel":4},"reason":"x"}]}"#).is_err());
    assert!(ModelProposalBatch::parse(&vec![b' '; MAX_OUTPUT_BYTES + 1]).is_err());
    assert!(ModelProposalBatch::parse(b"```json\n{}\n```").is_err());
}
#[test]
fn model_scope_is_not_authority_and_metadata_cannot_make_private_targets_public() {
    let (policy, scope, mut source, mut proofs) = fixture();
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
        topic(),
    )
    .unwrap()
    .remove(0);
    assert_eq!(p.status, ProposalStatus::Pending);
    assert!(p.decision.is_none());
    assert!(!ProposalStore::default().executable(&p.action()));
    proofs.channels.get_mut(&4).unwrap().public = false;
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            topic()
        )
        .is_err()
    );
    proofs.channels.get_mut(&4).unwrap().public = true;
    source.channels.clear();
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            topic()
        )
        .is_err()
    );
}
#[test]
fn pending_conflicts_budget_unknown_ids_and_access_drift_fail_closed() {
    let (policy, scope, source, mut proofs) = fixture();
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
        topic(),
    )
    .unwrap()
    .remove(0);
    let mut store = ProposalStore::default();
    store.proposals.insert(p.id.clone(), p.clone());
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &store
            },
            10,
            topic()
        )
        .is_err()
    );
    let mut unknown = topic();
    unknown.proposals[0].operation = Operation::Topic {
        channel: 999,
        topic: "x".into(),
    };
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            unknown
        )
        .is_err()
    );
    let mut duplicate = topic();
    duplicate.proposals.push(duplicate.proposals[0].clone());
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            duplicate
        )
        .is_err()
    );
    proofs.channels.get_mut(&4).unwrap().permissions_digest = "changed".into();
    assert!(FreshOperationProof::verified(&policy, &scope, &p, &proofs, 10).is_err());
    assert!(FreshOwnerProof::verified(1, 2, 8, 7, 10).is_err());
    assert!(
        FreshOwnerProof::verified(1, 2, 2, 8, 10)
            .unwrap()
            .check(&policy, &scope, 10)
            .is_err()
    );
    assert!(
        FreshOwnerProof::verified(1, 2, 2, 7, 10)
            .unwrap()
            .check(&policy, &scope, 71)
            .is_err()
    );
    assert!(
        FreshOwnerProof::verified(1, 2, 2, 7, 11)
            .unwrap()
            .check(&policy, &scope, 10)
            .is_err()
    );
}
#[test]
fn forbidden_operation_and_creation_batch_budget_never_create_pending_rows() {
    let (mut policy, scope, source, proofs) = fixture();
    let mut forbidden = topic();
    forbidden.proposals[0].operation = Operation::Membership {
        member: 8,
        role: 5,
        grant: true,
    };
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            forbidden
        )
        .is_err()
    );
    policy.daily_limit = 1;
    policy.daily_creations = 1;
    let mut source = source;
    source.scope_digest = scope_digest(&policy, &scope).unwrap();
    let batch = ModelProposalBatch {
        version: 1,
        proposals: vec![
            ModelProposal {
                operation: Operation::CreateInterestRole { name: "A".into() },
                reason: "x".into(),
            },
            ModelProposal {
                operation: Operation::CreateInterestRole { name: "B".into() },
                reason: "x".into(),
            },
        ],
    };
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            batch
        )
        .is_err()
    );
}
#[test]
fn queued_owner_inventory_and_unprovable_archive_are_not_model_candidates() {
    let (mut policy, scope, source, mut proofs) = fixture();
    policy.actions.push(Action {
        key: "owner-topic".into(),
        reason: "owner queued".into(),
        operation: Operation::Topic {
            channel: 4,
            topic: "queued".into(),
        },
    });
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            topic()
        )
        .is_err()
    );
    policy.actions.clear();
    proofs.channels.get_mut(&4).unwrap().archive_safe = false;
    let batch = ModelProposalBatch {
        version: 1,
        proposals: vec![ModelProposal {
            operation: Operation::Archive {
                channel: 4,
                category: 3,
            },
            reason: "archive".into(),
        }],
    };
    assert!(
        validate_drafts(
            &policy,
            &scope,
            &source,
            &proofs,
            ProposalInventory {
                execution: &Ledger::default(),
                pending: &ProposalStore::default()
            },
            10,
            batch
        )
        .is_err()
    );
}
