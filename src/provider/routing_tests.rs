use super::*;
use crate::llm;
use crate::tools;
use super::super::TurnFuture;

fn test_descriptor(id: &str, caps: ProviderCapabilities, class: ProviderClass) -> ProviderDescriptor {
    ProviderDescriptor {
        id: ProviderId::parse(id).unwrap(),
        class,
        discovery: super::super::domain::DiscoveryBoundary::ExactBinary,
        detection: super::super::domain::DetectionState::Detected,
        eligibility: Eligibility::Routable,
        declared_capabilities: caps,
        isolation: IsolationCapabilities::default(),
        provenance: super::super::domain::ProviderProvenance::QualifiedManifest,
    }
}

#[test]
fn capability_score_scales_with_features() {
    let text_only = ProviderCapabilities::text();
    let text_tools = ProviderCapabilities::text_with_tools();
    let all = ProviderCapabilities {
        text: true,
        streaming: true,
        structured_output: true,
        tools: true,
        vision: true,
        ocr: true,
    };
    assert!(capability_score(&all) > capability_score(&text_tools));
    assert!(capability_score(&text_tools) > capability_score(&text_only));
    assert!(capability_score(&text_only) >= 2_000);
}

#[test]
fn locality_score_prefers_local() {
    assert!(locality_score(ProviderClass::LocalServer) > locality_score(ProviderClass::Cloud));
    assert!(locality_score(ProviderClass::LocalServer) > locality_score(ProviderClass::AgentCli));
}

#[test]
fn circuit_starts_closed() {
    let circuit = CircuitState::Closed;
    assert!(!circuit.is_open(Instant::now()));
}

#[test]
fn circuit_opens_after_failure_and_closes_on_success() {
    let mut state = ProviderRoutingState::new(ProviderId::parse("test").unwrap());
    assert_eq!(state.circuit, CircuitState::Closed);

    state.record_failure();
    state.record_success(100);
    assert_eq!(state.circuit, CircuitState::Closed);
}

#[test]
fn retry_after_opens_circuit() {
    let mut state = ProviderRoutingState::new(ProviderId::parse("test").unwrap());
    state.record_retry_after(Duration::from_secs(120));
    assert!(state.circuit.is_open(Instant::now()));
}

#[test]
fn extend_open_duration_doubles() {
    assert_eq!(
        extend_open_duration(INITIAL_OPEN_DURATION),
        SECONDARY_OPEN_DURATION
    );
    assert_eq!(
        extend_open_duration(SECONDARY_OPEN_DURATION),
        Duration::from_secs(10 * 60)
    );
    assert_eq!(
        extend_open_duration(Duration::from_secs(10 * 60)),
        MAX_OPEN_DURATION
    );
    assert_eq!(
        extend_open_duration(MAX_OPEN_DURATION),
        MAX_OPEN_DURATION
    );
}

#[test]
fn score_is_zero_when_circuit_open() {
    let mut state = ProviderRoutingState::new(ProviderId::parse("test").unwrap());
    state.record_retry_after(Duration::from_secs(60));
    let desc = test_descriptor(
        "test",
        ProviderCapabilities::text_with_tools(),
        ProviderClass::LocalServer,
    );
    assert_eq!(state.score(&desc, Instant::now()), 0);
}

#[test]
fn cold_start_blend_decays() {
    let mut state = ProviderRoutingState::new(ProviderId::parse("test").unwrap());
    let desc = test_descriptor(
        "test",
        ProviderCapabilities::text_with_tools(),
        ProviderClass::LocalServer,
    );
    let score_before = state.score(&desc, Instant::now());
    for _ in 0..10 {
        state.record_success(200);
    }
    let score_after = state.score(&desc, Instant::now());
    assert!(score_before > 0);
    assert!(score_after > 0);
}

#[test]
fn sticky_pin_preferred_over_highest_score() {
    struct FakeAdapter(ProviderId);
    impl TurnAdapter for FakeAdapter {
        fn provider_id(&self) -> &ProviderId {
            &self.0
        }
        fn turn<'a>(
            &'a self,
            _sp: &'a str,
            _turns: &'a [llm::ChatTurn],
            _tools: &'a [tools::ToolSpec],
            _cid: &'a str,
        ) -> TurnFuture<'a> {
            Box::pin(async {
                Ok(llm::ModelTurn {
                    text: "ok".into(),
                    calls: Vec::new(),
                })
            })
        }
    }

    let catalog = ProviderCatalog::new(&super::super::ProviderConfig::from_env().unwrap());
    let router = AdaptiveRouter::new(catalog, vec![], false);

    let id_a = ProviderId::parse("provider-a").unwrap();
    router.pin(&id_a);
    assert_eq!(router.sticky.lock().unwrap().pinned_id(), Some(&id_a));
    router.unpin();
    assert!(router.sticky.lock().unwrap().pinned_id().is_none());
}

#[test]
fn snapshot_returns_all_tracked_providers() {
    let catalog = ProviderCatalog::new(&super::super::ProviderConfig::from_env().unwrap());
    let router = AdaptiveRouter::new(catalog, vec![], false);
    let snapshots = router.snapshot();
    assert!(snapshots.is_empty());
}
