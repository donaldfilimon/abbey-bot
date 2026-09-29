//! FM route registration order and one-hop PCC-to-system fallback.
use super::*;

fn qualified(mode: FmMode, primary: bool) -> FoundationModels {
    FoundationModels::new_qualified(
        FmConfig {
            mode,
            endpoint: None,
            cli: "/not-executed/fm".into(),
            fallback: true,
            primary,
            timeout_secs: 1,
        },
        None,
        true,
        VerifiedFmCapabilities {
            server: None,
            cli: ProviderCapabilities::text_with_tools(),
        },
    )
}

fn endpoint() -> Option<Backend> {
    Backend::from_values(None, Some("http://127.0.0.1:11434".into()), None)
}

fn route(primary: bool) -> ProviderRuntime {
    ProviderRuntime::legacy(
        endpoint(),
        None,
        vec![
            qualified(FmMode::Pcc, primary),
            qualified(FmMode::System, primary),
        ],
        None,
        true,
        1,
        1,
    )
}

fn order(runtime: &ProviderRuntime) -> Vec<&str> {
    runtime.order.iter().map(ProviderId::as_str).collect()
}

async fn first_choice(runtime: &ProviderRuntime) -> String {
    let mut conversation = runtime.begin(false, false);
    conversation.reserve().await.unwrap();
    lock(&conversation.effects.0)
        .selected()
        .unwrap()
        .as_str()
        .to_string()
}

/// Replaces a registered adapter in place, keeping its position in the order.
fn swap(
    runtime: &mut ProviderRuntime,
    name: &str,
    answers: Vec<Result<ModelTurn, LlmError>>,
) -> Arc<Fake> {
    let id = ProviderId::parse(name).unwrap();
    let adapter = Arc::new(Fake {
        id: id.clone(),
        answers: Mutex::new(answers.into()),
        calls: AtomicUsize::new(0),
        deltas: Vec::new(),
        tools: Mutex::new(Vec::new()),
    });
    runtime.entries.get_mut(&id).unwrap().adapter = Some(adapter.clone());
    adapter
}

fn pcc_refusal() -> Result<ModelTurn, LlmError> {
    Err(LlmError::classified(
        "synthetic PCC quota refusal",
        ProviderFailureKind::RateLimited,
    )
    .with_retry_after(RetryAfter::from_seconds(Some("60"))))
}

#[tokio::test]
async fn primary_role_registers_pcc_then_system_before_endpoint() {
    let runtime = route(true);
    assert_eq!(
        order(&runtime),
        [
            "foundation-models-cli-pcc",
            "foundation-models-cli",
            "primary"
        ]
    );
    assert_eq!(
        runtime.generation_label(),
        Some("Apple Foundation Models Private Cloud Compute")
    );
    assert_eq!(first_choice(&runtime).await, "foundation-models-cli-pcc");
    let rendered = crate::inspect::render_provider(&runtime.inspect_snapshot());
    assert!(
        rendered.contains("foundation-models-cli-pcc: routable yes"),
        "{rendered}"
    );
    // The vision/dashboard handle is the first qualified mode.
    assert_eq!(
        runtime.foundation_models().map(|fm| fm.config.mode),
        Some(FmMode::Pcc)
    );
}

#[tokio::test]
async fn fallback_role_keeps_endpoint_first() {
    let runtime = route(false);
    assert_eq!(
        order(&runtime),
        [
            "primary",
            "foundation-models-cli-pcc",
            "foundation-models-cli"
        ]
    );
    assert_eq!(first_choice(&runtime).await, "primary");

    // An unqualified mode registers but is never admitted.
    let degraded = ProviderRuntime::legacy(
        None,
        None,
        vec![
            FoundationModels::new(
                FmConfig {
                    primary: true,
                    ..qualified(FmMode::Pcc, true).config.clone()
                },
                None,
                true,
            )
            .with_qualification_state(FmQualificationState::Stale),
            qualified(FmMode::System, true),
        ],
        None,
        true,
        1,
        1,
    );
    assert_eq!(
        order(&degraded),
        ["foundation-models-cli-pcc", "foundation-models-cli"]
    );
    assert_eq!(first_choice(&degraded).await, "foundation-models-cli");
    let states = degraded
        .foundation_model_modes()
        .map(|fm| fm.qualification_state().as_str())
        .collect::<Vec<_>>();
    assert_eq!(states, ["stale", "qualified"]);
    assert_eq!(
        degraded.foundation_models().map(|fm| fm.config.mode),
        Some(FmMode::System)
    );
}

#[tokio::test]
async fn pcc_failure_falls_back_to_system_once() {
    let mut runtime = route(true);
    let pcc = swap(
        &mut runtime,
        "foundation-models-cli-pcc",
        vec![pcc_refusal()],
    );
    let system = swap(&mut runtime, "foundation-models-cli", Vec::new());
    let endpoint = swap(&mut runtime, "primary", Vec::new());
    let (_, label) = runtime
        .chat("policy", &[ChatTurn::user("request")])
        .await
        .unwrap();
    assert_eq!(label, "Apple Foundation Models on-device model");
    assert_eq!(pcc.calls.load(Ordering::Relaxed), 1);
    assert_eq!(system.calls.load(Ordering::Relaxed), 1);
    assert_eq!(endpoint.calls.load(Ordering::Relaxed), 0);

    // The refused PCC circuit is open: the next turn starts at system.
    runtime
        .chat("policy", &[ChatTurn::user("again")])
        .await
        .unwrap();
    assert_eq!(pcc.calls.load(Ordering::Relaxed), 1);
    assert_eq!(system.calls.load(Ordering::Relaxed), 2);
    assert_eq!(endpoint.calls.load(Ordering::Relaxed), 0);

    // Fallback stays one hop: PCC then system failing never reaches the endpoint.
    let mut runtime = route(true);
    let pcc = swap(
        &mut runtime,
        "foundation-models-cli-pcc",
        vec![pcc_refusal()],
    );
    let system = swap(
        &mut runtime,
        "foundation-models-cli",
        vec![failure(ProviderFailureKind::TransportUnavailable)],
    );
    let endpoint = swap(&mut runtime, "primary", Vec::new());
    assert!(
        runtime
            .chat("policy", &[ChatTurn::user("request")])
            .await
            .is_err()
    );
    assert_eq!(pcc.calls.load(Ordering::Relaxed), 1);
    assert_eq!(system.calls.load(Ordering::Relaxed), 1);
    assert_eq!(endpoint.calls.load(Ordering::Relaxed), 0);
}
