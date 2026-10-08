//! Synthetic benchmark receipts are measurements, never capability qualification.
use serde::{Deserialize, Serialize};

mod aggregate;
mod runner;
pub(crate) use runner::{Options, parse, run};

pub(super) const WORKLOAD: &str = "abbey-text-benchmark-v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Population {
    SyntheticProvider48,
    DiscordWitness6,
}

/// Explicit measured route/mode, not a capability admission assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MeasurementMode {
    PrimaryStreaming,
    FmSystemNonStreaming,
}
impl MeasurementMode {
    fn streaming(self) -> bool {
        self == Self::PrimaryStreaming
    }
    fn provider(self) -> &'static str {
        match self {
            Self::PrimaryStreaming => "primary",
            Self::FmSystemNonStreaming => "foundation-models-cli",
        }
    }
}
impl Population {
    fn count(self) -> usize {
        match self {
            Self::SyntheticProvider48 => 48,
            Self::DiscordWitness6 => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Success,
    Failure,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Missing {
    NotObserved,
    NotApplicable,
    AdmissionRefused,
    NoText,
    NonStreaming,
    Interrupted,
    NoSamples,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Measurement {
    duration_ms: Option<u64>,
    missing: Option<Missing>,
}
impl Measurement {
    fn observed(value: u64) -> Self {
        Self {
            duration_ms: Some(value),
            missing: None,
        }
    }
    fn missing(reason: Missing) -> Self {
        Self {
            duration_ms: None,
            missing: Some(reason),
        }
    }
    fn valid(&self) -> bool {
        self.duration_ms.is_some() != self.missing.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    QueueWait,
    ProviderFirstText,
    ProviderCompleted,
    FirstVisible,
    FinalDelivered,
}
impl Stage {
    const ALL: [Self; 5] = [
        Self::QueueWait,
        Self::ProviderFirstText,
        Self::ProviderCompleted,
        Self::FirstVisible,
        Self::FinalDelivered,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    artifact_sha256: String,
    declared_installed_sha256: String,
    provider: crate::provider::ProviderId,
    provider_config_sha256: String,
    model_sha256: String,
    hardware_sha256: String,
    os_sha256: String,
    workload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    Provider(crate::provider::ProviderFailureKind),
    EmptyResult,
    FixtureMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Probe {
    ordinal: usize,
    outcome: Outcome,
    failure: Option<Failure>,
    no_text: bool,
    queue_wait: Measurement,
    provider_first_text: Measurement,
    provider_completed: Measurement,
    first_visible: Measurement,
    final_delivered: Measurement,
}
impl Probe {
    fn stage(&self, stage: Stage) -> &Measurement {
        match stage {
            Stage::QueueWait => &self.queue_wait,
            Stage::ProviderFirstText => &self.provider_first_text,
            Stage::ProviderCompleted => &self.provider_completed,
            Stage::FirstVisible => &self.first_visible,
            Stage::FinalDelivered => &self.final_delivered,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    version: u32,
    population: Population,
    measurement_mode: MeasurementMode,
    identity: Identity,
    probes: Vec<Probe>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Counts {
    attempted: usize,
    success: usize,
    failure: usize,
    incomplete: usize,
    no_text: usize,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Distribution {
    stage: Stage,
    observed: usize,
    denominator: usize,
    p95: Measurement,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Summary {
    counts: Counts,
    planned: usize,
    stages: Vec<Distribution>,
    small_sample_witness: bool,
}

#[cfg(test)]
mod tests;
