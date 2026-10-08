//! Pure classification, aggregation and comparison over content-free observations.
use super::*;

pub(super) fn classify(failure: Option<&Failure>) -> Outcome {
    match failure {
        None => Outcome::Success,
        Some(Failure::Provider(
            crate::provider::ProviderFailureKind::Cancelled
            | crate::provider::ProviderFailureKind::Timeout,
        )) => Outcome::Incomplete,
        Some(_) => Outcome::Failure,
    }
}

pub(super) fn p95(values: &[u64]) -> Measurement {
    if values.is_empty() {
        return Measurement::missing(Missing::NoSamples);
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    // ceil(.95*n)-1, with integer arithmetic and no floating-point rounding.
    let rank = values.len() - values.len() / 20;
    Measurement::observed(values[rank - 1])
}

pub(super) fn summarize(probes: &[Probe], population: Population) -> Summary {
    Summary {
        planned: population.count(),
        counts: Counts {
            attempted: probes.len(),
            success: probes
                .iter()
                .filter(|p| p.outcome == Outcome::Success)
                .count(),
            failure: probes
                .iter()
                .filter(|p| p.outcome == Outcome::Failure)
                .count(),
            incomplete: probes
                .iter()
                .filter(|p| p.outcome == Outcome::Incomplete)
                .count(),
            no_text: probes.iter().filter(|p| p.no_text).count(),
        },
        stages: Stage::ALL
            .into_iter()
            .map(|stage| {
                let values: Vec<_> = probes
                    .iter()
                    .filter_map(|p| p.stage(stage).duration_ms)
                    .collect();
                Distribution {
                    stage,
                    observed: values.len(),
                    denominator: probes.len(),
                    p95: p95(&values),
                }
            })
            .collect(),
        small_sample_witness: population == Population::DiscordWitness6,
    }
}

pub(super) fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn validate(receipt: &Receipt) -> Result<(), &'static str> {
    let id = &receipt.identity;
    if receipt.version != 1
        || id.workload != WORKLOAD
        || [
            &id.artifact_sha256,
            &id.declared_installed_sha256,
            &id.provider_config_sha256,
            &id.model_sha256,
            &id.hardware_sha256,
            &id.os_sha256,
        ]
        .into_iter()
        .any(|v| !hash_valid(v))
        || id.artifact_sha256 != id.declared_installed_sha256
    {
        return Err("invalid_identity");
    }
    if receipt.probes.len() != receipt.population.count() {
        return Err("incomplete_population");
    }
    if id.provider.as_str() != receipt.measurement_mode.provider() {
        return Err("measurement_provider_mismatch");
    }
    let mut ordinals = std::collections::BTreeSet::new();
    for probe in &receipt.probes {
        if probe.ordinal >= receipt.population.count()
            || !ordinals.insert(probe.ordinal)
            || classify(probe.failure.as_ref()) != probe.outcome
            || Stage::ALL.into_iter().any(|s| !probe.stage(s).valid())
            || probe.no_text
                && (probe.provider_first_text.duration_ms.is_some()
                    || probe.outcome == Outcome::Success)
        {
            return Err("invalid_observation");
        }
        // Applicability comes from the exact measured route/mode on BOTH
        // receipts. A baseline's missing-reason label is never authority.
        if receipt.measurement_mode.streaming() {
            if probe.provider_first_text.missing == Some(Missing::NonStreaming) {
                return Err("invalid_streaming_applicability");
            }
        } else if probe.provider_first_text != Measurement::missing(Missing::NonStreaming) {
            return Err("invalid_streaming_applicability");
        }
        if probe.outcome == Outcome::Success {
            let necessary = match receipt.population {
                Population::SyntheticProvider48 => {
                    probe.queue_wait.duration_ms.is_some()
                        && probe.provider_completed.duration_ms.is_some()
                        && (!receipt.measurement_mode.streaming()
                            || probe.provider_first_text.duration_ms.is_some())
                }
                Population::DiscordWitness6 => {
                    probe.first_visible.duration_ms.is_some()
                        && probe.final_delivered.duration_ms.is_some()
                }
            };
            if !necessary {
                return Err("required_stage_unobserved");
            }
        }
        if receipt.population == Population::SyntheticProvider48
            && [&probe.first_visible, &probe.final_delivered]
                .into_iter()
                .any(|m| *m != Measurement::missing(Missing::NotApplicable))
        {
            return Err("synthetic_discord_stage");
        }
    }
    Ok(())
}

pub(super) fn compare(baseline: &Receipt, candidate: &Receipt) -> Result<(), &'static str> {
    validate(baseline)?;
    validate(candidate)?;
    let a = &baseline.identity;
    let b = &candidate.identity;
    if baseline.population != candidate.population
        || baseline.measurement_mode != candidate.measurement_mode
        || a.provider != b.provider
        || a.provider_config_sha256 != b.provider_config_sha256
        || a.model_sha256 != b.model_sha256
        || a.hardware_sha256 != b.hardware_sha256
        || a.os_sha256 != b.os_sha256
        || a.workload != b.workload
    {
        return Err("population_identity_mismatch");
    }
    let a = summarize(&baseline.probes, baseline.population);
    let b = summarize(&candidate.probes, candidate.population);
    if b.counts.failure > a.counts.failure {
        return Err("failure_count_increased");
    }
    if b.counts.incomplete > a.counts.incomplete {
        return Err("incomplete_count_increased");
    }
    if b.counts.no_text > a.counts.no_text {
        return Err("no_text_count_increased");
    }
    for (old, new) in a.stages.iter().zip(&b.stages) {
        if new.observed < old.observed {
            return Err("stage_coverage_decreased");
        }
        let required = match baseline.population {
            Population::SyntheticProvider48 => {
                matches!(old.stage, Stage::QueueWait | Stage::ProviderCompleted)
                    || old.stage == Stage::ProviderFirstText
                        && baseline.measurement_mode.streaming()
            }
            Population::DiscordWitness6 => {
                matches!(old.stage, Stage::FirstVisible | Stage::FinalDelivered)
            }
        };
        if required && (old.observed == 0 || new.observed == 0) {
            return Err("required_stage_unobserved");
        }
        if let (Some(old), Some(new)) = (old.p95.duration_ms, new.p95.duration_ms)
            && u128::from(new) * 10 > u128::from(old) * 11
        {
            return Err("stage_p95_regressed");
        }
    }
    Ok(())
}
