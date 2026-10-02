//! Production generation regressions: incremental text and every provider admission.
use super::*;
use crate::{
    observability::EventCode,
    provider::{AdapterRequest, ProviderId, TurnAdapter, TurnFuture},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Adapter {
    id: ProviderId,
    calls: AtomicUsize,
    observers: AtomicUsize,
    fail: bool,
    tool_first: bool,
    incremental: bool,
}
impl Adapter {
    fn new(name: &str, fail: bool, tool_first: bool, incremental: bool) -> Arc<Self> {
        Arc::new(Self {
            id: ProviderId::parse(name).unwrap(),
            calls: AtomicUsize::new(0),
            observers: AtomicUsize::new(0),
            fail,
            tool_first,
            incremental,
        })
    }
}
impl TurnAdapter for Adapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        unreachable!("the production execute seam carries incremental observation")
    }
    fn execute<'a>(&'a self, request: AdapterRequest<'a>) -> TurnFuture<'a> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if request.deltas.is_some() {
                self.observers.fetch_add(1, Ordering::SeqCst);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.fail {
                return Err(llm::LlmError::classified(
                    "synthetic timeout",
                    crate::provider::ProviderFailureKind::Timeout,
                ));
            }
            if self.tool_first && call == 0 {
                assert!(
                    request
                        .tools
                        .iter()
                        .any(|tool| tool.name == "inspect_status")
                );
                return Ok(llm::ModelTurn {
                    text: String::new(),
                    calls: vec![crate::tools::ToolCall {
                        id: "synthetic".into(),
                        name: "inspect_status".into(),
                        arguments: serde_json::json!({"aspect":"runtime"}),
                    }],
                });
            }
            if self.incremental
                && let Some(tx) = request.deltas
            {
                tx.send("First incremental answer".into()).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(900)).await;
            Ok(llm::ModelTurn {
                text: "First incremental answer completed.".into(),
                calls: vec![],
            })
        })
    }
}
fn state(adapters: &[Arc<Adapter>], streaming: bool) -> Arc<AppState> {
    let mut state = AppState::in_memory();
    let mut runtime = crate::provider::ProviderRuntime::empty();
    for adapter in adapters {
        if streaming {
            runtime.register_test_streaming_adapter(adapter.clone());
        } else {
            runtime.register_test_adapter(adapter.clone());
        }
    }
    Arc::get_mut(&mut state).unwrap().providers = runtime;
    state
}
fn request(context: &PersonaContext) -> Ask<'_> {
    Ask {
        session_mode: SessionMode::SourceOnly,
        subject: None,
        scope: "discord:7",
        context,
        user_input: "Freshly authorized source",
        now: 1,
    }
}
fn phases(timing: &timing::Timing<'_>) -> Vec<(EventCode, u64)> {
    AppState::lock(&timing.observed).clone()
}
#[tokio::test(start_paused = true)]
async fn production_no_delivery_observes_early_delta_from_one_request() {
    let adapter = Adapter::new("primary", false, false, true);
    let state = state(std::slice::from_ref(&adapter), true);
    let timing = timing::Timing::new(&state, true);
    let context = PersonaContext::default();
    let ask = request(&context);
    let mut conversation = state.providers.begin_source_only(false, true);
    let result = generate_conversation_timed::<NoDelivery>(
        &state,
        &mut conversation,
        ToolAccess::Disabled(Persona::Abbey),
        &ask,
        None,
        None,
        llm::ResponseStyle::Default,
        Some(&timing),
    )
    .await;
    timing.finish(&result);
    assert_eq!(result.unwrap().1, None);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        phases(&timing),
        vec![
            (EventCode::GenerationQueue, 0),
            (EventCode::GenerationFirstText, 100),
            (EventCode::GenerationCompleted, 1000)
        ]
    );
}
#[tokio::test(start_paused = true)]
async fn production_nonstream_completion_never_claims_incremental_text() {
    let adapter = Adapter::new("primary", false, false, false);
    let state = state(std::slice::from_ref(&adapter), false);
    let timing = timing::Timing::new(&state, true);
    let context = PersonaContext::default();
    let ask = request(&context);
    let mut conversation = state.providers.begin_source_only(false, true);
    let result = generate_conversation_timed::<NoDelivery>(
        &state,
        &mut conversation,
        ToolAccess::Disabled(Persona::Abbey),
        &ask,
        None,
        None,
        llm::ResponseStyle::Default,
        Some(&timing),
    )
    .await;
    timing.finish(&result);
    assert!(result.is_ok());
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        phases(&timing),
        vec![
            (EventCode::GenerationQueue, 0),
            (EventCode::GenerationCompleted, 1000)
        ]
    );
}
#[tokio::test(start_paused = true)]
async fn production_fallback_queue_is_measured_and_excluded_from_first_text() {
    let first = Adapter::new("primary", true, false, true);
    let second = Adapter::new("secondary", false, false, true);
    let state = state(&[first.clone(), second.clone()], true);
    let mut blocker = state.providers.voice(&second.id);
    blocker.reserve().await.unwrap();
    let timing = timing::Timing::new(&state, true);
    let context = PersonaContext::default();
    let ask = request(&context);
    let mut conversation = state.providers.begin_source_only(false, true);
    let generation = generate_conversation_timed::<NoDelivery>(
        &state,
        &mut conversation,
        ToolAccess::Disabled(Persona::Abbey),
        &ask,
        None,
        None,
        llm::ResponseStyle::Default,
        Some(&timing),
    );
    let release = async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(blocker);
    };
    let (result, ()) = tokio::join!(generation, release);
    timing.finish(&result);
    assert!(result.is_ok());
    assert_eq!(first.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        phases(&timing),
        vec![
            (EventCode::GenerationQueue, 0),
            (EventCode::GenerationQueue, 4900),
            (EventCode::GenerationFirstText, 200),
            (EventCode::GenerationCompleted, 6000)
        ]
    );
}
#[tokio::test(start_paused = true)]
async fn production_tool_round_queue_is_measured_and_excluded_from_first_text() {
    let adapter = Adapter::new("primary", false, true, true);
    let state = state(std::slice::from_ref(&adapter), true);
    let timing = timing::Timing::new(&state, true);
    let context = consent::tests::authorized_context(&state);
    let ask = Ask {
        subject: Some(("discord:g", "discord:u")),
        scope: "discord:c",
        ..request(&context)
    };
    let mut host = crate::runtime::ToolScope {
        memory_turn: None,
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 1,
        persona: Persona::Abbey,
    };
    let mut conversation = state.providers.begin_source_only(true, true);
    let generation = generate_conversation_timed::<NoDelivery>(
        &state,
        &mut conversation,
        ToolAccess::Enabled(&mut host),
        &ask,
        None,
        None,
        llm::ResponseStyle::Default,
        Some(&timing),
    );
    let competing_request = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let blocker = state.providers.hold_test_slot(&adapter.id).await;
        tokio::time::sleep(Duration::from_millis(4900)).await;
        drop(blocker);
    };
    let (result, ()) = tokio::join!(generation, competing_request);
    timing.finish(&result);
    assert!(result.is_ok());
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        phases(&timing),
        vec![
            (EventCode::GenerationQueue, 0),
            (EventCode::GenerationQueue, 4900),
            (EventCode::GenerationFirstText, 200),
            (EventCode::GenerationCompleted, 6000)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn production_preparation_is_excluded_from_admission_wait() {
    let adapter = Adapter::new("primary", false, false, true);
    let state = state(std::slice::from_ref(&adapter), true);
    let mut blocker = state.providers.voice(&adapter.id);
    blocker.reserve().await.unwrap();
    let timing = timing::Timing::new(&state, true);
    // Setup/preparation precedes the canonical reservation interval.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let context = PersonaContext::default();
    let ask = request(&context);
    let mut conversation = state.providers.begin_source_only(false, true);
    let generation = generate_conversation_timed::<NoDelivery>(
        &state,
        &mut conversation,
        ToolAccess::Disabled(Persona::Abbey),
        &ask,
        None,
        None,
        llm::ResponseStyle::Default,
        Some(&timing),
    );
    let release = async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(blocker);
    };
    let (result, ()) = tokio::join!(generation, release);
    timing.finish(&result);
    assert!(result.is_ok());
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        phases(&timing),
        vec![
            (EventCode::GenerationQueue, 300),
            (EventCode::GenerationFirstText, 100),
            (EventCode::GenerationCompleted, 2000)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn production_read_only_entry_without_delivery_observes_one_streamed_request() {
    let adapter = Adapter::new("primary", false, false, true);
    let state = state(std::slice::from_ref(&adapter), true);
    let context = PersonaContext::default();
    let ask = request(&context);
    let result = generate_read_only::<NoDelivery>(&state, Persona::Abbey, &ask, None)
        .await
        .unwrap();
    assert_eq!(result.1, None);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.observers.load(Ordering::SeqCst), 1);
}
