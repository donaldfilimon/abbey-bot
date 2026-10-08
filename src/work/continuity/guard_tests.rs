//! Confirmation races and independent boundaries identified by domain review.
use super::tests::{confirm, fixture, issue, personal, team};
use super::*;

#[test]
fn held_grant_rechecks_expiry_and_exact_actor_scope() {
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Exact text",
        refs.clone(),
        100,
    );
    let grant = registry
        .resolve_confirmation(p.id, 7, &access.scope(), 101)
        .unwrap();
    assert_eq!(
        store.confirm(grant, &access, &work, 400),
        Err(WorkError::Stale)
    );
    assert!(store.cards.is_empty());
    for foreign in [personal(8), team()] {
        let p = issue(
            &mut registry,
            &work,
            access,
            0,
            "Exact text",
            refs.clone(),
            401,
        );
        let grant = registry
            .resolve_confirmation(p.id, 7, &access.scope(), 402)
            .unwrap();
        assert_eq!(
            store.confirm(grant, &foreign, &work, 403),
            Err(WorkError::Denied)
        );
        assert!(store.cards.is_empty());
    }
}

#[test]
fn held_grant_rechecks_native_member_and_manager_revocation() {
    let access = team();
    let (work, refs) = fixture(access);
    for revoke_membership in [false, true] {
        let mut current = work.clone();
        let mut registry = ProposalRegistry::new([1; 16]);
        let mut store = ContinuityStore::default();
        let p = issue(
            &mut registry,
            &current,
            access,
            0,
            "Team exact text",
            refs.clone(),
            100,
        );
        let grant = registry
            .resolve_confirmation(p.id, 7, &access.scope(), 101)
            .unwrap();
        let project = current.projects.values_mut().next().unwrap();
        if revoke_membership {
            project.members.remove(&7);
        } else {
            project.managers.remove(&7);
        }
        assert_eq!(
            store.confirm(grant, &access, &current, 102),
            Err(WorkError::Denied)
        );
        assert!(store.cards.is_empty());
    }
}

#[test]
fn loading_card_capacity_uses_distinct_valid_scopes() {
    let access = personal(7);
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(&mut registry, &work, access, 0, "Exact text", refs, 100);
    let card = confirm(&mut store, &mut registry, &p, &work, access, 101).unwrap();
    let cards: Vec<_> = (1..=257)
        .map(|owner| {
            let mut card = card.clone();
            card.scope = WorkScope::Personal { owner };
            card.confirmed_by = owner;
            card
        })
        .collect();
    assert!(
        serde_json::from_value::<ContinuityStore>(serde_json::json!({"cards": &cards[..256]}))
            .is_ok()
    );
    assert!(
        serde_json::from_value::<ContinuityStore>(serde_json::json!({"cards": cards})).is_err()
    );
}

#[test]
fn full_capacity_allows_existing_scope_replacement() {
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    for owner in 1..=256 {
        let access = personal(owner);
        let (work, refs) = fixture(access);
        let p = issue(&mut registry, &work, access, 0, "Initial", refs, 100);
        confirm(&mut store, &mut registry, &p, &work, access, 100).unwrap();
    }
    let access = personal(1);
    let (work, refs) = fixture(access);
    let p = issue(&mut registry, &work, access, 1, "Replacement", refs, 101);
    let card = confirm(&mut store, &mut registry, &p, &work, access, 102).unwrap();
    assert_eq!(card.revision, 2);
    assert_eq!(card.confirmed_text, "Replacement");
    assert_eq!(store.cards.len(), 256);
    assert_eq!(
        store
            .cards
            .get(&personal(2).scope())
            .unwrap()
            .confirmed_text,
        "Initial"
    );
}

#[test]
fn exactly_eight_selected_sources_are_accepted() {
    let access = personal(7);
    let (mut work, mut refs) = fixture(access);
    let project = *work.projects.keys().next().unwrap();
    for i in 0..7 {
        let id = work
            .record_decision(project, access, "Choice", 100, &format!("choice-{i}"))
            .unwrap();
        refs.insert(WorkContentRef::Decision {
            project,
            id,
            revision: 1,
        });
    }
    let mut registry = ProposalRegistry::new([1; 16]);
    let mut store = ContinuityStore::default();
    let p = issue(&mut registry, &work, access, 0, "Eight choices", refs, 100);
    assert_eq!(
        confirm(&mut store, &mut registry, &p, &work, access, 101)
            .unwrap()
            .source_refs
            .len(),
        8
    );
}

#[test]
fn source_from_other_native_scope_is_refused() {
    let access = personal(7);
    let (mut work, _) = fixture(access);
    let project = work
        .create_project(personal(8), "Private", "other")
        .unwrap();
    let id = work
        .record_decision(
            project,
            personal(8),
            "Private choice",
            100,
            "other-decision",
        )
        .unwrap();
    let mut registry = ProposalRegistry::new([1; 16]);
    let refs = BTreeSet::from([WorkContentRef::Decision {
        project,
        id,
        revision: 1,
    }]);
    assert_eq!(
        registry.propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Foreign".into(),
                source_refs: refs
            },
            &access,
            &work,
            100
        ),
        Err(WorkError::Stale)
    );
}
