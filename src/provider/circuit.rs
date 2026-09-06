//! Injected monotonic milliseconds; no clocks, sleeps, or provider diagnostics.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureKind {
    Success,
    TransportUnavailable,
    Timeout,
    Http5xx,
    RateLimited,
    Authentication,
    Authorization,
    Configuration,
    ExecutableIdentity,
    ModelIdentity,
    SandboxIdentity,
    ToolSchema,
    ResponseSchema,
    ProtocolDrift,
    Cancelled,
    InvalidRequest,
    Busy,
}
impl ProviderFailureKind {
    pub const fn is_transient(self) -> bool {
        matches!(
            self,
            Self::TransportUnavailable | Self::Timeout | Self::Http5xx | Self::RateLimited
        )
    }
    pub const fn is_blocked(self) -> bool {
        matches!(
            self,
            Self::Authentication
                | Self::Authorization
                | Self::Configuration
                | Self::ExecutableIdentity
                | Self::ModelIdentity
                | Self::SandboxIdentity
                | Self::ToolSchema
                | Self::ResponseSchema
                | Self::ProtocolDrift
        )
    }
    pub const fn is_neutral(self) -> bool {
        matches!(self, Self::Cancelled | Self::InvalidRequest | Self::Busy)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAfter {
    Absent,
    Valid(Duration),
    Invalid,
}
impl RetryAfter {
    pub fn from_duration(value: Duration) -> Self {
        if (Duration::from_secs(1)..=Duration::from_secs(900)).contains(&value) {
            Self::Valid(value)
        } else {
            Self::Invalid
        }
    }
    /// Numeric delay-seconds metadata. Malformed, nonfinite, negative and overflow fail closed.
    pub fn from_seconds(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::Absent;
        };
        value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|seconds| seconds.is_finite() && (1.0..=900.0).contains(seconds))
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .map_or(Self::Invalid, Self::from_duration)
    }
    pub fn classify(self, kind: ProviderFailureKind) -> ProviderFailureKind {
        match self {
            Self::Absent => kind,
            Self::Valid(delay)
                if matches!(
                    kind,
                    ProviderFailureKind::Http5xx | ProviderFailureKind::RateLimited
                ) && matches!(Self::from_duration(delay), Self::Valid(_)) =>
            {
                kind
            }
            _ => ProviderFailureKind::ProtocolDrift,
        }
    }
    fn millis(self) -> Option<u64> {
        match self {
            Self::Valid(d) => u64::try_from(d.as_nanos().div_ceil(1_000_000)).ok(),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitPhase {
    Closed,
    Open,
    HalfOpen,
    Blocked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitSnapshot {
    pub phase: CircuitPhase,
    pub reason: Option<ProviderFailureKind>,
    pub opening_level: u8,
    pub open_until_ms: Option<u64>,
    pub probe_reserved: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitUnavailable {
    Open,
    Blocked,
    Busy,
}
#[derive(Debug, PartialEq, Eq)]
pub struct AttemptPermit(u64);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Circuit {
    snapshot: CircuitSnapshot,
    recent: VecDeque<u64>,
    outstanding: BTreeSet<u64>,
    probe: Option<u64>,
    next_permit: u64,
}
impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}
impl Circuit {
    pub fn new() -> Self {
        Self {
            snapshot: CircuitSnapshot {
                phase: CircuitPhase::Closed,
                reason: None,
                opening_level: 0,
                open_until_ms: None,
                probe_reserved: false,
            },
            recent: VecDeque::new(),
            outstanding: BTreeSet::new(),
            probe: None,
            next_permit: 0,
        }
    }
    pub const fn snapshot(&self) -> CircuitSnapshot {
        self.snapshot
    }
    /// A persisted identity block is restored without treating a restart as qualification.
    pub fn restore_blocked(reason: ProviderFailureKind) -> Option<Self> {
        if !reason.is_blocked() {
            return None;
        }
        let mut circuit = Self::new();
        circuit.block(reason);
        Some(circuit)
    }
    pub fn availability(&self, now_ms: u64) -> Result<(), CircuitUnavailable> {
        match self.snapshot.phase {
            CircuitPhase::Blocked => Err(CircuitUnavailable::Blocked),
            CircuitPhase::Open if now_ms < self.snapshot.open_until_ms.unwrap_or(u64::MAX) => {
                Err(CircuitUnavailable::Open)
            }
            CircuitPhase::HalfOpen if self.probe.is_some() => Err(CircuitUnavailable::Busy),
            _ if self.next_permit == u64::MAX => Err(CircuitUnavailable::Busy),
            _ => Ok(()),
        }
    }
    /// The owner serializes this mutation. Only selection reserves, ranking does not.
    pub fn reserve(&mut self, now_ms: u64) -> Result<AttemptPermit, CircuitUnavailable> {
        self.availability(now_ms)?;
        self.next_permit += 1;
        let permit = self.next_permit;
        if self.snapshot.phase == CircuitPhase::Open {
            self.snapshot.phase = CircuitPhase::HalfOpen;
            self.snapshot.open_until_ms = None;
        }
        if self.snapshot.phase == CircuitPhase::HalfOpen {
            self.probe = Some(permit);
            self.snapshot.probe_reserved = true;
        }
        self.outstanding.insert(permit);
        Ok(AttemptPermit(permit))
    }
    /// Returns a single accepted effective metrics outcome; stale/duplicate/blocked work returns None.
    pub fn complete(
        &mut self,
        permit: AttemptPermit,
        kind: ProviderFailureKind,
        retry: RetryAfter,
        now_ms: u64,
    ) -> Option<ProviderFailureKind> {
        if !self.outstanding.remove(&permit.0) || self.snapshot.phase == CircuitPhase::Blocked {
            return None;
        }
        let kind = retry.classify(kind);
        if kind.is_blocked() {
            self.block(kind);
            return Some(kind);
        }
        let owns_probe = self.probe == Some(permit.0);
        if owns_probe {
            self.probe = None;
            self.snapshot.probe_reserved = false;
        }
        if kind.is_neutral() {
            return Some(kind);
        }
        match self.snapshot.phase {
            CircuitPhase::Closed => {
                self.recent
                    .retain(|time| now_ms.saturating_sub(*time) <= 300000);
                if kind.is_transient() {
                    self.recent.push_back(now_ms);
                    let normal = if self.recent.len() >= 3 { 60000 } else { 0 };
                    if normal != 0 || retry.millis().is_some() {
                        self.open(now_ms, normal.max(retry.millis().unwrap_or(0)), 0, kind);
                    }
                }
            }
            CircuitPhase::Open => {
                if kind.is_transient()
                    && let Some(delay) = retry.millis()
                {
                    self.snapshot.open_until_ms = Some(
                        self.snapshot
                            .open_until_ms
                            .unwrap_or(0)
                            .max(now_ms.saturating_add(delay)),
                    );
                }
            }
            CircuitPhase::HalfOpen if owns_probe => {
                if kind == ProviderFailureKind::Success {
                    self.snapshot = Self::new().snapshot;
                    self.recent.clear();
                } else if kind.is_transient() {
                    let normal = if self.snapshot.opening_level == 0 {
                        300000
                    } else {
                        900000
                    };
                    self.open(
                        now_ms,
                        normal.max(retry.millis().unwrap_or(0)).min(900000),
                        (self.snapshot.opening_level + 1).min(2),
                        kind,
                    );
                }
            }
            // Older in-flight completions may update metrics, never consume or close another caller's probe.
            CircuitPhase::HalfOpen | CircuitPhase::Blocked => {}
        }
        Some(kind)
    }
    fn open(&mut self, now_ms: u64, delay_ms: u64, level: u8, reason: ProviderFailureKind) {
        self.recent.clear();
        self.probe = None;
        self.snapshot = CircuitSnapshot {
            phase: CircuitPhase::Open,
            reason: Some(reason),
            opening_level: level,
            open_until_ms: Some(now_ms.saturating_add(delay_ms)),
            probe_reserved: false,
        };
    }
    fn block(&mut self, reason: ProviderFailureKind) {
        self.recent.clear();
        self.probe = None;
        self.outstanding.clear();
        self.snapshot.phase = CircuitPhase::Blocked;
        self.snapshot.reason = Some(reason);
        self.snapshot.open_until_ms = None;
        self.snapshot.probe_reserved = false;
    }
}
#[cfg(test)]
#[path = "circuit_tests.rs"]
mod tests;
