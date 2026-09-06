use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

struct FakeClock(AtomicU64);
impl ProviderClock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}
struct Fake {
    id: ProviderId,
    answers: Mutex<VecDeque<Result<ModelTurn, LlmError>>>,
    calls: AtomicUsize,
    deltas: Vec<String>,
    tools: Mutex<Vec<Vec<String>>>,
}
impl TurnAdapter for Fake {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [ChatTurn],
        tools: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        lock(&self.tools).push(tools.iter().map(|tool| tool.name.to_string()).collect());
        Box::pin(std::future::ready(
            lock(&self.answers).pop_front().unwrap_or_else(|| {
                Ok(ModelTurn {
                    text: "answer".into(),
                    calls: Vec::new(),
                })
            }),
        ))
    }
    fn execute<'a>(&'a self, request: AdapterRequest<'a>) -> TurnFuture<'a> {
        Box::pin(async move {
            if let Some(sender) = request.deltas {
                for delta in &self.deltas {
                    let _ = sender.send(delta.clone());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.turn(
                request.system,
                request.turns,
                request.tools,
                request.call_id,
            )
            .await
        })
    }
}
fn failure(kind: ProviderFailureKind) -> Result<ModelTurn, LlmError> {
    Err(LlmError::classified("synthetic provider failure", kind))
}
fn fake(
    runtime: &mut ProviderRuntime,
    name: &str,
    answers: Vec<Result<ModelTurn, LlmError>>,
    deltas: Vec<String>,
    local: bool,
) -> Arc<Fake> {
    let id = ProviderId::parse(name).unwrap();
    let adapter = Arc::new(Fake {
        id: id.clone(),
        answers: Mutex::new(answers.into()),
        calls: AtomicUsize::new(0),
        deltas,
        tools: Mutex::new(Vec::new()),
    });
    runtime.register(
        id,
        "synthetic",
        ProviderClass::LocalServer,
        ProviderCapabilities {
            streaming: true,
            ..ProviderCapabilities::text_with_tools()
        },
        ExecutionLocality::SameHost,
        ProviderProvenance::Configuration,
        true,
        config_identity(name.as_bytes()),
        Some(adapter.clone()),
        None,
        local,
        false,
    );
    adapter
}
#[tokio::test]
async fn legacy_precedence_and_labels_are_stable() {
    for (key, expected) in [
        (Some("key".to_string()), "external Anthropic API"),
        (Some(" ".into()), "configured OpenAI-compatible endpoint"),
    ] {
        let primary = Backend::from_values(key, Some("http://127.0.0.1:11434".into()), None);
        let expected_label = primary.as_ref().unwrap().label();
        let runtime = ProviderRuntime::legacy(primary, None, None, None, true, 1, 1);
        assert_eq!(runtime.generation_label(), Some(expected_label));
        assert_eq!(expected_label, expected);
        let mut conversation = runtime.begin(true, false);
        conversation.reserve().await.unwrap();
        assert_eq!(
            lock(&conversation.effects.0).selected().unwrap().as_str(),
            "primary"
        );
    }
    assert!(ProviderRuntime::empty().generation_label().is_none());
}
#[tokio::test]
async fn fallback_is_single_and_terminal_neutral_errors_never_replay() {
    for kind in [
        ProviderFailureKind::TransportUnavailable,
        ProviderFailureKind::Authentication,
        ProviderFailureKind::Cancelled,
        ProviderFailureKind::InvalidRequest,
    ] {
        let mut runtime = ProviderRuntime::empty();
        let first = fake(&mut runtime, "primary", vec![failure(kind)], vec![], true);
        let second = fake(&mut runtime, "secondary", vec![], vec![], true);
        let result = runtime.chat("policy", &[ChatTurn::user("request")]).await;
        let fallback = kind.is_transient() || kind.is_blocked();
        assert_eq!(result.is_ok(), fallback);
        assert_eq!(first.calls.load(Ordering::Relaxed), 1);
        assert_eq!(second.calls.load(Ordering::Relaxed), usize::from(fallback));
    }
    let mut runtime = ProviderRuntime::empty();
    fake(
        &mut runtime,
        "primary",
        vec![failure(ProviderFailureKind::Timeout)],
        vec![],
        true,
    );
    fake(
        &mut runtime,
        "secondary",
        vec![failure(ProviderFailureKind::Timeout)],
        vec![],
        true,
    );
    let third = fake(&mut runtime, "third", vec![], vec![], true);
    assert!(runtime.chat("", &[]).await.is_err());
    assert_eq!(third.calls.load(Ordering::Relaxed), 0);
}
#[tokio::test]
async fn actual_stream_post_closes_fallback_but_unposted_failure_can_retry() {
    for posted in [false, true] {
        let mut runtime = ProviderRuntime::empty();
        let first = fake(
            &mut runtime,
            "primary",
            vec![failure(ProviderFailureKind::Timeout)],
            if posted {
                vec!["A long synthetic answer with enough characters to cross the initial post threshold.".into()]
            } else {
                vec!["short".into()]
            },
            true,
        );
        let second = fake(&mut runtime, "secondary", vec![], vec![], true);
        let mut state = crate::runtime::AppState::in_memory();
        Arc::get_mut(&mut state).unwrap().providers = runtime;
        let context = crate::memory::PersonaContext::empty();
        let out = crate::pipeline::testing::FakeOut::default();
        let result = crate::generation::generate_read_only(
            &state,
            crate::persona::Persona::Abbey,
            &crate::generation::Ask {
                session_mode: crate::generation::SessionMode::Ephemeral,
                scope: "scope",
                context: &context,
                user_input: "question",
                now: 1,
            },
            Some(crate::generation::Delivery {
                out: &out,
                native_channel_id: "channel",
                reply_to: None,
            }),
        )
        .await;
        assert_eq!(result.is_ok(), !posted);
        assert_eq!(first.calls.load(Ordering::Relaxed), 1);
        assert_eq!(second.calls.load(Ordering::Relaxed), usize::from(!posted));
    }
}
#[tokio::test]
async fn tool_continuation_is_pinned_and_a_validated_host_effect_cannot_replay() {
    let mut runtime = ProviderRuntime::empty();
    let call = crate::tools::ToolCall {
        id: "call".into(),
        name: "remember_fact".into(),
        arguments: serde_json::json!({"fact":"synthetic fact"}),
    };
    let first = fake(
        &mut runtime,
        "primary",
        vec![
            Ok(ModelTurn {
                text: String::new(),
                calls: vec![call],
            }),
            failure(ProviderFailureKind::Timeout),
        ],
        vec![],
        true,
    );
    let second = fake(&mut runtime, "secondary", vec![], vec![], true);
    let mut state = crate::runtime::AppState::in_memory();
    Arc::get_mut(&mut state).unwrap().providers = runtime;
    let context = crate::memory::PersonaContext::empty();
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "guild".into(),
        scoped_user: "user".into(),
        scoped_channel: "channel".into(),
        now: 1,
        persona: crate::persona::Persona::Abbey,
    };
    assert!(
        crate::generation::generate_with_tools_without_delivery(
            &state,
            &mut host,
            &crate::generation::Ask {
                session_mode: crate::generation::SessionMode::Ephemeral,
                scope: "scope",
                context: &context,
                user_input: "question",
                now: 1
            }
        )
        .await
        .is_err()
    );
    assert_eq!(first.calls.load(Ordering::Relaxed), 2);
    assert_eq!(second.calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        lock(&first.tools)[0],
        crate::tools::production_tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>()
    );
}
#[tokio::test]
async fn simultaneous_conversations_keep_independent_pins_and_cancellation_releases_slots() {
    let mut runtime = ProviderRuntime::empty();
    fake(&mut runtime, "primary", vec![], vec![], true);
    fake(&mut runtime, "secondary", vec![], vec![], true);
    let mut first = runtime.begin(false, false);
    first.reserve().await.unwrap();
    let mut second = runtime.begin(false, false);
    second.reserve().await.unwrap();
    assert_eq!(
        lock(&first.effects.0).selected().unwrap().as_str(),
        "primary"
    );
    assert_eq!(
        lock(&second.effects.0).selected().unwrap().as_str(),
        "secondary"
    );
    drop(first);
    assert_eq!(
        runtime.entries[&ProviderId::parse("primary").unwrap()]
            .slots
            .available_permits(),
        1
    );
    assert_eq!(
        lock(&second.effects.0).selected().unwrap().as_str(),
        "secondary"
    );
}
#[tokio::test]
async fn reserved_half_open_cancellation_leaves_a_probe_available() {
    let mut runtime = ProviderRuntime::empty();
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    runtime.clock = clock.clone();
    fake(
        &mut runtime,
        "primary",
        vec![failure(ProviderFailureKind::Timeout); 3],
        vec![],
        true,
    );
    for _ in 0..3 {
        let _ = runtime.chat("", &[]).await;
    }
    clock.0.store(60000, Ordering::Relaxed);
    let mut probe = runtime.begin(false, false);
    probe.reserve().await.unwrap();
    drop(probe);
    let mut another = runtime.begin(false, false);
    another.reserve().await.unwrap();
}
#[tokio::test]
async fn voice_is_local_read_only_and_tool_incapable() {
    let mut runtime = ProviderRuntime::empty();
    let cloud = fake(&mut runtime, "cloud", vec![], vec![], false);
    let local = fake(&mut runtime, "local", vec![], vec![], true);
    let id = runtime.local_voice_route().unwrap();
    let mut voice = runtime.voice(&id);
    voice
        .execute("", &[], &[], ResponseStyle::Spoken, None)
        .await
        .unwrap();
    assert!(!voice.tools_available());
    assert_eq!(cloud.calls.load(Ordering::Relaxed), 0);
    assert_eq!(local.calls.load(Ordering::Relaxed), 1);
    assert!(lock(&local.tools)[0].is_empty());
}

struct ImageFake {
    calls: AtomicUsize,
}
impl VisionAdapter for ImageFake {
    fn image(
        &self,
        _: bool,
        _: Vec<u8>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, VisionError>> + Send + '_>>
    {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(std::future::ready(Err(VisionError::classified(
            "synthetic image failure",
            ProviderFailureKind::Timeout,
        ))))
    }
}
#[tokio::test]
async fn image_submission_never_invokes_a_second_provider() {
    let mut runtime = ProviderRuntime::empty();
    let first = Arc::new(ImageFake {
        calls: AtomicUsize::new(0),
    });
    let second = Arc::new(ImageFake {
        calls: AtomicUsize::new(0),
    });
    for (name, image) in [("vision", first.clone()), ("other-vision", second.clone())] {
        runtime.register(
            ProviderId::parse(name).unwrap(),
            "synthetic image",
            ProviderClass::LocalServer,
            ProviderCapabilities {
                vision: true,
                ocr: true,
                ..ProviderCapabilities::default()
            },
            ExecutionLocality::SameHost,
            ProviderProvenance::Configuration,
            true,
            config_identity(name.as_bytes()),
            None,
            Some(image),
            false,
            false,
        );
    }
    assert!(runtime.describe(vec![1, 2, 3]).await.is_err());
    assert_eq!(first.calls.load(Ordering::Relaxed), 1);
    assert_eq!(second.calls.load(Ordering::Relaxed), 0);
}
#[tokio::test]
async fn full_capacity_waits_then_times_out_without_executing_and_cancelled_wait_releases() {
    let mut runtime = ProviderRuntime::empty();
    runtime.queue_secs = 1;
    let adapter = fake(&mut runtime, "primary", vec![], vec![], true);
    let mut first = runtime.begin(false, false);
    first.reserve().await.unwrap();
    let mut second = runtime.begin(false, false);
    let started = Instant::now();
    let error = second.reserve().await.unwrap_err();
    assert_eq!(error.unavailable(), Some(RouteUnavailableReason::Busy));
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
    drop(first);
    second.reserve().await.unwrap();
    drop(second);
    assert_eq!(
        runtime.entries[&ProviderId::parse("primary").unwrap()]
            .slots
            .available_permits(),
        1
    );
}
#[tokio::test]
async fn operational_blocks_survive_restart_match_identity_and_reject_pending_publication() {
    let dir = std::env::temp_dir().join(format!("abbey-runtime-blocks-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("private").join("provider-blocks.json");
    let mut original = ProviderRuntime::empty();
    fake(
        &mut original,
        "primary",
        vec![failure(ProviderFailureKind::Authentication)],
        vec![],
        true,
    );
    original.restore_blocks(path.clone()).unwrap();
    assert!(original.chat("", &[]).await.is_err());
    let mut restarted = ProviderRuntime::empty();
    let adapter = fake(&mut restarted, "primary", vec![], vec![], true);
    restarted.restore_blocks(path.clone()).unwrap();
    let error = restarted.chat("", &[]).await.unwrap_err();
    assert_eq!(
        error.unavailable(),
        Some(RouteUnavailableReason::BlockedPendingRequalification)
    );
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
    let mut replaced = ProviderRuntime::empty();
    fake(&mut replaced, "primary", vec![], vec![], true);
    let id = ProviderId::parse("primary").unwrap();
    let identity = config_identity(b"new explicit identity");
    let profile = ScoreProducerPolicy::V1
        .compatibility(
            RequestClass::TextReadOnly,
            ProviderCapabilities::text(),
            ExecutionLocality::SameHost,
        )
        .unwrap();
    lock(&replaced.state).router.requalify(
        id.clone(),
        identity.clone(),
        vec![profile],
        RouteAdmission::QUALIFIED,
    );
    replaced.entries.get_mut(&id).unwrap().identity = identity;
    replaced.restore_blocks(path.clone()).unwrap();
    assert!(replaced.chat("", &[]).await.is_ok());
    std::fs::write(path.with_extension("pending"), b"pending").unwrap();
    assert!(ProviderRuntime::empty().restore_blocks(path).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn unqualified_and_unsupported_adapters_cannot_execute() {
    let mut runtime = ProviderRuntime::empty();
    let fake = fake(&mut runtime, "primary", vec![], vec![], true);
    let mut config = ProviderConfig::from_iter([
        ("ABBEY_PROVIDER_DISABLED", "primary"),
        ("ABBEY_PROVIDER_DISCOVERY", "future"),
    ])
    .unwrap();
    config.order = vec![ProviderId::parse("future").unwrap()];
    runtime.apply_configuration(config);
    assert!(runtime.chat("", &[]).await.is_err());
    assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    let inspect = crate::inspect::render_provider(&runtime.inspect_snapshot());
    assert!(inspect.contains("future: routable no"));
    assert!(!inspect.contains("http"));
}
#[tokio::test]
async fn explicit_adaptive_order_uses_live_scoring_while_legacy_retains_primary() {
    for adaptive in [false, true] {
        let mut runtime = ProviderRuntime::empty();
        fake(
            &mut runtime,
            "primary",
            vec![failure(ProviderFailureKind::Timeout)],
            vec![],
            true,
        );
        fake(&mut runtime, "secondary", vec![], vec![], true);
        assert!(runtime.chat("", &[]).await.is_ok());
        if adaptive {
            runtime.apply_configuration(
                ProviderConfig::from_iter([("ABBEY_PROVIDER_ORDER", "primary,secondary")]).unwrap(),
            );
        }
        let mut conversation = runtime.begin(false, false);
        conversation.reserve().await.unwrap();
        assert_eq!(
            lock(&conversation.effects.0).selected().unwrap().as_str(),
            if adaptive { "secondary" } else { "primary" }
        );
    }
}

#[derive(Default)]
struct UncertainDelivery {
    recorded: crate::pipeline::testing::FakeOut,
    accepted: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl crate::pipeline::Outbound for UncertainDelivery {
    async fn send(
        &self,
        channel: &str,
        message: &crate::platform::OutboundMessage,
    ) -> Result<String, String> {
        self.recorded.send(channel, message).await?;
        self.accepted.notify_one();
        self.release.notified().await;
        Err("synthetic delivery result lost after acceptance".into())
    }
    async fn edit(&self, channel: &str, id: &str, text: &str) -> Result<(), String> {
        self.recorded.edit(channel, id, text).await
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        Err("no network".into())
    }
}

#[tokio::test]
async fn accepted_delivery_with_lost_result_cannot_replay_on_another_provider() {
    let mut runtime = ProviderRuntime::empty();
    let first = fake(&mut runtime, "primary", vec![failure(ProviderFailureKind::Timeout)],
        vec!["A sufficiently long synthetic answer that must be posted before the provider finishes.".into()], true);
    let second = fake(&mut runtime, "secondary", vec![], vec![], true);
    let mut state = crate::runtime::AppState::in_memory();
    Arc::get_mut(&mut state).unwrap().providers = runtime;
    let context = crate::memory::PersonaContext::empty();
    let out = UncertainDelivery::default();
    let ask = crate::generation::Ask {
        session_mode: crate::generation::SessionMode::Ephemeral,
        scope: "scope",
        context: &context,
        user_input: "question",
        now: 1,
    };
    let generation = crate::generation::generate_read_only(
        &state,
        crate::persona::Persona::Abbey,
        &ask,
        Some(crate::generation::Delivery {
            out: &out,
            native_channel_id: "channel",
            reply_to: None,
        }),
    );
    tokio::pin!(generation);
    tokio::select! {
        result = &mut generation => panic!("delivery must still be pending: {result:?}"),
        _ = out.accepted.notified() => {}
    }
    assert_eq!(out.recorded.sent.lock().unwrap().len(), 1);
    assert_eq!(second.calls.load(Ordering::Relaxed), 0);
    out.release.notify_one();
    assert!(generation.await.is_err());
    assert_eq!(
        first.calls.load(Ordering::Relaxed),
        0,
        "adapter was cancelled while delivering deltas"
    );
    assert_eq!(second.calls.load(Ordering::Relaxed), 0);
    assert_eq!(out.recorded.sent.lock().unwrap().len(), 1);
}

#[cfg(unix)]
struct FakeWallClock(AtomicU64);
#[cfg(unix)]
impl ProviderClock for FakeWallClock {
    fn now_ms(&self) -> u64 {
        0
    }
    fn unix_secs(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(unix)]
#[tokio::test]
async fn verified_new_qualification_witness_clears_only_its_exact_persisted_block() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = std::env::temp_dir().join(format!(
        "abbey-runtime-requalification-{}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let executable = root.join("synthetic-fm");
    std::fs::write(
        &executable,
        b"synthetic executable identity only; never run",
    )
    .unwrap();
    let cfg = FmConfig {
        mode: FmMode::System,
        endpoint: None,
        cli: executable,
        fallback: true,
        timeout_secs: 1,
    };
    let record: super::super::ProviderRecord = serde_json::from_value(serde_json::json!({
        "version": 2, "fixture_version": super::super::FIXTURE_VERSION,
        "provider_id": "foundation-models", "provider_class": "os_managed_local",
        "identity": super::super::qualification::fm_manifest_identity(&cfg).unwrap(),
        "declared_capabilities": { "text": true, "streaming": false, "structured_output": true,
            "tools": true, "vision": true, "ocr": true },
        "isolation_capabilities": { "environment_cleared": true, "absolute_no_shell_execution": true,
            "process_tree_contained": false, "private_runtime_state": true, "loopback_only": false, "sandbox_attested": false },
        "qualification_status": "qualified"
    })).unwrap();
    let manifest = root.join("providers.json");
    let blocks = root.join("provider-blocks.json");
    let mut base = super::super::qualification::unix_now();
    let clock = Arc::new(FakeWallClock(AtomicU64::new(base - 10)));
    let write = |records: &[super::super::ProviderRecord]| {
        std::fs::write(
            &manifest,
            super::super::manifest::encode_v2(records).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    let build = || {
        let verified = super::super::qualification::verify_fm_manifest(&manifest, &cfg).unwrap();
        let fm = FoundationModels::new_qualified(cfg.clone(), None, true, verified);
        let mut runtime = ProviderRuntime::legacy(None, None, Some(fm), None, true, 1, 1);
        runtime.clock = clock.clone();
        runtime.apply_fm_qualification(&manifest).unwrap();
        runtime.restore_blocks(blocks.clone()).unwrap();
        runtime
    };
    async fn block(runtime: &ProviderRuntime) {
        let mut conversation = runtime.begin(false, false);
        conversation.reserve().await.unwrap();
        conversation
            .lease
            .take()
            .unwrap()
            .complete(Some(&LlmError::classified(
                "synthetic",
                ProviderFailureKind::Authentication,
            )));
    }
    write(std::slice::from_ref(&record));
    block(&build()).await;
    assert!(
        !build().generation_available(),
        "unwitnessed restart cannot clear a block"
    );
    super::super::manifest::publish_v2(&manifest, std::slice::from_ref(&record)).unwrap();
    base = super::super::qualification::unix_now();
    clock.0.store(base + 1, Ordering::Relaxed);
    let freshly_qualified = build();
    assert!(freshly_qualified.generation_available());
    block(&freshly_qualified).await;
    assert!(
        !build().generation_available(),
        "same witnessed qualification remains blocked"
    );
    let super::super::manifest::ManifestDocument::V2(saved) =
        super::super::manifest::read_manifest(&manifest).unwrap()
    else {
        panic!("v2")
    };
    let previous = saved.records()[0].clone();
    let mut unrelated = previous.clone();
    unrelated.provider_id = ProviderId::parse("unrelated").unwrap();
    unrelated.qualification_run_nonce = Some("ab".repeat(32));
    write(&[previous.clone(), unrelated]);
    assert!(
        !build().generation_available(),
        "another provider's evidence cannot unblock this one"
    );
    let mut newer = previous.clone();
    newer.qualification_generation = Some(previous.qualification_generation.unwrap() + 1);
    newer.qualification_run_nonce = Some("cd".repeat(32));
    // A qualification can have a generation higher than the startup record yet
    // still have finished before the later runtime failure. It is not recovery.
    newer.qualification_completed_unix_secs = Some(base);
    write(std::slice::from_ref(&newer));
    assert!(
        !build().generation_available(),
        "pre-failure publication cannot clear later block"
    );
    newer.qualification_completed_unix_secs = Some(base + 2);
    write(std::slice::from_ref(&newer));
    assert!(
        !build().generation_available(),
        "future qualification cannot clear block"
    );
    clock.0.store(base + 3, Ordering::Relaxed);
    assert!(
        build().generation_available(),
        "new completed qualification clears same identity"
    );
    block(&build()).await;
    write(std::slice::from_ref(&previous));
    assert!(
        !build().generation_available(),
        "older valid witness replay cannot clear newer block"
    );
    write(std::slice::from_ref(&newer));
    assert!(!build().generation_available());
    newer.qualification_generation = Some(newer.qualification_generation.unwrap() + 1);
    newer.qualification_completed_unix_secs = Some(base + 4);
    newer.qualification_run_nonce = Some("ef".repeat(32));
    write(std::slice::from_ref(&newer));
    clock.0.store(base + 5, Ordering::Relaxed);
    assert!(build().generation_available());
    // Legacy block records without a failure-time baseline remain blocked.
    let mut old_blocks: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&blocks).unwrap()).unwrap();
    old_blocks[0]
        .as_object_mut()
        .unwrap()
        .remove("blocked_unix_secs");
    std::fs::write(&blocks, serde_json::to_vec(&old_blocks).unwrap()).unwrap();
    assert!(!build().generation_available());

    std::fs::remove_file(&blocks).unwrap();
    use super::super::qualification::{
        CapabilityEvidence, CapabilityEvidenceSet, ProviderEvidence, QualificationReport,
        QualificationTarget,
    };
    let identity = super::super::qualification::fm_identity(&cfg).unwrap();
    let mut report = QualificationReport {
        version: 1,
        fixture_version: super::super::FIXTURE_VERSION.into(),
        generated_unix_secs: base,
        target: QualificationTarget::Fm,
        overall_pass: true,
        primary: ProviderEvidence::skipped(),
        fm_server: ProviderEvidence::skipped(),
        fm_cli: ProviderEvidence {
            configured: true,
            identity: Some(identity.clone()),
            vision_identity: Some(identity),
            capabilities: CapabilityEvidenceSet {
                text: CapabilityEvidence::pass(),
                streaming: CapabilityEvidence::unsupported(),
                structured_output: CapabilityEvidence::pass(),
                tools: CapabilityEvidence::pass(),
                vision: CapabilityEvidence::pass(),
                ocr: CapabilityEvidence::pass(),
            },
        },
    };
    std::fs::write(&manifest, serde_json::to_vec(&report).unwrap()).unwrap();
    block(&build()).await;
    assert!(!build().generation_available());
    report.primary.capabilities.text = CapabilityEvidence::unsupported();
    std::fs::write(&manifest, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(
        !build().generation_available(),
        "format or unrelated route changes cannot clear v1 block"
    );
    report.generated_unix_secs = base + 4;
    std::fs::write(&manifest, serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(
        !build().generation_available(),
        "higher-than-startup v1 report predates failure"
    );
    report.generated_unix_secs = base + 6;
    std::fs::write(&manifest, serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(
        !build().generation_available(),
        "future v1 report cannot recover"
    );
    clock.0.store(base + 7, Ordering::Relaxed);
    let recovered_v1 = build();
    assert!(recovered_v1.generation_available());
    clock.0.store(base + 1, Ordering::Relaxed);
    block(&recovered_v1).await;
    clock.0.store(base + 7, Ordering::Relaxed);
    report.generated_unix_secs = base + 4;
    std::fs::write(&manifest, serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(
        !build().generation_available(),
        "clock regression cannot make older qualification evidence newer than its persisted high-water"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn readiness_is_request_specific_and_never_invokes_adapters() {
    let mut runtime = ProviderRuntime::empty();
    let image = Arc::new(ImageFake {
        calls: AtomicUsize::new(0),
    });
    runtime.register(
        ProviderId::parse("ocr-only").unwrap(),
        "synthetic OCR",
        ProviderClass::LocalServer,
        ProviderCapabilities {
            ocr: true,
            ..ProviderCapabilities::default()
        },
        ExecutionLocality::SameHost,
        ProviderProvenance::Configuration,
        true,
        config_identity(b"ocr-only"),
        None,
        Some(image.clone()),
        false,
        false,
    );
    assert_eq!(runtime.request_readiness(RequestClass::VisionOcr), Ok(()));
    assert_eq!(
        runtime.request_readiness(RequestClass::VisionDescribe),
        Err(RouteUnavailableReason::CapabilityUnavailable)
    );
    assert_eq!(image.calls.load(Ordering::Relaxed), 0);
    let adapter = fake(&mut runtime, "text", vec![], vec![], false);
    for tools in [false, true] {
        runtime.tools_enabled = tools;
        let class = RequestClass::text(runtime.tools_enabled());
        assert_eq!(runtime.begin(true, false).class, class);
        assert_eq!(runtime.request_readiness(class), Ok(()));
    }
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
}
