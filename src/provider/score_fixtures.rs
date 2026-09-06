//! One synthetic policy oracle shared by the producer, writer, reader and router tests.
use super::scoring::{ExecutionLocality, QualificationScoreEvidence, RequestClass};
use serde::Deserialize;
#[derive(Deserialize)]
pub struct Fixtures {
    pub profiles: Vec<ProfileCase>,
    pub latency: Vec<LatencyCase>,
    pub locality: Vec<LocalityCase>,
    pub compatibility: Vec<CompatibilityCase>,
    pub outcomes: Vec<OutcomeCase>,
    pub rejections: Vec<RejectionCase>,
    pub partitions: Vec<PartitionCase>,
}
#[derive(Deserialize)]
pub struct ProfileCase {
    pub evidence: QualificationScoreEvidence,
    pub components: [f64; 4],
    pub weighted: f64,
}
#[derive(Deserialize)]
pub struct LatencyCase {
    pub class: RequestClass,
    pub duration_ms: u64,
    pub score: f64,
}
#[derive(Deserialize)]
pub struct LocalityCase {
    pub address: String,
    pub locality: ExecutionLocality,
    pub score: f64,
}
#[derive(Deserialize)]
pub struct CompatibilityCase {
    pub locality: ExecutionLocality,
    pub components: [f64; 4],
    pub weighted: f64,
}
pub fn fixtures() -> Fixtures {
    serde_json::from_str(include_str!("../../tests/fixtures/provider-score-v1.json")).unwrap()
}
pub fn assert_components(actual: super::scoring::ScoreComponents, expected: [f64; 4]) {
    for (a, e) in [
        actual.quality.get(),
        actual.reliability.get(),
        actual.latency.get(),
        actual.locality.get(),
    ]
    .into_iter()
    .zip(expected)
    {
        assert!((a - e).abs() < 1e-14, "{a} != {e}");
    }
}

#[derive(Deserialize)]
pub struct OutcomeCase {
    pub kind: super::circuit::ProviderFailureKind,
    pub quality: Option<f64>,
    pub reliability: Option<f64>,
    pub latency: Option<f64>,
}
#[derive(Deserialize)]
pub struct PartitionCase {
    pub operation: String,
    pub request_class: RequestClass,
}

#[derive(Deserialize)]
pub struct RejectionCase {
    pub field: String,
    pub value: serde_json::Value,
}
impl RejectionCase {
    pub fn apply(&self, evidence: &QualificationScoreEvidence) -> serde_json::Value {
        let mut value = serde_json::to_value(evidence).unwrap();
        value[&self.field] = self.value.clone();
        value
    }
}
