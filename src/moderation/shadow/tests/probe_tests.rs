//! Intended as moderation::shadow::tests::probe_tests.
use super::*;

fn new(at: u64) -> NewCase {
    match capture(at) {
        Mutation::Capture(new) => new,
        _ => unreachable!(),
    }
}

#[test]
fn exact_existing_probe_preserves_receipt_after_stop_disable_or_scope_removal() {
    let (store, _) = seeded();
    let before = store.clone();
    let mut stopped = policy();
    stopped.stopped = true;
    let mut disabled = policy();
    disabled.scope = ShadowScope::default();
    let mut elsewhere = policy();
    elsewhere.scope.source_channels = BTreeSet::from([9]);
    for current in [policy(), stopped, disabled, elsewhere] {
        assert_eq!(
            store.has_existing_capture(&new(101), &current, POLICY_DIGEST, 101),
            Ok(true)
        );
        assert_eq!(store, before);
    }
}

#[test]
fn absent_existing_probe_is_false_and_cannot_create_under_stop_or_disabled_scope() {
    let store = CaseStore::default();
    let before = store.clone();
    let mut stopped = policy();
    stopped.stopped = true;
    let mut disabled = policy();
    disabled.scope = ShadowScope::default();
    for current in [stopped, disabled] {
        let candidate = new(101);
        assert_eq!(
            store.has_existing_capture(&candidate, &current, POLICY_DIGEST, 101),
            Ok(false)
        );
        let mut attempted = store.clone();
        assert!(
            attempted
                .apply(Mutation::Capture(candidate), &current, POLICY_DIGEST, 101)
                .is_err()
        );
        assert_eq!(attempted, before);
        assert_eq!(store, before);
    }
}

#[test]
fn conflicting_assessment_and_severity_are_errors_rather_than_new_or_absent() {
    let (store, _) = seeded();
    let before = store.clone();
    for (assessment, severity) in [
        (Assessment::Ambiguous, Severity::Severe),
        (Assessment::QuotationOrReport, Severity::Severe),
        (Assessment::ConfirmedOffending, Severity::Minor),
    ] {
        let observed = source();
        let candidate = NewCase::human_assessed(
            observed.clone(),
            input(&observed, assessment, severity),
            authority(3, 101, Some(observed), true),
        )
        .unwrap();
        assert_eq!(
            store.has_existing_capture(&candidate, &policy(), POLICY_DIGEST, 101),
            Err("contextual repeated assessment conflicts; independent review required")
        );
        assert_eq!(store, before);
    }
}

#[test]
fn current_owner_mismatch_denies_and_new_owner_cannot_adopt_old_receipt() {
    let (store, _) = seeded();
    let before = store.clone();
    let mut replaced = policy();
    replaced.owner = 99;
    assert_eq!(
        store.has_existing_capture(&new(101), &replaced, POLICY_DIGEST, 101),
        Err("contextual authority is stale or mismatched")
    );
    let observed = source();
    let mut current_authority = authority(3, 101, Some(observed.clone()), true);
    current_authority.facts.owner = 99;
    let candidate = NewCase::human_assessed(
        observed.clone(),
        input(&observed, Assessment::ConfirmedOffending, Severity::Severe),
        current_authority,
    )
    .unwrap();
    assert_eq!(
        store.has_existing_capture(&candidate, &replaced, POLICY_DIGEST, 101),
        Err("contextual repeated assessment conflicts; independent review required")
    );
    assert_eq!(store, before);
}

#[test]
fn late_probe_rechecks_ttl_future_time_and_staff_even_for_existing_receipt() {
    let (store, _) = seeded();
    let before = store.clone();
    let candidate = new(100);
    assert_eq!(
        store.has_existing_capture(&candidate, &policy(), POLICY_DIGEST, 160),
        Ok(true)
    );
    for now in [99, 161] {
        assert_eq!(
            store.has_existing_capture(&candidate, &policy(), POLICY_DIGEST, now),
            Err("contextual authority is stale or mismatched")
        );
    }
    let observed = source();
    let staff_revoked = NewCase::human_assessed(
        observed.clone(),
        input(&observed, Assessment::ConfirmedOffending, Severity::Severe),
        authority(3, 101, Some(observed), false),
    )
    .unwrap();
    assert_eq!(
        store.has_existing_capture(&staff_revoked, &policy(), POLICY_DIGEST, 101),
        Err("current contextual staff authority is unproved")
    );
    assert_eq!(store, before);
}

#[test]
fn changed_source_is_absent_and_stopped_probe_never_adds_new_version() {
    let (store, _) = seeded();
    let before = store.clone();
    let changed = SourceVersion::capture(1, 7, 8, 4, 50, Some(101), "edited source").unwrap();
    let candidate = NewCase::human_assessed(
        changed.clone(),
        input(&changed, Assessment::ConfirmedOffending, Severity::Severe),
        authority(3, 101, Some(changed), true),
    )
    .unwrap();
    let mut stopped = policy();
    stopped.stopped = true;
    assert_eq!(
        store.has_existing_capture(&candidate, &stopped, POLICY_DIGEST, 101),
        Ok(false)
    );
    assert_eq!(store, before);
    let mut attempted = store.clone();
    assert!(
        attempted
            .apply(Mutation::Capture(candidate), &stopped, POLICY_DIGEST, 101)
            .is_err()
    );
    assert_eq!(attempted, before);
}

#[test]
fn missing_store_never_bypasses_digest_guild_or_owner_validation() {
    let store = CaseStore::default();
    let before = store.clone();
    assert_eq!(
        store.has_existing_capture(&new(101), &policy(), "invalid", 101),
        Err("invalid contextual policy digest")
    );
    let mut wrong_guild = policy();
    wrong_guild.guild = 99;
    assert_eq!(
        store.has_existing_capture(&new(101), &wrong_guild, POLICY_DIGEST, 101),
        Err("contextual authority is stale or mismatched")
    );
    let mut wrong_owner = policy();
    wrong_owner.owner = 99;
    assert_eq!(
        store.has_existing_capture(&new(101), &wrong_owner, POLICY_DIGEST, 101),
        Err("contextual authority is stale or mismatched")
    );
    assert_eq!(store, before);
}
