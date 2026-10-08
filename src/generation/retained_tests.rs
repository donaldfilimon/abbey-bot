//! Exercises the production retained generation seam and real service joins.
use super::*;
use crate::service::{
    OperationKind, OwnedTaskKind, ReapOutcome, ServiceSupervisor, ShutdownReason,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Mode {
    Held,
    NonstreamHeld,
    EmptyFinal,
    Overflow,
    FinalOverflow,
}
struct Adapter {
    id: ProviderId,
    entered: Notify,
    finish: Notify,
    completed: Notify,
    dropped: AtomicUsize,
    calls: AtomicUsize,
    mode: Mode,
}
impl Adapter {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            id: ProviderId::parse("retained").unwrap(),
            entered: Notify::new(),
            finish: Notify::new(),
            completed: Notify::new(),
            dropped: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            mode,
        })
    }
}
impl crate::provider::TurnAdapter for Adapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> crate::provider::TurnFuture<'a> {
        unreachable!()
    }
    fn execute<'a>(
        &'a self,
        request: crate::provider::AdapterRequest<'a>,
    ) -> crate::provider::TurnFuture<'a> {
        Box::pin(async move {
            struct Done<'a>(&'a AtomicUsize);
            impl Drop for Done<'_> {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
            let _done = Done(&self.dropped);
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            match self.mode {
                Mode::Held | Mode::EmptyFinal => {
                    let sender = request.deltas.expect("streaming selected");
                    sender.send("A synthetic progressive response exceeding sixty characters for delivery.".into()).unwrap();
                    self.finish.notified().await;
                }
                Mode::NonstreamHeld => {
                    assert!(request.deltas.is_none());
                    self.finish.notified().await;
                }
                Mode::Overflow => {
                    let sender = request.deltas.expect("streaming selected");
                    sender.send("é".repeat(32_768)).unwrap();
                    assert!(sender.send("!".into()).is_err());
                    // A misbehaving adapter may ignore publication refusal.
                    // The owning round must still refuse its final tool turn.
                }
                Mode::FinalOverflow => {}
            }
            self.completed.notify_one();
            Ok(llm::ModelTurn {
                text: if matches!(self.mode, Mode::FinalOverflow) {
                    "x".repeat(65_537)
                } else if matches!(self.mode, Mode::EmptyFinal) {
                    String::new()
                } else {
                    "A complete bounded response.".into()
                },
                calls: if matches!(self.mode, Mode::Held | Mode::EmptyFinal) {
                    vec![]
                } else {
                    vec![crate::tools::ToolCall {
                        id: "synthetic".into(),
                        name: "switch_persona".into(),
                        arguments: serde_json::json!({"persona":"abi"}),
                    }]
                },
            })
        })
    }
}
fn fixture(
    mode: Mode,
) -> (
    Arc<AppState>,
    Arc<Adapter>,
    ServiceSupervisor,
    crate::service::persistence::PersistenceWriter,
) {
    let adapter = Adapter::new(mode);
    let mut state = AppState::in_memory();
    if matches!(mode, Mode::NonstreamHeld) {
        Arc::get_mut(&mut state)
            .unwrap()
            .providers
            .register_test_adapter(adapter.clone());
    } else {
        Arc::get_mut(&mut state)
            .unwrap()
            .providers
            .register_test_streaming_adapter(adapter.clone());
    }
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = state.attach_service(supervisor.operations());
    (state, adapter, supervisor, writer)
}
fn ask(context: &PersonaContext) -> Ask<'_> {
    Ask {
        session_mode: SessionMode::SourceOnly,
        subject: None,
        scope: "discord:synthetic",
        context,
        user_input: "synthetic",
        now: 1,
    }
}
async fn reap(supervisor: &mut ServiceSupervisor) {
    let start = supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    let report = supervisor
        .cancel_and_reap(start.budget.stage(tokio::time::Instant::now()))
        .await;
    assert_eq!(report.outcome, ReapOutcome::Joined);
    assert!(supervisor.outstanding().is_empty());
    assert!(supervisor.try_freeze(true).is_ok());
}

struct WithdrawingOut {
    state: Arc<AppState>,
    adapter: Arc<Adapter>,
    sent: AtomicUsize,
    edits: std::sync::Mutex<Vec<(String, String)>>,
    failure: Option<OutboundFailure>,
}
impl Outbound for WithdrawingOut {
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        {
            let mut stores = AppState::lock(&self.state.stores);
            let subject = stores
                .personal_memory
                .get_mut(&crate::personal_memory::subject_key(
                    "discord:g",
                    "discord:u",
                ))
                .unwrap();
            subject.choice = crate::personal_memory::UseChoice::Off;
            subject.advance().unwrap();
            stores.personal_memory_exposure.epoch += 1;
        }
        self.adapter.finish.notify_one();
        Ok("accepted-preview".into())
    }
    async fn edit(&self, _: &str, id: &str, text: &str) -> Result<(), OutboundFailure> {
        self.edits.lock().unwrap().push((id.into(), text.into()));
        self.failure.map_or(Ok(()), Err)
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        unreachable!()
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        unreachable!()
    }
}

async fn withdrawal_after_preview(failure: Option<OutboundFailure>) {
    let (state, adapter, mut supervisor, mut writer) = fixture(Mode::Held);
    let context = super::consent::tests::authorized_context(&state);
    let request = Ask {
        subject: Some(("discord:g", "discord:u")),
        scope: "discord:c",
        ..ask(&context)
    };
    let (mut telemetry, observed) =
        crate::service::telemetry::TelemetryWriter::recording_for_test();
    let requests = telemetry.requests();
    let status = crate::service::status::ManagedStatus::new(
        crate::readiness::RunIdentity::current().unwrap(),
        requests.clone(),
        crate::persist::PersistReport::memory_only(),
        false,
        false,
    );
    state.attach_observability(requests, Arc::new(status));
    let out = WithdrawingOut {
        state: state.clone(),
        adapter: adapter.clone(),
        sent: AtomicUsize::new(0),
        edits: Default::default(),
        failure,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        generate_read_only(
            &state,
            Persona::Abbey,
            &request,
            Some(Delivery {
                out: &out,
                native_channel_id: "c",
                reply_to: None,
            }),
        ),
    )
    .await
    .unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(1), supervisor.next_completion())
        .await
        .unwrap();
    assert_eq!(
        joined.kind,
        OwnedTaskKind::Operation(OperationKind::ProviderStream)
    );
    let permit = tokio::time::timeout(
        Duration::from_secs(1),
        state.providers.hold_test_slot(&adapter.id),
    )
    .await
    .unwrap();
    drop(permit);
    reap(&mut supervisor).await;
    writer.stop();
    writer.joined().await.unwrap();
    telemetry.stop();
    telemetry.joined().await.unwrap();
    let error = result.unwrap_err();
    assert_eq!(out.sent.load(Ordering::SeqCst), 1);
    assert_eq!(
        *out.edits.lock().unwrap(),
        [(
            "accepted-preview".into(),
            "**Abbey** — The context for this answer changed while I was checking it. Please ask again.".into()
        )]
    );
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(AppState::lock(&state.engine).session_len("discord:c"), 0);
    assert!(AppState::lock(&state.rewards).export_pending().is_empty());
    if let Some(failure) = failure {
        assert_eq!(error.kind(), llm::LlmErrorKind::Delivery);
        assert_eq!(error.outbound_failure(), Some(failure));
    } else {
        assert_eq!(error.kind(), llm::LlmErrorKind::ContextChanged);
    }
    let events = observed.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["code"] == "discord_first_post")
    );
    assert!(
        !events
            .iter()
            .any(|event| event["code"] == "discord_final_delivered")
    );
}

#[tokio::test]
async fn retained_withdrawal_replaces_accepted_preview_and_joins_producer() {
    withdrawal_after_preview(None).await;
}

#[tokio::test]
async fn retained_withdrawal_replacement_preserves_uncertain_delivery() {
    withdrawal_after_preview(Some(OutboundFailure::new(
        OutboundFailureCategory::Transport,
        DeliveryCertainty::PossiblySent,
        None,
    )))
    .await;
}
#[derive(Default)]
struct HeldOut {
    entered: Notify,
    release: Notify,
    edits: std::sync::Mutex<Vec<String>>,
}
impl Outbound for HeldOut {
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok("synthetic".into())
    }
    async fn edit(&self, _: &str, _: &str, text: &str) -> Result<(), OutboundFailure> {
        self.edits.lock().unwrap().push(text.into());
        Ok(())
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        Ok(())
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        Ok(vec![])
    }
}
#[tokio::test]
async fn managed_blocked_delivery_keeps_producer_polled_and_joined() {
    let (state, adapter, mut supervisor, mut writer) = fixture(Mode::Held);
    let out = HeldOut::default();
    let context = PersonaContext::default();
    let request = ask(&context);
    let delivery = Delivery {
        out: &out,
        native_channel_id: "synthetic",
        reply_to: None,
    };
    let work = generate_read_only(&state, Persona::Abbey, &request, Some(delivery));
    let witness = async {
        out.entered.notified().await;
        adapter.finish.notify_one();
        tokio::time::timeout(Duration::from_secs(1), adapter.completed.notified())
            .await
            .unwrap();
        let joined = tokio::time::timeout(Duration::from_secs(1), supervisor.next_completion())
            .await
            .unwrap();
        assert_eq!(
            joined.kind,
            OwnedTaskKind::Operation(OperationKind::ProviderStream)
        );
        assert_eq!(adapter.dropped.load(Ordering::SeqCst), 1);
        let permit = tokio::time::timeout(
            Duration::from_secs(1),
            state.providers.hold_test_slot(&adapter.id),
        )
        .await
        .unwrap();
        drop(permit);
        out.release.notify_one();
    };
    let (result, ()) = tokio::join!(work, witness);
    assert!(result.is_ok());
    assert_eq!(
        *out.edits.lock().unwrap(),
        ["A complete bounded response."],
        "the validated final turn must replace a longer progressive snapshot"
    );
    reap(&mut supervisor).await;
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn cancelled_queue_and_producer_are_joined() {
    for queued in [false, true] {
        for shutdown in [false, true] {
            let (state, adapter, mut supervisor, mut writer) = fixture(Mode::Held);
            let (mut telemetry, observed) =
                crate::service::telemetry::TelemetryWriter::recording_for_test();
            let requests = telemetry.requests();
            let status = crate::service::status::ManagedStatus::new(
                crate::readiness::RunIdentity::current().unwrap(),
                requests.clone(),
                crate::persist::PersistReport::memory_only(),
                false,
                false,
            );
            state.attach_observability(requests, Arc::new(status));
            let permit = if queued {
                Some(state.providers.hold_test_slot(&adapter.id).await)
            } else {
                None
            };
            let owned = state.clone();
            let caller = tokio::spawn(async move {
                let context = PersonaContext::default();
                generate_read_only::<NoDelivery>(&owned, Persona::Abbey, &ask(&context), None).await
            });
            if queued {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while supervisor.outstanding().is_empty() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            } else {
                adapter.entered.notified().await;
            }
            if shutdown {
                reap(&mut supervisor).await;
                let error = caller.await.unwrap().unwrap_err();
                assert!(
                    error.outbound_failure().is_none(),
                    "shutdown is cancellation, not delivery capacity"
                );
                assert_eq!(
                    error.provider_failure(),
                    crate::provider::ProviderFailureKind::Cancelled
                );
            } else {
                caller.abort();
                assert!(caller.await.unwrap_err().is_cancelled());
                let joined =
                    tokio::time::timeout(Duration::from_secs(1), supervisor.next_completion())
                        .await
                        .unwrap();
                assert_eq!(
                    joined.kind,
                    OwnedTaskKind::Operation(OperationKind::ProviderStream)
                );
                reap(&mut supervisor).await;
            }
            assert_eq!(adapter.dropped.load(Ordering::SeqCst), usize::from(!queued));
            drop(permit);
            let permit = tokio::time::timeout(
                Duration::from_secs(1),
                state.providers.hold_test_slot(&adapter.id),
            )
            .await
            .unwrap();
            drop(permit);
            writer.stop();
            writer.joined().await.unwrap();
            telemetry.stop();
            telemetry.joined().await.unwrap();
            let events = AppState::lock(&observed);
            let terminals: Vec<_> = events
                .iter()
                .filter(|event| {
                    matches!(
                        event["code"].as_str(),
                        Some("generation_failure" | "generation_completed")
                    )
                })
                .collect();
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0]["outcome"], "cancelled");
            assert!(terminals[0].get("error_category").is_none());
        }
    }
}
#[tokio::test]
async fn managed_overflow_never_dispatches_or_replays_ignored_delta_or_final_tools() {
    for (mode, streaming) in [
        (Mode::Overflow, true),
        (Mode::FinalOverflow, true),
        (Mode::FinalOverflow, false),
    ] {
        let primary = Adapter::new(mode);
        let mut secondary = Adapter::new(Mode::FinalOverflow);
        Arc::get_mut(&mut secondary).unwrap().id = ProviderId::parse("fallback").unwrap();
        let mut state = AppState::in_memory();
        let runtime = &mut Arc::get_mut(&mut state).unwrap().providers;
        if streaming {
            runtime.register_test_streaming_adapter(primary.clone());
        } else {
            runtime.register_test_adapter(primary.clone());
        }
        runtime.register_test_streaming_adapter(secondary.clone());
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let mut writer = state.attach_service(supervisor.operations());
        let context = consent::tests::authorized_context(&state);
        let before = AppState::lock(&state.stores)
            .memory
            .facts("discord:g", "discord:u")
            .to_vec();
        let request = Ask {
            subject: Some(("discord:g", "discord:u")),
            ..ask(&context)
        };
        let mut host = crate::runtime::ToolScope {
            state: &state,
            memory_turn: None,
            network: crate::platform::SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:synthetic".into(),
            now: 1,
            persona: Persona::Abbey,
        };
        let mut conversation = state.providers.begin_source_only(true, true);
        let result = generate_conversation_timed::<NoDelivery>(
            &state,
            &mut conversation,
            ToolAccess::Enabled(&mut host),
            &request,
            None,
            None,
            llm::ResponseStyle::Default,
            None,
        )
        .await;
        assert_eq!(
            result.unwrap_err().outbound_failure().unwrap().category(),
            OutboundFailureCategory::Capacity
        );
        assert_eq!(
            host.persona,
            Persona::Abbey,
            "switch_persona was never dispatched"
        );
        // Every EffectHost dispatch closes this existing authority predicate.
        // Checking it here does not execute another provider attempt.
        assert!(
            conversation.fallback(&llm::LlmError::classified(
                "synthetic",
                crate::provider::ProviderFailureKind::Timeout
            )),
            "no tool dispatch, visible output, or fallback budget consumption occurred"
        );
        assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
        assert_eq!(primary.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(
            secondary.calls.load(Ordering::SeqCst),
            0,
            "capacity failure never replays on fallback"
        );
        assert_eq!(
            AppState::lock(&state.stores)
                .memory
                .facts("discord:g", "discord:u"),
            before
        );
        reap(&mut supervisor).await;
        writer.stop();
        writer.joined().await.unwrap();
    }
}

#[test]
fn utf8_buffer_bound() {
    let mut buffer = stream_owner::CoalescedText::default();
    buffer.append(&"é".repeat(32_768)).unwrap();
    assert_eq!(buffer.snapshot().len(), 65_536);
    assert_eq!(buffer.append("!"), Err(stream_owner::BufferFull));
    assert_eq!(buffer.snapshot().len(), 65_536);
    let mut buffer = stream_owner::CoalescedText::default();
    buffer.append(&"x".repeat(65_535)).unwrap();
    assert_eq!(buffer.append("é"), Err(stream_owner::BufferFull));
    assert_eq!(buffer.snapshot().len(), 65_535);
}

#[tokio::test]
async fn managed_empty_final_replaces_partial_without_claiming_final_delivery() {
    let (state, adapter, mut supervisor, mut writer) = fixture(Mode::EmptyFinal);
    let out = HeldOut::default();
    let context = PersonaContext::default();
    let request = ask(&context);
    let timing = timing::Timing::new(&state, true);
    let mut conversation = state.providers.begin_source_only(false, true);
    let work = generate_conversation_timed(
        &state,
        &mut conversation,
        ToolAccess::Disabled(Persona::Abbey),
        &request,
        Some(Delivery {
            out: &out,
            native_channel_id: "synthetic",
            reply_to: None,
        }),
        None,
        llm::ResponseStyle::Default,
        Some(&timing),
    );
    let witness = async {
        out.entered.notified().await;
        adapter.finish.notify_one();
        supervisor.next_completion().await;
        out.release.notify_one();
    };
    let (result, ()) = tokio::join!(work, witness);
    timing.finish(&result);
    assert!(result.is_err());
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    let edits = out.edits.lock().unwrap().clone();
    assert_eq!(edits.len(), 1);
    assert!(edits[0].contains("there is no answer"), "{}", edits[0]);
    assert!(
        !AppState::lock(&timing.observed)
            .iter()
            .any(|(code, _)| *code == crate::observability::EventCode::DiscordFinalDelivered)
    );
    reap(&mut supervisor).await;
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn managed_capacity_has_one_serialized_generation_failure_without_delivery_claims() {
    for mode in [Mode::Overflow, Mode::FinalOverflow] {
        let (state, _, mut supervisor, mut persistence) = fixture(mode);
        let (mut telemetry, observed) =
            crate::service::telemetry::TelemetryWriter::recording_for_test();
        let requests = telemetry.requests();
        let status = crate::service::status::ManagedStatus::new(
            crate::readiness::RunIdentity::current().unwrap(),
            requests.clone(),
            crate::persist::PersistReport::memory_only(),
            false,
            false,
        );
        state.attach_observability(requests, Arc::new(status));
        let routes_before = state.providers.inspect_snapshot();
        let context = PersonaContext::default();
        let result =
            generate_read_only::<NoDelivery>(&state, Persona::Abbey, &ask(&context), None).await;
        assert_eq!(
            result.unwrap_err().outbound_failure().unwrap().category(),
            OutboundFailureCategory::Capacity
        );
        assert_eq!(
            state.providers.inspect_snapshot(),
            routes_before,
            "local overflow leaves provider circuit eligibility unchanged"
        );
        reap(&mut supervisor).await;
        persistence.stop();
        persistence.joined().await.unwrap();
        telemetry.stop();
        telemetry.joined().await.unwrap();
        let events = AppState::lock(&observed);
        let attempts: Vec<_> = events
            .iter()
            .filter(|event| event["code"] == "provider_attempt")
            .collect();
        assert_eq!(attempts.len(), 1);
        assert_eq!(
            attempts[0]["outcome"], "cancelled",
            "latched local overflow must not charge provider schema failure"
        );
        let failures: Vec<_> = events
            .iter()
            .filter(|event| event["code"] == "generation_failure")
            .collect();
        assert_eq!(
            failures.len(),
            1,
            "local capacity has one request-owned terminal failure"
        );
        assert_eq!(failures[0]["error_category"], "capacity");
        assert_eq!(failures[0]["component"], "provider");
        assert!(!events.iter().any(|event| matches!(
            event["code"].as_str(),
            Some("generation_completed" | "discord_post_failure" | "discord_final_delivered")
        )));
        for event in events.iter() {
            assert!(event.as_object().unwrap().keys().all(|key| {
                [
                    "schema_version",
                    "occurred_at_unix_ms",
                    "component",
                    "code",
                    "outcome",
                    "provider_id",
                    "duration_ms",
                    "error_category",
                ]
                .contains(&key.as_str())
            }));
        }
    }
}

#[tokio::test]
async fn managed_nonstream_cancellation_observes_join_without_normal_completion() {
    for withdrawn in [false, true] {
        let (state, adapter, mut supervisor, mut writer) = fixture(Mode::NonstreamHeld);
        let mut conversation = state.providers.begin_source_only(true, true);
        let seed = conversation.seed();
        let owned = state.clone();
        let caller = tokio::spawn(async move {
            let context = consent::tests::authorized_context(&owned);
            let request = Ask {
                subject: Some(("discord:g", "discord:u")),
                ..ask(&context)
            };
            let mut conversation = seed.resume(&owned.providers);
            let mut host = crate::runtime::ToolScope {
                state: &owned,
                memory_turn: None,
                network: crate::platform::SocialNetwork::Discord,
                scoped_guild: "discord:g".into(),
                scoped_user: "discord:u".into(),
                scoped_channel: "discord:synthetic".into(),
                now: 1,
                persona: Persona::Abbey,
            };
            let result = generate_conversation_timed::<NoDelivery>(
                &owned,
                &mut conversation,
                ToolAccess::Enabled(&mut host),
                &request,
                None,
                None,
                llm::ResponseStyle::Default,
                None,
            )
            .await;
            assert_eq!(host.persona, Persona::Abbey);
            result
        });
        adapter.entered.notified().await;
        if withdrawn {
            AppState::lock(&state.stores).personal_memory_exposure.epoch += 1;
        } else {
            caller.abort();
        }
        let joined = tokio::time::timeout(Duration::from_secs(1), supervisor.next_completion())
            .await
            .expect("nonstream cancellation must join before normal completion");
        assert_eq!(
            joined.kind,
            OwnedTaskKind::Operation(OperationKind::ProviderStream)
        );
        if withdrawn {
            let error = caller.await.unwrap().unwrap_err();
            assert_eq!(
                error.provider_failure(),
                crate::provider::ProviderFailureKind::Cancelled
            );
            assert_eq!(error.to_string(), consent::WITHDRAWN_REPLY);
        } else {
            assert!(caller.await.unwrap_err().is_cancelled());
        }
        assert!(
            conversation.fallback(&llm::LlmError::classified(
                "synthetic unexecuted admission probe",
                crate::provider::ProviderFailureKind::Timeout,
            )),
            "no tool dispatch, visible output, or consumed fallback attempt"
        );
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.dropped.load(Ordering::SeqCst), 1);
        let permit = tokio::time::timeout(
            Duration::from_secs(1),
            state.providers.hold_test_slot(&adapter.id),
        )
        .await
        .unwrap();
        drop(permit);
        reap(&mut supervisor).await;
        writer.stop();
        writer.joined().await.unwrap();
    }
}

mod continuity_access;
