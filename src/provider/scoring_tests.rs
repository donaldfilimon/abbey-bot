use super::super::score_fixtures::{assert_components, fixtures};
use super::*;
#[test]
fn normalized_domain_rejects_every_invalid_number() {
    for invalid in [
        -f64::EPSILON,
        1.0 + f64::EPSILON,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert_eq!(
            NormalizedScore::new(invalid),
            Err(ScoreError::InvalidNumber)
        );
    }
    for value in [0.0, -0.0, 0.5, 1.0] {
        assert_eq!(NormalizedScore::new(value).unwrap().get(), value);
    }
}
#[test]
fn shared_v1_profiles_and_latency_are_the_single_producer_policy() {
    for case in fixtures().profiles {
        let profile = ScoreProducerPolicy::V1
            .qualification(&case.evidence)
            .unwrap();
        assert_components(profile.components(), case.components);
        assert!((profile.components().weighted().get() - case.weighted).abs() < 1e-14);
        assert_eq!(profile.request_class(), case.evidence.request_class);
    }
    for case in fixtures().latency {
        assert_eq!(
            ScoreProducerPolicy::V1
                .latency(case.class, case.duration_ms)
                .unwrap()
                .get(),
            case.score
        );
    }
}
#[test]
fn shared_locality_and_compatibility_project_only_the_validated_boundary() {
    for case in fixtures().locality {
        assert_eq!(
            ExecutionLocality::address(case.address.parse().unwrap()),
            case.locality
        );
        assert_eq!(
            ScoreProducerPolicy::V1.locality(case.locality).get(),
            case.score
        );
    }
    for case in fixtures().compatibility {
        let profile = ScoreProducerPolicy::V1
            .compatibility(
                RequestClass::TextReadOnly,
                ProviderCapabilities::text(),
                case.locality,
            )
            .unwrap();
        assert_components(profile.components(), case.components);
        assert!((profile.components().weighted().get() - case.weighted).abs() < 1e-14);
    }
    assert_eq!(
        ExecutionLocality::least_local([
            ExecutionLocality::SameHost,
            ExecutionLocality::PrivateNetwork
        ]),
        ExecutionLocality::PrivateNetwork
    );
    assert_eq!(
        ExecutionLocality::least_local([
            ExecutionLocality::PublicRemote,
            ExecutionLocality::SameHost
        ]),
        ExecutionLocality::PublicRemote
    );
    assert_eq!(
        ExecutionLocality::least_local([]),
        ExecutionLocality::PublicRemote
    );
}
#[test]
fn five_attempt_qualification_requires_four_successes_and_all_mandatory_checks() {
    for case in fixtures().profiles {
        let e = case.evidence;
        let attempts = std::array::from_fn(|i| QualificationAttempt {
            succeeded: i < usize::from(e.successful_attempts),
            duration_ms: e.successful_duration_ms.get(i).copied(),
            mandatory_check_mask: e.mandatory_check_mask,
        });
        assert_eq!(
            ScoreProducerPolicy::V1
                .qualify(e.request_class, &attempts, e.locality)
                .unwrap(),
            e
        );
        let mut bad = attempts;
        bad[0].duration_ms = None;
        bad[1].duration_ms = Some(900001);
        assert!(
            ScoreProducerPolicy::V1
                .qualify(e.request_class, &bad, e.locality)
                .is_err()
        );
        let mut bad = attempts;
        for attempt in &mut bad {
            attempt.mandatory_check_mask &= !1;
        }
        assert!(
            ScoreProducerPolicy::V1
                .qualify(e.request_class, &bad, e.locality)
                .is_err()
        );
        bad[0].mandatory_check_mask = 255;
        assert!(
            ScoreProducerPolicy::V1
                .qualify(e.request_class, &bad, e.locality)
                .is_err()
        );
    }
}
#[test]
fn nearest_rank_p95_is_not_an_interpolated_percentile() {
    assert_eq!(nearest_rank_p95(&[]), None);
    assert_eq!(nearest_rank_p95(&[4, 1, 3, 2]), Some(4));
    assert_eq!(nearest_rank_p95(&[5, 1, 4, 2, 3]), Some(5));
    assert_eq!(
        nearest_rank_p95(&(1..=20).rev().collect::<Vec<_>>()),
        Some(19)
    );
    assert_eq!(nearest_rank_p95(&(1..=100).collect::<Vec<_>>()), Some(95));
}
#[test]
fn component_local_ewmas_use_alpha_point_two_and_n_over_twenty() {
    let e = &fixtures().profiles[0].evidence;
    let mut p = ScoreProducerPolicy::V1.qualification(e).unwrap();
    let baseline = p.baseline();
    ScoreProducerPolicy::V1
        .observe(&mut p, ProviderFailureKind::Timeout, None)
        .unwrap();
    assert_eq!(
        (p.quality.count(), p.reliability.count(), p.latency.count()),
        (0, 1, 0)
    );
    assert_eq!(p.reliability.ewma().unwrap().get(), 0.0);
    assert_eq!(
        p.components().reliability.get(),
        baseline.reliability.get() * 0.95
    );
    ScoreProducerPolicy::V1
        .observe(&mut p, ProviderFailureKind::Success, Some(0))
        .unwrap();
    assert_eq!(
        (p.quality.count(), p.reliability.count(), p.latency.count()),
        (1, 2, 1)
    );
    assert_eq!(p.reliability.ewma().unwrap().get(), 0.2);
    assert_eq!(
        p.components().reliability.get(),
        baseline.reliability.get() * 0.9 + 0.2 * 0.1
    );
    assert_eq!(p.components().latency.get(), 0.5 * 0.95 + 0.05);
    for _ in 0..25 {
        ScoreProducerPolicy::V1
            .observe(&mut p, ProviderFailureKind::Timeout, None)
            .unwrap();
    }
    assert_eq!(p.components().reliability, p.reliability.ewma().unwrap());
    assert_eq!(
        (p.quality.count(), p.reliability.count(), p.latency.count()),
        (1, 27, 1)
    );
    assert_eq!(p.components().locality, baseline.locality);
}
#[test]
fn neutral_and_invalid_success_metrics_are_atomic() {
    let mut p = ScoreProducerPolicy::V1
        .qualification(&fixtures().profiles[0].evidence)
        .unwrap();
    let initial = p.clone();
    for outcome in [
        ProviderFailureKind::Cancelled,
        ProviderFailureKind::InvalidRequest,
        ProviderFailureKind::Busy,
    ] {
        ScoreProducerPolicy::V1
            .observe(&mut p, outcome, Some(0))
            .unwrap();
        assert_eq!(p, initial);
    }
    for duration in [None, Some(900001), Some(u64::MAX)] {
        assert!(
            ScoreProducerPolicy::V1
                .observe(&mut p, ProviderFailureKind::Success, duration)
                .is_err()
        );
        assert_eq!(p, initial);
    }
    ScoreProducerPolicy::V1
        .observe(&mut p, ProviderFailureKind::ResponseSchema, None)
        .unwrap();
    assert_eq!(
        (p.quality.count(), p.reliability.count(), p.latency.count()),
        (0, 1, 0)
    );
}
#[test]
fn classes_cannot_borrow_a_neighbor_capability() {
    let caps = ProviderCapabilities::text();
    for class in RequestClass::ALL {
        assert_eq!(
            class.supported_by(caps),
            class == RequestClass::TextReadOnly
        );
    }
    let caps = ProviderCapabilities {
        ocr: true,
        ..ProviderCapabilities::default()
    };
    assert!(RequestClass::VisionOcr.supported_by(caps));
    assert!(!RequestClass::VisionDescribe.supported_by(caps));
}

#[test]
fn shared_outcome_table_updates_only_comparable_components() {
    for case in fixtures().outcomes {
        let mut profile = ScoreProducerPolicy::V1
            .qualification(&fixtures().profiles[0].evidence)
            .unwrap();
        ScoreProducerPolicy::V1
            .observe(&mut profile, case.kind, Some(0))
            .unwrap();
        for (component, expected) in [
            (profile.quality, case.quality),
            (profile.reliability, case.reliability),
            (profile.latency, case.latency),
        ] {
            assert_eq!(component.count(), u64::from(expected.is_some()));
            assert_eq!(component.ewma().map(NormalizedScore::get), expected);
        }
    }
}
#[test]
fn blend_uses_baseline_at_zero_and_live_at_exactly_twenty() {
    let mut p = ScoreProducerPolicy::V1
        .qualification(&fixtures().profiles[0].evidence)
        .unwrap();
    assert_eq!(p.components(), p.baseline());
    for _ in 0..19 {
        ScoreProducerPolicy::V1
            .observe(&mut p, ProviderFailureKind::Timeout, None)
            .unwrap();
    }
    assert!((p.components().reliability.get() - 0.8 * 0.05).abs() < 1e-14);
    ScoreProducerPolicy::V1
        .observe(&mut p, ProviderFailureKind::Timeout, None)
        .unwrap();
    assert_eq!(p.components().reliability.get(), 0.0);
    assert_eq!(p.reliability.count(), 20);
    assert_eq!(p.components().quality, p.baseline().quality);
    assert_eq!(p.components().latency, p.baseline().latency);
}

#[test]
fn shared_request_partition_maps_text_tool_and_image_effect_requirements() {
    for case in fixtures().partitions {
        let class = match case.operation.as_str() {
            "voice" | "summary" | "unsolicited" => RequestClass::text(false),
            "mention_tools" | "dm_tools" | "persona_ask_tools" => RequestClass::text(true),
            "describe_image" => RequestClass::image(false),
            "ocr_image" => RequestClass::image(true),
            _ => panic!("unknown synthetic operation"),
        };
        assert_eq!(class, case.request_class);
    }
}

#[test]
fn basis_vectors_pin_exact_forty_thirty_twenty_five_five_weights() {
    for (index, weight) in [0.40, 0.30, 0.25, 0.05].into_iter().enumerate() {
        let scores = std::array::from_fn::<_, 4, _>(|i| {
            NormalizedScore::new(if i == index { 1.0 } else { 0.0 }).unwrap()
        });
        let c = ScoreComponents {
            quality: scores[0],
            reliability: scores[1],
            latency: scores[2],
            locality: scores[3],
        };
        assert_eq!(c.weighted().get(), weight);
    }
    assert_eq!(
        NormalizedScore::new(-0.0).unwrap().get().to_bits(),
        0.0_f64.to_bits()
    );
}

#[test]
fn shared_rejection_evidence_never_produces_a_baseline() {
    for case in fixtures().rejections {
        let value = case.apply(&fixtures().profiles[0].evidence);
        if let Ok(evidence) = serde_json::from_value::<QualificationScoreEvidence>(value) {
            assert!(ScoreProducerPolicy::V1.qualification(&evidence).is_err());
        }
    }
}
