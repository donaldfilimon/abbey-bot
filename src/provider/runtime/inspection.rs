//! Content-free provider eligibility and health inspection.
use super::*;

impl ProviderRuntime {
    pub fn inspect_snapshot(&self) -> Vec<crate::inspect::ProviderRouteInspect> {
        use super::domain::TemporaryUnavailableReason as T;
        use crate::inspect::{ProviderProvenance as P, ProviderRouteInspect, ProviderRouteLabel};
        let state = lock(&self.state);
        let circuits = state.router.snapshot();
        self.catalog
            .descriptors()
            .map(|descriptor| {
                let id = &descriptor.id;
                let route = match id.as_str() {
                    "primary" => ProviderRouteLabel::Primary,
                    "local-fallback" => ProviderRouteLabel::LocalFallback,
                    "foundation-models-server" => ProviderRouteLabel::FoundationModelsServer,
                    "foundation-models-cli" => ProviderRouteLabel::FoundationModelsCli,
                    _ => ProviderRouteLabel::Vision,
                };
                let snapshot = circuits.iter().find(|snapshot| &snapshot.provider_id == id);
                let eligibility = if !descriptor.eligibility.is_routable() {
                    descriptor.eligibility
                } else if state.blocks.failed() {
                    Eligibility::TemporarilyUnavailable(T::BudgetExhausted)
                } else if let Some(snapshot) = snapshot {
                    match snapshot.circuit.phase {
                        CircuitPhase::Blocked => {
                            Eligibility::Blocked(match snapshot.circuit.reason {
                                Some(
                                    ProviderFailureKind::ExecutableIdentity
                                    | ProviderFailureKind::ModelIdentity
                                    | ProviderFailureKind::SandboxIdentity,
                                ) => BlockedReason::IdentityMismatch,
                                _ => BlockedReason::RequalificationRequired,
                            })
                        }
                        CircuitPhase::Open => Eligibility::TemporarilyUnavailable(
                            if matches!(
                                snapshot.circuit.reason,
                                Some(
                                    ProviderFailureKind::RateLimited | ProviderFailureKind::Http5xx
                                )
                            ) {
                                T::RetryAfter
                            } else {
                                T::CircuitOpen
                            },
                        ),
                        CircuitPhase::HalfOpen if snapshot.circuit.probe_reserved => {
                            Eligibility::TemporarilyUnavailable(T::Busy)
                        }
                        _ if !RequestClass::ALL
                            .into_iter()
                            .any(|class| state.router.profile(id, class).is_some()) =>
                        {
                            Eligibility::Blocked(BlockedReason::CapabilityUnavailable)
                        }
                        _ if self
                            .entries
                            .get(id)
                            .is_some_and(|entry| entry.slots.available_permits() == 0) =>
                        {
                            Eligibility::TemporarilyUnavailable(T::Busy)
                        }
                        _ => Eligibility::Routable,
                    }
                } else {
                    Eligibility::Blocked(BlockedReason::Unqualified)
                };
                let caps = descriptor.declared_capabilities;
                ProviderRouteInspect::new(
                    route,
                    eligibility.is_routable(),
                    caps.text,
                    caps.tools
                        && self.tools_enabled
                        && self
                            .entries
                            .get(id)
                            .and_then(|entry| entry.adapter.as_ref())
                            .is_some_and(|adapter| adapter.tools_enabled()),
                    caps.vision,
                    caps.ocr,
                    if descriptor.provenance == ProviderProvenance::QualifiedManifest {
                        P::QualifiedManifest
                    } else {
                        P::Configuration
                    },
                )
                .with_runtime(
                    descriptor.clone(),
                    eligibility,
                    snapshot.and_then(|snapshot| snapshot.circuit.reason),
                )
            })
            .collect()
    }
}
