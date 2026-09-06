//! Versioned, content-free score production. Routing only consumes these scores.
use super::ProviderCapabilities;
use super::circuit::ProviderFailureKind;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct NormalizedScore(f64);
impl NormalizedScore {
    pub fn new(value: f64) -> Result<Self, ScoreError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(if value == 0.0 { 0.0 } else { value }))
        } else {
            Err(ScoreError::InvalidNumber)
        }
    }
    pub const fn get(self) -> f64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreError {
    InvalidNumber,
    InvalidDuration,
    InvalidQualification,
    CapabilityMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestClass {
    TextReadOnly,
    TextWithTools,
    VisionDescribe,
    VisionOcr,
}
impl RequestClass {
    /// Voice, summaries and unsolicited calls pass false and cannot construct tools.
    pub const fn text(with_tools: bool) -> Self {
        if with_tools {
            Self::TextWithTools
        } else {
            Self::TextReadOnly
        }
    }
    pub const fn image(ocr: bool) -> Self {
        if ocr {
            Self::VisionOcr
        } else {
            Self::VisionDescribe
        }
    }

    pub const ALL: [Self; 4] = [
        Self::TextReadOnly,
        Self::TextWithTools,
        Self::VisionDescribe,
        Self::VisionOcr,
    ];
    pub const fn mandatory_mask(self) -> u8 {
        match self {
            Self::TextReadOnly => 7,
            Self::TextWithTools => 15,
            Self::VisionDescribe | Self::VisionOcr => 3,
        }
    }
    pub const fn latency_bounds(self) -> (u64, u64) {
        match self {
            Self::TextReadOnly => (1000, 30000),
            Self::TextWithTools => (1500, 45000),
            Self::VisionDescribe | Self::VisionOcr => (2000, 60000),
        }
    }
    pub const fn supported_by(self, caps: ProviderCapabilities) -> bool {
        match self {
            Self::TextReadOnly => caps.text,
            Self::TextWithTools => caps.text && caps.tools,
            Self::VisionDescribe => caps.vision,
            Self::VisionOcr => caps.ocr,
        }
    }
}

/// Evidence from validated addresses/hops, never an inference made by the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLocality {
    SameHost,
    PrivateNetwork,
    PublicRemote,
}
impl ExecutionLocality {
    pub fn address(address: IpAddr) -> Self {
        match address {
            IpAddr::V4(ip) if ip.is_loopback() => Self::SameHost,
            IpAddr::V4(ip) if ip.is_private() => Self::PrivateNetwork,
            IpAddr::V6(ip) if ip.is_loopback() => Self::SameHost,
            IpAddr::V6(ip) if ip.segments()[0] & 0xfe00 == 0xfc00 => Self::PrivateNetwork,
            _ => Self::PublicRemote,
        }
    }
    /// Include every resolved address and redirect hop; empty evidence is remote.
    pub fn least_local(hops: impl IntoIterator<Item = Self>) -> Self {
        hops.into_iter().max().unwrap_or(Self::PublicRemote)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationScoreEvidence {
    pub request_class: RequestClass,
    pub successful_attempts: u8,
    pub mandatory_check_mask: u8,
    pub successful_duration_ms: Vec<u64>,
    pub locality: ExecutionLocality,
}
/// A failed/malformed duration makes an attempt unsuccessful even if its checks passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualificationAttempt {
    pub succeeded: bool,
    pub duration_ms: Option<u64>,
    pub mandatory_check_mask: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreComponents {
    pub quality: NormalizedScore,
    pub reliability: NormalizedScore,
    pub latency: NormalizedScore,
    pub locality: NormalizedScore,
}
impl ScoreComponents {
    pub fn weighted(self) -> NormalizedScore {
        NormalizedScore(
            0.40 * self.quality.0
                + 0.30 * self.reliability.0
                + 0.25 * self.latency.0
                + 0.05 * self.locality.0,
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveComponent {
    ewma: Option<NormalizedScore>,
    count: u64,
}
impl LiveComponent {
    const fn new() -> Self {
        Self {
            ewma: None,
            count: 0,
        }
    }
    fn observe(&mut self, value: NormalizedScore) {
        self.ewma = Some(match self.ewma {
            None => value,
            Some(old) => NormalizedScore(0.2 * value.0 + 0.8 * old.0),
        });
        self.count = self.count.saturating_add(1);
    }
    pub const fn count(self) -> u64 {
        self.count
    }
    pub const fn ewma(self) -> Option<NormalizedScore> {
        self.ewma
    }
    pub fn blended(self, baseline: NormalizedScore) -> NormalizedScore {
        match self.ewma {
            None => baseline,
            Some(live) => {
                let n = self.count.min(20) as f64 / 20.0;
                NormalizedScore(baseline.0 * (1.0 - n) + live.0 * n)
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderScoreProfile {
    request_class: RequestClass,
    baseline: ScoreComponents,
    pub quality: LiveComponent,
    pub reliability: LiveComponent,
    pub latency: LiveComponent,
}
impl ProviderScoreProfile {
    fn new(request_class: RequestClass, baseline: ScoreComponents) -> Self {
        Self {
            request_class,
            baseline,
            quality: LiveComponent::new(),
            reliability: LiveComponent::new(),
            latency: LiveComponent::new(),
        }
    }
    pub const fn request_class(&self) -> RequestClass {
        self.request_class
    }
    pub const fn baseline(&self) -> ScoreComponents {
        self.baseline
    }
    pub fn components(&self) -> ScoreComponents {
        ScoreComponents {
            quality: self.quality.blended(self.baseline.quality),
            reliability: self.reliability.blended(self.baseline.reliability),
            latency: self.latency.blended(self.baseline.latency),
            locality: self.baseline.locality,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreProducerPolicy {
    V1,
}
impl ScoreProducerPolicy {
    pub fn latency(
        self,
        class: RequestClass,
        duration_ms: u64,
    ) -> Result<NormalizedScore, ScoreError> {
        let (fast, slow) = class.latency_bounds();
        let value = if duration_ms <= fast {
            1.0
        } else if duration_ms >= slow {
            0.0
        } else {
            slow.checked_sub(duration_ms)
                .ok_or(ScoreError::InvalidDuration)? as f64
                / slow.checked_sub(fast).ok_or(ScoreError::InvalidDuration)? as f64
        };
        NormalizedScore::new(value)
    }
    pub const fn locality(self, locality: ExecutionLocality) -> NormalizedScore {
        NormalizedScore(match locality {
            ExecutionLocality::SameHost => 1.0,
            ExecutionLocality::PrivateNetwork => 0.5,
            ExecutionLocality::PublicRemote => 0.0,
        })
    }
    pub fn qualify(
        self,
        class: RequestClass,
        attempts: &[QualificationAttempt; 5],
        locality: ExecutionLocality,
    ) -> Result<QualificationScoreEvidence, ScoreError> {
        let mut durations = Vec::new();
        let mut mask = 0;
        for attempt in attempts {
            if attempt.mandatory_check_mask & !class.mandatory_mask() != 0 {
                return Err(ScoreError::InvalidQualification);
            }
            if let Some(duration) = attempt
                .duration_ms
                .filter(|d| *d <= 900000 && attempt.succeeded)
            {
                durations.push(duration);
                mask |= attempt.mandatory_check_mask;
            }
        }
        let evidence = QualificationScoreEvidence {
            request_class: class,
            successful_attempts: durations.len() as u8,
            mandatory_check_mask: mask,
            successful_duration_ms: durations,
            locality,
        };
        self.qualification(&evidence)?;
        Ok(evidence)
    }
    pub fn qualification(
        self,
        evidence: &QualificationScoreEvidence,
    ) -> Result<ProviderScoreProfile, ScoreError> {
        if !(4..=5).contains(&evidence.successful_attempts)
            || evidence.successful_duration_ms.len() != usize::from(evidence.successful_attempts)
            || evidence.mandatory_check_mask != evidence.request_class.mandatory_mask()
            || evidence.successful_duration_ms.iter().any(|d| *d > 900000)
        {
            return Err(ScoreError::InvalidQualification);
        }
        let p95 = nearest_rank_p95(&evidence.successful_duration_ms)
            .ok_or(ScoreError::InvalidQualification)?;
        Ok(ProviderScoreProfile::new(
            evidence.request_class,
            ScoreComponents {
                quality: NormalizedScore(1.0),
                reliability: NormalizedScore(f64::from(evidence.successful_attempts) / 5.0),
                latency: self.latency(evidence.request_class, p95)?,
                locality: self.locality(evidence.locality),
            },
        ))
    }
    /// Called only after the legacy identity and configured boundary were validated.
    pub fn compatibility(
        self,
        class: RequestClass,
        capabilities: ProviderCapabilities,
        locality: ExecutionLocality,
    ) -> Result<ProviderScoreProfile, ScoreError> {
        if !class.supported_by(capabilities) {
            return Err(ScoreError::CapabilityMismatch);
        }
        Ok(ProviderScoreProfile::new(
            class,
            ScoreComponents {
                quality: NormalizedScore(1.0),
                reliability: NormalizedScore(1.0),
                latency: NormalizedScore(0.5),
                locality: self.locality(locality),
            },
        ))
    }
    /// Runtime serializes calls in completion order; failed successes are protocol drift.
    pub fn observe(
        self,
        profile: &mut ProviderScoreProfile,
        outcome: ProviderFailureKind,
        duration_ms: Option<u64>,
    ) -> Result<(), ScoreError> {
        if outcome == ProviderFailureKind::Success {
            let duration = duration_ms
                .filter(|d| *d <= 900000)
                .ok_or(ScoreError::InvalidDuration)?;
            let latency = self.latency(profile.request_class, duration)?;
            profile.quality.observe(NormalizedScore(1.0));
            profile.reliability.observe(NormalizedScore(1.0));
            profile.latency.observe(latency);
        } else if outcome.is_transient() || outcome.is_blocked() {
            profile.reliability.observe(NormalizedScore(0.0));
        }
        Ok(())
    }
}
pub fn nearest_rank_p95(durations: &[u64]) -> Option<u64> {
    if durations.is_empty() {
        return None;
    }
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let rank = sorted.len().checked_mul(95)?.div_ceil(100);
    sorted.get(rank.checked_sub(1)?).copied()
}
#[cfg(test)]
#[path = "scoring_tests.rs"]
mod tests;
