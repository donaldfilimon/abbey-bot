use super::*;

fn probe(ordinal: usize, outcome: Outcome, no_text: bool) -> Probe {
    Probe {
        ordinal,
        outcome,
        no_text,
        failure: match outcome {
            Outcome::Success => None,
            Outcome::Failure => Some(Failure::EmptyResult),
            Outcome::Incomplete => Some(Failure::Provider(
                crate::provider::ProviderFailureKind::Cancelled,
            )),
        },
        queue_wait: Measurement::observed(0),
        provider_first_text: Measurement::missing(Missing::NoText),
        provider_completed: Measurement::observed(10),
        first_visible: Measurement::missing(Missing::NotApplicable),
        final_delivered: Measurement::missing(Missing::NotApplicable),
    }
}
pub(super) fn receipt(population: Population) -> Receipt {
    Receipt {
        version: 1,
        population,
        measurement_mode: MeasurementMode::PrimaryStreaming,
        identity: Identity {
            artifact_sha256: "a".repeat(64),
            declared_installed_sha256: "a".repeat(64),
            provider: crate::provider::ProviderId::parse("primary").unwrap(),
            provider_config_sha256: "b".repeat(64),
            model_sha256: "c".repeat(64),
            hardware_sha256: "d".repeat(64),
            os_sha256: "e".repeat(64),
            workload: WORKLOAD.into(),
        },
        probes: (0..population.count())
            .map(|i| {
                let mut p = probe(i, Outcome::Success, false);
                p.provider_first_text = Measurement::observed(i as u64);
                if population == Population::DiscordWitness6 {
                    p.first_visible = Measurement::observed(i as u64);
                    p.final_delivered = Measurement::observed(i as u64 + 10);
                }
                p
            })
            .collect(),
    }
}

#[test]
fn previous_nonce_workload_receipts_cannot_be_compared() {
    let current = receipt(Population::SyntheticProvider48);
    let mut previous = current.clone();
    previous.identity.workload = "abbey-text-benchmark-v1".into();
    for (baseline, candidate) in [
        (&previous, &current),
        (&current, &previous),
        (&previous, &previous),
    ] {
        assert_eq!(
            aggregate::compare(baseline, candidate),
            Err("invalid_identity")
        );
    }
    assert!(aggregate::compare(&current, &current).is_ok());
}

#[test]
fn benchmark_counts_incomplete_and_empty_results() {
    let probes = [
        probe(0, Outcome::Success, false),
        probe(1, Outcome::Failure, true),
        probe(2, Outcome::Incomplete, true),
    ];
    let s = aggregate::summarize(&probes, Population::SyntheticProvider48);
    assert_eq!(
        (
            s.counts.attempted,
            s.counts.success,
            s.counts.failure,
            s.counts.incomplete,
            s.counts.no_text
        ),
        (3, 1, 1, 1, 2)
    );
    assert_eq!(s.stages[0].p95.duration_ms, Some(0));
    assert_eq!(s.stages[1].p95, Measurement::missing(Missing::NoSamples));
    let encoded = serde_json::to_value(s).unwrap();
    assert!(encoded["stages"][1]["p95"]["duration_ms"].is_null());
}
#[test]
fn nearest_rank_boundaries_and_six_sample_witness() {
    for n in [1, 6, 19, 20, 21, 48] {
        let values: Vec<u64> = (1..=n).rev().collect();
        assert_eq!(
            aggregate::p95(&values).duration_ms,
            Some((95 * n).div_ceil(100))
        );
    }
    let r = receipt(Population::DiscordWitness6);
    let s = aggregate::summarize(&r.probes, r.population);
    assert!(s.small_sample_witness);
    assert_eq!(s.stages[3].p95.duration_ms, Some(5));
}
#[test]
fn identity_changes_require_population_match_and_independent_artifact_binding() {
    let baseline = receipt(Population::SyntheticProvider48);
    let mut candidate = baseline.clone();
    candidate.identity.artifact_sha256 = "f".repeat(64);
    candidate.identity.declared_installed_sha256 = "f".repeat(64);
    assert!(aggregate::compare(&baseline, &candidate).is_ok());
    candidate.identity.hardware_sha256 = "0".repeat(64);
    assert!(aggregate::compare(&baseline, &candidate).is_err());
    candidate = baseline.clone();
    candidate.identity.declared_installed_sha256 = "f".repeat(64);
    assert!(aggregate::compare(&baseline, &candidate).is_err());
}
#[test]
fn fixed_workload_non_regression_rejects_each_count_increase_and_coverage_loss() {
    let baseline = receipt(Population::SyntheticProvider48);
    for outcome in [Outcome::Failure, Outcome::Incomplete] {
        let mut candidate = baseline.clone();
        candidate.probes[0] = probe(0, outcome, false);
        assert!(aggregate::compare(&baseline, &candidate).is_err());
    }
    let mut candidate = baseline.clone();
    candidate.probes[0].no_text = true;
    assert!(aggregate::compare(&baseline, &candidate).is_err());
    candidate = baseline.clone();
    candidate.probes[0].queue_wait = Measurement::missing(Missing::NotObserved);
    assert!(aggregate::compare(&baseline, &candidate).is_err());
    candidate = baseline.clone();
    for p in &mut candidate.probes {
        p.provider_completed = Measurement::observed(12);
    }
    assert!(aggregate::compare(&baseline, &candidate).is_err());
}

#[test]
fn each_nonregression_rule_has_its_own_closed_rejection() {
    let baseline = receipt(Population::SyntheticProvider48);
    for (outcome, failure, expected) in [
        (
            Outcome::Failure,
            Failure::EmptyResult,
            "failure_count_increased",
        ),
        (
            Outcome::Incomplete,
            Failure::Provider(crate::provider::ProviderFailureKind::Timeout),
            "incomplete_count_increased",
        ),
    ] {
        let mut candidate = baseline.clone();
        candidate.probes[0].outcome = outcome;
        candidate.probes[0].failure = Some(failure);
        assert_eq!(aggregate::compare(&baseline, &candidate), Err(expected));
    }
    let mut partial_failure = baseline.clone();
    partial_failure.probes[0].outcome = Outcome::Failure;
    partial_failure.probes[0].failure = Some(Failure::FixtureMismatch);
    let mut candidate = partial_failure.clone();
    candidate.probes[0].no_text = true;
    candidate.probes[0].provider_first_text = Measurement::missing(Missing::NoText);
    assert_eq!(
        aggregate::compare(&partial_failure, &candidate),
        Err("no_text_count_increased")
    );
    candidate = partial_failure.clone();
    candidate.probes[0].queue_wait = Measurement::missing(Missing::NotObserved);
    assert_eq!(
        aggregate::compare(&partial_failure, &candidate),
        Err("stage_coverage_decreased")
    );
    candidate = baseline.clone();
    for p in &mut candidate.probes {
        p.provider_completed = Measurement::observed(11);
    }
    assert!(aggregate::compare(&baseline, &candidate).is_ok());
    for p in &mut candidate.probes {
        p.provider_completed = Measurement::observed(12);
    }
    assert_eq!(
        aggregate::compare(&baseline, &candidate),
        Err("stage_p95_regressed")
    );
}
#[test]
fn missing_identity_population_or_required_stages_never_passes_vacuously() {
    let baseline = receipt(Population::SyntheticProvider48);
    for field in 0..6 {
        let mut candidate = baseline.clone();
        match field {
            0 => candidate.identity.model_sha256.clear(),
            1 => candidate.identity.provider_config_sha256 = "0".repeat(64),
            2 => candidate.identity.os_sha256 = "0".repeat(64),
            3 => candidate.identity.workload = "different".into(),
            4 => candidate.identity.provider = crate::provider::ProviderId::parse("other").unwrap(),
            _ => candidate.population = Population::DiscordWitness6,
        }
        assert!(aggregate::compare(&baseline, &candidate).is_err());
    }
    let mut missing = baseline.clone();
    for p in &mut missing.probes {
        p.provider_first_text = Measurement::missing(Missing::NotObserved);
    }
    assert_eq!(
        aggregate::compare(&missing, &missing),
        Err("required_stage_unobserved")
    );
    missing.measurement_mode = MeasurementMode::FmSystemNonStreaming;
    missing.identity.provider =
        crate::provider::ProviderId::parse("foundation-models-cli").unwrap();
    for p in &mut missing.probes {
        p.provider_first_text = Measurement::missing(Missing::NonStreaming);
    }
    assert!(aggregate::compare(&missing, &missing).is_ok());
    missing.probes.pop();
    assert_eq!(
        aggregate::compare(&missing, &missing),
        Err("incomplete_population")
    );
    let mut invalid = baseline.clone();
    invalid.probes[0].queue_wait = Measurement {
        duration_ms: None,
        missing: None,
    };
    assert_eq!(
        aggregate::compare(&baseline, &invalid),
        Err("invalid_observation")
    );
    invalid = baseline.clone();
    invalid.probes[1].ordinal = 0;
    assert_eq!(
        aggregate::compare(&baseline, &invalid),
        Err("invalid_observation")
    );
    invalid = baseline.clone();
    invalid.probes[0].first_visible = Measurement::observed(0);
    assert_eq!(
        aggregate::compare(&baseline, &invalid),
        Err("synthetic_discord_stage")
    );
}
#[test]
fn classification_is_closed_and_timeout_or_cancel_is_incomplete() {
    use crate::provider::ProviderFailureKind as K;
    assert_eq!(aggregate::classify(None), Outcome::Success);
    for k in [K::Timeout, K::Cancelled] {
        assert_eq!(
            aggregate::classify(Some(&Failure::Provider(k))),
            Outcome::Incomplete
        );
    }
    for f in [
        Failure::EmptyResult,
        Failure::FixtureMismatch,
        Failure::Provider(K::Authentication),
    ] {
        assert_eq!(aggregate::classify(Some(&f)), Outcome::Failure);
    }
    assert_eq!(
        aggregate::p95(&[]),
        Measurement::missing(Missing::NoSamples)
    );
    assert_eq!(aggregate::p95(&[u64::MAX]), Measurement::observed(u64::MAX));
}

#[test]
fn equally_sparse_success_populations_are_refused_on_both_sides() {
    for population in [Population::SyntheticProvider48, Population::DiscordWitness6] {
        let mut sparse = receipt(population);
        for p in &mut sparse.probes[1..] {
            if population == Population::SyntheticProvider48 {
                p.queue_wait = Measurement::missing(Missing::NotObserved);
                p.provider_completed = Measurement::missing(Missing::NotObserved);
            } else {
                p.first_visible = Measurement::missing(Missing::NotObserved);
                p.final_delivered = Measurement::missing(Missing::NotObserved);
            }
        }
        // This recomputed summary is internally accurate, but necessary proof
        // for each claimed success is absent. Identically sparse baseline and
        // candidate must not qualify each other.
        let summary = aggregate::summarize(&sparse.probes, population);
        assert_eq!(summary.counts.success, population.count());
        assert_eq!(
            summary.stages[if population == Population::SyntheticProvider48 {
                0
            } else {
                3
            }]
            .observed,
            1
        );
        assert!(aggregate::compare(&sparse, &sparse).is_err());
    }
}
#[test]
fn baseline_nonstream_labels_cannot_exempt_unobserved_candidate_successes() {
    let mut baseline = receipt(Population::SyntheticProvider48);
    baseline.measurement_mode = MeasurementMode::FmSystemNonStreaming;
    baseline.identity.provider =
        crate::provider::ProviderId::parse("foundation-models-cli").unwrap();
    for p in &mut baseline.probes {
        p.provider_first_text = Measurement::missing(Missing::NonStreaming);
    }
    let mut candidate = baseline.clone();
    for p in &mut candidate.probes {
        p.provider_first_text = Measurement::missing(Missing::NotObserved);
    }
    assert!(aggregate::compare(&baseline, &candidate).is_err());
}

#[test]
fn explicit_mode_and_success_consistency_bind_both_populations() {
    for population in [Population::SyntheticProvider48, Population::DiscordWitness6] {
        let valid = receipt(population);
        let mut invalid = valid.clone();
        invalid.probes[0].no_text = true;
        assert_eq!(aggregate::validate(&invalid), Err("invalid_observation"));
        invalid = valid.clone();
        invalid.measurement_mode = MeasurementMode::FmSystemNonStreaming;
        assert_eq!(
            aggregate::validate(&invalid),
            Err("measurement_provider_mismatch")
        );
        invalid.identity.provider =
            crate::provider::ProviderId::parse("foundation-models-cli").unwrap();
        assert_eq!(
            aggregate::validate(&invalid),
            Err("invalid_streaming_applicability")
        );
        for p in &mut invalid.probes {
            p.provider_first_text = Measurement::missing(Missing::NonStreaming);
        }
        assert!(aggregate::validate(&invalid).is_ok());
        assert!(aggregate::compare(&valid, &invalid).is_err());
        // Failed and interrupted attempts retain genuinely unavailable stages.
        for failure in [
            Failure::FixtureMismatch,
            Failure::Provider(crate::provider::ProviderFailureKind::Cancelled),
        ] {
            let mut partial = valid.clone();
            let p = &mut partial.probes[0];
            p.outcome = aggregate::classify(Some(&failure));
            p.failure = Some(failure);
            p.queue_wait = Measurement::missing(Missing::NotObserved);
            p.provider_completed = Measurement::missing(Missing::NotObserved);
            p.provider_first_text = Measurement::missing(Missing::NoText);
            p.no_text = true;
            if population == Population::DiscordWitness6 {
                p.first_visible = Measurement::missing(Missing::NotObserved);
                p.final_delivered = Measurement::missing(Missing::NotObserved);
            }
            assert!(aggregate::compare(&partial, &partial).is_ok());
        }
    }
}
