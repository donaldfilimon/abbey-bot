//! Clear/attributed erasure and retained receipt join behavior.
use super::tests::{confirm, fixture, issue, personal, team};
use super::*;
fn seeded(access: WorkAccess) -> (WorkStore, ContinuityStore) {
    let (work, refs) = fixture(access);
    let mut registry = ProposalRegistry::new([3; 16]);
    let p = issue(
        &mut registry,
        &work,
        access,
        0,
        "Exact confirmed card",
        refs,
        100,
    );
    let mut cards = ContinuityStore::default();
    confirm(&mut cards, &mut registry, &p, &work, access, 101).unwrap();
    (work, cards)
}
#[test]
fn continuity_clear_is_exact_authorized_and_preserves_native_work() {
    let access = team();
    let (work, mut cards) = seeded(access);
    let before = work.clone();
    let denied = WorkAccess { actor: 8, ..access };
    assert_eq!(
        cards.clear(&access.scope(), &denied, &work),
        Err(WorkError::Denied)
    );
    assert_eq!(cards.cards.len(), 1);
    assert!(
        cards
            .clear(&access.scope(), &access, &work)
            .unwrap()
            .is_some()
    );
    assert!(cards.cards.is_empty());
    assert_eq!(work, before);
    assert!(
        cards
            .clear(&access.scope(), &access, &work)
            .unwrap()
            .is_none()
    );
}
#[test]
fn continuity_erasure_matches_confirmer_and_current_selected_contributor_only() {
    let access = team();
    let (mut work, cards) = seeded(access);
    assert_eq!(
        cards
            .erasure_targets("discord:10", Some(access.actor), &work)
            .len(),
        1
    );
    assert!(
        cards
            .erasure_targets("discord:11", Some(access.actor), &work)
            .is_empty()
    );
    assert!(
        cards
            .erasure_targets("discord:10", Some(8), &work)
            .is_empty()
    );
    work.decisions.values_mut().next().unwrap().author = 8;
    assert_eq!(cards.erasure_targets("discord:10", Some(8), &work).len(), 1);
    assert_eq!(cards.erasure_targets("discord:10", None, &work).len(), 1);
    let access = personal(7);
    let (work, cards) = seeded(access);
    assert_eq!(
        cards.erasure_targets("discord:dm:7", Some(7), &work).len(),
        1
    );
    assert!(
        cards
            .erasure_targets("discord:10", Some(7), &work)
            .is_empty()
    );
    assert!(
        cards
            .erasure_targets("discord:dm:8", Some(7), &work)
            .is_empty()
    );
}
#[test]
fn continuity_receipt_bounds_reject_malformed_and_keep_inert_receipted_cards() {
    let access = team();
    let (_work, cards) = seeded(access);
    for bad in [
        "x".repeat(64),
        "A".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
    ] {
        let mut json = serde_json::to_value(&cards).unwrap();
        json["cards"][0]["episode_receipt"] = bad.into();
        assert!(serde_json::from_value::<ContinuityStore>(json).is_err());
    }
}

#[test]
fn continuity_inert_receipt_join_survives_restart_pruning_until_admitted_delete() {
    let access = team();
    let (work, cards) = seeded(access);
    let mut json = serde_json::to_value(&cards).unwrap();
    json["cards"][0]["episode_receipt"] = "a".repeat(64).into();
    let mut receipted: ContinuityStore = serde_json::from_value(json).unwrap();
    receipted.prune_current(&work, 604901);
    assert_eq!(
        receipted.cards.len(),
        1,
        "load/final snapshot must preserve the admitted receipt join"
    );
    assert!(
        receipted
            .context(&access.scope(), &access, &work, 604901)
            .is_none()
    );
}
