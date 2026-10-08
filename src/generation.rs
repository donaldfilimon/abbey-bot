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
use crate::outbound_failure::{DeliveryCertainty, OutboundFailure, OutboundFailureCategory};
use crate::persona::Persona;
use crate::pipeline::Outbound;
use crate::platform::OutboundMessage;
use crate::prompt_budget::{self, PromptParts};
use crate::provider::{ConversationEffects, ProviderConversation, ProviderId};
use crate::runtime::AppState;

mod capability_guidance;
pub(crate) mod consent;
mod delivery_timing;
mod retained;
mod stream_delivery;
use stream_delivery::stream_received_timed;
#[cfg(test)]
use stream_delivery::{stream_received, stream_reply};
pub(crate) mod stream_owner;
pub(crate) mod timing;
pub use delivery_timing::DeliveryTiming;

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

fn delivery_error(error: OutboundFailure, timing: Option<&timing::Timing>) -> llm::LlmError {
    if let Some(timing) = timing {
        timing.post_failed(&error);
    }
    llm::LlmError::delivery(error)
}

/// Internal outbound type for generation that intentionally has no delivery
/// channel. Public callers use one of the concrete no-delivery entry points
/// instead of supplying an uninhabited generic themselves.
enum NoDelivery {}

impl Outbound for NoDelivery {
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
        match *self {}
    }
    async fn typing(&self, _: &str) {
        match *self {}
    }
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        match *self {}
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        match *self {}
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
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
pub enum SessionMode<'a> {
    Shared,
    Ephemeral,
    /// Read only caller-supplied context; do not read shared session history.
    SourceOnly,
    /// Read-only correction bound to a still-current scoped bot turn.
    Repair(&'a crate::brain::correction::CorrectionSource),
}

/// What generation is asked to do, independent of delivery and capabilities.
pub struct Ask<'a> {
    pub session_mode: SessionMode<'a>,
    /// Authenticated subject, independent of the service-produced context seal.
    pub subject: Option<(&'a str, &'a str)>,
    pub scope: &'a str,
    pub context: &'a PersonaContext,
    pub user_input: &'a str,
    pub now: u64,
}

impl Ask<'_> {
    fn prepare(&self, state: &AppState, persona: Persona) -> crate::engine::PreparedTurn {
        // Sessions contain legacy or mixed contributors. Until each turn has
        // typed provenance, only this request and its sealed facts enter a model.
        if matches!(self.session_mode, SessionMode::Shared) {
            AppState::lock(&state.engine).set_session_persona(self.scope, persona, self.now);
        }
        if matches!(self.session_mode, SessionMode::Repair(_)) {
            crate::engine::Engine::prepare_repair(persona, self.context, self.user_input)
        } else {
            crate::engine::Engine::prepare_source_only(persona, self.context, self.user_input)
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
        personal_memory_allowed: bool,
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
                            personal_memory_allowed,
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
) -> Result<
    (
        String,
        Option<String>,
        Persona,
        &'static str,
        Option<DeliveryTiming>,
    ),
    llm::LlmError,
> {
    let mut conversation = state.providers.begin_source_only(true, true);
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
) -> Result<(String, Persona, &'static str, Option<DeliveryTiming>), llm::LlmError> {
    let (text, _, persona, label, timing) =
        generate_with_tools::<NoDelivery>(state, host, ask, None).await?;
    Ok((text, persona, label, timing))
}
pub async fn generate_read_only<O: Outbound + Sync>(
    state: &AppState,
    persona: Persona,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
) -> Result<
    (
        String,
        Option<String>,
        Persona,
        &'static str,
        Option<DeliveryTiming>,
    ),
    llm::LlmError,
> {
    let mut conversation = state.providers.begin_source_only(false, true);
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
    let (text, _, persona, _, _) = generate_conversation::<NoDelivery>(
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
    access: ToolAccess<'_, '_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
    system_suffix: Option<&str>,
    response_style: llm::ResponseStyle,
) -> Result<
    (
        String,
        Option<String>,
        Persona,
        &'static str,
        Option<DeliveryTiming>,
    ),
    llm::LlmError,
> {
    let timing = (response_style != llm::ResponseStyle::Spoken)
        .then(|| timing::Timing::new(state, ask.scope.starts_with("discord:")));
    let result = generate_conversation_timed(
        state,
        conversation,
        access,
        ask,
        delivery,
        system_suffix,
        response_style,
        timing.as_ref(),
    )
    .await;
    if let Some(timing) = &timing {
        timing.finish(&result);
    }
    result.map(|(text, posted, persona, label)| {
        (
            text,
            posted,
            persona,
            label,
            timing.map(|timing| timing.delivery.clone()),
        )
    })
}
#[expect(
    clippy::too_many_arguments,
    reason = "canonical generation timing retains the existing authorization seam"
)]
async fn generate_conversation_timed<O: Outbound + Sync>(
    state: &AppState,
    conversation: &mut ProviderConversation<'_>,
    mut access: ToolAccess<'_, '_>,
    ask: &Ask<'_>,
    delivery: Option<Delivery<'_, O>>,
    system_suffix: Option<&str>,
    response_style: llm::ResponseStyle,
    timing: Option<&timing::Timing>,
) -> Result<(String, Option<String>, Persona, &'static str), llm::LlmError> {
    let guard = consent::GenerationGuard::capture(state, ask)?;
    if !guard.permits_delivery(
        delivery.as_ref().map(|value| value.native_channel_id),
        response_style == llm::ResponseStyle::Spoken,
    ) {
        return Err(llm::LlmError::context_changed());
    }
    if let ToolAccess::Enabled(host) = &access
        && !guard.matches_host(host)
    {
        return Err(llm::LlmError::context_changed());
    }
    let repair = matches!(ask.session_mode, SessionMode::Repair(_));
    if repair && access.is_enabled() {
        return Err(llm::LlmError::backend(
            "correction requires read-only generation".into(),
        ));
    }
    if repair && ask.context.grounding_sources(ask.user_input).is_empty() {
        return Ok((
            crate::brain::correction::INSUFFICIENT_EVIDENCE.into(),
            None,
            access.persona(),
            conversation.label(),
        ));
    }
    let system_suffix = if repair {
        Some(crate::brain::correction::RECOVERY_INSTRUCTIONS)
    } else {
        system_suffix
    };
    let vocabulary = crate::tools::production_tools_when_enabled(
        access.is_enabled() && conversation.tools_available(),
    );
    let effects = conversation.effects();
    let mut extra_turns = Vec::new();
    let mut grounding_results = Vec::new();
    for round_index in 0..=crate::tools::MAX_TOOL_ROUNDS {
        guard.check_fresh(state).await?;
        let persona = access.persona();
        let prepared = ask.prepare(state, persona);
        if !guard.validates_prepared(&prepared) {
            return Err(llm::LlmError::context_changed());
        }
        let mut turns = prepared.turns.clone();
        turns.extend(extra_turns.iter().cloned());
        let grounding = grounding_for_round(&prepared, &grounding_results);
        let tools = if round_index < crate::tools::MAX_TOOL_ROUNDS && conversation.tools_available()
        {
            vocabulary.as_deref().unwrap_or_default()
        } else {
            &[]
        };
        let tool_names: Vec<_> = tools.iter().map(|tool| tool.name).collect();
        let parts = PromptParts {
            addenda: prepared.addenda.clone(),
            ..PromptParts::new(
                prepared.persona_core.clone(),
                &prepared.context,
                capability_guidance::guidance(
                    ask.scope,
                    &tool_names,
                    system_suffix,
                    ask.user_input,
                ),
                turns,
            )
        };
        let (text, posted, calls) = loop {
            guard.check(state)?;
            if response_style != llm::ResponseStyle::Spoken
                && let Some(owned) = state.owned_state()
            {
                let retained::Round {
                    work,
                    deltas: rx,
                    selected,
                } = retained::start(retained::Request {
                    state: owned,
                    seed: conversation.seed(),
                    parts: parts.clone(),
                    tools: tools.to_vec(),
                    style: response_style,
                    guard: guard.clone(),
                    timing: timing.map(timing::Timing::observer),
                })?;
                let streaming = selected.await.unwrap_or(false);
                let result: RoundOutcome = if streaming && let Some(ref delivery) = delivery {
                    stream_received_timed(
                        work,
                        rx,
                        delivery,
                        conversation.label(),
                        persona,
                        &grounding,
                        &effects,
                        Some((state, &guard)),
                        timing,
                    )
                    .await
                    .map(|end| match end {
                        StreamEnd::Text(text, posted) => (Some(text), posted, vec![]),
                        StreamEnd::Calls(calls) => (None, None, calls),
                    })
                } else {
                    timing::observe(work, rx, timing).await.map(|turn| {
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
                let result = match result {
                    Err(error) if error.outbound_failure().is_some() => return Err(error),
                    result => result,
                };
                guard.check(state)?;
                match result {
                    Ok(turn) => break turn,
                    Err(error) if conversation.fallback(&error) => continue,
                    Err(error) => return Err(error),
                }
            }
            let queued = tokio::time::Instant::now();
            let admission = conversation.reserve().await;
            if let Some(timing) = timing {
                timing.delivery.provider(conversation.selected_provider());
                timing.admitted(queued.elapsed(), &admission);
            }
            match admission {
                Ok(()) => {}
                Err(error) if conversation.fallback(&error) => continue,
                Err(error) => return Err(error),
            }
            // Fit per reserved provider from the untrimmed parts, so a
            // fallback re-fits to the new window (or sends everything).
            guard.check_fresh(state).await?;
            let fitted = prompt_budget::fitted(&parts, conversation.prompt_budget());
            let label = conversation.label();
            let started = || {
                if let Some(timing) = timing {
                    timing.provider_started();
                }
            };
            let result: RoundOutcome = if let Some(ref delivery) = delivery
                && conversation.streams()
            {
                let (tx, rx) = crate::generation::stream_owner::channel();
                let work = conversation.execute_parts(
                    &fitted,
                    tools,
                    response_style,
                    Some(tx),
                    Some(&started),
                );
                stream_received_timed(
                    work,
                    rx,
                    delivery,
                    label,
                    persona,
                    &grounding,
                    &effects,
                    Some((state, &guard)),
                    timing,
                )
                .await
                .map(|end| match end {
                    StreamEnd::Text(text, posted) => (Some(text), posted, Vec::new()),
                    StreamEnd::Calls(calls) => (None, None, calls),
                })
            } else {
                guard
                    .while_current(state, async {
                        if conversation.streams() {
                            let (tx, rx) = crate::generation::stream_owner::channel();
                            timing::observe(
                                conversation.execute_parts(
                                    &fitted,
                                    tools,
                                    response_style,
                                    Some(tx),
                                    Some(&started),
                                ),
                                rx,
                                timing,
                            )
                            .await
                        } else {
                            conversation
                                .execute_parts(&fitted, tools, response_style, None, Some(&started))
                                .await
                        }
                    })
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
            let result = match result {
                // A lost acknowledgement remains a delivery fault even if
                // consent changed while the outbound await was pending.
                Err(error) if error.outbound_failure().is_some() => return Err(error),
                result => result,
            };
            guard.check(state)?;
            match result {
                Ok(turn) => break turn,
                Err(error) if conversation.fallback(&error) => continue,
                Err(error) => return Err(error),
            }
        };
        if calls.is_empty() {
            return text
                .filter(|text| !text.trim().is_empty())
                .map(|text| (text, posted, persona, conversation.label()))
                .ok_or_else(|| {
                    llm::LlmError::backend("the response carried no answer text".into())
                });
        }
        guard.check(state)?;
        let results =
            access.dispatch(tools, &calls, &effects, guard.authorizes_personal_memory())?;
        guard.check(state)?;
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
    personal_memory_allowed: bool,
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
        if self.personal_memory_allowed {
            self.host.recall(query)
        } else {
            "Nothing eligible for generated use.".into()
        }
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
        if self.personal_memory_allowed {
            self.host.list_facts()
        } else {
            "Nothing eligible for generated use.".into()
        }
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

#[cfg(test)]
#[path = "generation/timing_tests.rs"]
mod timing_tests;

#[cfg(test)]
#[path = "generation/outbound_tests.rs"]
mod outbound_tests;

#[cfg(test)]
mod stream_owner_tests;

#[cfg(test)]
mod retained_tests;
