//! Actual commands-owned registry authority and private presentation.
use super::*;
use crate::work::continuity::ContinuityStore;
use std::collections::BTreeSet;
fn fixture() -> (HumanContinuity, WorkAccess, WorkStore) {
    let access = WorkAccess {
        actor: 7,
        channel: 70,
        guild: None,
        can_view: true,
        can_manage: false,
    };
    let mut work = WorkStore::default();
    work.create_project(access, "Allocator", "project").unwrap();
    (HumanContinuity::new().unwrap(), access, work)
}
fn draft(access: WorkAccess) -> ContinuityDraft {
    ContinuityDraft {
        scope: access.scope(),
        base_revision: 0,
        presented_text: "Exact human preview".into(),
        source_refs: BTreeSet::new(),
    }
}
#[test]
fn continuity_shell_binds_generation_actor_and_consumes_exact_proposal_once() {
    let (shell, access, work) = fixture();
    let p = shell
        .propose(draft(access), &access, &work, 1, 100)
        .unwrap();
    shell.mark_presented(&p, &access, 1, 100).unwrap();
    let mut other = access;
    other.actor = 8;
    assert!(matches!(
        shell.resolve(p.id, &other, 1, 100),
        Err(WorkError::Denied)
    ));
    let (grant, generation) = shell.resolve(p.id, &access, 1, 100).unwrap();
    assert_eq!(generation, 1);
    let mut cards = ContinuityStore::default();
    assert_eq!(
        cards
            .confirm(grant, &access, &work, 100)
            .unwrap()
            .confirmed_text,
        "Exact human preview"
    );
    assert!(shell.resolve(p.id, &access, 1, 100).is_err());
}
#[test]
fn continuity_shell_clear_epoch_and_failed_preview_cannot_be_confirmed() {
    let (shell, access, work) = fixture();
    let p = shell
        .propose(draft(access), &access, &work, 1, 100)
        .unwrap();
    assert!(matches!(
        shell.resolve(p.id, &access, 2, 100),
        Err(WorkError::Stale)
    ));
    let p = shell
        .propose(draft(access), &access, &work, 2, 100)
        .unwrap();
    shell.discard(p.id, &access, 100);
    assert!(shell.resolve(p.id, &access, 2, 100).is_err());
}

#[test]
fn continuity_shell_requires_observed_private_preview_before_confirmation() {
    let (shell, access, work) = fixture();
    let p = shell
        .propose(draft(access), &access, &work, 1, 100)
        .unwrap();
    assert!(matches!(
        shell.resolve(p.id, &access, 1, 100),
        Err(WorkError::Stale)
    ));
    shell.mark_presented(&p, &access, 1, 100).unwrap();
    assert!(shell.resolve(p.id, &access, 1, 100).is_ok());
}
