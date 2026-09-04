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
use crate::grounding::{self, Grounding};
use crate::llm;
use crate::memory::PersonaContext;
use crate::persona::Persona;
use crate::pipeline::Outbound;
use crate::platform::OutboundMessage;
use crate::provider::{ProviderCapabilities, ProviderRoute};
use crate::runtime::AppState;

mod foundation_models;

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
/// If the stream ends with tool calls and no text was produced or posted,
/// returns [`StreamEnd::Calls`] so the caller can run the tools and stream
/// again. A mixed text-and-tool turn is rejected: dispatching its calls would
/// let an already-visible claim get ahead of the actual side effect, while
/// ignoring them would make streaming disagree with completed generation. If
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
        grounding,
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
        grounding: &Grounding,
        posted: &mut Option<String>,
        last_edited_len: &mut usize,
    ) -> Result<(), String> {
        if text.trim().is_empty() || text.chars().count() == *last_edited_len {
            return Ok(());
        }
        let visible = apply_grounding(text, grounding);
        match posted {
            None => {
                let message = OutboundMessage {
                    text: visible,
                    reply_to_native_message_id: reply_to.map(str::to_string),
                    ..OutboundMessage::default()
                };
                let id = out.send(channel, &message).await?;
                *posted = Some(id);
            }
            Some(id) => out.edit(channel, id, &visible).await?,
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
                    flush(out, native_channel_id, reply_to, &text, grounding, &mut posted, &mut last_edited_len)
                        .await
                        .map_err(llm::LlmError::backend)?;
                }
            }
            _ = tick.tick() => {
                if posted.is_some() || started.elapsed().as_secs() >= STREAM_FIRST_POST_SECS {
                    flush(out, native_channel_id, reply_to, &text, grounding, &mut posted, &mut last_edited_len)
                        .await
                        .map_err(llm::LlmError::backend)?;
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
            if !turn.calls.is_empty() {
                if posted.is_none() && text.trim().is_empty() && turn.text.trim().is_empty() {
                    return Ok(StreamEnd::Calls(turn.calls));
                }
                let error = llm::LlmError::backend(
                    "backend returned text and tool calls in one streamed turn".into(),
                );
                if let Some(id) = &posted {
                    let failure = ask::render_failure(persona, backend.label(), &error);
                    let _ = out.edit(native_channel_id, id, &failure).await;
                }
                return Err(error);
            }
            let full = if turn.text.len() >= text.len() {
                turn.text
            } else {
                text
            };
            let tidy = finalize_reply(persona, &full, grounding);
            if let Some(id) = &posted {
                out.edit(native_channel_id, id, &tidy)
                    .await
                    .map_err(llm::LlmError::backend)?;
            }
            Ok(StreamEnd::Text(tidy, posted))
        }
        Err(e) => {
            if let Some(id) = &posted {
                let failure = ask::render_failure(persona, backend.label(), &e);
                let _ = out.edit(native_channel_id, id, &failure).await;
            }
            Err(e)
        }
    }
}

/// Internal outbound type for generation that intentionally has no delivery
/// channel. Public callers use one of the concrete no-delivery entry points
/// instead of supplying an uninhabited generic themselves.
enum NoDelivery {}

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
    /// Immutable pre-candidate sources used to guard every visible form of
    /// this round's reply.
    pub grounding: &'a Grounding,
}

/// Add only evidence-bearing read results from the current request to the
/// immutable grounding snapshot prepared by the engine. Successful execution
/// is not itself factual authority: mutation acknowledgements echo model input,
/// persona switches carry no evidence, and tool errors must not ground claims.
fn grounding_for_round(
    prepared: &crate::engine::PreparedTurn,
    tool_results: &[crate::tools::ToolResult],
) -> Grounding {
    let mut grounding = prepared.grounding().clone();
    for result in tool_results {
        if let Some(source) = result.grounding_source() {
            grounding.push_source(source);
        }
    }
    grounding
}

/// Apply the existing hedge policy without changing reply shape. Streaming
/// uses this on each accumulated candidate before it becomes visible.
fn apply_grounding(reply: &str, grounding: &Grounding) -> String {
    grounding::hedged(reply, &grounding::check(reply, grounding))
}

/// Canonical completed-reply boundary for streaming and non-streaming paths.
fn finalize_reply(persona: Persona, reply: &str, grounding: &Grounding) -> String {
    apply_grounding(&ask::tidy_reply(persona, reply), grounding)
}

/// What a round produced: text (tidied), the id of a message already holding
/// it, and any tool calls.
type RoundOutcome =
    Result<(Option<String>, Option<String>, Vec<crate::tools::ToolCall>), llm::LlmError>;

/// Whether prompt preparation may update shared conversation state.
#[derive(Clone, Copy)]
pub enum SessionMode {
    Shared,
    Ephemeral,
}

/// What generation is asked to do, independent of delivery and capabilities.
pub struct Ask<'a> {
    pub session_mode: SessionMode,
    pub scope: &'a str,
    pub context: &'a PersonaContext,
    pub user_input: &'a str,
    pub now: u64,
}

impl Ask<'_> {
    fn prepare(&self, state: &AppState, persona: Persona) -> crate::engine::PreparedTurn {
        match self.session_mode {
            SessionMode::Shared => AppState::lock(&state.engine).prepare(
                self.scope,
                persona,
                self.context,
                self.user_input,
                self.now,
            ),
            SessionMode::Ephemeral => AppState::lock(&state.engine).prepare_ephemeral(
                self.scope,
                persona,
                self.context,
                self.user_input,
            ),
        }
    }
}

/// The complete tool-capability boundary for one generation. Disabled turns
/// carry no host at all, so a read-only caller cannot accidentally expose a
/// live runtime scope to the model. Enabled turns use the canonical scope that
/// also owns persona switches.
enum ToolAccess<'host, 'state> {
    Disabled(Persona),
    Enabled(&'host mut crate::runtime::ToolScope<'state>),
}

impl ToolAccess<'_, '_> {
    fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    fn persona(&self) -> Persona {
        match self {
            Self::Disabled(persona) => *persona,
            Self::Enabled(host) => host.persona,
        }
    }

    fn dispatch(
        &mut self,
        offered: &[crate::tools::ToolSpec],
        calls: &[crate::tools::ToolCall],
    ) -> Result<Vec<crate::tools::ToolResult>, llm::LlmError> {
        if offered.is_empty() && !calls.is_empty() {
            return Err(llm::LlmError::backend(
                "backend returned unrequested tool calls".into(),
            ));
        }
        if calls
            .iter()
            .any(|call| !offered.iter().any(|tool| tool.name == call.name))
        {
            return Err(llm::LlmError::backend(
                "backend requested a tool that was not offered".into(),
            ));
        }
        match self {
            Self::Disabled(_) if calls.is_empty() => Ok(Vec::new()),
            Self::Disabled(_) => Err(llm::LlmError::backend("tool access is disabled".into())),
            Self::Enabled(host) => Ok(calls
                .iter()
                .map(|call| crate::tools::dispatch(call, &mut **host))
                .collect()),
        }
    }
}

/// Generate with Abbey's model-callable tool vocabulary and canonical runtime
/// scope. This is the only public entry point that accepts a live tool host.
///
/// The generation loop builds the prompt for the scope's persona,
/// calls the backend (streamed to `delivery` on the local path, single-shot
/// otherwise), run any tool calls against `host`, and repeat up to
/// [`crate::tools::MAX_TOOL_ROUNDS`] times until the model answers in text.
///
/// Returns the tidied text, the id of the message already holding it (if the
/// stream posted it), the persona that ended up answering (tools may switch
/// it), and the provider label. A backend that rejects tooled requests (HTTP
/// 4xx) is retried once without tools and only that provider's tool route is
/// disabled for the process. Cross-provider fallback is allowed only when the
/// failed attempt could not already have dispatched a tool; the established
/// Anthropic-to-local order is preserved before Foundation Models.
pub async fn generate_with_tools<O: Outbound + Sync>(
    state: &AppState,
    host: &mut crate::runtime::ToolScope<'_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
) -> Result<(String, Option<String>, Persona, &'static str), llm::LlmError> {
    let primary_streamed = delivery.is_some()
        && matches!(
            state.backend.as_ref(),
            Some(llm::Backend::OpenAiCompatible { .. })
        );
    let primary_tools_possible = state.backend.is_some()
        && foundation_models::primary_tools_are_available(
            state.foundation_models.as_ref(),
            state
                .tools_enabled
                .load(std::sync::atomic::Ordering::Relaxed),
        );
    let primary = match &state.backend {
        Some(backend) => {
            generate_with_backend_and_access(
                state,
                backend,
                ToolAccess::Enabled(&mut *host),
                ask,
                delivery,
                None,
                llm::ResponseStyle::Default,
            )
            .await
        }
        None => Err(no_backend_error()),
    };
    let primary_error = match primary {
        Ok(answer) => {
            let label = state
                .backend
                .as_ref()
                .expect("a primary result requires a primary backend")
                .label();
            return Ok((answer.0, answer.1, answer.2, label));
        }
        Err(error) => error,
    };
    let fm_cli_available = state.foundation_models.as_ref().is_some_and(|fm| {
        fm.router
            .candidates(ProviderCapabilities::text_with_tools())
            .contains(&ProviderRoute::FoundationModelsCli)
    });
    let routes = foundation_models::fallback_routes(
        delivery.is_some(),
        primary_streamed,
        primary_tools_possible,
        state.fallback.is_some(),
        false,
        fm_cli_available,
    );
    let mut last_error = primary_error;
    for route in routes {
        match route {
            foundation_models::FallbackRoute::Local => {
                let local = state
                    .fallback
                    .as_ref()
                    .expect("the route planner requires an available local fallback");
                tracing::warn!(error = %last_error, "primary backend failed; trying the configured local endpoint");
                match generate_with_backend_and_access(
                    state,
                    local,
                    ToolAccess::Enabled(&mut *host),
                    ask,
                    delivery,
                    None,
                    llm::ResponseStyle::Default,
                )
                .await
                {
                    Ok(answer) => return Ok((answer.0, answer.1, answer.2, local.label())),
                    Err(error) => {
                        last_error = error;
                        if delivery.is_some() {
                            return Err(last_error);
                        }
                    }
                }
            }
            foundation_models::FallbackRoute::FoundationModelsCli => {
                let fm = state
                    .foundation_models
                    .as_ref()
                    .expect("the route planner requires an available FM CLI");
                tracing::warn!(error = %last_error, "configured backends failed; trying explicit Foundation Models CLI fallback");
                return foundation_models::generate_with_fm_cli_and_access(
                    state,
                    fm,
                    ToolAccess::Enabled(&mut *host),
                    ask,
                )
                .await
                .map(|answer| (answer.0, answer.1, answer.2, fm.label()));
            }
            foundation_models::FallbackRoute::FoundationModelsServer => {
                unreachable!("tool-capable generation never routes through fm serve")
            }
        }
    }
    Err(last_error)
}

/// Generate with Abbey's model-callable tools but without a delivery channel.
///
/// This is the non-streaming entry point for callers such as slash commands:
/// it accepts the canonical live tool scope, but does not expose the internal
/// uninhabited outbound type or require callers to spell a generic parameter.
pub async fn generate_with_tools_without_delivery(
    state: &AppState,
    host: &mut crate::runtime::ToolScope<'_>,
    ask: &Ask<'_>,
) -> Result<(String, Persona, &'static str), llm::LlmError> {
    let (text, posted, persona, provider) =
        generate_with_tools::<NoDelivery>(state, host, ask, None).await?;
    debug_assert!(posted.is_none(), "no-delivery generation cannot post");
    Ok((text, persona, provider))
}

/// Generate against the configured backend without constructing a tool host
/// or vocabulary. Persona is explicit because no mutable tool scope exists to
/// smuggle that state into a read-only turn.
pub async fn generate_read_only<O: Outbound + Sync>(
    state: &AppState,
    persona: Persona,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
) -> Result<(String, Option<String>, Persona, &'static str), llm::LlmError> {
    let primary_streamed = delivery.is_some()
        && matches!(
            state.backend.as_ref(),
            Some(llm::Backend::OpenAiCompatible { .. })
        );
    let primary = match &state.backend {
        Some(backend) => {
            generate_with_backend_and_access(
                state,
                backend,
                ToolAccess::Disabled(persona),
                ask,
                delivery,
                None,
                llm::ResponseStyle::Default,
            )
            .await
        }
        None => Err(no_backend_error()),
    };
    let primary_error = match primary {
        Ok(answer) => {
            let label = state
                .backend
                .as_ref()
                .expect("a primary result requires a primary backend")
                .label();
            return Ok((answer.0, answer.1, answer.2, label));
        }
        Err(error) => error,
    };
    // `fm serve` defaults to SSE even on an otherwise non-streaming request,
    // so qualify it only on the delivery path where Abbey explicitly streams.
    let streamed_text = ProviderCapabilities {
        text: true,
        streaming: true,
        ..ProviderCapabilities::default()
    };
    let fm_server_available = delivery.is_some()
        && state.foundation_models.as_ref().is_some_and(|fm| {
            fm.router
                .candidates(streamed_text)
                .contains(&ProviderRoute::FoundationModelsServer)
        });
    let fm_cli_available = state.foundation_models.as_ref().is_some_and(|fm| {
        fm.router
            .candidates(ProviderCapabilities::text())
            .contains(&ProviderRoute::FoundationModelsCli)
    });
    let routes = foundation_models::fallback_routes(
        delivery.is_some(),
        primary_streamed,
        false,
        state.fallback.is_some(),
        fm_server_available,
        fm_cli_available,
    );
    let mut last_error = primary_error;
    for route in routes {
        match route {
            foundation_models::FallbackRoute::Local => {
                let local = state
                    .fallback
                    .as_ref()
                    .expect("the route planner requires an available local fallback");
                tracing::warn!(error = %last_error, "primary backend failed; trying the configured local endpoint");
                match generate_with_backend_and_access(
                    state,
                    local,
                    ToolAccess::Disabled(persona),
                    ask,
                    delivery,
                    None,
                    llm::ResponseStyle::Default,
                )
                .await
                {
                    Ok(answer) => return Ok((answer.0, answer.1, answer.2, local.label())),
                    Err(error) => {
                        last_error = error;
                        // The local endpoint streams on delivery paths. Its
                        // error carries no posted id, so another provider
                        // could double-post after the visible failure edit.
                        if delivery.is_some() {
                            return Err(last_error);
                        }
                    }
                }
            }
            foundation_models::FallbackRoute::FoundationModelsServer => {
                let fm = state
                    .foundation_models
                    .as_ref()
                    .expect("the route planner requires an available FM server");
                let server = fm
                    .server_backend()
                    .expect("the route planner requires an FM server endpoint");
                tracing::warn!(error = %last_error, "configured backends failed; trying explicit Foundation Models server fallback");
                match generate_with_backend_and_access(
                    state,
                    &server,
                    ToolAccess::Disabled(persona),
                    ask,
                    delivery,
                    None,
                    llm::ResponseStyle::Default,
                )
                .await
                {
                    Ok(answer) => return Ok((answer.0, answer.1, answer.2, fm.label())),
                    // `stream_reply` may already have posted before failing.
                    // Never chain its failure into the CLI.
                    Err(error) => return Err(error),
                }
            }
            foundation_models::FallbackRoute::FoundationModelsCli => {
                let fm = state
                    .foundation_models
                    .as_ref()
                    .expect("the route planner requires an available FM CLI");
                tracing::warn!(error = %last_error, "configured backends failed; trying explicit Foundation Models CLI fallback");
                return foundation_models::generate_with_fm_cli_and_access(
                    state,
                    fm,
                    ToolAccess::Disabled(persona),
                    ask,
                )
                .await
                .map(|answer| (answer.0, answer.1, answer.2, fm.label()));
            }
        }
    }
    Err(last_error)
}

fn no_backend_error() -> llm::LlmError {
    llm::LlmError::backend("no generation backend is configured".into())
}

/// Generate read-only text through an explicitly selected backend, with no
/// delivery channel and no model-callable tools. The caller must choose the
/// persona explicitly; no live [`crate::runtime::ToolScope`] is constructed or
/// exposed, and the tool vocabulary is never allocated.
///
/// The voice surface uses this seam to require a loopback backend even when a
/// remote text provider is configured as the process-wide default. The
/// optional suffix adds presentation constraints without replacing persona
/// policy. Its explicit spoken response style skips optional model thinking
/// only on the measured local Ollama/Gemma deployment; ordinary text and
/// tool-capable generation retain provider defaults.
pub async fn generate_without_delivery(
    state: &AppState,
    backend: &llm::Backend,
    persona: Persona,
    ask: &Ask<'_>,
    system_suffix: Option<&str>,
) -> Result<(String, Persona), llm::LlmError> {
    let (text, posted, persona) = generate_with_backend_and_access::<NoDelivery>(
        state,
        backend,
        ToolAccess::Disabled(persona),
        ask,
        None,
        system_suffix,
        llm::ResponseStyle::Spoken,
    )
    .await?;
    debug_assert!(posted.is_none(), "no-delivery generation cannot post");
    Ok((text, persona))
}

async fn generate_with_backend_and_access<O: Outbound + Sync>(
    state: &AppState,
    backend: &llm::Backend,
    mut access: ToolAccess<'_, '_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
    system_suffix: Option<&str>,
    response_style: llm::ResponseStyle,
) -> Result<(String, Option<String>, Persona), llm::LlmError> {
    use std::sync::atomic::Ordering;
    let scope = ask.scope;
    // Constructing the vocabulary is intentionally capability-gated. This is
    // more than an empty slice at dispatch time: disabled voice turns never
    // allocate or even materialize model-callable tool descriptions.
    let vocabulary = (access.is_enabled()
        && foundation_models::primary_tools_are_available(
            state.foundation_models.as_ref(),
            state.tools_enabled.load(Ordering::Relaxed),
        ))
    .then(crate::tools::production_tools);
    let mut extra_turns: Vec<llm::ChatTurn> = Vec::new();
    let mut grounding_results: Vec<crate::tools::ToolResult> = Vec::new();
    for round in 0..=crate::tools::MAX_TOOL_ROUNDS {
        let persona = access.persona();
        let prepared = ask.prepare(state, persona);
        let system_prompt = match system_suffix {
            Some(suffix) if !suffix.trim().is_empty() => {
                format!("{}\n\n{}", prepared.system_prompt, suffix.trim())
            }
            _ => prepared.system_prompt.clone(),
        };
        let mut turns = prepared.turns.clone();
        turns.extend(extra_turns.iter().cloned());
        let grounding = grounding_for_round(&prepared, &grounding_results);
        let offer = vocabulary.is_some()
            && round < crate::tools::MAX_TOOL_ROUNDS
            && foundation_models::primary_tools_are_available(
                state.foundation_models.as_ref(),
                state.tools_enabled.load(Ordering::Relaxed),
            );
        let tools: &[crate::tools::ToolSpec] = if offer {
            vocabulary.as_deref().unwrap_or_default()
        } else {
            &[]
        };

        let round = Round {
            backend,
            system_prompt: &system_prompt,
            turns: &turns,
            tools,
            persona,
            grounding: &grounding,
        };
        let turn: RoundOutcome = match (&delivery, backend) {
            (Some(d), llm::Backend::OpenAiCompatible { .. }) => {
                match stream_reply(&state.llm, d, &round).await {
                    Ok(StreamEnd::Text(text, posted)) => Ok((Some(text), posted, Vec::new())),
                    Ok(StreamEnd::Calls(calls)) => Ok((None, None, calls)),
                    Err(e) => Err(e),
                }
            }
            _ => llm::chat_turn_with_style(
                &state.llm,
                backend,
                &system_prompt,
                &turns,
                tools,
                response_style,
            )
            .await
            .map(|t| {
                let is_final = t.calls.is_empty();
                let text = if t.text.trim().is_empty() {
                    None
                } else if is_final {
                    Some(finalize_reply(persona, &t.text, &grounding))
                } else {
                    // Preserve prior tool-round shaping, but do not append
                    // a user-visible hedge to assistant prose that will
                    // only be sent back to the model for continuation.
                    Some(ask::tidy_reply(persona, &t.text))
                };
                (text, None, t.calls)
            }),
        };

        let (text, posted, calls) = match turn {
            Ok(v) => v,
            Err(e) if offer && looks_like_tool_rejection(e.detail()) => {
                tracing::warn!(error = %e, "backend rejected a tooled request; continuing without tools for this process");
                if let Some(fm) = &state.foundation_models {
                    fm.router.disable_tools(ProviderRoute::Primary);
                } else {
                    state.tools_enabled.store(false, Ordering::Relaxed);
                }
                continue;
            }
            Err(e) => return Err(e),
        };

        if calls.is_empty() {
            if let Some(text) = text {
                return Ok((text, posted, persona));
            }
            return Err(llm::LlmError::backend(
                "the response carried no answer text".into(),
            ));
        }
        // A model is not an authority boundary. If this request did not offer
        // tools, unsolicited calls must stop here before they can mutate
        // memory, WDBX, persona state, or any future capability.
        let results = access.dispatch(tools, &calls)?;
        for call in &calls {
            // Tool results can contain private recalled facts. Keep operational
            // evidence (which scoped tool completed) without copying its
            // payload into the durable process log.
            tracing::info!(tool = %call.name, scope, "tool call completed");
        }
        extra_turns.push(llm::ChatTurn::assistant_calls(
            text.unwrap_or_default(),
            calls,
        ));
        extra_turns.extend(results.iter().map(llm::ChatTurn::tool_result));
        grounding_results.extend(results);
    }
    Err(llm::LlmError::backend(format!(
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
#[path = "generation/tests.rs"]
mod tests;
