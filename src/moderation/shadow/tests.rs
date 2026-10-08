use super::*;
use crate::moderation::Severity;

pub(crate) const POLICY_DIGEST: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(crate) fn policy() -> ShadowPolicy {
    ShadowPolicy {
        guild: 1,
        owner: 2,
        stopped: false,
        scope: ShadowScope {
            enabled: true,
            source_channels: BTreeSet::from([7]),
            review_channel: None,
        },
    }
}
pub(crate) fn source() -> SourceVersion {
    SourceVersion::capture(1, 7, 8, 4, 50, None, "synthetic secret source text").unwrap()
}
pub(crate) fn authority(
    actor: u64,
    at: u64,
    observed: Option<SourceVersion>,
    staff: bool,
) -> FreshAuthority {
    FreshAuthority::verified(NativeAuthorityFacts {
        guild: 1,
        owner: 2,
        actor,
        origin: 7,
        at,
        current_member: true,
        can_view_origin: true,
        can_view_source: true,
        complete_permissions: true,
        can_delete: staff,
        can_timeout: staff,
        observed_source: observed,
    })
    .unwrap()
}
pub(crate) fn input(source: &SourceVersion, assessment: Assessment, severity: Severity) -> Input {
    Input {
        guild: source.guild,
        channel: source.channel,
        message: source.message,
        source_author: source.author,
        target: source.author,
        source_matches_scope: true,
        content_available: true,
        target_is_bot: false,
        target_is_staff: false,
        moderator_can_delete: true,
        moderator_can_timeout: true,
        hierarchy_allows: true,
        assessment,
        severity,
    }
}
pub(crate) fn capture(at: u64) -> Mutation {
    let source = source();
    Mutation::Capture(
        NewCase::human_assessed(
            source.clone(),
            input(&source, Assessment::ConfirmedOffending, Severity::Severe),
            authority(3, at, Some(source), true),
        )
        .unwrap(),
    )
}
fn seeded() -> (CaseStore, ExpectedCase) {
    let mut store = CaseStore::default();
    let (id, _) = store
        .apply(capture(100), &policy(), POLICY_DIGEST, 100)
        .unwrap();
    (store, ExpectedCase { id, revision: 1 })
}
fn review(
    expected: &ExpectedCase,
    actor: u64,
    decision: ReviewDecision,
    observed: Option<SourceVersion>,
) -> Mutation {
    Mutation::Review {
        expected: expected.clone(),
        authority: authority(actor, 101, observed, true),
        decision,
    }
}

#[test]
fn default_scope_omits_no_authority_and_bounds_are_explicit() {
    let scope = ShadowScope::default();
    assert!(!scope.enabled);
    assert!(scope.is_disabled_default());
    scope.validate(&BTreeSet::new()).unwrap();
    assert!(
        ShadowScope {
            enabled: true,
            ..scope.clone()
        }
        .validate(&BTreeSet::new())
        .is_err()
    );
    assert!(
        ShadowScope {
            source_channels: BTreeSet::from([0]),
            ..scope.clone()
        }
        .validate(&BTreeSet::new())
        .is_err()
    );
    assert!(policy().scope.validate(&BTreeSet::from([7])).is_err());
    assert!(
        ShadowScope {
            review_channel: Some(0),
            ..scope
        }
        .validate(&BTreeSet::new())
        .is_err()
    );
}
#[test]
fn absent_default_policy_round_trip_preserves_normalized_identity() {
    // Do not add a new default field to old normalized policy authority hashes.
    let old = serde_json::json!({
        "version": 1, "guild": 1, "owner": 2, "mode": "propose",
        "daily_limit": 5, "daily_creations": 2,
        "public_categories": [9], "protected_channels": [],
        "ordinary_roles": [], "membership_matrix": [],
        "assessment": {"enabled": false, "source_channels": [], "review_channel": null, "allowed_kinds": []},
        "actions": []
    });
    let old_policy: crate::community_ops::Policy = serde_json::from_value(old.clone()).unwrap();
    old_policy.validate().unwrap();
    assert!(old_policy.contextual_shadow.is_disabled_default());
    // This is the prior Policy and AssessmentScope struct field order. Exact
    // normalized bytes preserve any existing hash built over that encoding.
    let old_normalized = concat!(
        "{\"version\":1,\"guild\":1,\"owner\":2,\"mode\":\"propose\",",
        "\"daily_limit\":5,\"daily_creations\":2,\"public_categories\":[9],",
        "\"protected_channels\":[],\"ordinary_roles\":[],\"membership_matrix\":[],",
        "\"assessment\":{\"enabled\":false,\"allowed_kinds\":[],\"source_channels\":[],",
        "\"review_channel\":null},\"actions\":[]}"
    );
    assert_eq!(serde_json::to_string(&old_policy).unwrap(), old_normalized);
    assert_eq!(serde_json::to_value(old_policy).unwrap(), old);
}
#[test]
fn exact_source_versions_are_stable_scoped_and_content_free() {
    let source = source();
    assert_eq!(source, self::source());
    assert_eq!(source.case_id().unwrap(), self::source().case_id().unwrap());
    for changed in [
        SourceVersion::capture(1, 7, 9, 4, 50, None, "synthetic secret source text").unwrap(),
        SourceVersion::capture(1, 7, 8, 4, 50, Some(51), "synthetic secret source text").unwrap(),
        SourceVersion::capture(1, 7, 8, 4, 50, None, "synthetic changed source text").unwrap(),
        SourceVersion::capture(5, 7, 8, 4, 50, None, "synthetic secret source text").unwrap(),
    ] {
        assert_ne!(source.case_id().unwrap(), changed.case_id().unwrap());
    }
    assert!(SourceVersion::capture(1, 7, 8, 0, 50, None, "message").is_err());
    assert!(SourceVersion::capture(1, 7, 8, 4, 50, Some(49), "message").is_err());
    assert!(SourceVersion::capture(1, 7, 8, 4, 50, None, " ").is_err());
    let (store, _) = seeded();
    let bytes = serde_json::to_vec(&store).unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("synthetic secret source text")
    );
    let restored: CaseStore = serde_json::from_slice(&bytes).unwrap();
    restored.validate().unwrap();
    assert_eq!(store, restored);
}
#[test]
fn unchanged_capture_deduplicates_and_conflicting_assessment_never_overwrites() {
    let (mut store, expected) = seeded();
    let before = store.clone();
    assert_eq!(
        store
            .apply(capture(101), &policy(), POLICY_DIGEST, 101)
            .unwrap(),
        (expected.id.clone(), Change::AlreadyObserved)
    );
    assert_eq!(store, before);
    let source = source();
    let different_actor = Mutation::Capture(
        NewCase::human_assessed(
            source.clone(),
            input(&source, Assessment::ConfirmedOffending, Severity::Severe),
            authority(5, 101, Some(source.clone()), true),
        )
        .unwrap(),
    );
    assert_eq!(
        store
            .apply(different_actor, &policy(), POLICY_DIGEST, 101)
            .unwrap()
            .1,
        Change::AlreadyObserved
    );
    assert_eq!(store.cases[&expected.id].captured.actor, 3);
    let conflict = Mutation::Capture(
        NewCase::human_assessed(
            source.clone(),
            input(&source, Assessment::Ambiguous, Severity::Severe),
            authority(3, 101, Some(source), true),
        )
        .unwrap(),
    );
    assert!(
        store
            .apply(conflict, &policy(), POLICY_DIGEST, 101)
            .is_err()
    );
    assert_eq!(store, before);
}
#[test]
fn referrals_have_no_sanction_and_confirmed_severities_are_bounded() {
    for assessment in [
        Assessment::ConfirmedOffending,
        Assessment::Ambiguous,
        Assessment::QuotationOrReport,
    ] {
        for severity in [Severity::Minor, Severity::Serious, Severity::Severe] {
            let source = source();
            let new = NewCase::human_assessed(
                source.clone(),
                input(&source, assessment, severity),
                authority(3, 100, Some(source), true),
            )
            .unwrap();
            let mut store = CaseStore::default();
            let (id, _) = store
                .apply(Mutation::Capture(new), &policy(), POLICY_DIGEST, 100)
                .unwrap();
            assert_eq!(store.cases[&id].origin, CaseOrigin::HumanModerator);
            if assessment == Assessment::ConfirmedOffending {
                let Disposition::ConfirmedProposal { timeout_minutes } =
                    store.cases[&id].disposition
                else {
                    panic!("confirmed proposal expected");
                };
                assert!(timeout_minutes.is_none_or(|m| m <= 10));
            } else {
                assert_eq!(store.cases[&id].disposition, Disposition::HumanReview);
            }
        }
    }
}
#[test]
fn native_unknown_wrongscope_missing_view_and_expired_capabilities_deny() {
    let valid = authority(3, 100, Some(source()), true).facts;
    for facts in [
        NativeAuthorityFacts {
            current_member: false,
            ..valid.clone()
        },
        NativeAuthorityFacts {
            can_view_origin: false,
            ..valid.clone()
        },
        NativeAuthorityFacts {
            can_view_source: false,
            ..valid.clone()
        },
        NativeAuthorityFacts {
            complete_permissions: false,
            ..valid.clone()
        },
        NativeAuthorityFacts {
            actor: 0,
            ..valid.clone()
        },
        NativeAuthorityFacts {
            owner: 0,
            ..valid.clone()
        },
    ] {
        assert!(FreshAuthority::verified(facts).is_err());
    }
    for now in [99, 161] {
        let mut store = CaseStore::default();
        assert!(
            store
                .apply(capture(100), &policy(), POLICY_DIGEST, now)
                .is_err()
        );
        assert_eq!(store, CaseStore::default());
    }
    for changed in [
        ShadowPolicy {
            guild: 5,
            ..policy()
        },
        ShadowPolicy {
            owner: 5,
            ..policy()
        },
        ShadowPolicy {
            stopped: true,
            ..policy()
        },
        ShadowPolicy {
            scope: ShadowScope::default(),
            ..policy()
        },
    ] {
        let mut store = CaseStore::default();
        assert!(
            store
                .apply(capture(100), &changed, POLICY_DIGEST, 100)
                .is_err()
        );
        assert_eq!(store, CaseStore::default());
    }
}
#[test]
fn independent_review_requires_current_version_and_is_exactly_idempotent() {
    let (mut store, expected) = seeded();
    let before = store.clone();
    for mutation in [
        review(&expected, 3, ReviewDecision::Agree, Some(source())),
        review(&expected, 4, ReviewDecision::Agree, Some(source())),
        review(&expected, 5, ReviewDecision::Agree, None),
        review(
            &ExpectedCase {
                revision: 2,
                ..expected.clone()
            },
            5,
            ReviewDecision::Agree,
            Some(source()),
        ),
    ] {
        assert!(
            store
                .apply(mutation, &policy(), POLICY_DIGEST, 101)
                .is_err()
        );
        assert_eq!(store, before);
    }
    let mutation = review(&expected, 5, ReviewDecision::Agree, Some(source()));
    assert_eq!(
        store
            .apply(mutation.clone(), &policy(), POLICY_DIGEST, 101)
            .unwrap()
            .1,
        Change::Changed
    );
    let after = store.clone();
    assert_eq!(
        store
            .apply(mutation, &policy(), POLICY_DIGEST, 102)
            .unwrap()
            .1,
        Change::AlreadyObserved
    );
    assert_eq!(store, after);
    assert!(
        store
            .apply(
                review(&expected, 6, ReviewDecision::Disagree, Some(source())),
                &policy(),
                POLICY_DIGEST,
                102
            )
            .is_err()
    );
    assert_eq!(store, after);
}
#[test]
fn unavailable_or_edited_evidence_can_only_record_needs_context() {
    let (mut store, expected) = seeded();
    let edited =
        SourceVersion::capture(1, 7, 8, 4, 50, Some(100), "edited synthetic source").unwrap();
    assert!(
        store
            .apply(
                review(&expected, 5, ReviewDecision::Disagree, Some(edited)),
                &policy(),
                POLICY_DIGEST,
                101
            )
            .is_err()
    );
    store
        .apply(
            review(&expected, 5, ReviewDecision::NeedsContext, None),
            &policy(),
            POLICY_DIGEST,
            101,
        )
        .unwrap();
    assert_eq!(
        store.cases[&expected.id].review.as_ref().unwrap().decision,
        ReviewDecision::NeedsContext
    );
}
#[test]
fn only_subject_appeals_and_only_an_independent_resolver_can_decide() {
    let (mut store, expected) = seeded();
    let appeal = |actor, expected: ExpectedCase| Mutation::Appeal {
        expected,
        authority: authority(actor, 101, None, false),
        reason: AppealReason::ContextMissing,
    };
    for actor in [3, 5] {
        assert!(
            store
                .apply(
                    appeal(actor, expected.clone()),
                    &policy(),
                    POLICY_DIGEST,
                    101
                )
                .is_err()
        );
    }
    let mut changed_origin = authority(4, 101, None, false).facts;
    changed_origin.origin = 9;
    let wrong_origin = Mutation::Appeal {
        expected: expected.clone(),
        authority: FreshAuthority::verified(changed_origin).unwrap(),
        reason: AppealReason::ContextMissing,
    };
    assert!(
        store
            .apply(wrong_origin, &policy(), POLICY_DIGEST, 101)
            .is_err()
    );
    let mutation = appeal(4, expected.clone());
    store
        .apply(mutation.clone(), &policy(), POLICY_DIGEST, 101)
        .unwrap();
    assert_eq!(
        store
            .apply(mutation, &policy(), POLICY_DIGEST, 102)
            .unwrap()
            .1,
        Change::AlreadyObserved
    );
    let current = ExpectedCase {
        revision: 2,
        ..expected.clone()
    };
    let resolver = |actor| Mutation::ResolveAppeal {
        expected: current.clone(),
        authority: authority(actor, 102, Some(source()), true),
        decision: AppealDecision::Upheld,
    };
    for actor in [3, 4] {
        assert!(
            store
                .apply(resolver(actor), &policy(), POLICY_DIGEST, 102)
                .is_err()
        );
    }
    let mutation = resolver(5);
    store
        .apply(mutation.clone(), &policy(), POLICY_DIGEST, 102)
        .unwrap();
    assert_eq!(
        store
            .apply(mutation, &policy(), POLICY_DIGEST, 103)
            .unwrap()
            .1,
        Change::AlreadyObserved
    );
    // A resolved appeal's actor cannot later become its independent reviewer.
    let late = ExpectedCase {
        revision: 3,
        ..expected
    };
    assert!(
        store
            .apply(
                review(&late, 5, ReviewDecision::Agree, Some(source())),
                &policy(),
                POLICY_DIGEST,
                103
            )
            .is_err()
    );
    store.validate().unwrap();
}
#[test]
fn stop_prevents_new_capture_but_preserves_appeal_and_exact_retry() {
    let (mut store, expected) = seeded();
    let stopped = ShadowPolicy {
        stopped: true,
        scope: ShadowScope::default(),
        ..policy()
    };
    assert_eq!(
        store
            .apply(capture(101), &stopped, POLICY_DIGEST, 101)
            .unwrap()
            .1,
        Change::AlreadyObserved
    );
    store
        .apply(
            Mutation::Appeal {
                expected: expected.clone(),
                authority: authority(4, 101, None, false),
                reason: AppealReason::AssessmentDisputed,
            },
            &stopped,
            POLICY_DIGEST,
            101,
        )
        .unwrap();
    assert!(
        store
            .inspect(&expected.id, &authority(4, 101, None, false), &stopped, 101)
            .is_ok()
    );
}
#[test]
fn serialized_receipts_are_not_access_capabilities_and_malformed_state_is_rejected() {
    let (store, expected) = seeded();
    assert!(
        store
            .inspect(
                &expected.id,
                &authority(9, 100, None, false),
                &policy(),
                100
            )
            .is_err()
    );
    let mut broken = store.clone();
    broken.cases.get_mut(&expected.id).unwrap().disposition = Disposition::ConfirmedProposal {
        timeout_minutes: Some(600),
    };
    assert!(broken.validate().is_err());
    let mut broken = store.clone();
    broken.revision = 2;
    assert!(broken.validate().is_err());
    let mut value = serde_json::to_value(store).unwrap();
    value["cases"][&expected.id]["source"]["raw_content"] = serde_json::json!("must be refused");
    assert!(serde_json::from_value::<CaseStore>(value).is_err());
}
#[test]
fn full_store_fails_without_eviction_or_retention_policy() {
    let (mut store, expected) = seeded();
    let template = store.cases[&expected.id].clone();
    for message in 9..=(MAX_CASES as u64 + 7) {
        let mut case = template.clone();
        case.source.message = message;
        case.id = case.source.case_id().unwrap();
        store.cases.insert(case.id.clone(), case);
    }
    store.revision = MAX_CASES as u64;
    store.validate().unwrap();
    let next = SourceVersion::capture(1, 7, 2000, 4, 50, None, "new source").unwrap();
    let capture = NewCase::human_assessed(
        next.clone(),
        input(&next, Assessment::Ambiguous, Severity::Minor),
        authority(3, 100, Some(next), true),
    )
    .unwrap();
    let before = store.clone();
    assert!(
        store
            .apply(Mutation::Capture(capture), &policy(), POLICY_DIGEST, 100)
            .is_err()
    );
    assert_eq!(store, before);
}
#[test]
fn measured_counts_are_partitioned_observations_and_require_staff_access() {
    let (mut store, expected) = seeded();
    store
        .apply(
            review(&expected, 5, ReviewDecision::Disagree, Some(source())),
            &policy(),
            POLICY_DIGEST,
            101,
        )
        .unwrap();
    let counts = store
        .measured_counts(&authority(5, 101, None, true), &policy(), 101)
        .unwrap();
    assert_eq!(counts.confirmed_proposals, 1);
    assert_eq!(counts.review_disagree, 1);
    assert_eq!(counts.review_agree, 0);
    assert_eq!(
        counts.appeal_open
            + counts.appeal_upheld
            + counts.appeal_rejected
            + counts.appeal_needs_context,
        0
    );
    assert!(
        store
            .measured_counts(&authority(4, 101, None, false), &policy(), 101)
            .is_err()
    );
}

mod probe_tests;
