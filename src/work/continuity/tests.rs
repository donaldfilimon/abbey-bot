//! Real pure continuity transitions. Every expectation below is hand-derived;
//! WorkStore supplies native records and the actual existing scope authority.
use super::super::WorkTask;
use super::*;

pub(super) fn personal(actor: u64) -> WorkAccess {
    WorkAccess {
        actor,
        guild: None,
        channel: actor * 10,
        can_view: true,
        can_manage: false,
    }
}

pub(super) fn fixture(access: WorkAccess) -> (WorkStore, BTreeSet<WorkContentRef>) {
    let mut work = WorkStore::default();
    let project = work
        .create_project(
            WorkAccess {
                can_manage: true,
                ..access
            },
            "Compiler",
            "create",
        )
        .unwrap();
    let decision = work
        .record_decision(
            project,
            access,
            "Keep the checked allocator.",
            50,
            "decision",
        )
        .unwrap();
    (
        work,
        BTreeSet::from([WorkContentRef::Decision {
            project,
            id: decision,
            revision: 1,
        }]),
    )
}

pub(super) fn issue(
    registry: &mut ProposalRegistry,
    work: &WorkStore,
    access: WorkAccess,
    revision: u64,
    text: &str,
    refs: BTreeSet<WorkContentRef>,
    now: u64,
) -> ContinuityProposal {
    registry
        .propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: revision,
                presented_text: text.into(),
                source_refs: refs,
            },
            &access,
            work,
            now,
        )
        .unwrap()
}

pub(super) fn confirm(
    store: &mut ContinuityStore,
    registry: &mut ProposalRegistry,
    proposal: &ContinuityProposal,
    work: &WorkStore,
    access: WorkAccess,
    now: u64,
) -> Result<ContinuityCard, WorkError> {
    let grant = registry.resolve_confirmation(proposal.id, access.actor, &access.scope(), now)?;
    store.confirm(grant, &access, work, now)
}

#[test]
fn stale_or_model_confirmation_refused() {
    // Break caught: storing a generated proposal before a resolved human control,
    // or allowing a second immutable proposal to overwrite a changed revision.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let a = issue(
        &mut registry,
        &work,
        access,
        0,
        "Finish checked allocation.",
        refs.clone(),
        100,
    );
    let b = issue(
        &mut registry,
        &work,
        access,
        0,
        "Change the allocator.",
        refs,
        100,
    );
    assert!(
        store
            .context(&access.scope(), &access, &work, 100)
            .is_none()
    );
    assert!(
        store.cards.is_empty(),
        "A proposal alone is never confirmation"
    );
    confirm(&mut store, &mut registry, &a, &work, access, 101).unwrap();
    let before = store.clone();
    assert_eq!(
        confirm(&mut store, &mut registry, &b, &work, access, 102),
        Err(WorkError::Stale)
    );
    assert_eq!(store, before);
}

#[test]
fn card_limits_and_expiry() {
    // Break caught: character-count instead of UTF-8-byte limits, silent
    // truncation, unbounded references, or inclusive expiry.
    let access = personal(7);
    let (mut work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let text = "é".repeat(800);
    let p = issue(&mut registry, &work, access, 0, &text, refs, 100);
    let card = confirm(&mut store, &mut registry, &p, &work, access, 100).unwrap();
    assert_eq!(card.confirmed_text, text);
    assert_eq!(card.confirmed_text.len(), 1600);
    assert_eq!(card.expires_at, 604900);
    assert_eq!(card.confirmed_by, 7);
    assert_eq!(card.revision, 1);
    assert!(
        store
            .context(&access.scope(), &access, &work, 604899)
            .is_some()
    );
    assert!(
        store
            .context(&access.scope(), &access, &work, 604900)
            .is_none()
    );
    assert!(store.context(&access.scope(), &access, &work, 99).is_none());
    for text in ["é".repeat(801), " ".into(), "bad\0text".into()] {
        assert_eq!(
            registry.propose(
                ContinuityDraft {
                    scope: access.scope(),
                    base_revision: 1,
                    presented_text: text,
                    source_refs: BTreeSet::new()
                },
                &access,
                &work,
                100
            ),
            Err(WorkError::Invalid)
        );
    }
    let project = *work.projects.keys().next().unwrap();
    let refs: BTreeSet<_> = (0..9)
        .map(|i| {
            let id = work
                .record_decision(project, access, "Confirmed choice", 100, &format!("d{i}"))
                .unwrap();
            WorkContentRef::Decision {
                project,
                id,
                revision: 1,
            }
        })
        .collect();
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 1,
                presented_text: "Nine sources".into(),
                source_refs: refs
            },
            &access,
            &work,
            100
        ),
        Err(WorkError::Invalid)
    );
}

#[test]
fn missing_source_excludes_context() {
    // Break caught: retaining freeform text when a referenced native source disappears.
    let access = personal(7);
    let (mut work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Resume the allocator",
        refs,
        100,
    );
    confirm(&mut store, &mut registry, &p, &work, access, 101).unwrap();
    work.decisions.clear();
    assert!(
        store
            .context(&access.scope(), &access, &work, 102)
            .is_none()
    );
}

#[test]
fn two_proposals_same_base_revision_bind_exact_text() {
    // Break caught: accepting text sent back by the client instead of the
    // immutable registry copy, or treating different proposals as interchangeable.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let mut a = issue(
        &mut registry,
        &work,
        access,
        0,
        "Keep checked allocation",
        refs.clone(),
        100,
    );
    let b = issue(
        &mut registry,
        &work,
        access,
        0,
        "Drop allocation checks",
        refs,
        100,
    );
    assert_ne!(a.id, b.id);
    a.presented_text = "Client supplied replacement".into();
    let card = confirm(&mut store, &mut registry, &a, &work, access, 101).unwrap();
    assert_eq!(card.confirmed_text, "Keep checked allocation");
    assert_eq!(
        confirm(&mut store, &mut registry, &b, &work, access, 102),
        Err(WorkError::Stale)
    );
    assert_eq!(
        store
            .context(&access.scope(), &access, &work, 102)
            .unwrap()
            .text(),
        "Keep checked allocation"
    );
}

#[test]
fn other_actor_control_denied() {
    // Break caught: a copied control can be consumed by a different actor,
    // including denial that also destroys the legitimate member's proposal.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let p = issue(&mut registry, &work, access, 0, "Keep checks", refs, 100);
    assert!(matches!(
        registry.resolve_confirmation(p.id, 8, &access.scope(), 101),
        Err(WorkError::Denied)
    ));
    assert!(matches!(
        registry.resolve_confirmation(p.id, 7, &personal(8).scope(), 101),
        Err(WorkError::Denied)
    ));
    let mut store = ContinuityStore::default();
    confirm(&mut store, &mut registry, &p, &work, access, 101).unwrap();
    assert!(matches!(
        registry.resolve_confirmation(p.id, 7, &access.scope(), 102),
        Err(WorkError::Missing)
    ));
}

#[test]
fn proposal_expired_or_missing_after_restart_denied() {
    // Break caught: inclusive five-minute expiry, or matching only sequence
    // and thereby resolving an old control to a new boot's immutable text.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut old = ProposalRegistry::new([1; 16]);
    let p = issue(&mut old, &work, access, 0, "Old text", refs.clone(), 100);
    assert_eq!(p.expires_at, 400);
    assert!(matches!(
        old.resolve_confirmation(p.id, 7, &access.scope(), 400),
        Err(WorkError::Stale)
    ));
    let mut new = ProposalRegistry::new([2; 16]);
    let q = issue(&mut new, &work, access, 0, "New text", refs, 401);
    assert_eq!(p.id.sequence, q.id.sequence);
    assert!(matches!(
        new.resolve_confirmation(p.id, 7, &access.scope(), 402),
        Err(WorkError::Missing)
    ));
    let mut store = ContinuityStore::default();
    assert_eq!(
        confirm(&mut store, &mut new, &q, &work, access, 402)
            .unwrap()
            .confirmed_text,
        "New text"
    );
}

#[test]
fn changed_source_between_preview_and_click_denied() {
    // Break caught: stale task revision or changed native project join accepted
    // between an immutable preview and final confirmation.
    let access = personal(7);
    let (mut work, _) = fixture(access);
    let project = *work.projects.keys().next().unwrap();
    let task = WorkTask {
        id: 0,
        project_id: project,
        title: "Audit allocator".into(),
        owner: 7,
        assignee: None,
        goal_id: None,
        priority: 1,
        status: super::super::WorkStatus::Open,
        due_at: None,
        remind_at: None,
        reminder_revision: 0,
        snoozed_until: None,
        source: None,
        github: None,
        revision: 0,
    };
    let id = work.add_task(access, task, "task").unwrap();
    let refs = BTreeSet::from([WorkContentRef::Task {
        project,
        id,
        revision: 0,
    }]);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Audit allocator",
        refs,
        100,
    );
    work.update_task(access, id, 0, super::super::WorkStatus::Done, None)
        .unwrap();
    assert_eq!(
        confirm(&mut store, &mut registry, &p, &work, access, 101),
        Err(WorkError::Stale)
    );
    assert!(store.cards.is_empty());
}

pub(super) fn team() -> WorkAccess {
    WorkAccess {
        actor: 7,
        guild: Some(10),
        channel: 20,
        can_view: true,
        can_manage: false,
    }
}

#[test]
fn delegated_manager_without_guild_manage_can_confirm() {
    // Break caught: replacing existing native Work manager authority with MANAGE_GUILD.
    let access = team();
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Team allocator review",
        refs,
        100,
    );
    assert_eq!(
        confirm(&mut store, &mut registry, &p, &work, access, 101)
            .unwrap()
            .confirmed_by,
        7
    );
}

#[test]
fn guild_administrator_outside_work_membership_denied() {
    // Break caught: guild permission bits bypassing current native membership.
    let access = team();
    let (work, refs) = fixture(access);
    let outsider = WorkAccess {
        actor: 8,
        can_manage: true,
        ..access
    };
    let mut registry = ProposalRegistry::new([1; 16]);
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Overwrite".into(),
                source_refs: refs
            },
            &outsider,
            &work,
            100
        ),
        Err(WorkError::Denied)
    );
}

#[test]
fn fresh_membership_and_room_scope_are_rechecked() {
    // Break caught: permission/member removal between proposal and resolution,
    // or a saved authorized card later crossing rooms/DM principals.
    let access = team();
    let (mut work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Keep checks",
        refs.clone(),
        100,
    );
    let no_view = WorkAccess {
        can_view: false,
        ..access
    };
    let grant = registry
        .resolve_confirmation(p.id, access.actor, &access.scope(), 101)
        .unwrap();
    assert_eq!(
        store.confirm(grant, &no_view, &work, 101),
        Err(WorkError::Denied)
    );
    let q = issue(&mut registry, &work, access, 0, "Keep checks", refs, 102);
    confirm(&mut store, &mut registry, &q, &work, access, 103).unwrap();
    assert!(
        store
            .context(&access.scope(), &no_view, &work, 104)
            .is_none()
    );
    assert!(
        store
            .context(
                &access.scope(),
                &WorkAccess {
                    channel: 21,
                    ..access
                },
                &work,
                104
            )
            .is_none()
    );
    assert!(
        store
            .context(&access.scope(), &personal(7), &work, 104)
            .is_none()
    );
    work.projects
        .values_mut()
        .next()
        .unwrap()
        .members
        .remove(&7);
    assert!(
        store
            .context(&access.scope(), &access, &work, 104)
            .is_none()
    );
}

#[test]
fn scope_wide_management_empty_or_mixed_projects_refused() {
    // Break caught: any-manager-in-room or vacuous empty-reference authority
    // replacing the existing all-project scope-wide control rule.
    let access = team();
    let (mut work, refs) = fixture(access);
    let other = work
        .create_project(
            WorkAccess {
                can_manage: true,
                ..access
            },
            "Other project",
            "other",
        )
        .unwrap();
    work.projects.get_mut(&other).unwrap().managers.clear();
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(&mut registry, &work, access, 0, "Keep checks", refs, 100);
    assert_eq!(
        confirm(&mut store, &mut registry, &p, &work, access, 101),
        Err(WorkError::Denied)
    );
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "No project".into(),
                source_refs: BTreeSet::new()
            },
            &access,
            &WorkStore::default(),
            100
        ),
        Err(WorkError::Missing)
    );
}

#[test]
fn card_count_bounded_and_expired_slots_reusable() {
    // Break caught: unbounded cards, capacity preventing existing-scope
    // replacement, or expired cards preventing a legitimate new scope.
    let mut store = ContinuityStore::default();
    let mut registry = ProposalRegistry::new([1; 16]);
    for actor in 1..=256 {
        let access = personal(actor);
        let (work, refs) = fixture(access);
        let p = issue(&mut registry, &work, access, 0, "Resume work", refs, 100);
        confirm(&mut store, &mut registry, &p, &work, access, 100).unwrap();
    }
    assert_eq!(store.cards.len(), 256);
    let access = personal(257);
    let (work, refs) = fixture(access);
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "New scope",
        refs.clone(),
        101,
    );
    let before = store.clone();
    assert_eq!(
        confirm(&mut store, &mut registry, &p, &work, access, 101),
        Err(WorkError::Full)
    );
    assert_eq!(store, before);
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "After expiry",
        refs,
        604900,
    );
    confirm(&mut store, &mut registry, &p, &work, access, 604900).unwrap();
    assert_eq!(store.cards.len(), 1);
}

#[test]
fn proposal_capacity_expiry_and_counter_are_checked() {
    // Break caught: unbounded pending proposals, reusing expired IDs or a
    // wrapping counter whose old controls would alias a new proposal.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    for _ in 0..256 {
        issue(
            &mut registry,
            &work,
            access,
            0,
            "Proposal",
            refs.clone(),
            100,
        );
    }
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Overflow".into(),
                source_refs: refs.clone()
            },
            &access,
            &work,
            100
        ),
        Err(WorkError::Full)
    );
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "After expiry",
        refs.clone(),
        400,
    );
    assert_eq!(p.id.sequence, 257);
    registry.sequence = u64::MAX;
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Wrapped".into(),
                source_refs: refs
            },
            &access,
            &work,
            401
        ),
        Err(WorkError::Full)
    );
}

#[test]
fn selected_native_sources_are_exact_and_unrelated_additions_are_inert() {
    // Break caught: comparing all native refs instead of selected immutable
    // refs, or accepting a moved/wrong-kind/foreign source as still authorized.
    let access = personal(7);
    let (mut work, refs) = fixture(access);
    let project = *work.projects.keys().next().unwrap();
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Keep checked allocation",
        refs,
        100,
    );
    work.record_decision(project, access, "Unrelated choice", 100, "extra")
        .unwrap();
    confirm(&mut store, &mut registry, &p, &work, access, 101).unwrap();
    assert!(
        store
            .context(&access.scope(), &access, &work, 102)
            .is_some()
    );
    work.decisions.get_mut(&2).unwrap().project_id = 999;
    assert!(
        store
            .context(&access.scope(), &access, &work, 102)
            .is_none()
    );
    let wrong = BTreeSet::from([WorkContentRef::Task {
        project,
        id: 2,
        revision: 1,
    }]);
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 1,
                presented_text: "Wrong native kind".into(),
                source_refs: wrong
            },
            &access,
            &work,
            103
        ),
        Err(WorkError::Stale)
    );
}

#[test]
fn dm_owner_and_guild_context_never_share_card() {
    // Break caught: generic DM scope or actor-scope mismatch includes a card.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(&mut registry, &work, access, 0, "Private work", refs, 100);
    confirm(&mut store, &mut registry, &p, &work, access, 101).unwrap();
    assert!(
        store
            .context(&personal(8).scope(), &personal(8), &work, 102)
            .is_none()
    );
    assert!(
        store
            .context(&access.scope(), &personal(8), &work, 102)
            .is_none()
    );
    assert!(
        store
            .context(&team().scope(), &team(), &work, 102)
            .is_none()
    );
}

#[test]
fn loading_rejects_unowned_invalid_duplicate_or_oversize_cards() {
    // Break caught: serde bypasses bounds, overwrites duplicate scopes, or
    // reconstructs unowned team/personal text into confirmation authority.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Confirmed exact text",
        refs,
        100,
    );
    confirm(&mut store, &mut registry, &p, &work, access, 101).unwrap();
    let value = serde_json::to_value(&store).unwrap();
    let restored: ContinuityStore = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        restored
            .context(&access.scope(), &access, &work, 102)
            .unwrap()
            .text(),
        "Confirmed exact text"
    );
    for (field, invalid) in [
        ("revision", serde_json::json!(0)),
        ("schema_version", serde_json::json!(2)),
        ("confirmed_by", serde_json::json!(0)),
        ("confirmed_by", serde_json::json!(8)),
        ("confirmed_text", serde_json::json!("é".repeat(801))),
        ("expires_at", serde_json::json!(1)),
    ] {
        let mut bad = value.clone();
        bad["cards"][0][field] = invalid;
        assert!(
            serde_json::from_value::<ContinuityStore>(bad).is_err(),
            "{field}"
        );
    }
    let mut missing = value.clone();
    missing["cards"][0]
        .as_object_mut()
        .unwrap()
        .remove("confirmed_by");
    assert!(serde_json::from_value::<ContinuityStore>(missing).is_err());
    let card = value["cards"][0].clone();
    assert!(
        serde_json::from_value::<ContinuityStore>(
            serde_json::json!({"cards": [card.clone(), card]})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ContinuityStore>(
            serde_json::json!({"cards": vec![value["cards"][0].clone(); 257]})
        )
        .is_err()
    );
}

#[test]
fn expiry_and_revision_overflow_refuse_without_replacement() {
    // Break caught: saturating expiry/revision gives a permanently admissible
    // card or overwrites state on a rejected overflow.
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Overflow".into(),
                source_refs: refs.clone()
            },
            &access,
            &work,
            u64::MAX
        ),
        Err(WorkError::Invalid)
    );
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Initial",
        refs.clone(),
        100,
    );
    confirm(&mut store, &mut registry, &p, &work, access, 100).unwrap();
    store.cards.get_mut(&access.scope()).unwrap().revision = u64::MAX;
    let p = issue(
        &mut registry,
        &work,
        access,
        u64::MAX,
        "Revision overflow",
        refs,
        101,
    );
    let before = store.clone();
    assert_eq!(
        confirm(&mut store, &mut registry, &p, &work, access, 102),
        Err(WorkError::Full)
    );
    assert_eq!(store, before);
}

#[test]
fn proposal_id_encoding_binds_nonce_and_rejects_noncanonical_controls() {
    // Break caught: an ambiguous/truncated ID resolves to a newer proposal.
    let id = ProposalId {
        boot_nonce: [1; 16],
        sequence: 9,
    };
    assert_eq!(id.encode(), "01010101010101010101010101010101:9");
    assert_eq!(
        ProposalId::decode("01010101010101010101010101010101:9"),
        Ok(id)
    );
    for invalid in [
        "1:9",
        "01010101010101010101010101010101:09",
        "01010101010101010101010101010101:0",
        "01010101010101010101010101010101:9:1",
        "zz010101010101010101010101010101:9",
    ] {
        assert_eq!(ProposalId::decode(invalid), Err(WorkError::Invalid));
    }
}
