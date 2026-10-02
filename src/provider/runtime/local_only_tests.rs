use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAdapter {
    id: ProviderId,
    calls: Arc<AtomicUsize>,
    fails: bool,
}
impl TurnAdapter for CountingAdapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(if self.fails {
            Err(LlmError::classified(
                "synthetic outage",
                ProviderFailureKind::Http5xx,
            ))
        } else {
            Ok(ModelTurn {
                text: "local answer".into(),
                calls: vec![],
            })
        }))
    }
}
fn route(
    runtime: &mut ProviderRuntime,
    name: &str,
    locality: ExecutionLocality,
    fails: bool,
) -> Arc<AtomicUsize> {
    let id = ProviderId::parse(name).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    runtime.register(
        id.clone(),
        "synthetic",
        ProviderClass::LocalServer,
        ProviderCapabilities::text_with_tools(),
        locality,
        ProviderProvenance::Configuration,
        true,
        config_identity(name.as_bytes()),
        Some(Arc::new(CountingAdapter {
            id,
            calls: calls.clone(),
            fails,
        })),
        None,
        false,
        false,
    );
    calls
}
fn enable(runtime: &mut ProviderRuntime) {
    runtime.apply_configuration(ProviderConfig::from_iter([("ABBEY_LOCAL_ONLY", "1")]).unwrap());
}
#[tokio::test]
async fn local_only_never_executes_remote_primary_or_outage_fallback() {
    let mut runtime = ProviderRuntime::empty();
    let remote = route(
        &mut runtime,
        "primary",
        ExecutionLocality::PublicRemote,
        false,
    );
    let local = route(
        &mut runtime,
        "local-fallback",
        ExecutionLocality::SameHost,
        true,
    );
    enable(&mut runtime);
    assert!(runtime.chat("synthetic", &[]).await.is_err());
    assert_eq!(local.load(Ordering::SeqCst), 1);
    assert_eq!(remote.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn local_only_blocks_new_offhost_routes_after_configuration() {
    let mut runtime = ProviderRuntime::empty();
    enable(&mut runtime);
    let remote = route(
        &mut runtime,
        "late-route",
        ExecutionLocality::PrivateNetwork,
        false,
    );
    assert!(!runtime.generation_available());
    assert!(
        runtime
            .request_readiness_for(RequestClass::TextReadOnly, false)
            .is_err()
    );
    assert!(runtime.chat("synthetic", &[]).await.is_err());
    assert_eq!(remote.load(Ordering::SeqCst), 0);
}
#[test]
fn local_only_blocks_anthropic_pcc_and_remote_vision_but_keeps_loopback() {
    let mut runtime = ProviderRuntime::legacy(
        Backend::from_values(Some("synthetic-key".into()), None, None),
        Backend::from_values(None, Some("http://localhost:11434".into()), None),
        vec![],
        None,
        true,
        1,
        1,
    );
    route(
        &mut runtime,
        "foundation-models-cli-pcc",
        ExecutionLocality::PublicRemote,
        false,
    );
    route(
        &mut runtime,
        "vision",
        ExecutionLocality::PublicRemote,
        false,
    );
    enable(&mut runtime);
    assert!(!runtime.eligible(
        &ProviderId::parse("primary").unwrap(),
        RequestClass::TextReadOnly
    ));
    assert!(!runtime.eligible(
        &ProviderId::parse("foundation-models-cli-pcc").unwrap(),
        RequestClass::TextReadOnly
    ));
    assert!(!runtime.eligible(
        &ProviderId::parse("vision").unwrap(),
        RequestClass::TextReadOnly
    ));
    assert!(runtime.eligible(
        &ProviderId::parse("local-fallback").unwrap(),
        RequestClass::TextReadOnly
    ));
    for endpoint in ["http://localhost:1", "http://127.0.0.1:1", "http://[::1]:1"] {
        assert_eq!(endpoint_locality(endpoint), ExecutionLocality::SameHost);
    }
}
#[test]
fn local_only_flag_rejects_ambiguous_values_without_echoing() {
    for value in ["true", "secret", "2"] {
        let error = ProviderConfig::from_iter([("ABBEY_LOCAL_ONLY", value)]).unwrap_err();
        assert_eq!(error.to_string(), "ABBEY_LOCAL_ONLY: must be 0 or 1");
    }
    assert!(
        !ProviderConfig::from_iter([("ABBEY_LOCAL_ONLY", "0")])
            .unwrap()
            .local_only
    );
}

struct CountingVision(Arc<AtomicUsize>);
impl VisionAdapter for CountingVision {
    fn image(
        &self,
        _: bool,
        _: Vec<u8>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, VisionError>> + Send + '_>>
    {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok("synthetic image".into())))
    }
}
#[tokio::test]
async fn local_only_does_not_submit_image_or_ocr_bytes_offhost() {
    let mut runtime = ProviderRuntime::empty();
    let calls = Arc::new(AtomicUsize::new(0));
    runtime.register(
        ProviderId::parse("vision").unwrap(),
        "synthetic vision",
        ProviderClass::LocalServer,
        ProviderCapabilities {
            vision: true,
            ocr: true,
            ..ProviderCapabilities::default()
        },
        ExecutionLocality::PublicRemote,
        ProviderProvenance::Configuration,
        true,
        config_identity(b"synthetic vision"),
        None,
        Some(Arc::new(CountingVision(calls.clone()))),
        false,
        false,
    );
    enable(&mut runtime);
    assert!(!runtime.vision_available());
    assert!(runtime.describe(vec![1, 2, 3]).await.is_err());
    assert!(runtime.extract_text(vec![1, 2, 3]).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn personal_memory_adapter_source_only_never_executes_remote_primary_or_fallback() {
    let mut runtime = ProviderRuntime::empty();
    let remote = route(
        &mut runtime,
        "primary",
        ExecutionLocality::PublicRemote,
        false,
    );
    let local = route(&mut runtime, "local", ExecutionLocality::SameHost, true);
    let private = route(
        &mut runtime,
        "private",
        ExecutionLocality::PrivateNetwork,
        false,
    );
    let mut conversation = runtime.begin_source_only(false, false);
    loop {
        match conversation.reserve().await {
            Ok(()) => {}
            Err(error) if conversation.fallback(&error) => continue,
            Err(_) => break,
        }
        let result = conversation
            .execute(
                "source",
                &[ChatTurn::user("current request")],
                &[],
                crate::llm::ResponseStyle::Default,
                None,
            )
            .await;
        match result {
            Err(error) if conversation.fallback(&error) => continue,
            Err(_) => break,
            Ok(_) => panic!("same-host outage must not submit a remote fallback"),
        }
    }
    assert_eq!(local.load(Ordering::SeqCst), 1);
    assert_eq!(remote.load(Ordering::SeqCst), 0);
    assert_eq!(private.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn personal_memory_adapter_source_only_rechecks_existing_remote_lease_before_execution() {
    let mut runtime = ProviderRuntime::empty();
    let remote = route(
        &mut runtime,
        "primary",
        ExecutionLocality::PublicRemote,
        false,
    );
    let mut conversation = runtime.begin(false, false);
    conversation.reserve().await.unwrap();
    // Strengthening a conversation after reservation must not leave a hole
    // through the already-owned lease. No configuration mutation is inferred.
    conversation.same_host_only = true;
    assert!(
        conversation
            .execute(
                "source",
                &[ChatTurn::user("current request")],
                &[],
                crate::llm::ResponseStyle::Default,
                None
            )
            .await
            .is_err()
    );
    assert_eq!(remote.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn personal_memory_adapter_source_only_succeeds_on_same_host_without_global_flag() {
    let mut runtime = ProviderRuntime::empty();
    let local = route(&mut runtime, "local", ExecutionLocality::SameHost, false);
    let mut conversation = runtime.begin_source_only(false, false);
    let answer = conversation
        .execute(
            "source",
            &[ChatTurn::user("current request")],
            &[],
            crate::llm::ResponseStyle::Default,
            None,
        )
        .await
        .unwrap();
    assert_eq!(answer.text, "local answer");
    assert_eq!(local.load(Ordering::SeqCst), 1);
}

fn image_route(
    runtime: &mut ProviderRuntime,
    name: &str,
    locality: ExecutionLocality,
) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    runtime.register(
        ProviderId::parse(name).unwrap(),
        "synthetic vision",
        ProviderClass::LocalServer,
        ProviderCapabilities {
            vision: true,
            ocr: true,
            ..ProviderCapabilities::default()
        },
        locality,
        ProviderProvenance::Configuration,
        true,
        config_identity(name.as_bytes()),
        None,
        Some(Arc::new(CountingVision(calls.clone()))),
        false,
        false,
    );
    calls
}

#[tokio::test]
async fn personal_memory_adapter_welcome_requires_same_host_without_global_flag() {
    let mut runtime = ProviderRuntime::empty();
    let remote = route(
        &mut runtime,
        "remote",
        ExecutionLocality::PublicRemote,
        false,
    );
    assert!(
        runtime
            .chat_source_only("member display name", &[ChatTurn::user("Say hello.")])
            .await
            .is_err()
    );
    assert_eq!(remote.load(Ordering::SeqCst), 0);
    let local = route(&mut runtime, "local", ExecutionLocality::SameHost, false);
    assert!(
        runtime
            .chat_source_only("member display name", &[ChatTurn::user("Say hello.")])
            .await
            .is_ok()
    );
    assert_eq!(local.load(Ordering::SeqCst), 1);
    assert_eq!(remote.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn personal_memory_adapter_image_and_ocr_require_same_host_without_global_flag() {
    for locality in [
        ExecutionLocality::PublicRemote,
        ExecutionLocality::PrivateNetwork,
    ] {
        let mut runtime = ProviderRuntime::empty();
        let remote = image_route(&mut runtime, "remote", locality);
        assert!(runtime.describe_source_only(vec![1, 2, 3]).await.is_err());
        assert!(
            runtime
                .extract_text_source_only(vec![1, 2, 3])
                .await
                .is_err()
        );
        assert_eq!(remote.load(Ordering::SeqCst), 0);
        let local = image_route(&mut runtime, "local", ExecutionLocality::SameHost);
        assert_eq!(
            runtime.describe_source_only(vec![1, 2, 3]).await.unwrap(),
            "synthetic image"
        );
        assert_eq!(
            runtime
                .extract_text_source_only(vec![1, 2, 3])
                .await
                .unwrap(),
            "synthetic image"
        );
        assert_eq!(local.load(Ordering::SeqCst), 2);
        assert_eq!(remote.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn personal_memory_adapter_image_execution_rejects_reserved_remote_lease() {
    for ocr in [false, true] {
        let mut runtime = ProviderRuntime::empty();
        let remote = image_route(&mut runtime, "remote", ExecutionLocality::PublicRemote);
        let mut conversation = runtime.conversation(RequestClass::image(ocr), false, false, None);
        conversation.reserve().await.unwrap();
        conversation.same_host_only = true;
        assert!(
            conversation
                .execute_image(ocr, vec![1, 2, 3])
                .await
                .is_err()
        );
        assert_eq!(remote.load(Ordering::SeqCst), 0);
    }
}
