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
use crate::provider::{ConversationEffects, ProviderConversation, ProviderId};
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
/// If the stream ends with tool calls and no text was produced or posted,
/// returns [`StreamEnd::Calls`] so the caller can run the tools and stream
/// again. A mixed text-and-tool turn is rejected: dispatching its calls would
/// let an already-visible claim get ahead of the actual side effect, while
/// ignoring them would make streaming disagree with completed generation. If
/// the stream fails after a partial message went out, that message is edited
/// to the honest failure line so a half-answer never stands as if whole.
#[cfg(test)]
pub async fn stream_reply<T: llm::StreamTransport + Sync, O: Outbound + Sync>(
    transport: &T,
    delivery: &Delivery<'_, O>,
    round: &Round<'_>,
) -> Result<StreamEnd, llm::LlmError> {
    let request =
        llm::build_stream_request(round.backend, round.system_prompt, round.turns, round.tools);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    stream_received(
        transport.post_stream(&request, tx),
        rx,
        delivery,
        round.backend.label(),
        round.persona,
        round.grounding,
        &ConversationEffects::default(),
    )
    .await
}

async fn stream_received<O: Outbound + Sync>(
    stream: impl Future<Output = Result<llm::ModelTurn, llm::LlmError>>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    delivery: &Delivery<'_, O>,
    provider_label: &'static str,
    persona: Persona,
    grounding: &Grounding,
    effects: &ConversationEffects,
) -> Result<StreamEnd, llm::LlmError> {
    let Delivery {
        out,
        native_channel_id,
        reply_to: _,
    } = *delivery;
    let mut stream = std::pin::pin!(stream);
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(STREAM_EDIT_EVERY_SECS));
    tick.tick().await;
    let mut text = String::new();
    let mut posted: Option<String> = None;
    let mut last_edited_len = 0usize;
    let mut finished: Option<Result<llm::ModelTurn, llm::LlmError>> = None;

    // Post-or-edit with whatever has arrived, honouring the pacing rules.
    async fn flush<O: Outbound + Sync>(
        delivery: &Delivery<'_, O>,
        text: &str,
        grounding: &Grounding,
        posted: &mut Option<String>,
        last_edited_len: &mut usize,
        effects: &ConversationEffects,
    ) -> Result<(), String> {
        let Delivery {
            out,
            native_channel_id: channel,
            reply_to,
        } = *delivery;
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
        effects.mark_visible_output();
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
                    flush(delivery, &text, grounding, &mut posted, &mut last_edited_len, effects)
                        .await
                        .map_err(llm::LlmError::backend)?;
                }
            }
            _ = tick.tick() => {
                if posted.is_some() || started.elapsed().as_secs() >= STREAM_FIRST_POST_SECS {
                    flush(delivery, &text, grounding, &mut posted, &mut last_edited_len, effects)
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
                    let failure = ask::render_failure(persona, provider_label, &error);
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
                effects.mark_visible_output();
            }
            Ok(StreamEnd::Text(tidy, posted))
        }
        Err(e) => {
            if let Some(id) = &posted {
                let failure = ask::render_failure(persona, provider_label, &e);
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
#[cfg(test)]
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
        effects: &ConversationEffects,
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
                .map(|call| {
                    crate::tools::dispatch(
                        call,
                        &mut EffectHost {
                            host: &mut **host,
                            effects,
                        },
                    )
                })
                .collect()),
        }
    }
}

/// One runtime conversation spans every tool round and its one pre-effect fallback.
pub async fn generate_with_tools<O: Outbound + Sync>(
    state: &AppState,
    host: &mut crate::runtime::ToolScope<'_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
) -> Result<(String, Option<String>, Persona, &'static str), llm::LlmError> {
    let mut conversation = state.providers.begin(true, delivery.is_some());
    generate_conversation(
        state,
        &mut conversation,
        ToolAccess::Enabled(host),
        ask,
        delivery,
        None,
        llm::ResponseStyle::Default,
    )
    .await
}
pub async fn generate_with_tools_without_delivery(
    state: &AppState,
    host: &mut crate::runtime::ToolScope<'_>,
    ask: &Ask<'_>,
) -> Result<(String, Persona, &'static str), llm::LlmError> {
    let (text, _, persona, label) =
        generate_with_tools::<NoDelivery>(state, host, ask, None).await?;
    Ok((text, persona, label))
}
pub async fn generate_read_only<O: Outbound + Sync>(
    state: &AppState,
    persona: Persona,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
) -> Result<(String, Option<String>, Persona, &'static str), llm::LlmError> {
    let mut conversation = state.providers.begin(false, delivery.is_some());
    generate_conversation(
        state,
        &mut conversation,
        ToolAccess::Disabled(persona),
        ask,
        delivery,
        None,
        llm::ResponseStyle::Default,
    )
    .await
}
pub async fn generate_without_delivery(
    state: &AppState,
    provider: &ProviderId,
    persona: Persona,
    ask: &Ask<'_>,
    system_suffix: Option<&str>,
) -> Result<(String, Persona), llm::LlmError> {
    let mut conversation = state.providers.voice(provider);
    let (text, _, persona, _) = generate_conversation::<NoDelivery>(
        state,
        &mut conversation,
        ToolAccess::Disabled(persona),
        ask,
        None,
        system_suffix,
        llm::ResponseStyle::Spoken,
    )
    .await?;
    Ok((text, persona))
}
async fn generate_conversation<O: Outbound + Sync>(
    state: &AppState,
    conversation: &mut ProviderConversation<'_>,
    mut access: ToolAccess<'_, '_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
    system_suffix: Option<&str>,
    response_style: llm::ResponseStyle,
) -> Result<(String, Option<String>, Persona, &'static str), llm::LlmError> {
    let vocabulary = crate::tools::production_tools_when_enabled(
        access.is_enabled() && conversation.tools_available(),
    );
    let effects = conversation.effects();
    let mut extra_turns = Vec::new();
    let mut grounding_results = Vec::new();
    for round_index in 0..=crate::tools::MAX_TOOL_ROUNDS {
        let persona = access.persona();
        let prepared = ask.prepare(state, persona);
        let system = match system_suffix.filter(|s| !s.trim().is_empty()) {
            Some(suffix) => format!("{}\n\n{}", prepared.system_prompt, suffix.trim()),
            None => prepared.system_prompt.clone(),
        };
        let mut turns = prepared.turns.clone();
        turns.extend(extra_turns.iter().cloned());
        let grounding = grounding_for_round(&prepared, &grounding_results);
        let tools = if round_index < crate::tools::MAX_TOOL_ROUNDS && conversation.tools_available()
        {
            vocabulary.as_deref().unwrap_or_default()
        } else {
            &[]
        };
        let (text, posted, calls) = loop {
            match conversation.reserve().await {
                Ok(()) => {}
                Err(error) if conversation.fallback(&error) => continue,
                Err(error) => return Err(error),
            }
            let label = conversation.label();
            let result: RoundOutcome = if let Some(ref delivery) = delivery
                && conversation.streams()
            {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let work = conversation.execute(&system, &turns, tools, response_style, Some(tx));
                stream_received(work, rx, delivery, label, persona, &grounding, &effects)
                    .await
                    .map(|end| match end {
                        StreamEnd::Text(text, posted) => (Some(text), posted, Vec::new()),
                        StreamEnd::Calls(calls) => (None, None, calls),
                    })
            } else {
                conversation
                    .execute(&system, &turns, tools, response_style, None)
                    .await
                    .map(|turn| {
                        let text = (!turn.text.trim().is_empty()).then(|| {
                            if turn.calls.is_empty() {
                                finalize_reply(persona, &turn.text, &grounding)
                            } else {
                                ask::tidy_reply(persona, &turn.text)
                            }
                        });
                        (text, None, turn.calls)
                    })
            };
            match result {
                Ok(turn) => break turn,
                Err(error) if conversation.fallback(&error) => continue,
                Err(error) => return Err(error),
            }
        };
        if calls.is_empty() {
            return text
                .map(|text| (text, posted, persona, conversation.label()))
                .ok_or_else(|| {
                    llm::LlmError::backend("the response carried no answer text".into())
                });
        }
        let results = access.dispatch(tools, &calls, &effects)?;
        for call in &calls {
            tracing::info!(tool = %call.name, "tool call completed");
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

/// Mark only validated invocations, immediately before entering the actual host.
struct EffectHost<'a> {
    host: &'a mut dyn crate::tools::ToolHost,
    effects: &'a ConversationEffects,
}
impl crate::tools::ToolHost for EffectHost<'_> {
    fn remember_fact(&mut self, fact: &str, supersedes: Option<&str>) -> String {
        self.effects.mark_tool_dispatched();
        self.host.remember_fact(fact, supersedes)
    }
    fn lookup_reputation(&mut self, user: Option<&str>) -> String {
        self.effects.mark_tool_dispatched();
        self.host.lookup_reputation(user)
    }
    fn recall(&mut self, query: &str) -> String {
        self.effects.mark_tool_dispatched();
        self.host.recall(query)
    }
    fn switch_persona(&mut self, persona: Persona) -> String {
        self.effects.mark_tool_dispatched();
        self.host.switch_persona(persona)
    }
    fn recent_messages(&mut self, limit: usize) -> String {
        self.effects.mark_tool_dispatched();
        self.host.recent_messages(limit)
    }
    fn inspect_status(&mut self, aspect: crate::tools::InspectAspect) -> String {
        self.effects.mark_tool_dispatched();
        self.host.inspect_status(aspect)
    }
    fn list_facts(&mut self) -> String {
        self.effects.mark_tool_dispatched();
        self.host.list_facts()
    }
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
