use super::super::score_fixtures::{assert_components, fixtures};
use super::super::scoring::{ExecutionLocality, QualificationScoreEvidence};
use super::*;
fn id(raw: &str) -> ProviderId {
    ProviderId::parse(raw).unwrap()
}
fn identity() -> ProviderIdentityHashes {
    ProviderIdentityHashes {
        abbey_binary_sha256: "11".repeat(32),
        provider_binary_sha256: None,
        model_sha256: None,
        os_sha256: None,
        tool_schema_sha256: "55".repeat(32),
        sandbox_sha256: None,
    }
}
fn add(r: &mut AdaptiveRouter, name: &str, evidence: &QualificationScoreEvidence) {
    assert!(r.requalify(
        id(name),
        identity(),
        vec![ScoreProducerPolicy::V1.qualification(evidence).unwrap()],
        RouteAdmission::QUALIFIED
    ));
}
#[test]
fn shared_router_scores_are_exact_and_have_no_implicit_priors() {
    for case in fixtures().profiles {
        let mut r = AdaptiveRouter::new(vec![]);
        add(&mut r, "fixture", &case.evidence);
        let (d, _) = r
            .select(case.evidence.request_class, 0, None, &BTreeSet::new())
            .unwrap();
        assert_components(d.components, case.components);
        assert!((d.weighted_score.get() - case.weighted).abs() < 1e-14);
    }
}
#[test]
fn configured_order_then_stable_id_break_exact_ties() {
    let e = &fixtures().profiles[0].evidence;
    for order in [vec![], vec![id("z")], vec![id("z"), id("a")]] {
        let mut r = AdaptiveRouter::new(order.clone());
        add(&mut r, "z", e);
        add(&mut r, "a", e);
        let (d, _) = r
            .select(e.request_class, 0, None, &BTreeSet::new())
            .unwrap();
        assert_eq!(
            d.provider_id,
            if order.is_empty() { id("a") } else { id("z") }
        );
        assert_eq!(
            d.tie_position,
            if order.is_empty() { usize::MAX } else { 0 }
        );
    }
}
#[test]
fn every_hard_gate_excludes_a_sole_or_pinned_candidate_before_scoring() {
    let e = &fixtures().profiles[0].evidence;
    let rows = [
        (
            RouteAdmission {
                configured: false,
                ..RouteAdmission::QUALIFIED
            },
            RouteUnavailableReason::NoConfiguredProvider,
        ),
        (
            RouteAdmission {
                identity_current: false,
                ..RouteAdmission::QUALIFIED
            },
            RouteUnavailableReason::BlockedPendingRequalification,
        ),
        (
            RouteAdmission {
                capability_allowed: false,
                ..RouteAdmission::QUALIFIED
            },
            RouteUnavailableReason::CapabilityUnavailable,
        ),
        (
            RouteAdmission {
                policy_allowed: false,
                ..RouteAdmission::QUALIFIED
            },
            RouteUnavailableReason::PolicyDenied,
        ),
        (
            RouteAdmission {
                budget_available: false,
                ..RouteAdmission::QUALIFIED
            },
            RouteUnavailableReason::BudgetExhausted,
        ),
        (
            RouteAdmission {
                capacity_available: false,
                ..RouteAdmission::QUALIFIED
            },
            RouteUnavailableReason::Busy,
        ),
    ];
    for (admission, reason) in rows {
        for pinned in [None, Some(id("a"))] {
            let mut r = AdaptiveRouter::new(vec![]);
            add(&mut r, "a", e);
            r.set_admission(&id("a"), admission);
            assert_eq!(
                r.select(e.request_class, 0, pinned.as_ref(), &BTreeSet::new())
                    .unwrap_err(),
                reason
            );
        }
    }
}
#[test]
fn open_blocked_and_half_open_capacity_are_exclusions_even_when_pinned() {
    for failure in [
        ProviderFailureKind::Timeout,
        ProviderFailureKind::Authentication,
    ] {
        let mut r = AdaptiveRouter::new(vec![]);
        let e = &fixtures().profiles[0].evidence;
        add(&mut r, "a", e);
        for _ in 0..if failure.is_transient() { 3 } else { 1 } {
            let (_, a) = r
                .select(e.request_class, 0, None, &BTreeSet::new())
                .unwrap();
            r.complete(a, failure, RetryAfter::Absent, None, 0);
        }
        let reason = if failure.is_transient() {
            RouteUnavailableReason::AllOpen
        } else {
            RouteUnavailableReason::BlockedPendingRequalification
        };
        for pin in [None, Some(id("a"))] {
            assert_eq!(
                r.select(e.request_class, 0, pin.as_ref(), &BTreeSet::new())
                    .unwrap_err(),
                reason
            );
        }
        if failure.is_transient() {
            let (_, probe) = r
                .select(e.request_class, 60000, None, &BTreeSet::new())
                .unwrap();
            assert_eq!(
                r.select(e.request_class, 60000, None, &BTreeSet::new())
                    .unwrap_err(),
                RouteUnavailableReason::Busy
            );
            r.complete(
                probe,
                ProviderFailureKind::Cancelled,
                RetryAfter::Absent,
                None,
                60000,
            );
            assert!(
                r.select(e.request_class, 60000, None, &BTreeSet::new())
                    .is_ok()
            );
        } else {
            assert_eq!(
                r.select(e.request_class, u64::MAX, None, &BTreeSet::new())
                    .unwrap_err(),
                reason
            );
            add(&mut r, "a", e);
            assert!(r.select(e.request_class, 0, None, &BTreeSet::new()).is_ok());
        }
    }
}
#[test]
fn metrics_are_class_local_and_stale_work_cannot_update_requalified_identity() {
    let mut r = AdaptiveRouter::new(vec![]);
    let profiles = fixtures()
        .profiles
        .into_iter()
        .filter(|c| c.evidence.locality == ExecutionLocality::SameHost)
        .map(|c| ScoreProducerPolicy::V1.qualification(&c.evidence).unwrap())
        .collect();
    assert!(r.requalify(id("a"), identity(), profiles, RouteAdmission::QUALIFIED));
    let (_, a) = r
        .select(RequestClass::TextReadOnly, 0, None, &BTreeSet::new())
        .unwrap();
    r.complete(a, ProviderFailureKind::Timeout, RetryAfter::Absent, None, 1);
    for class in RequestClass::ALL {
        assert_eq!(
            r.profile(&id("a"), class).unwrap().reliability.count(),
            u64::from(class == RequestClass::TextReadOnly)
        );
    }
    let (_, stale) = r
        .select(RequestClass::TextReadOnly, 2, None, &BTreeSet::new())
        .unwrap();
    add(&mut r, "a", &fixtures().profiles[0].evidence);
    assert_eq!(
        r.complete(
            stale,
            ProviderFailureKind::Authentication,
            RetryAfter::Absent,
            None,
            3
        ),
        None
    );
    assert_eq!(
        r.profile(&id("a"), RequestClass::TextReadOnly)
            .unwrap()
            .reliability
            .count(),
        0
    );
}
#[test]
fn blocked_reliability_updates_once_and_cannot_be_cleared_by_stale_success_or_restart() {
    let mut r = AdaptiveRouter::new(vec![]);
    let e = &fixtures().profiles[0].evidence;
    add(&mut r, "a", e);
    let (_, a) = r
        .select(e.request_class, 0, None, &BTreeSet::new())
        .unwrap();
    let (_, old) = r
        .select(e.request_class, 0, None, &BTreeSet::new())
        .unwrap();
    r.complete(
        a,
        ProviderFailureKind::Authentication,
        RetryAfter::Absent,
        None,
        0,
    );
    assert_eq!(
        r.complete(
            old,
            ProviderFailureKind::Success,
            RetryAfter::Absent,
            Some(0),
            1
        ),
        None
    );
    assert_eq!(
        r.profile(&id("a"), e.request_class)
            .unwrap()
            .reliability
            .count(),
        1
    );
    let mut restarted = AdaptiveRouter::new(vec![]);
    add(&mut restarted, "a", e);
    assert!(restarted.restore_blocked(&id("a"), &identity(), ProviderFailureKind::Authentication));
    assert_eq!(
        restarted
            .select(e.request_class, u64::MAX, None, &BTreeSet::new())
            .unwrap_err(),
        RouteUnavailableReason::BlockedPendingRequalification
    );
}
#[test]
fn conversation_pins_and_the_exact_one_pre_effect_fallback_table() {
    let e = &fixtures().profiles[0].evidence;
    let mut r = AdaptiveRouter::new(vec![id("a"), id("b")]);
    add(&mut r, "a", e);
    add(&mut r, "b", e);
    let (a, _) = r
        .select(e.request_class, 0, None, &BTreeSet::new())
        .unwrap();
    let (b, _) = r
        .select(e.request_class, 0, Some(&id("b")), &BTreeSet::new())
        .unwrap();
    let mut other = ConversationRoute::default();
    other.accept_selection(&b);
    let outcomes = [
        ProviderFailureKind::Success,
        ProviderFailureKind::TransportUnavailable,
        ProviderFailureKind::Timeout,
        ProviderFailureKind::Http5xx,
        ProviderFailureKind::RateLimited,
        ProviderFailureKind::Authentication,
        ProviderFailureKind::Authorization,
        ProviderFailureKind::Configuration,
        ProviderFailureKind::ExecutableIdentity,
        ProviderFailureKind::ModelIdentity,
        ProviderFailureKind::SandboxIdentity,
        ProviderFailureKind::ToolSchema,
        ProviderFailureKind::ResponseSchema,
        ProviderFailureKind::ProtocolDrift,
        ProviderFailureKind::Cancelled,
        ProviderFailureKind::InvalidRequest,
        ProviderFailureKind::Busy,
    ];
    for kind in outcomes {
        for effect in 0..4 {
            let mut c = ConversationRoute::default();
            c.accept_selection(&a);
            match effect {
                1 => c.mark_visible_output(),
                2 => c.mark_tool_dispatched(),
                3 => c.mark_image_submitted(),
                _ => {}
            }
            let eligible = effect == 0
                && (kind.is_transient() || kind.is_blocked() || kind == ProviderFailureKind::Busy);
            assert_eq!(c.begin_fallback(kind), eligible);
            if eligible {
                assert!(c.excluded().contains(&id("a")));
                let (d, _) = r
                    .select(e.request_class, 0, c.selected(), c.excluded())
                    .unwrap();
                assert_eq!(d.provider_id, id("b"));
                c.accept_selection(&d);
                assert!(!c.begin_fallback(kind));
            }
            assert_eq!(other.selected(), Some(&id("b")));
        }
    }
}

#[test]
fn shared_outcome_metrics_cannot_manufacture_cross_class_samples() {
    for case in fixtures().outcomes {
        let e = &fixtures().profiles[0].evidence;
        let mut r = AdaptiveRouter::new(vec![]);
        add(&mut r, "a", e);
        let (_, attempt) = r
            .select(e.request_class, 0, None, &BTreeSet::new())
            .unwrap();
        assert_eq!(
            r.complete(attempt, case.kind, RetryAfter::Absent, Some(0), 0),
            Some(case.kind)
        );
        let p = r.profile(&id("a"), e.request_class).unwrap();
        for (component, expected) in [
            (p.quality, case.quality),
            (p.reliability, case.reliability),
            (p.latency, case.latency),
        ] {
            assert_eq!(component.count(), u64::from(expected.is_some()));
            assert_eq!(component.ewma().map(NormalizedScore::get), expected);
        }
    }
}
#[test]
fn ranking_uses_exact_weights_and_never_reserves_a_losing_probe() {
    // Qualification fixtures provide latency and locality differences independently.
    let mut r = AdaptiveRouter::new(vec![id("low"), id("high")]);
    let mut low = fixtures().profiles[0].evidence.clone();
    low.locality = ExecutionLocality::PublicRemote;
    let mut high = low.clone();
    high.locality = ExecutionLocality::SameHost;
    add(&mut r, "low", &low);
    add(&mut r, "high", &high);
    let (d, _) = r
        .select(low.request_class, 0, None, &BTreeSet::new())
        .unwrap();
    assert_eq!(d.provider_id, id("high"));
    // Open the lower score provider, then let the deadline expire while the other remains eligible.
    for _ in 0..3 {
        let (_, a) = r
            .select(low.request_class, 0, Some(&id("low")), &BTreeSet::new())
            .unwrap();
        r.complete(a, ProviderFailureKind::Timeout, RetryAfter::Absent, None, 0);
    }
    let (d, _) = r
        .select(low.request_class, 60000, None, &BTreeSet::new())
        .unwrap();
    assert_eq!(d.provider_id, id("high"));
    let lower = r
        .snapshot()
        .into_iter()
        .find(|s| s.provider_id == id("low"))
        .unwrap();
    assert!(!lower.circuit.probe_reserved);
    assert_eq!(lower.circuit.phase, super::super::CircuitPhase::Open);
}
#[test]
fn invalid_success_duration_becomes_protocol_drift_before_any_success_metric() {
    for duration in [None, Some(900001), Some(u64::MAX)] {
        let mut r = AdaptiveRouter::new(vec![]);
        let e = &fixtures().profiles[0].evidence;
        add(&mut r, "a", e);
        let (_, a) = r
            .select(e.request_class, 0, None, &BTreeSet::new())
            .unwrap();
        assert_eq!(
            r.complete(
                a,
                ProviderFailureKind::Success,
                RetryAfter::Absent,
                duration,
                0
            ),
            Some(ProviderFailureKind::ProtocolDrift)
        );
        let p = r.profile(&id("a"), e.request_class).unwrap();
        assert_eq!(p.quality.count(), 0);
        assert_eq!(p.latency.count(), 0);
        assert_eq!(p.reliability.ewma().unwrap().get(), 0.0);
    }
}

#[test]
fn shared_rejected_class_evidence_cannot_route_through_declared_capabilities() {
    use super::super::manifest::{ManifestDocument, decode_manifest};
    for case in fixtures().rejections {
        let mut records: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/provider-capability-only-v2.json"
        ))
        .unwrap();
        records[0]["score_policy"] = 1.into();
        records[0]["score_profiles"] =
            serde_json::json!([case.apply(&fixtures().profiles[0].evidence)]);
        let ManifestDocument::V2(manifest) =
            decode_manifest(&serde_json::to_vec(&records).unwrap()).unwrap()
        else {
            panic!()
        };
        let record = &manifest.records()[0];
        let profiles = record
            .score_profile(RequestClass::TextReadOnly, ExecutionLocality::SameHost)
            .into_iter()
            .collect();
        let mut router = AdaptiveRouter::new(vec![]);
        assert!(router.requalify(
            record.provider_id.clone(),
            record.identity.clone(),
            profiles,
            RouteAdmission::QUALIFIED
        ));
        assert_eq!(
            router
                .select(RequestClass::TextReadOnly, 0, None, &BTreeSet::new())
                .unwrap_err(),
            RouteUnavailableReason::CapabilityUnavailable
        );
    }
}
