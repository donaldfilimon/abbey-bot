//! Pure adaptive selection. Conversation ownership is explicit and never router-global.
use super::circuit::{
    AttemptPermit, Circuit, CircuitSnapshot, CircuitUnavailable, ProviderFailureKind, RetryAfter,
};
use super::domain::ProviderId;
use super::manifest::ProviderIdentityHashes;
use super::scoring::{
    NormalizedScore, ProviderScoreProfile, RequestClass, ScoreComponents, ScoreProducerPolicy,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteUnavailableReason {
    NoConfiguredProvider,
    CapabilityUnavailable,
    PolicyDenied,
    AllOpen,
    BlockedPendingRequalification,
    Busy,
    BudgetExhausted,
}
/// Each hard gate is supplied by the runtime's validated catalog and capacity policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteAdmission {
    pub configured: bool,
    pub identity_current: bool,
    pub capability_allowed: bool,
    pub policy_allowed: bool,
    pub budget_available: bool,
    pub capacity_available: bool,
}
impl RouteAdmission {
    pub const QUALIFIED: Self = Self {
        configured: true,
        identity_current: true,
        capability_allowed: true,
        policy_allowed: true,
        budget_available: true,
        capacity_available: true,
    };
    fn check(self) -> Result<(), RouteUnavailableReason> {
        if !self.configured {
            Err(RouteUnavailableReason::NoConfiguredProvider)
        } else if !self.identity_current {
            Err(RouteUnavailableReason::BlockedPendingRequalification)
        } else if !self.capability_allowed {
            Err(RouteUnavailableReason::CapabilityUnavailable)
        } else if !self.policy_allowed {
            Err(RouteUnavailableReason::PolicyDenied)
        } else if !self.budget_available {
            Err(RouteUnavailableReason::BudgetExhausted)
        } else if !self.capacity_available {
            Err(RouteUnavailableReason::Busy)
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct RouteDecision {
    pub provider_id: ProviderId,
    pub components: ScoreComponents,
    pub weighted_score: NormalizedScore,
    pub tie_position: usize,
}
#[derive(Debug)]
pub struct RouteAttempt {
    provider_id: ProviderId,
    generation: u64,
    class: RequestClass,
    permit: AttemptPermit,
}
#[derive(Debug, Clone)]
struct ProviderState {
    identity: ProviderIdentityHashes,
    generation: u64,
    profiles: BTreeMap<RequestClass, ProviderScoreProfile>,
    circuit: Circuit,
    admission: RouteAdmission,
}
impl ProviderState {
    fn assess(
        &self,
        class: RequestClass,
        now_ms: u64,
        admission: RouteAdmission,
    ) -> Result<(), RouteUnavailableReason> {
        admission.check()?;
        if !self.profiles.contains_key(&class) {
            return Err(RouteUnavailableReason::CapabilityUnavailable);
        }
        self.circuit.availability(now_ms).map_err(circuit_reason)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingSnapshot {
    pub provider_id: ProviderId,
    pub circuit: CircuitSnapshot,
}
#[derive(Debug, Default)]
pub struct AdaptiveRouter {
    providers: BTreeMap<ProviderId, ProviderState>,
    configured_order: Vec<ProviderId>,
    next_generation: u64,
}
impl AdaptiveRouter {
    pub fn new(configured_order: Vec<ProviderId>) -> Self {
        Self {
            configured_order,
            ..Self::default()
        }
    }
    pub fn set_order(&mut self, configured_order: Vec<ProviderId>) {
        self.configured_order = configured_order;
    }
    /// Explicit exact-identity requalification is the only replacement/reset path.
    /// The caller must pass profiles produced from already-validated qualification evidence.
    pub fn requalify(
        &mut self,
        id: ProviderId,
        identity: ProviderIdentityHashes,
        profiles: Vec<ProviderScoreProfile>,
        admission: RouteAdmission,
    ) -> bool {
        let Some(generation) = self.next_generation.checked_add(1) else {
            return false;
        };
        let mut by_class = BTreeMap::new();
        for profile in profiles {
            if by_class.insert(profile.request_class(), profile).is_some() {
                return false;
            }
        }
        self.next_generation = generation;
        self.providers.insert(
            id,
            ProviderState {
                identity,
                generation,
                profiles: by_class,
                circuit: Circuit::new(),
                admission,
            },
        );
        true
    }
    pub fn restore_blocked(
        &mut self,
        id: &ProviderId,
        identity: &ProviderIdentityHashes,
        reason: ProviderFailureKind,
    ) -> bool {
        let Some(state) = self
            .providers
            .get_mut(id)
            .filter(|s| &s.identity == identity)
        else {
            return false;
        };
        let Some(circuit) = Circuit::restore_blocked(reason) else {
            return false;
        };
        state.circuit = circuit;
        true
    }
    pub fn set_admission(&mut self, id: &ProviderId, admission: RouteAdmission) {
        if let Some(state) = self.providers.get_mut(id) {
            state.admission = admission;
        }
    }
    pub fn profile(&self, id: &ProviderId, class: RequestClass) -> Option<&ProviderScoreProfile> {
        self.providers.get(id)?.profiles.get(&class)
    }
    pub fn snapshot(&self) -> Vec<RoutingSnapshot> {
        self.providers
            .iter()
            .map(|(id, state)| RoutingSnapshot {
                provider_id: id.clone(),
                circuit: state.circuit.snapshot(),
            })
            .collect()
    }
    /// Inspect the same hard gates as selection without changing admission or reserving a probe.
    pub fn assess(
        &self,
        id: &ProviderId,
        class: RequestClass,
        now_ms: u64,
        admission: RouteAdmission,
    ) -> Result<(), RouteUnavailableReason> {
        self.providers
            .get(id)
            .ok_or(RouteUnavailableReason::NoConfiguredProvider)?
            .assess(class, now_ms, admission)
    }
    /// A supplied conversation selection is still subjected to every hard gate.
    pub fn select(
        &mut self,
        class: RequestClass,
        now_ms: u64,
        pinned: Option<&ProviderId>,
        excluded: &BTreeSet<ProviderId>,
    ) -> Result<(RouteDecision, RouteAttempt), RouteUnavailableReason> {
        let mut candidates = Vec::new();
        let mut failures = BTreeSet::new();
        for (id, state) in &self.providers {
            if excluded.contains(id) || pinned.is_some_and(|pin| pin != id) {
                continue;
            }
            let eligibility = state.assess(class, now_ms, state.admission);
            if let Err(reason) = eligibility {
                failures.insert(reason);
                continue;
            }
            let components = state.profiles[&class].components();
            candidates.push(RouteDecision {
                provider_id: id.clone(),
                components,
                weighted_score: components.weighted(),
                tie_position: self
                    .configured_order
                    .iter()
                    .position(|p| p == id)
                    .unwrap_or(usize::MAX),
            });
        }
        candidates.sort_by(|a, b| {
            b.weighted_score
                .get()
                .total_cmp(&a.weighted_score.get())
                .then(a.tie_position.cmp(&b.tie_position))
                .then(a.provider_id.cmp(&b.provider_id))
        });
        let Some(decision) = candidates.into_iter().next() else {
            // Most advanced failed gate wins, with a stable, documented enum order.
            return Err(failures
                .last()
                .copied()
                .unwrap_or(RouteUnavailableReason::NoConfiguredProvider));
        };
        let state = self
            .providers
            .get_mut(&decision.provider_id)
            .expect("ranked registered provider");
        let permit = state.circuit.reserve(now_ms).map_err(circuit_reason)?;
        let attempt = RouteAttempt {
            provider_id: decision.provider_id.clone(),
            generation: state.generation,
            class,
            permit,
        };
        Ok((decision, attempt))
    }
    pub fn complete(
        &mut self,
        attempt: RouteAttempt,
        kind: ProviderFailureKind,
        retry: RetryAfter,
        duration_ms: Option<u64>,
        now_ms: u64,
    ) -> Option<ProviderFailureKind> {
        let state = self.providers.get_mut(&attempt.provider_id)?;
        if state.generation != attempt.generation {
            return None;
        }
        let kind = if kind == ProviderFailureKind::Success && duration_ms.is_none_or(|d| d > 900000)
        {
            ProviderFailureKind::ProtocolDrift
        } else {
            kind
        };
        let effective = state
            .circuit
            .complete(attempt.permit, kind, retry, now_ms)?;
        let profile = state.profiles.get_mut(&attempt.class)?;
        ScoreProducerPolicy::V1
            .observe(profile, effective, duration_ms)
            .expect("validated bounded success duration");
        Some(effective)
    }
}
fn circuit_reason(reason: CircuitUnavailable) -> RouteUnavailableReason {
    match reason {
        CircuitUnavailable::Open => RouteUnavailableReason::AllOpen,
        CircuitUnavailable::Blocked => RouteUnavailableReason::BlockedPendingRequalification,
        CircuitUnavailable::Busy => RouteUnavailableReason::Busy,
    }
}

/// Pure conversation-owned effect/fallback interface, to be wired at effect boundaries by runtime.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConversationRoute {
    selected: Option<ProviderId>,
    failed: BTreeSet<ProviderId>,
    fallback_used: bool,
    visible_output_posted: bool,
    tool_dispatched: bool,
    image_submitted: bool,
}
impl ConversationRoute {
    pub fn selected(&self) -> Option<&ProviderId> {
        self.selected.as_ref()
    }
    pub fn accept_selection(&mut self, decision: &RouteDecision) {
        self.selected = Some(decision.provider_id.clone());
    }
    pub fn mark_visible_output(&mut self) {
        self.visible_output_posted = true;
    }
    pub fn mark_tool_dispatched(&mut self) {
        self.tool_dispatched = true;
    }
    pub fn mark_image_submitted(&mut self) {
        self.image_submitted = true;
    }
    pub fn excluded(&self) -> &BTreeSet<ProviderId> {
        &self.failed
    }
    pub fn can_fallback(&self, kind: ProviderFailureKind) -> bool {
        self.selected.is_some()
            && !self.fallback_used
            && !self.visible_output_posted
            && !self.tool_dispatched
            && !self.image_submitted
            && (kind.is_transient() || kind.is_blocked() || kind == ProviderFailureKind::Busy)
    }
    /// Consumes the one pass before routing it. A typed unavailable result is terminal.
    pub fn begin_fallback(&mut self, kind: ProviderFailureKind) -> bool {
        if !self.can_fallback(kind) {
            return false;
        }
        self.fallback_used = true;
        if let Some(id) = self.selected.take() {
            self.failed.insert(id);
        }
        true
    }
}
#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
