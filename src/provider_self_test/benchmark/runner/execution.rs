//! Real reserve and adapter-start clocks, with synthetic tool results and scoped cleanup.
use super::*;
use crate::provider_self_test::TOOL_NONCE;
use crate::{
    llm::{ChatTurn, LlmError, ModelTurn, ResponseStyle},
    prompt_budget::PromptParts,
    tools::{ToolResult, ToolSpec},
};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const SYSTEM: &str = "Follow the synthetic test instruction exactly. Do not add explanations.";
fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn fixture(ordinal: usize) -> (String, String, bool) {
    if ordinal < 24 {
        (
            format!("Return only the decimal sum of {ordinal} and 7."),
            (ordinal + 7).to_string(),
            false,
        )
    } else if ordinal < 36 {
        let code = format!("fn abbey_probe() -> u32 {{ {} }}", ordinal - 24);
        (
            format!(
                "Write a Rust function named abbey_probe returning the u32 constant {}. Return exactly: {code}",
                ordinal - 24
            ),
            code,
            false,
        )
    } else {
        (
            format!(
                "Fixture {ordinal}. Call probe_status exactly once with nonce {TOOL_NONCE}. Do not answer yet."
            ),
            format!("ABBEY_BENCHMARK_DONE_{ordinal}"),
            true,
        )
    }
}
fn tool() -> ToolSpec {
    ToolSpec {
        name: "probe_status",
        description: "Return a synthetic fixture result without side effects.",
        parameters: serde_json::json!({"type":"object", "properties":{"nonce":{"type":"string","enum":[TOOL_NONCE]}},"required":["nonce"],"additionalProperties":false}),
    }
}
#[derive(Default)]
struct Clock {
    latest_start: Option<tokio::time::Instant>,
    first_start: Option<tokio::time::Instant>,
    first_text: Option<u64>,
    any_text: bool,
}

pub(super) async fn measure(
    runtime: &ProviderRuntime,
    cancel: CancellationToken,
    mode: MeasurementMode,
) -> Vec<Probe> {
    let mut probes = Vec::new();
    for ordinal in 0..48 {
        if cancel.is_cancelled() {
            break;
        }
        probes.push(attempt(runtime, ordinal, cancel.clone(), mode).await);
        if cancel.is_cancelled() {
            break;
        }
    }
    probes
}

async fn execute(
    conversation: &mut crate::provider::ProviderConversation<'_>,
    turns: Vec<ChatTurn>,
    tools: &[ToolSpec],
    clocks: &Arc<Mutex<Clock>>,
    cancel: CancellationToken,
) -> Result<ModelTurn, LlmError> {
    let parts = PromptParts::new(SYSTEM.into(), "", String::new(), turns);
    let parts = crate::prompt_budget::fitted(&parts, conversation.prompt_budget());
    let (sender, _) = crate::generation::stream_owner::channel();
    let observer = clocks.clone();
    sender.observe(Arc::new(move |text| {
        if text.trim().is_empty() {
            return;
        }
        let mut clocks = lock(&observer);
        clocks.any_text = true;
        if clocks.first_text.is_none() {
            clocks.first_text = clocks.latest_start.map(|s| millis(s.elapsed()));
        }
    }));
    let started = || {
        let mut clocks = lock(clocks);
        let now = tokio::time::Instant::now();
        clocks.latest_start = Some(now);
        clocks.first_start.get_or_insert(now);
    };
    let streaming = conversation.streams();
    // No spawned waiter is dropped on interruption: the runtime's cancellable
    // adapter future is awaited to its terminal cleanup, including FM child exit.
    let result = conversation
        .execute_parts_cancellable(
            &parts,
            tools,
            ResponseStyle::Default,
            streaming.then_some(sender),
            Some(&started),
            Some(cancel),
        )
        .await;
    if let Ok(turn) = &result
        && !turn.text.trim().is_empty()
    {
        lock(clocks).any_text = true;
    }
    result
}

async fn attempt(
    runtime: &ProviderRuntime,
    ordinal: usize,
    cancel: CancellationToken,
    mode: MeasurementMode,
) -> Probe {
    let (input, expected, with_tools) = fixture(ordinal);
    let clocks = Arc::new(Mutex::new(Clock::default()));
    let mut conversation = runtime.begin_source_only(with_tools, true);
    let queued = tokio::time::Instant::now();
    let admission = tokio::select! {
        biased;
        () = cancel.cancelled() => Err(crate::generation::stream_owner::cancelled()),
        result = conversation.reserve() => result,
    };
    let queue_wait = if admission.is_ok() {
        Measurement::observed(millis(queued.elapsed()))
    } else {
        Measurement::missing(if cancel.is_cancelled() {
            Missing::Interrupted
        } else {
            Missing::AdmissionRefused
        })
    };
    let first_text_missing = if !mode.streaming() {
        Missing::NonStreaming
    } else if admission.is_err() {
        Missing::AdmissionRefused
    } else {
        Missing::NoText
    };
    let mut failure = match admission {
        Err(error) => Some(Failure::Provider(error.provider_failure())),
        Ok(()) => {
            let tools = if with_tools { vec![tool()] } else { Vec::new() };
            let mut turns = vec![ChatTurn::user(input)];
            let first = execute(
                &mut conversation,
                turns.clone(),
                &tools,
                &clocks,
                cancel.clone(),
            )
            .await;
            let final_result = if with_tools {
                match first {
                    Ok(turn)
                        if turn.text.trim().is_empty()
                            && turn.calls.len() == 1
                            && turn.calls[0].name == "probe_status"
                            && turn.calls[0].arguments
                                == serde_json::json!({"nonce": TOOL_NONCE}) =>
                    {
                        // The only host dispatch is construction of this synthetic result.
                        conversation.effects().mark_tool_dispatched();
                        turns.push(ChatTurn::assistant_calls("", turn.calls.clone()));
                        turns.push(ChatTurn::tool_result(&ToolResult {
                            call_id: turn.calls[0].id.clone(),
                            name: "probe_status".into(),
                            content: format!(
                                "Synthetic fixture succeeded. Return exactly {expected}"
                            ),
                        }));
                        execute(&mut conversation, turns, &[], &clocks, cancel.clone()).await
                    }
                    Ok(_) => Err(LlmError::classified(
                        "synthetic tool fixture mismatch",
                        crate::provider::ProviderFailureKind::ToolSchema,
                    )),
                    Err(error) => Err(error),
                }
            } else {
                first
            };
            match final_result {
                Err(error) => Some(if error.detail() == "the response carried no answer text" {
                    Failure::EmptyResult
                } else {
                    Failure::Provider(error.provider_failure())
                }),
                Ok(turn) if turn.calls.is_empty() && turn.text.trim() == expected => None,
                Ok(_) => Some(Failure::FixtureMismatch),
            }
        }
    };
    if cancel.is_cancelled() {
        failure = Some(Failure::Provider(
            crate::provider::ProviderFailureKind::Cancelled,
        ));
    }
    let clocks = lock(&clocks);
    let completed = if failure.is_none() || matches!(failure, Some(Failure::FixtureMismatch)) {
        clocks
            .first_start
            .map(|start| Measurement::observed(millis(start.elapsed())))
            .unwrap_or_else(|| Measurement::missing(Missing::NotObserved))
    } else {
        Measurement::missing(if cancel.is_cancelled() {
            Missing::Interrupted
        } else {
            Missing::NotObserved
        })
    };
    Probe {
        ordinal,
        outcome: aggregate::classify(failure.as_ref()),
        failure,
        no_text: !clocks.any_text,
        queue_wait,
        provider_first_text: clocks
            .first_text
            .map(Measurement::observed)
            .unwrap_or_else(|| Measurement::missing(first_text_missing)),
        provider_completed: completed,
        first_visible: Measurement::missing(Missing::NotApplicable),
        final_delivered: Measurement::missing(Missing::NotApplicable),
    }
}
