use super::*;
use crate::{
    llm::{ChatTurn, ModelTurn},
    provider::{ProviderId, TurnAdapter, TurnFuture},
    tools::{ToolCall, ToolSpec},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

struct Fake {
    id: ProviderId,
    calls: AtomicUsize,
    empty: bool,
}
impl TurnAdapter for Fake {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [ChatTurn],
        _: &'a [ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        unreachable!("benchmark must use execution seam")
    }
    fn execute<'a>(&'a self, request: crate::provider::AdapterRequest<'a>) -> TurnFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.empty {
                return Ok(ModelTurn {
                    text: String::new(),
                    calls: Vec::new(),
                });
            }
            let ordinal = (0..48)
                .find(|&i| execution::fixture(i).0 == request.turns[0].text)
                .expect("only deterministic synthetic source fixture");
            let (_, expected, tool) = execution::fixture(ordinal);
            if tool && request.turns.len() == 1 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                return Ok(ModelTurn {
                    text: String::new(),
                    calls: vec![ToolCall {
                        id: "fixture-call".into(),
                        name: "probe_status".into(),
                        arguments: serde_json::json!({"nonce": crate::provider_self_test::TOOL_NONCE}),
                    }],
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if let Some(sender) = request.deltas {
                sender.send(String::new())?;
                sender.send(expected.clone())?;
            }
            Ok(ModelTurn {
                text: expected,
                calls: Vec::new(),
            })
        })
    }
}
fn runtime(empty: bool, streaming: bool) -> (ProviderRuntime, Arc<Fake>) {
    let fake = Arc::new(Fake {
        id: ProviderId::parse("synthetic").unwrap(),
        calls: AtomicUsize::new(0),
        empty,
    });
    let mut runtime = ProviderRuntime::empty();
    if streaming {
        runtime.register_test_streaming_adapter(fake.clone());
    } else {
        runtime.register_test_adapter(fake.clone());
    }
    (runtime, fake)
}
#[tokio::test(start_paused = true)]
async fn runner_executes_48_synthetic_probes_and_12_fixture_only_tool_continuations() {
    let (runtime, fake) = runtime(false, true);
    let probes = execution::measure(
        &runtime,
        CancellationToken::new(),
        MeasurementMode::PrimaryStreaming,
    )
    .await;
    assert_eq!(probes.len(), 48);
    assert_eq!(fake.calls.load(Ordering::Relaxed), 60);
    let summary = aggregate::summarize(&probes, Population::SyntheticProvider48);
    assert_eq!(summary.counts.success, 48);
    assert_eq!(summary.counts.no_text, 0);
    for probe in &probes {
        assert_eq!(
            probe.provider_first_text.duration_ms,
            Some(10),
            "tool continuation uses current execution start"
        );
        assert_eq!(
            probe.first_visible,
            Measurement::missing(Missing::NotApplicable)
        );
        assert_eq!(
            probe.final_delivered,
            Measurement::missing(Missing::NotApplicable)
        );
    }
    // Print one content-free example for the required rendered-copy review.
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "example_probe": probes[36], "stage_origins": StageOrigins::current(),
        "qualification_gaps": [Gap::ManagedServiceIdentityUnwitnessed, Gap::ModelAndHardwareOperatorDeclared, Gap::DiscordDeliveryUnobserved, Gap::VoiceContentionUnmeasured, Gap::NotCapabilityQualification]
    })).unwrap());
}
#[tokio::test(start_paused = true)]
async fn nonstreaming_final_text_is_not_a_first_delta_and_empty_results_remain_in_denominator() {
    let (runtime, _) = runtime(false, false);
    let probes = execution::measure(
        &runtime,
        CancellationToken::new(),
        MeasurementMode::FmSystemNonStreaming,
    )
    .await;
    assert_eq!(
        probes
            .iter()
            .filter(|p| p.outcome == Outcome::Success)
            .count(),
        48
    );
    assert!(probes.iter().all(|p| p.provider_first_text
        == Measurement::missing(Missing::NonStreaming)
        && !p.no_text));
    let (runtime, _) = self::runtime(true, true);
    let probes = execution::measure(
        &runtime,
        CancellationToken::new(),
        MeasurementMode::PrimaryStreaming,
    )
    .await;
    let summary = aggregate::summarize(&probes, Population::SyntheticProvider48);
    assert_eq!(
        (
            summary.counts.attempted,
            summary.counts.failure,
            summary.counts.no_text
        ),
        (48, 48, 48)
    );
    assert!(
        probes
            .iter()
            .all(|p| p.provider_first_text.duration_ms.is_none())
    );
}
// Exercise the production FM decoder without starting a CLI or contacting a provider.
struct FmShaped {
    id: ProviderId,
    calls: AtomicUsize,
    continuations: AtomicUsize,
    wrong_nonce: bool,
}
impl TurnAdapter for FmShaped {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [ChatTurn],
        _: &'a [ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        unreachable!("benchmark must use execution seam")
    }
    fn execute<'a>(&'a self, request: crate::provider::AdapterRequest<'a>) -> TurnFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert!(request.deltas.is_none());
            let ordinal = (0..48)
                .find(|&i| execution::fixture(i).0 == request.turns[0].text)
                .expect("only deterministic synthetic source fixture");
            let (_, expected, with_tools) = execution::fixture(ordinal);
            let raw = if with_tools && request.turns.len() == 1 {
                let nonce = crate::provider_self_test::TOOL_NONCE;
                assert!(request.turns[0].text.contains(&format!("nonce {nonce}.")));
                assert_eq!(request.tools.len(), 1);
                assert_eq!(request.tools[0].name, "probe_status");
                assert_eq!(
                    request.tools[0].parameters,
                    serde_json::json!({"type":"object", "properties":{"nonce":{"type":"string","enum":[nonce]}},"required":["nonce"],"additionalProperties":false})
                );
                serde_json::json!({"probe_status": if self.wrong_nonce { "wrong-nonce" } else { nonce }})
            } else {
                assert!(request.tools.is_empty());
                if with_tools {
                    self.continuations.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(request.turns.len(), 3);
                    assert!(request.turns[2].text.contains(&expected));
                }
                serde_json::json!({"answer": expected})
            };
            let turn =
                crate::provider::parse_cli_output(&raw.to_string(), request.tools, "fm-fixture")?;
            if !turn.calls.is_empty() {
                assert_eq!(turn.calls.len(), 1);
                assert_eq!(
                    turn.calls[0].arguments,
                    serde_json::json!({"nonce": crate::provider_self_test::TOOL_NONCE})
                );
            }
            Ok(turn)
        })
    }
}
fn fm_shaped_runtime(wrong_nonce: bool) -> (ProviderRuntime, Arc<FmShaped>) {
    let fake = Arc::new(FmShaped {
        id: ProviderId::parse("synthetic").unwrap(),
        calls: AtomicUsize::new(0),
        continuations: AtomicUsize::new(0),
        wrong_nonce,
    });
    let mut runtime = ProviderRuntime::empty();
    runtime.register_test_adapter(fake.clone());
    (runtime, fake)
}
#[tokio::test]
async fn fm_shaped_runner_parses_all_48_probes_and_12_tool_continuations() {
    let (runtime, fake) = fm_shaped_runtime(false);
    let probes = execution::measure(
        &runtime,
        CancellationToken::new(),
        MeasurementMode::FmSystemNonStreaming,
    )
    .await;
    assert_eq!(probes.len(), 48);
    assert!(probes.iter().all(|p| p.outcome == Outcome::Success));
    assert!(
        probes
            .iter()
            .all(|p| p.provider_first_text == Measurement::missing(Missing::NonStreaming))
    );
    assert_eq!(fake.calls.load(Ordering::Relaxed), 60);
    assert_eq!(fake.continuations.load(Ordering::Relaxed), 12);
}
#[tokio::test]
async fn fm_shaped_runner_rejects_wrong_nonce_before_tool_continuation() {
    let (runtime, fake) = fm_shaped_runtime(true);
    let probes = execution::measure(
        &runtime,
        CancellationToken::new(),
        MeasurementMode::FmSystemNonStreaming,
    )
    .await;
    assert_eq!(probes.len(), 48);
    assert!(probes[..36].iter().all(|p| p.outcome == Outcome::Success));
    assert!(probes[36..].iter().all(|p| p.outcome == Outcome::Failure));
    // The first malformed FM decision blocks this provider pending requalification.
    // Later probes are still counted, but cannot reach the adapter.
    assert_eq!(
        probes[36].failure,
        Some(Failure::Provider(
            crate::provider::ProviderFailureKind::ResponseSchema
        ))
    );
    assert!(probes[37..].iter().all(|p| p.failure
        == Some(Failure::Provider(
            crate::provider::ProviderFailureKind::InvalidRequest
        ))
        && p.queue_wait == Measurement::missing(Missing::AdmissionRefused)));
    assert_eq!(fake.calls.load(Ordering::Relaxed), 36 + 1);
    assert_eq!(fake.continuations.load(Ordering::Relaxed), 0);
}
struct Cleanup {
    id: ProviderId,
    started: tokio::sync::Notify,
    requested: tokio::sync::Notify,
    release: tokio::sync::Notify,
    cleaned: AtomicUsize,
}
impl TurnAdapter for Cleanup {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [ChatTurn],
        _: &'a [ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        unreachable!()
    }
    fn execute_cancellable<'a>(
        &'a self,
        _: crate::provider::AdapterRequest<'a>,
        cancel: CancellationToken,
    ) -> TurnFuture<'a> {
        Box::pin(async move {
            self.started.notify_one();
            cancel.cancelled().await;
            self.requested.notify_one();
            self.release.notified().await;
            self.cleaned.fetch_add(1, Ordering::Relaxed);
            Err(crate::generation::stream_owner::cancelled())
        })
    }
}
#[tokio::test]
async fn cancelled_probe_waits_for_actual_adapter_cleanup_and_counts_only_attempted_work() {
    let fake = Arc::new(Cleanup {
        id: ProviderId::parse("synthetic").unwrap(),
        started: tokio::sync::Notify::new(),
        requested: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        cleaned: AtomicUsize::new(0),
    });
    let mut runtime = ProviderRuntime::empty();
    runtime.register_test_adapter(fake.clone());
    let cancel = CancellationToken::new();
    let work = execution::measure(
        &runtime,
        cancel.clone(),
        MeasurementMode::FmSystemNonStreaming,
    );
    tokio::pin!(work);
    tokio::select! { _ = fake.started.notified() => {}, _ = &mut work => panic!("work completed before cancellation") }
    cancel.cancel();
    tokio::select! { _ = fake.requested.notified() => {}, _ = &mut work => panic!("cancellation request claimed cleanup") }
    assert_eq!(fake.cleaned.load(Ordering::Relaxed), 0);
    fake.release.notify_one();
    let probes = work.await;
    assert_eq!(fake.cleaned.load(Ordering::Relaxed), 1);
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].outcome, Outcome::Incomplete);
    let s = aggregate::summarize(&probes, Population::SyntheticProvider48);
    assert_eq!(
        (s.planned, s.counts.attempted, s.counts.incomplete),
        (48, 1, 1)
    );
    // Permit returned after observed cleanup, not merely a cancelled waiter.
    drop(runtime.hold_test_slot(&fake.id).await);
}
#[test]
fn explicit_cli_rejects_remote_targets_extra_args_and_missing_identity() {
    let parse_args = |args: Vec<String>| parse(args.into_iter().map(OsString::from));
    let args = vec![
        "primary".into(),
        "--installed-artifact".into(),
        "/operator/abbey".into(),
        "--model-sha256".into(),
        "a".repeat(64),
        "--hardware-sha256".into(),
        "b".repeat(64),
        "--json".into(),
    ];
    assert!(parse_args(args.clone()).is_ok());
    let mut fm = args.clone();
    fm[0] = "fm-system".into();
    assert!(parse_args(fm).is_ok());
    for mode in ["fm", "pcc", "all"] {
        let mut v = args.clone();
        v[0] = mode.into();
        assert!(parse_args(v).is_err());
    }
    let mut v = args.clone();
    v.push("--managed-service".into());
    assert!(parse_args(v).is_err());
    let mut v = args;
    v[4] = String::new();
    assert!(parse_args(v).is_err());
    assert!(
        parse_args(vec![
            "compare".into(),
            "baseline.json".into(),
            "candidate.json".into(),
            "--json".into()
        ])
        .is_ok()
    );
}

#[test]
fn receipt_input_is_bounded_recomputed_and_never_a_capability_manifest() {
    let receipt = super::super::tests::receipt(Population::SyntheticProvider48);
    let mut report = Report {
        summary: aggregate::summarize(&receipt.probes, receipt.population),
        receipt,
        qualification_gaps: vec![Gap::NotCapabilityQualification],
        stage_origins: StageOrigins::current(),
    };
    let dir = std::env::temp_dir().join(format!(
        "abbey-benchmark-report-test-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("report.json");
    let encoded = serde_json::to_vec(&report).unwrap();
    assert!(serde_json::from_slice::<crate::provider::QualificationReport>(&encoded).is_err());
    std::fs::write(&path, &encoded).unwrap();
    assert!(read_report(&path).is_ok());
    report.summary.counts.no_text = 1;
    std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(read_report(&path).unwrap_err(), "invalid_summary");
    std::fs::write(&path, vec![b' '; 256 * 1024 + 1]).unwrap();
    assert_eq!(read_report(&path).unwrap_err(), "receipt_too_large");
    std::fs::write(&path, b"").unwrap();
    assert_eq!(
        artifact_hash(&path).unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
