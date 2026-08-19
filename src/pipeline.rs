//! The ingest pipeline — the spec's `SocialRouter.handle` / `handleMessage`,
//! written once for every network.
//!
//! A shell (Discord in [`crate::gateway`], Telegram in the same file) turns its
//! native event into a [`SocialEvent`] and hands it here with an [`Outbound`]
//! it can deliver through. Everything that decides — triage, intent, state
//! encoding, the per-guild policy, cooldown, persona routing, reward
//! bookkeeping — happens in this file against the pure modules, and nothing in
//! here knows which network it is talking to. That is what keeps the learning
//! loop identical across platforms, and what lets the whole decision path run
//! in tests behind a recording `Outbound`.

use std::future::Future;

use crate::ask;
use crate::brain::intent::{self, Intent};
use crate::brain::state::{self, BotAction, StateInput};
use crate::engine;
use crate::guild::GuildSettings;
use crate::llm;
use crate::memory::PersonaContext;
use crate::persona::Persona;
use crate::platform::{OutboundMessage, RemoteAttachment, RouteDecision, SocialEvent, triage};
use crate::runtime::{self, AppState};
use crate::vision::{self, ImageUnderstanding};

/// Window for "channel heat" — messages in the last five minutes (spec).
pub const HEAT_WINDOW_SECS: u64 = 300;
/// How many WDBX recollections are folded into the persona context.
pub const RECALL_K: usize = 3;
/// The emoji Abbey uses for the `react` action.
pub const REACT_EMOJI: &str = "👍";

/// What a shell must be able to do for the pipeline. Returns the sent
/// message's native id so rewards can be keyed to it.
pub trait Outbound {
    fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> impl Future<Output = Result<String, String>> + Send;
    fn typing(&self, native_channel_id: &str) -> impl Future<Output = ()> + Send;
    fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    /// Fetch an attachment's bytes (capped by the caller's `max`).
    fn fetch(&self, url: &str, max: usize) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
    /// Replace the text of a message this bot sent (progressive replies).
    fn edit(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        text: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

/// Progressive-reply pacing: post once this many characters have arrived…
pub const STREAM_FIRST_POST_CHARS: usize = 60;
/// …or this many seconds have passed since generation started, whichever first.
pub const STREAM_FIRST_POST_SECS: u64 = 4;
/// Then edit at most this often (Discord tolerates ~5 edits / 5 s per channel).
pub const STREAM_EDIT_EVERY_SECS: u64 = 2;

/// What the pipeline did with an event — for logs and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ignored(&'static str),
    Rewarded,
    Welcomed,
    Stayed,
    CooledDown,
    /// The guild's hourly budget is spent; nothing sent, nothing learned.
    OverBudget,
    Reacted,
    Replied,
    ReplyFailed(String),
}

/// Intent → persona, per `multiguild.md`'s `ABIRouter.route`: mod requests and
/// commands go to Aviva, greetings and small talk to Abi, everything else to
/// the guild's default.
pub fn persona_for(intent: Intent, settings: &GuildSettings) -> Persona {
    match intent {
        Intent::ModRequest | Intent::Command => Persona::Aviva,
        Intent::Greeting | Intent::SmallTalk => Persona::Abi,
        _ => settings.default_persona,
    }
}

/// Handle one inbound event. `mentions_bot` and `reply_to` are facts only the
/// shell knows (they are platform-shaped), so they travel beside the event.
pub async fn handle<O: Outbound + Sync>(
    state: &AppState,
    out: &O,
    event: SocialEvent,
    mentions_bot: bool,
    reply_to: Option<&str>,
) -> Outcome {
    let now = runtime::now();
    let scoped_guild = event.scoped_guild_id();
    let scoped_channel = event.scoped_channel_id();
    let scoped_user = event.scoped_user_id();

    if state.is_self(&scoped_user) {
        return Outcome::Ignored("own traffic");
    }
    let settings = {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).config(&scoped_guild, &mut *stores)
    };

    let (text, attachments) = match triage(&event, settings.enabled) {
        RouteDecision::Ignore(_) => return Outcome::Ignored("triage"),
        RouteDecision::Reward {
            emoji,
            target_message_id,
            added,
        } => {
            AppState::lock(&state.rewards).reaction(&emoji, &target_message_id, added);
            return Outcome::Rewarded;
        }
        RouteDecision::Welcome { display_name } => {
            return welcome(state, out, &event.native_channel_id, &display_name).await;
        }
        RouteDecision::Consider { text, attachments } => (text, attachments),
    };

    // Bookkeeping that happens whether or not Abbey speaks.
    if let Some(id) = reply_to {
        AppState::lock(&state.rewards).human_replied(id);
    }
    let heat = {
        let mut stores = AppState::lock(&state.stores);
        stores
            .memory
            .record_message(&scoped_channel, &event.user_display_name, &text, now);
        let ctx = stores.memory.channel_mut(&scoped_channel);
        if ctx.guild.is_none() {
            ctx.guild = Some(scoped_guild.clone());
        }
        u32::try_from(
            ctx.recent
                .iter()
                .filter(|m| now.saturating_sub(m.at) <= HEAT_WINDOW_SECS)
                .count(),
        )
        .unwrap_or(u32::MAX)
    };

    // Without MESSAGE_CONTENT Discord delivers an empty body for anything that
    // is not a mention or a DM. There is nothing to learn from a blank, so the
    // policy is not consulted — it would be training on noise.
    let forced = mentions_bot || event.native_guild_id.is_none();
    if text.trim().is_empty() && attachments.is_empty() && !forced {
        return Outcome::Ignored("no content available");
    }
    // Gates on unsolicited speech, checked before the policy so nothing is
    // learned from a message Abbey was never allowed to answer — in order:
    // the operator's `ABBEY_QUIET=1` (wins over any guild), the guild's own
    // `/admin act on` (opt-in, default off), and `/admin learning off`.
    if !forced && state.quiet {
        return Outcome::Ignored("quiet");
    }
    if !forced && !settings.unsolicited {
        return Outcome::Ignored("act off");
    }
    if !forced && !settings.learning_enabled {
        return Outcome::Ignored("learning off");
    }

    let enriched = enrich_with_vision(state, out, &settings, &text, &attachments).await;
    let intent = intent::classify(&enriched);
    let reputation = {
        let stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).reputation(&scoped_user, &scoped_guild, &*stores)
    };
    let encoded = state::encode(&StateInput {
        text: &enriched,
        intent,
        reputation,
        channel_heat: heat,
        mentions_bot,
        has_image: attachments.iter().any(RemoteAttachment::is_image),
        hour_of_day: runtime::hour_of_day(now),
    });

    let action = {
        // Load the guild's brain even on the forced path: the reward for a
        // mention/DM reply settles 150 s later into `BrainRegistry::remember`,
        // which drops experiences for guilds that are not loaded.
        let stores = AppState::lock(&state.stores);
        let mut brains = AppState::lock(&state.brains);
        let brain = brains.brain(&scoped_guild, &*stores, now);
        if let Some(eps) = settings.epsilon_override {
            brain.set_epsilon(eps);
        }
        if forced {
            if let Some(stats) = brains.stats_mut(&scoped_guild) {
                stats.record_forced();
            }
            BotAction::Reply
        } else {
            let q = brain.q_values(&encoded);
            let chosen =
                BotAction::from_index(brain.select_action(&encoded)).unwrap_or(BotAction::Stay);
            if let Some(stats) = brains.stats_mut(&scoped_guild) {
                stats.record_decision(&encoded, &q, chosen);
            }
            tracing::info!(
                guild = %scoped_guild,
                action = crate::brain::telemetry::action_name(chosen),
                q = ?q,
                intent = ?intent,
                heat,
                "policy decision"
            );
            chosen
        }
    };

    if action == BotAction::Stay {
        AppState::lock(&state.brains).remember(
            &scoped_guild,
            crate::brain::reward::RewardCollector::silence_experience(encoded.to_vec()),
        );
        return Outcome::Stayed;
    }

    // Unsolicited output is rate-limited twice: per channel (cooldown, the
    // burst guard) and per guild (hourly budget, the volume guard). Mentions
    // and DMs bypass both. Over budget, the decision is not acted on and not
    // learned — silence was not the policy's choice.
    if !forced {
        // Budget is only *checked* here; the token is spent at the point of
        // acting (below), so a reply the bot cannot make — no backend — or a
        // failed react never burns quota a later action could use.
        let within_budget = AppState::lock(&state.budget).tokens_left(
            &scoped_guild,
            settings.unsolicited_per_hour,
            now,
        ) >= 1.0;
        if !within_budget {
            return Outcome::OverBudget;
        }
        // The cooldown is reserved atomically (check + record in one lock), so
        // two messages in the same channel handled concurrently cannot both
        // pass. Reserved before the budget is spent and before any network
        // call; a reservation that then fails to send still counts — quiet is
        // the safe direction.
        let reserved = AppState::lock(&state.cooldown).try_reserve(
            &scoped_channel,
            settings.reply_cooldown_seconds,
            now,
        );
        if !reserved {
            return Outcome::CooledDown;
        }
    }

    if action == BotAction::React {
        if !forced
            && !AppState::lock(&state.budget).try_take(
                &scoped_guild,
                settings.unsolicited_per_hour,
                now,
            )
        {
            return Outcome::OverBudget;
        }
        if let Err(e) = out
            .react(
                &event.native_channel_id,
                &event.native_message_id,
                REACT_EMOJI,
            )
            .await
        {
            return Outcome::ReplyFailed(e);
        }
        // A reaction back lands on the user's own message, so that is the key.
        AppState::lock(&state.rewards).register_reply(
            encoded.to_vec(),
            BotAction::React.index(),
            event.native_message_id.clone(),
            scoped_guild.clone(),
            now,
        );
        return Outcome::Reacted;
    }

    // Reply.
    let persona = persona_for(intent, &settings);
    let Some(backend) = &state.backend else {
        if !forced {
            // A policy that wants to speak with nothing to speak through is a
            // policy we cannot follow; treat it as silence without learning.
            return Outcome::Stayed;
        }
        let reply = OutboundMessage {
            text: ask::degraded_reply(persona),
            reply_to_native_message_id: Some(event.native_message_id.clone()),
            ..OutboundMessage::default()
        };
        return match out.send(&event.native_channel_id, &reply).await {
            Ok(_) => Outcome::Replied,
            Err(e) => Outcome::ReplyFailed(e),
        };
    };

    // Discord's typing indicator expires after ~10 s and a local model takes
    // ~25 s, so one broadcast reads as "dead" mid-generation. Keep it alive
    // until the answer is in hand; the keepalive task is dropped with the
    // guard below.
    if !forced
        && !AppState::lock(&state.budget).try_take(
            &scoped_guild,
            settings.unsolicited_per_hour,
            now,
        )
    {
        return Outcome::OverBudget;
    }
    out.typing(&event.native_channel_id).await;
    let context = assemble_context(
        state,
        &scoped_guild,
        &scoped_user,
        &scoped_channel,
        &enriched,
    );
    let reply_to = Some(event.native_message_id.clone());
    let mut host = crate::runtime::ToolScope {
        state,
        scoped_guild: scoped_guild.clone(),
        scoped_user: scoped_user.clone(),
        scoped_channel: scoped_channel.clone(),
        persona,
    };
    let generated = with_typing(out, &event.native_channel_id, async {
        // One local generation at a time; the typing indicator keeps going
        // while this turn waits for its slot. Tools are offered only when
        // someone addressed Abbey — budgeted policy replies stay single-shot.
        let _slot = state.acquire_generation().await.map_err(llm::LlmError)?;
        generate(
            state,
            &mut host,
            &Ask {
                scope: &scoped_channel,
                context: &context,
                user_input: &enriched,
                offer_tools: forced,
                now,
            },
            Some(Delivery {
                out,
                native_channel_id: &event.native_channel_id,
                reply_to: reply_to.as_deref(),
            }),
        )
        .await
    })
    .await;
    let (answer, already_sent, persona) = match generated {
        Ok(triple) => triple,
        Err(e) => {
            tracing::warn!(error = %e.0, backend = backend.label(), "reply generation failed");
            if forced {
                // Someone addressed Abbey and waited; dead air is worse than
                // the same honest failure line `/persona ask` already posts.
                let failure = OutboundMessage {
                    text: ask::render_failure(persona, backend.label(), &e.0),
                    reply_to_native_message_id: reply_to.clone(),
                    ..OutboundMessage::default()
                };
                let _ = out.send(&event.native_channel_id, &failure).await;
            }
            return Outcome::ReplyFailed(e.0);
        }
    };
    // A tool may have switched the persona; the transcript keeps its history
    // and the next turn prepares with the new persona.
    if persona != persona_for(intent, &settings) {
        let _ = AppState::lock(&state.engine).prepare(
            &scoped_channel,
            persona,
            &context,
            &enriched,
            now,
        );
    }
    AppState::lock(&state.engine).commit(&scoped_channel, &enriched, &answer, now);

    let sent_id = match already_sent {
        Some(id) => id,
        None => {
            let reply = OutboundMessage {
                text: answer,
                reply_to_native_message_id: reply_to,
                title: None,
                accent_color: None,
            };
            match out.send(&event.native_channel_id, &reply).await {
                Ok(id) => id,
                Err(e) => return Outcome::ReplyFailed(e),
            }
        }
    };

    AppState::lock(&state.rewards).register_reply(
        encoded.to_vec(),
        BotAction::Reply.index(),
        sent_id,
        scoped_guild.clone(),
        now,
    );
    {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).record_interaction(
            &scoped_user,
            &scoped_guild,
            intent.quality(),
            now,
            &mut *stores,
        );
    }
    Outcome::Replied
}

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

/// Channel summary + remembered facts + WDBX recollections + standing.
pub fn assemble_context(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    scoped_channel: &str,
    query: &str,
) -> PersonaContext {
    let mut context =
        AppState::lock(&state.stores)
            .memory
            .context_for(scoped_guild, scoped_user, scoped_channel);
    let recalled = AppState::lock(&state.recall).recall(scoped_guild, query, RECALL_K);
    for fact in recalled {
        if !context.user_facts.contains(&fact.text) {
            context.user_facts.push(fact.text);
        }
    }
    context
}

/// Describe image attachments and fold the descriptions into the text (max 3),
/// when vision is configured and the guild has it on. Failures degrade to the
/// bare text — an image Abbey cannot read is not a reason to ignore the words.
async fn enrich_with_vision<O: Outbound + Sync>(
    state: &AppState,
    out: &O,
    settings: &GuildSettings,
    text: &str,
    attachments: &[RemoteAttachment],
) -> String {
    let Some(vision_client) = &state.vision else {
        return text.to_string();
    };
    if !settings.vision_enabled {
        return text.to_string();
    }
    let mut described = Vec::new();
    for att in attachments
        .iter()
        .filter(|a| a.is_image())
        .take(vision::MAX_DESCRIBED_IMAGES)
    {
        let Ok(bytes) = out.fetch(&att.url, vision::MAX_IMAGE_BYTES).await else {
            continue;
        };
        if let Ok(desc) = vision_client.describe(&bytes).await {
            described.push((att.filename.clone(), desc));
        }
    }
    vision::fold_descriptions(text, &described)
}

async fn welcome<O: Outbound + Sync>(
    state: &AppState,
    out: &O,
    native_channel_id: &str,
    display_name: &str,
) -> Outcome {
    // Welcomes are generated, never templated: with no backend there is
    // nothing honest to say, so Abbey stays quiet.
    let Some(backend) = &state.backend else {
        return Outcome::Ignored("welcome needs a backend");
    };
    if native_channel_id.is_empty() {
        return Outcome::Ignored("welcome has no channel");
    }
    let system = engine::welcome_prompt(display_name);
    let Ok(_slot) = state.acquire_generation().await else {
        return Outcome::Ignored("welcome skipped: model busy");
    };
    let text = match llm::ask_backend(&state.llm, backend, &system, "Say hello.").await {
        Ok(t) => ask::tidy_reply(Persona::Abi, &t),
        Err(e) => return Outcome::ReplyFailed(e.0),
    };
    match out
        .send(
            native_channel_id,
            &OutboundMessage {
                text,
                ..OutboundMessage::default()
            },
        )
        .await
    {
        Ok(_) => Outcome::Welcomed,
        Err(e) => Outcome::ReplyFailed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{EventKind, SocialNetwork};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeOut {
        sent: Mutex<Vec<(String, OutboundMessage)>>,
        reacted: Mutex<Vec<(String, String, String)>>,
        edited: Mutex<Vec<(String, String, String)>>,
    }

    impl Outbound for FakeOut {
        async fn send(&self, ch: &str, m: &OutboundMessage) -> Result<String, String> {
            self.sent.lock().unwrap().push((ch.to_string(), m.clone()));
            Ok(format!("sent-{}", self.sent.lock().unwrap().len()))
        }
        async fn typing(&self, _ch: &str) {}
        async fn react(&self, ch: &str, id: &str, emoji: &str) -> Result<(), String> {
            self.reacted
                .lock()
                .unwrap()
                .push((ch.into(), id.into(), emoji.into()));
            Ok(())
        }
        async fn fetch(&self, _url: &str, _max: usize) -> Result<Vec<u8>, String> {
            Err("no network in tests".into())
        }
        async fn edit(&self, ch: &str, id: &str, text: &str) -> Result<(), String> {
            self.edited
                .lock()
                .unwrap()
                .push((ch.into(), id.into(), text.into()));
            Ok(())
        }
    }

    fn message(text: &str, guild: Option<&str>, from: &str) -> SocialEvent {
        SocialEvent {
            network: SocialNetwork::Discord,
            kind: EventKind::Message {
                text: text.into(),
                attachments: vec![],
            },
            native_message_id: "m1".into(),
            native_channel_id: "c1".into(),
            native_guild_id: guild.map(Into::into),
            native_user_id: from.into(),
            user_display_name: "Sam".into(),
            is_bot: false,
            timestamp: 0,
        }
    }

    #[tokio::test]
    async fn own_traffic_is_ignored() {
        let state = AppState::in_memory();
        state.register_self("discord:bot".into());
        let out = FakeOut::default();
        let outcome = handle(&state, &out, message("hi", Some("g"), "bot"), false, None).await;
        assert_eq!(outcome, Outcome::Ignored("own traffic"));
    }

    #[tokio::test]
    async fn a_mention_with_no_backend_gets_the_honest_degraded_reply() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let outcome = handle(
            &state,
            &out,
            message("hey abbey?", Some("g"), "u1"),
            true,
            None,
        )
        .await;
        assert_eq!(outcome, Outcome::Replied);
        let sent = out.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].1.text.contains("no generation backend"),
            "{}",
            sent[0].1.text
        );
        assert_eq!(sent[0].1.reply_to_native_message_id.as_deref(), Some("m1"));
    }

    #[tokio::test]
    async fn a_reaction_feeds_the_reward_collector() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        AppState::lock(&state.rewards).register_reply(
            vec![0.0; 18],
            1,
            "abbey-msg",
            "discord:g",
            0,
        );
        let event = SocialEvent {
            kind: EventKind::Reaction {
                emoji: "🔥".into(),
                target_message_id: "abbey-msg".into(),
                added: true,
            },
            ..message("", Some("g"), "u2")
        };
        assert_eq!(
            handle(&state, &out, event, false, None).await,
            Outcome::Rewarded
        );
        let settled = AppState::lock(&state.rewards).settle_expired(10_000);
        assert_eq!(settled.len(), 1);
        assert!((settled[0].1.reward - 0.8).abs() < 1e-6, "−0.2 + 1.0");
    }

    #[tokio::test]
    async fn blank_unsolicited_content_is_not_learned_from() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let outcome = handle(&state, &out, message("", Some("g"), "u1"), false, None).await;
        assert_eq!(outcome, Outcome::Ignored("no content available"));
        assert!(AppState::lock(&state.brains).loaded_guilds().is_empty());
    }

    #[tokio::test]
    async fn unsolicited_text_consults_the_policy_and_never_speaks_without_a_backend() {
        let state = AppState::in_memory();
        opt_in(&state, "discord:g", 6);
        let out = FakeOut::default();
        for _ in 0..20 {
            let outcome = handle(
                &state,
                &out,
                message("lol nice one", Some("g"), "u1"),
                false,
                None,
            )
            .await;
            assert!(
                matches!(
                    outcome,
                    Outcome::Stayed | Outcome::Reacted | Outcome::CooledDown | Outcome::OverBudget
                ),
                "{outcome:?}"
            );
        }
        assert!(
            out.sent.lock().unwrap().is_empty(),
            "no backend → no text ever"
        );
        assert_eq!(AppState::lock(&state.brains).loaded_guilds(), ["discord:g"]);
    }

    #[tokio::test]
    async fn a_disabled_guild_is_ignored_even_when_mentioned() {
        let state = AppState::in_memory();
        {
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.guilds).update("discord:g", &mut *stores, |s| s.enabled = false);
        }
        let out = FakeOut::default();
        let outcome = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
        assert_eq!(outcome, Outcome::Ignored("triage"));
    }

    /// Live: a DM through the real pipeline against whatever
    /// `ABBEY_BOT_LLM_ENDPOINT` / `ABBEY_BOT_LLM_MODEL` name. Ignored by
    /// default so the gate stays offline; run with
    /// `cargo test live_dm -- --ignored --nocapture` when a backend is up.
    #[tokio::test]
    #[ignore = "needs a running generation backend"]
    async fn live_dm_round_trip_against_the_configured_backend() {
        let Some(backend) = crate::llm::Backend::from_env() else {
            panic!("set ABBEY_BOT_LLM_ENDPOINT (and ABBEY_BOT_LLM_MODEL) to run this");
        };
        let mut state = AppState::in_memory();
        std::sync::Arc::get_mut(&mut state).unwrap().backend = Some(backend);
        let out = FakeOut::default();
        let first = handle(
            &state,
            &out,
            message("hey abbey", None, "donald"),
            false,
            None,
        )
        .await;
        assert_eq!(first, Outcome::Replied);
        let mut second_event = message("remember that I build in nightly Rust", None, "donald");
        second_event.native_message_id = "m2".into();
        let second = handle(&state, &out, second_event, false, None).await;
        assert_eq!(second, Outcome::Replied);
        let mut third_event = message("so what toolchain am I on?", None, "donald");
        third_event.native_message_id = "m3".into();
        let third = handle(&state, &out, third_event, false, None).await;
        assert_eq!(third, Outcome::Replied);
        let sent = out.sent.lock().unwrap();
        for (i, (_, m)) in sent.iter().enumerate() {
            eprintln!(
                "--- reply {} ({} chars):\n{}",
                i + 1,
                m.text.chars().count(),
                m.text
            );
            assert!(!m.text.contains("no generation backend"));
            assert!(m.text.chars().count() <= 2000);
        }
        assert_eq!(sent.len(), 3);
        assert_eq!(
            AppState::lock(&state.engine).session_len("discord:c1"),
            6,
            "three exchanges committed to one transcript"
        );
        assert!(
            sent[2].1.text.to_lowercase().contains("nightly"),
            "the transcript should carry the toolchain fact: {}",
            sent[2].1.text
        );
    }

    #[tokio::test]
    async fn quiet_and_learning_off_gate_unsolicited_speech_before_the_policy() {
        let mut state = AppState::in_memory();
        std::sync::Arc::get_mut(&mut state).unwrap().quiet = true;
        let out = FakeOut::default();
        let outcome = handle(
            &state,
            &out,
            message("lol nice", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        assert_eq!(outcome, Outcome::Ignored("quiet"));
        assert!(
            AppState::lock(&state.brains).loaded_guilds().is_empty(),
            "nothing learned"
        );
        // A mention still answers under quiet (degraded — no backend).
        let outcome = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
        assert_eq!(outcome, Outcome::Replied);

        let state = AppState::in_memory();
        {
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.guilds).update("discord:g", &mut *stores, |s| {
                s.unsolicited = true;
                s.learning_enabled = false;
            });
        }
        let outcome = handle(
            &state,
            &out,
            message("lol nice", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        assert_eq!(outcome, Outcome::Ignored("learning off"));
    }

    #[tokio::test]
    async fn a_forced_reply_loads_the_brain_so_its_reward_is_not_dropped() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        // No backend → degraded reply, but the brain must already be loaded.
        let outcome = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
        assert_eq!(outcome, Outcome::Replied);
        assert_eq!(AppState::lock(&state.brains).loaded_guilds(), ["discord:g"]);
        // Simulate a settled reward for that guild: it lands in the buffer.
        AppState::lock(&state.rewards).register_reply(vec![0.0; 18], 1, "sent-1", "discord:g", 0);
        AppState::lock(&state.rewards).reaction("👍", "sent-1", true);
        let settled = AppState::lock(&state.rewards).settle_expired(1_000);
        let mut brains = AppState::lock(&state.brains);
        for (g, exp) in settled {
            brains.remember(&g, exp);
        }
        assert_eq!(brains.get("discord:g").map(|b| b.buffer_len()), Some(1));
    }

    fn opt_in(state: &AppState, guild: &str, per_hour: u32) {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update(guild, &mut *stores, |s| {
            s.unsolicited = true;
            s.unsolicited_per_hour = per_hour;
            s.reply_cooldown_seconds = 0;
        });
    }

    #[tokio::test]
    async fn a_guild_that_has_not_opted_in_is_ignored_before_the_policy() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let outcome = handle(
            &state,
            &out,
            message("lol nice", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        assert_eq!(outcome, Outcome::Ignored("act off"));
        assert!(
            AppState::lock(&state.brains).loaded_guilds().is_empty(),
            "no brain, no experience"
        );
    }

    #[tokio::test]
    async fn an_opted_in_guild_consults_the_policy_and_records_the_decision() {
        let state = AppState::in_memory();
        opt_in(&state, "discord:g", 6);
        let out = FakeOut::default();
        let outcome = handle(
            &state,
            &out,
            message("lol nice", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        assert!(
            matches!(
                outcome,
                Outcome::Stayed | Outcome::Reacted | Outcome::OverBudget
            ),
            "{outcome:?} (no backend → reply degrades to Stayed)"
        );
        let brains = AppState::lock(&state.brains);
        let stats = brains
            .stats("discord:g")
            .expect("brain loaded by the decision");
        assert_eq!(
            stats.action_counts.iter().sum::<u64>(),
            1,
            "exactly one policy decision"
        );
        assert_eq!(stats.last_q.len(), 3);
        assert_eq!(stats.forced_replies, 0);
    }

    #[tokio::test]
    async fn a_mention_counts_as_forced_not_as_a_decision() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let _ = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").unwrap();
        assert_eq!(stats.forced_replies, 1);
        assert_eq!(stats.action_counts, [0, 0, 0]);
    }

    #[tokio::test]
    async fn over_budget_is_silent_and_unlearned() {
        let state = AppState::in_memory();
        opt_in(&state, "discord:g", 1);
        assert!(AppState::lock(&state.budget).try_take("discord:g", 1, runtime::now()));
        let out = FakeOut::default();
        let mut saw_over_budget = false;
        for i in 0..40 {
            let mut m = message("lol nice", Some("g"), "u1");
            m.native_message_id = format!("m{i}");
            match handle(&state, &out, m, false, None).await {
                Outcome::OverBudget => {
                    saw_over_budget = true;
                    break;
                }
                Outcome::Stayed => continue,
                other => panic!("acted past the budget: {other:?}"),
            }
        }
        assert!(
            saw_over_budget,
            "the policy never picked reply/react in 40 tries"
        );
        assert!(out.reacted.lock().unwrap().is_empty());
        assert!(out.sent.lock().unwrap().is_empty());
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").unwrap();
        let stays = stats.action_counts[BotAction::Stay.index()];
        assert_eq!(brains.get("discord:g").unwrap().buffer_len() as u64, stays);
    }

    #[tokio::test]
    async fn a_reply_the_bot_cannot_make_does_not_burn_budget() {
        let state = AppState::in_memory();
        opt_in(&state, "discord:g", 6);
        let out = FakeOut::default();
        // Walk the policy until it picks Reply at least a few times; with no
        // backend each one degrades to Stayed — and must leave the budget full
        // minus only the reacts that actually went out.
        let mut reacted = 0u32;
        for i in 0..60 {
            let mut m = message("lol nice", Some("g"), "u1");
            m.native_message_id = format!("m{i}");
            match handle(&state, &out, m, false, None).await {
                Outcome::Reacted => reacted += 1,
                Outcome::Stayed | Outcome::OverBudget => {}
                other => panic!("{other:?}"),
            }
        }
        let left = AppState::lock(&state.budget).tokens_left("discord:g", 6, runtime::now());
        assert!(
            (6.0 - left - reacted as f32).abs() < 0.05,
            "tokens spent ({}) should equal reacts sent ({reacted}); phantom replies must not cost quota",
            6.0 - left
        );
    }

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

    #[test]
    fn two_dm_users_never_share_recall_or_facts() {
        let state = AppState::in_memory();
        let alice = message("hi", None, "alice");
        let bob = message("hi", None, "bob");
        assert_ne!(alice.scoped_guild_id(), bob.scoped_guild_id());
        AppState::lock(&state.recall).remember(
            &alice.scoped_guild_id(),
            &alice.scoped_user_id(),
            "alice likes rust",
            1,
        );
        let for_bob = assemble_context(
            &state,
            &bob.scoped_guild_id(),
            &bob.scoped_user_id(),
            &bob.scoped_channel_id(),
            "rust",
        );
        assert!(for_bob.user_facts.is_empty(), "{:?}", for_bob.user_facts);
        let for_alice = assemble_context(
            &state,
            &alice.scoped_guild_id(),
            &alice.scoped_user_id(),
            &alice.scoped_channel_id(),
            "rust",
        );
        assert_eq!(for_alice.user_facts, ["alice likes rust"]);
    }

    #[test]
    fn persona_routing_follows_the_spec_table() {
        let s = GuildSettings::default();
        assert_eq!(persona_for(Intent::Command, &s), Persona::Aviva);
        assert_eq!(persona_for(Intent::ModRequest, &s), Persona::Aviva);
        assert_eq!(persona_for(Intent::Greeting, &s), Persona::Abi);
        assert_eq!(persona_for(Intent::SmallTalk, &s), Persona::Abi);
        assert_eq!(persona_for(Intent::Question, &s), Persona::Abbey);
        let aviva = GuildSettings {
            default_persona: Persona::Aviva,
            ..GuildSettings::default()
        };
        assert_eq!(persona_for(Intent::Question, &aviva), Persona::Aviva);
    }
}
