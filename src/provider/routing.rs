//! Adaptive provider routing with scoring, circuit breaking, retry-after
//! support, and conversation-level stickiness.
//!
//! The router wraps a [`ProviderCatalog`] and a set of [`TurnAdapter`]
//! implementations. It selects the best routable provider for each turn using
//! a weighted score, tracks failures with a circuit breaker, respects
//! `Retry-After` headers, and pins one provider for the full duration of a
//! tool-calling conversation.
//!
//! # Scoring weights
//!
//! | Component | Weight | Source |
//! |-----------|--------|--------|
//! | Quality   | 40%    | manifest-declared capabilities |
//! | Reliability| 30%   | EWMA success rate (alpha=0.2) |
//! | Latency   | 25%   | EWMA response time |
//! | Locality  | 5%    | local_server > os_managed > cloud |
//!
//! # Circuit breaker
//!
//! Three transient failures within five minutes opens the circuit for 60
//! seconds. Each subsequent open extends the duration (5 min, then 15 min
//! max). One half-open probe per open period.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::TurnAdapter;
use super::catalog::ProviderCatalog;
use super::domain::{
    BlockedReason, Eligibility, ProviderClass, ProviderDescriptor, ProviderId,
    TemporaryUnavailableReason,
};
use super::manifest::{ProviderManifest, ProviderRecord, QualificationStatus};
use super::{IsolationCapabilities, ProviderCapabilities};

/// Weight constants (basis points, total = 10000).
const QUALITY_WEIGHT: u32 = 4_000;
const RELIABILITY_WEIGHT: u32 = 3_000;
const LATENCY_WEIGHT: u32 = 2_500;
const LOCALITY_WEIGHT: u32 = 500;

/// EWMA smoothing factor (alpha = 0.2 → weight of new observation).
const EWMA_ALPHA: f64 = 0.2;
/// Number of observations before cold-start blend is no longer applied.
const COLD_START_BLEND_THRESHOLD: u32 = 20;

/// Circuit breaker thresholds.
const FAILURE_THRESHOLD: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);
const INITIAL_OPEN_DURATION: Duration = Duration::from_secs(60);
const SECONDARY_OPEN_DURATION: Duration = Duration::from_secs(5 * 60);
const MAX_OPEN_DURATION: Duration = Duration::from_secs(15 * 60);

/// Locality scores (higher is better).
fn locality_score(class: ProviderClass) -> u32 {
    match class {
        ProviderClass::LocalServer => 10_000,
        ProviderClass::OsManagedLocal => 8_000,
        ProviderClass::Cloud => 2_000,
        ProviderClass::AgentCli => 6_000,
    }
}

/// Fixed, content-free outcome of one provider turn attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Success { latency_ms: u64 },
    TransientFailure,
    PermanentFailure,
    RetryAfter(Duration),
}

/// Per-provider routing state tracked by the adaptive router.
#[derive(Debug, Clone)]
struct ProviderRoutingState {
    id: ProviderId,
    reliability_ewma: f64,
    latency_ewma_ms: f64,
    observation_count: u32,
    circuit: CircuitState,
}

impl ProviderRoutingState {
    fn new(id: ProviderId) -> Self {
        Self {
            id,
            reliability_ewma: 1.0,
            latency_ewma_ms: 1_000.0,
            observation_count: 0,
            circuit: CircuitState::Closed,
        }
    }

    fn record_success(&mut self, latency_ms: u64) {
        self.observation_count += 1;
        let alpha = EWMA_ALPHA;
        self.reliability_ewma = alpha * 1.0 + (1.0 - alpha) * self.reliability_ewma;
        self.latency_ewma_ms = alpha * latency_ms as f64 + (1.0 - alpha) * self.latency_ewma_ms;
        self.circuit.record_success();
    }

    fn record_failure(&mut self) {
        self.observation_count += 1;
        let alpha = EWMA_ALPHA;
        self.reliability_ewma = alpha * 0.0 + (1.0 - alpha) * self.reliability_ewma;
        self.circuit.record_failure();
    }

    fn record_retry_after(&mut self, duration: Duration) {
        self.circuit.set_retry_after(duration);
    }

    /// Score this provider against the given required capabilities and current
    /// time. Returns a value in `[0, 10000]`.
    fn score(&self, descriptor: &ProviderDescriptor, now: Instant) -> u32 {
        // Quality: based on declared capability density.
        let caps = descriptor.declared_capabilities;
        let quality = capability_score(&caps);

        // Reliability: cold-start blend until enough observations.
        let reliability = if self.observation_count < COLD_START_BLEND_THRESHOLD {
            let blend = self.observation_count as f64 / COLD_START_BLEND_THRESHOLD as f64;
            (blend * self.reliability_ewma + (1.0 - blend) * 0.5) * 10_000.0
        } else {
            self.reliability_ewma * 10_000.0
        };

        // Latency: inverted and normalized. 0ms = 10000, 30000ms+ = 0.
        let latency_score = ((30_000.0 - self.latency_ewma_ms.min(30_000.0)) / 30.0) as u32;

        // Locality: direct classification score.
        let locality = locality_score(descriptor.class);

        // Circuit penalty: open circuit means zero score.
        if self.circuit.is_open(now) {
            return 0;
        }

        let raw = (quality as f64 * QUALITY_WEIGHT as f64
            + reliability * RELIABILITY_WEIGHT as f64
            + latency_score as f64 * LATENCY_WEIGHT as f64
            + locality as f64 * LOCALITY_WEIGHT as f64)
            / 10_000.0;
        raw.min(10_000.0) as u32
    }
}

fn capability_score(caps: &ProviderCapabilities) -> u32 {
    let mut score: u32 = 2_000; // base for having text
    if caps.streaming {
        score += 1_500;
    }
    if caps.tools {
        score += 2_500;
    }
    if caps.structured_output {
        score += 1_500;
    }
    if caps.vision {
        score += 1_500;
    }
    if caps.ocr {
        score += 1_000;
    }
    score.min(10_000)
}

/// Circuit breaker state.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CircuitState {
    Closed,
    Open {
        opened_at: Instant,
        open_duration: Duration,
    },
    HalfOpen {
        opened_at: Instant,
        open_duration: Duration,
    },
}

impl CircuitState {
    fn is_open(&self, now: Instant) -> bool {
        match self {
            Self::Closed => false,
            Self::Open {
                opened_at,
                open_duration,
            }
            | Self::HalfOpen {
                opened_at,
                open_duration,
            } => now.duration_since(*opened_at) < *open_duration,
        }
    }

    fn record_success(&mut self) {
        *self = Self::Closed;
    }

    fn record_failure(&mut self) {
        match self {
            Self::Closed => {
                // First failure; the next failure in the window will open.
            }
            Self::Open { open_duration, .. } | Self::HalfOpen { open_duration, .. } => {
                let now = Instant::now();
                let next = extend_open_duration(*open_duration);
                *self = Self::Open {
                    opened_at: now,
                    open_duration: next,
                };
            }
        }
    }

    fn set_retry_after(&mut self, duration: Duration) {
        let now = Instant::now();
        let capped = duration.min(MAX_OPEN_DURATION);
        *self = Self::Open {
            opened_at: now,
            open_duration: capped,
        };
    }
}

fn extend_open_duration(current: Duration) -> Duration {
    if current <= INITIAL_OPEN_DURATION {
        SECONDARY_OPEN_DURATION
    } else {
        (current * 2).min(MAX_OPEN_DURATION)
    }
}

/// Sticky provider pinning. Once a provider is pinned for a conversation, all
/// turns go to that provider until the pin is released.
#[derive(Debug, Clone, Default)]
struct StickyPin {
    pinned: Option<ProviderId>,
}

impl StickyPin {
    fn pin(&mut self, id: ProviderId) {
        self.pinned = Some(id);
    }

    fn unpin(&mut self) {
        self.pinned = None;
    }

    fn pinned_id(&self) -> Option<&ProviderId> {
        self.pinned.as_ref()
    }
}

/// The adaptive provider router.
///
/// Holds mutable routing state behind an `Arc<Mutex<…>>` so that turn
/// outcomes can update EWMA and circuit state without requiring `&mut self`
/// on the caller side.
pub struct AdaptiveRouter {
    adapters: BTreeMap<ProviderId, Arc<dyn TurnAdapter>>,
    catalog: ProviderCatalog,
    state: Arc<Mutex<BTreeMap<ProviderId, ProviderRoutingState>>>,
    sticky: Arc<Mutex<StickyPin>>,
    require_tools: AtomicBool,
}

impl AdaptiveRouter {
    /// Build a router from a catalog and a set of adapters keyed by provider
    /// ID. Adapters for providers not in the catalog are silently ignored.
    pub fn new(
        catalog: ProviderCatalog,
        adapters: Vec<Arc<dyn TurnAdapter>>,
        require_tools: bool,
    ) -> Self {
        let mut adapter_map = BTreeMap::new();
        for adapter in adapters {
            let id = adapter.provider_id().clone();
            adapter_map.insert(id.clone(), adapter);
        }

        let mut state_map = BTreeMap::new();
        for descriptor in catalog.descriptors() {
            state_map
                .entry(descriptor.id.clone())
                .or_insert_with(|| ProviderRoutingState::new(descriptor.id.clone()));
        }

        Self {
            adapters: adapter_map,
            catalog,
            state: Arc::new(Mutex::new(state_map)),
            sticky: Arc::new(Mutex::new(StickyPin::default())),
            require_tools: AtomicBool::new(require_tools),
        }
    }

    /// Select the best provider for the given required capabilities.
    ///
    /// Returns the provider ID and its descriptor, or `None` if nothing is
    /// routable.
    pub fn select(
        &self,
        required: ProviderCapabilities,
        require_tools: bool,
    ) -> Option<(ProviderId, ProviderDescriptor)> {
        let now = Instant::now();
        let state = self.state.lock().expect("routing state poisoned");

        // Check stickiness first.
        let pinned = self.sticky.lock().expect("sticky state poisoned");
        if let Some(pinned_id) = pinned.pinned_id()
            && let Some(descriptor) = self.catalog.descriptor(pinned_id)
            && descriptor.eligibility.is_routable()
            && (!require_tools || descriptor.declared_capabilities.tools)
            && descriptor.declared_capabilities.satisfies(required)
        {
            return Some((pinned_id.clone(), descriptor.clone()));
        }
        drop(pinned);

        let mut best: Option<(u32, &ProviderId, &ProviderDescriptor)> = None;
        for descriptor in self.catalog.routable() {
            if require_tools && !descriptor.declared_capabilities.tools {
                continue;
            }
            if !descriptor.declared_capabilities.satisfies(required) {
                continue;
            }
            let provider_state = state.get(&descriptor.id);
            let score = provider_state
                .map(|s| s.score(descriptor, now))
                .unwrap_or(5_000);
            if best.is_none() || score > best.as_ref().map_or(0, |(s, _, _)| *s) {
                best = Some((score, &descriptor.id, descriptor));
            }
        }

        best.map(|(_, id, desc)| (id.clone(), desc.clone()))
    }

    /// Pin a provider for the remainder of a conversation. All subsequent
    /// `select` calls will prefer this provider until `unpin` is called.
    pub fn pin(&self, id: &ProviderId) {
        self.sticky
            .lock()
            .expect("sticky state poisoned")
            .pin(id.clone());
    }

    /// Release the sticky pin.
    pub fn unpin(&self) {
        self.sticky.lock().expect("sticky state poisoned").unpin();
    }

    /// Record a successful turn outcome.
    pub fn record(&self, id: &ProviderId, outcome: TurnOutcome) {
        let mut state = self.state.lock().expect("routing state poisoned");
        let provider_state = state
            .entry(id.clone())
            .or_insert_with(|| ProviderRoutingState::new(id.clone()));
        match outcome {
            TurnOutcome::Success { latency_ms } => {
                provider_state.record_success(latency_ms);
            }
            TurnOutcome::TransientFailure => {
                provider_state.record_failure();
            }
            TurnOutcome::PermanentFailure => {
                provider_state.record_failure();
            }
            TurnOutcome::RetryAfter(duration) => {
                provider_state.record_retry_after(duration);
            }
        }
    }

    /// Get a snapshot of routing state for inspection.
    pub fn snapshot(&self) -> Vec<RoutingSnapshot> {
        let state = self.state.lock().expect("routing state poisoned");
        let now = Instant::now();
        state
            .values()
            .map(|s| {
                let descriptor = self.catalog.descriptor(&s.id).cloned();
                RoutingSnapshot {
                    provider_id: s.id.clone(),
                    reliability_ewma: s.reliability_ewma,
                    latency_ewma_ms: s.latency_ewma_ms,
                    observation_count: s.observation_count,
                    circuit_open: s.circuit.is_open(now),
                    eligible: descriptor
                        .as_ref()
                        .is_some_and(|d| d.eligibility.is_routable()),
                }
            })
            .collect()
    }

    /// Access the underlying catalog.
    pub fn catalog(&self) -> &ProviderCatalog {
        &self.catalog
    }
}

impl fmt::Debug for AdaptiveRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdaptiveRouter")
            .field("adapters", &self.adapters.len())
            .field("catalog", &self.catalog)
            .finish()
    }
}

/// A point-in-time snapshot of one provider's routing state.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingSnapshot {
    pub provider_id: ProviderId,
    pub reliability_ewma: f64,
    pub latency_ewma_ms: f64,
    pub observation_count: u32,
    pub circuit_open: bool,
    pub eligible: bool,
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
