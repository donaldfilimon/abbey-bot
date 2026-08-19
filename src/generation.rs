//! The generation loop and its delivery: stream a reply from the backend,
//! post it early and edit it in place, run model-requested tools, repeat.
//!
//! [`crate::pipeline`] decides *whether* the bot speaks; this module decides
//! *how* a reply is produced once that decision is made. It is shared by the
//! forced path (mentions, DMs), policy replies, and `/persona ask`, and it
//! talks to the network only through [`Outbound`] and the llm transports, so
//! every branch below runs in tests behind fakes.

use std::future::Future;

use crate::ask;
use crate::llm;
use crate::memory::PersonaContext;
use crate::persona::Persona;
use crate::pipeline::Outbound;
use crate::platform::OutboundMessage;
use crate::runtime::AppState;

/// Progressive-reply pacing: post once this many characters have arrived…
pub const STREAM_FIRST_POST_CHARS: usize = 60;
/// …or this many seconds have passed since generation started, whichever first.
pub const STREAM_FIRST_POST_SECS: u64 = 4;
/// Then edit at most this often (Discord tolerates ~5 edits / 5 s per channel).
pub const STREAM_EDIT_EVERY_SECS: u64 = 2;

/// How one streamed round ended.
#[derive(Debug)]
pub enum StreamEnd {
    /// Final text (tidied) and the id of the message that holds it, if one
    /// was posted during streaming.
    Text(String, Option<String>),
    /// The model asked for tools instead of (or before) answering; nothing
    /// was posted. The caller runs them and streams again.
    Calls(Vec<crate::tools::ToolCall>),
}

/// Generate through a streaming transport, posting the reply as soon as
/// [`STREAM_FIRST_POST_CHARS`] have arrived or [`STREAM_FIRST_POST_SECS`]
/// have passed, then editing the message every [`STREAM_EDIT_EVERY_SECS`]
/// until the stream ends; the final edit carries the tidied full text.
///
/// If the stream ends with tool calls and no text was posted, returns
/// [`StreamEnd::Calls`] so the caller can run the tools and stream again. If
/// the stream fails after a partial message went out, that message is edited
/// to the honest failure line so a half-answer never stands as if whole.
pub async fn stream_reply<T: llm::StreamTransport + Sync, O: Outbound + Sync>(
    transport: &T,
    delivery: &Delivery<'_, O>,
    round: &Round<'_>,
) -> Result<StreamEnd, llm::LlmError> {
    let Delivery {
        out,
        native_channel_id,
        reply_to,
    } = *delivery;
    let Round {
        backend,
        system_prompt,
        turns,
        tools,
        persona,
    } = *round;
    let request = llm::build_stream_request(backend, system_prompt, turns, tools);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut stream = std::pin::pin!(transport.post_stream(&request, tx));
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(STREAM_EDIT_EVERY_SECS));
    tick.tick().await;
    let mut text = String::new();
    let mut posted: Option<String> = None;
    let mut last_edited_len = 0usize;
    let mut finished: Option<Result<llm::ModelTurn, llm::LlmError>> = None;

    // Post-or-edit with whatever has arrived, honouring the pacing rules.
    async fn flush<O: Outbound + Sync>(
        out: &O,
        channel: &str,
        reply_to: Option<&str>,
        text: &str,
        posted: &mut Option<String>,
        last_edited_len: &mut usize,
    ) -> Result<(), String> {
        if text.trim().is_empty() || text.chars().count() == *last_edited_len {
            return Ok(());
        }
        match posted {
            None => {
                let message = OutboundMessage {
                    text: text.to_string(),
                    reply_to_native_message_id: reply_to.map(str::to_string),
                    ..OutboundMessage::default()
                };
                let id = out.send(channel, &message).await?;
                *posted = Some(id);
            }
            Some(id) => out.edit(channel, id, text).await?,
        }
        *last_edited_len = text.chars().count();
        Ok(())
    }

    while finished.is_none() {
        tokio::select! {
            // Deltas first: a chunk that arrived just before completion must
            // be posted/edited before the final state is decided.
            biased;
            Some(delta) = rx.recv() => {
                text.push_str(&delta);
                let due = posted.is_none()
                    && (text.chars().count() >= STREAM_FIRST_POST_CHARS
                        || started.elapsed().as_secs() >= STREAM_FIRST_POST_SECS);
                if due {
                    flush(out, native_channel_id, reply_to, &text, &mut posted, &mut last_edited_len)
                        .await
                        .map_err(llm::LlmError)?;
                }
            }
            _ = tick.tick() => {
                if posted.is_some() || started.elapsed().as_secs() >= STREAM_FIRST_POST_SECS {
                    flush(out, native_channel_id, reply_to, &text, &mut posted, &mut last_edited_len)
                        .await
                        .map_err(llm::LlmError)?;
                }
            }
            result = &mut stream => finished = Some(result),
        }
    }
    // Drain anything that arrived between the last recv and completion.
    while let Ok(delta) = rx.try_recv() {
        text.push_str(&delta);
    }
    match finished.expect("loop exits only when finished is set") {
        Ok(turn) => {
            if !turn.calls.is_empty() && posted.is_none() && turn.text.trim().is_empty() {
                return Ok(StreamEnd::Calls(turn.calls));
            }
            let full = if turn.text.len() >= text.len() {
                turn.text
            } else {
                text
            };
            let tidy = ask::tidy_reply(persona, &full);
            if let Some(id) = &posted {
                out.edit(native_channel_id, id, &tidy)
                    .await
                    .map_err(llm::LlmError)?;
            }
            Ok(StreamEnd::Text(tidy, posted))
        }
        Err(e) => {
            if let Some(id) = &posted {
                let failure = ask::render_failure(persona, backend.label(), &e.0);
                let _ = out.edit(native_channel_id, id, &failure).await;
            }
            Err(e)
        }
    }
}

/// Type to name when a caller has no delivery channel (slash commands):
/// `generate::<NoDelivery>(…, None, …)`. Never constructed.
pub enum NoDelivery {}

impl Outbound for NoDelivery {
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, String> {
        match *self {}
    }
    async fn typing(&self, _: &str) {
        match *self {}
    }
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        match *self {}
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        match *self {}
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        match *self {}
    }
}

/// Where a generated reply should be delivered while it is being produced.
pub struct Delivery<'a, O> {
    pub out: &'a O,
    pub native_channel_id: &'a str,
    pub reply_to: Option<&'a str>,
}

impl<O> Clone for Delivery<'_, O> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<O> Copy for Delivery<'_, O> {}

/// One generation round: what to send the backend.
#[derive(Clone, Copy)]
pub struct Round<'a> {
    pub backend: &'a llm::Backend,
    pub system_prompt: &'a str,
    pub turns: &'a [llm::ChatTurn],
    pub tools: &'a [crate::tools::ToolSpec],
    pub persona: Persona,
}

/// What a round produced: text (tidied), the id of a message already holding
/// it, and any tool calls.
type RoundOutcome =
    Result<(Option<String>, Option<String>, Vec<crate::tools::ToolCall>), llm::LlmError>;

/// What `generate` is asked to do, independent of delivery.
pub struct Ask<'a> {
    pub scope: &'a str,
    pub context: &'a PersonaContext,
    pub user_input: &'a str,
    pub offer_tools: bool,
    pub now: u64,
}

/// The generation loop with tools: build the prompt for the scope's persona,
/// call the backend (streamed to `delivery` on the local path, single-shot
/// otherwise), run any tool calls against `host`, and repeat up to
/// [`crate::tools::MAX_TOOL_ROUNDS`] times until the model answers in text.
///
/// Returns the tidied text, the id of the message already holding it (if the
/// stream posted it), and the persona that ended up answering (tools may
/// switch it). A backend that rejects tooled requests (HTTP 4xx) is retried
/// once without tools and `tools_enabled` is cleared for the process.
pub async fn generate<O: Outbound + Sync>(
    state: &AppState,
    host: &mut crate::runtime::ToolScope<'_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
) -> Result<(String, Option<String>, Persona), llm::LlmError> {
    use std::sync::atomic::Ordering;
    let Ask {
        scope,
        context,
        user_input,
        offer_tools,
        now,
    } = *ask;
    let Some(backend) = &state.backend else {
        return Err(llm::LlmError("no generation backend is configured".into()));
    };
    let vocabulary = crate::tools::abbey_tools();
    let mut extra_turns: Vec<llm::ChatTurn> = Vec::new();
    for round in 0..=crate::tools::MAX_TOOL_ROUNDS {
        let persona = host.persona;
        let prepared =
            AppState::lock(&state.engine).prepare(scope, persona, context, user_input, now);
        let mut turns = prepared.turns.clone();
        turns.extend(extra_turns.iter().cloned());
        let offer = offer_tools
            && round < crate::tools::MAX_TOOL_ROUNDS
            && state.tools_enabled.load(Ordering::Relaxed);
        let tools: &[crate::tools::ToolSpec] = if offer { &vocabulary } else { &[] };

        let round = Round {
            backend,
            system_prompt: &prepared.system_prompt,
            turns: &turns,
            tools,
            persona,
        };
        let turn: RoundOutcome = match (&delivery, backend) {
            (Some(d), llm::Backend::OpenAiCompatible { .. }) => {
                match stream_reply(&state.llm, d, &round).await {
                    Ok(StreamEnd::Text(text, posted)) => Ok((Some(text), posted, Vec::new())),
                    Ok(StreamEnd::Calls(calls)) => Ok((None, None, calls)),
                    Err(e) => Err(e),
                }
            }
            _ => llm::chat_turn(&state.llm, backend, &prepared.system_prompt, &turns, tools)
                .await
                .map(|t| {
                    let text = if t.text.trim().is_empty() {
                        None
                    } else {
                        Some(ask::tidy_reply(persona, &t.text))
                    };
                    (text, None, t.calls)
                }),
        };

        let (text, posted, calls) = match turn {
            Ok(v) => v,
            Err(e) if offer && looks_like_tool_rejection(&e.0) => {
                tracing::warn!(error = %e.0, "backend rejected a tooled request; continuing without tools for this process");
                state.tools_enabled.store(false, Ordering::Relaxed);
                continue;
            }
            Err(e) => return Err(e),
        };

        if calls.is_empty() {
            if let Some(text) = text {
                return Ok((text, posted, persona));
            }
            return Err(llm::LlmError("the response carried no answer text".into()));
        }
        // Run the tools, append the round, go again.
        let mut results = Vec::with_capacity(calls.len());
        for call in &calls {
            let result = crate::tools::dispatch(call, host);
            tracing::info!(tool = %call.name, scope, result = %result.content.chars().take(80).collect::<String>(), "tool call");
            results.push(result);
        }
        extra_turns.push(llm::ChatTurn::assistant_calls(
            text.unwrap_or_default(),
            calls,
        ));
        extra_turns.extend(results.iter().map(llm::ChatTurn::tool_result));
    }
    Err(llm::LlmError(format!(
        "the model kept calling tools for {} rounds without answering",
        crate::tools::MAX_TOOL_ROUNDS
    )))
}

/// HTTP 4xx on a tooled request is how a backend without tool support says
/// so (ollama: "does not support tools"; others: 400 on unknown `tools`).
fn looks_like_tool_rejection(error: &str) -> bool {
    error.starts_with("HTTP 4")
        || error
            .to_ascii_lowercase()
            .contains("does not support tools")
}

/// Run `work` while re-broadcasting the typing indicator every 8 s. Discord's
/// indicator lasts ~10 s; a local model takes ~25 s; without this a successful
/// reply reads as silence for most of its generation.
pub async fn with_typing<O: Outbound + Sync, T>(
    out: &O,
    native_channel_id: &str,
    work: impl Future<Output = T>,
) -> T {
    let mut work = std::pin::pin!(work);
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(8));
    tick.tick().await; // the immediate first tick; the caller already typed once
    loop {
        tokio::select! {
            result = &mut work => return result,
            _ = tick.tick() => out.typing(native_channel_id).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::FakeOut;

    /// A streaming transport that replays canned deltas with small pauses.
    struct FakeStream {
        deltas: Vec<&'static str>,
        fail_at_end: bool,
        calls: Vec<crate::tools::ToolCall>,
    }

    impl llm::StreamTransport for FakeStream {
        async fn post_stream(
            &self,
            _request: &llm::LlmRequest,
            on_delta: tokio::sync::mpsc::UnboundedSender<String>,
        ) -> Result<llm::ModelTurn, llm::LlmError> {
            let mut full = String::new();
            for d in &self.deltas {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                full.push_str(d);
                let _ = on_delta.send((*d).to_string());
            }
            if self.fail_at_end {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                return Err(llm::LlmError("upstream died".into()));
            }
            Ok(llm::ModelTurn {
                text: full,
                calls: self.calls.clone(),
            })
        }
    }

    fn prepared() -> crate::engine::PreparedTurn {
        crate::engine::PreparedTurn {
            system_prompt: "S".into(),
            turns: vec![llm::ChatTurn::user("Q")],
        }
    }

    fn local_backend() -> llm::Backend {
        llm::Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma4:e4b".into(),
        }
    }

    #[tokio::test]
    async fn streaming_posts_early_then_edits_to_the_tidied_final_text() {
        let out = FakeOut::default();
        // 70+ chars across deltas → first post after the 60-char threshold,
        // final edit carries the whole tidied text.
        let transport = FakeStream {
            deltas: vec![
                "**Abbey**: Here is the first part of the answer, ",
                "which keeps going past sixty characters. ",
                "And then it finishes.",
            ],
            fail_at_end: false,
            calls: vec![],
        };
        let StreamEnd::Text(text, id) = stream_reply(
            &transport,
            &Delivery {
                out: &out,
                native_channel_id: "c1",
                reply_to: Some("m1"),
            },
            &Round {
                backend: &local_backend(),
                system_prompt: "S",
                turns: &prepared().turns,
                tools: &[],
                persona: Persona::Abbey,
            },
        )
        .await
        .expect("streamed") else {
            panic!("expected text")
        };
        assert_eq!(id.as_deref(), Some("sent-1"));
        assert!(
            text.starts_with("Here is the first part"),
            "persona echo stripped: {text}"
        );
        let sent = out.sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "exactly one post");
        assert_eq!(sent[0].1.reply_to_native_message_id.as_deref(), Some("m1"));
        let edited = out.edited.lock().unwrap();
        assert_eq!(
            edited.last().map(|e| e.2.as_str()),
            Some(text.as_str()),
            "last edit is the final text"
        );
    }

    #[tokio::test]
    async fn a_stream_that_ends_in_tool_calls_reports_them_unposted() {
        let out = FakeOut::default();
        let transport = FakeStream {
            deltas: vec![],
            fail_at_end: false,
            calls: vec![crate::tools::ToolCall {
                id: "call_1".into(),
                name: "recall".into(),
                arguments: serde_json::json!({"query": "rust"}),
            }],
        };
        let end = stream_reply(
            &transport,
            &Delivery {
                out: &out,
                native_channel_id: "c1",
                reply_to: None,
            },
            &Round {
                backend: &local_backend(),
                system_prompt: "S",
                turns: &prepared().turns,
                tools: &crate::tools::abbey_tools(),
                persona: Persona::Abbey,
            },
        )
        .await
        .expect("streamed");
        assert!(
            matches!(end, StreamEnd::Calls(ref c) if c.len() == 1 && c[0].name == "recall"),
            "{end:?}"
        );
        assert!(
            out.sent.lock().unwrap().is_empty(),
            "nothing posted for a tool round"
        );
    }

    #[tokio::test]
    async fn a_short_stream_is_returned_unposted_for_the_ordinary_send() {
        let out = FakeOut::default();
        let transport = FakeStream {
            deltas: vec!["Blue."],
            fail_at_end: false,
            calls: vec![],
        };
        let StreamEnd::Text(text, id) = stream_reply(
            &transport,
            &Delivery {
                out: &out,
                native_channel_id: "c1",
                reply_to: None,
            },
            &Round {
                backend: &local_backend(),
                system_prompt: "S",
                turns: &prepared().turns,
                tools: &[],
                persona: Persona::Abbey,
            },
        )
        .await
        .expect("streamed") else {
            panic!("expected text")
        };
        assert_eq!(text, "Blue.");
        assert!(
            id.is_none(),
            "under the threshold and under 4 s: nothing posted yet"
        );
        assert!(out.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stream_that_dies_after_posting_edits_in_the_failure_line() {
        let out = FakeOut::default();
        let transport = FakeStream {
            deltas: vec![
                "This is going to be a long and promising answer that then ",
                "stops abruptly mid",
            ],
            fail_at_end: true,
            calls: vec![],
        };
        let err = stream_reply(
            &transport,
            &Delivery {
                out: &out,
                native_channel_id: "c1",
                reply_to: None,
            },
            &Round {
                backend: &local_backend(),
                system_prompt: "S",
                turns: &prepared().turns,
                tools: &[],
                persona: Persona::Abbey,
            },
        )
        .await
        .expect_err("upstream died");
        assert_eq!(err.0, "upstream died");
        let edited = out.edited.lock().unwrap();
        assert!(
            edited.last().is_some_and(|e| e.2.contains("upstream died")),
            "{edited:?}"
        );
    }
}
