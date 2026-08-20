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
use crate::brain::intent;
use crate::brain::state::{self, BotAction, StateInput};
use crate::engine;
use crate::generation::{Ask, Delivery, generate_read_only, generate_with_tools, with_typing};
use crate::guild::GuildSettings;
use crate::llm;
use crate::memory::PersonaContext;
use crate::persona::{self, Persona};
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

/// Canonical ABI text routing with the guild default retained only for neutral
/// input. Explicit leading names and weighted ABI cues take precedence.
pub fn persona_for(text: &str, settings: &GuildSettings) -> Persona {
    persona_for_session(text, settings, None)
}

/// Explicit names and weighted ABI cues always win. A neutral follow-up keeps
/// the persona already attached to this transcript; only a new session falls
/// back to the guild default. This makes `switch_persona` a real conversational
/// handoff instead of a one-response costume change.
fn persona_for_session(
    text: &str,
    settings: &GuildSettings,
    session_persona: Option<Persona>,
) -> Persona {
    let route = persona::route(text, None);
    if matches!(route.reason, persona::Reason::Default) {
        session_persona.unwrap_or(settings.default_persona)
    } else {
        route.persona
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
            // A welcome is unsolicited speech: the operator's QUIET and the
            // guild's `/admin act on` gate it exactly like a policy reply.
            if state.quiet {
                return Outcome::Ignored("quiet");
            }
            if !settings.unsolicited {
                return Outcome::Ignored("act off");
            }
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
    let reputation = state.reputation_snapshot(&scoped_guild, &scoped_user);
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
        // Budget is only *checked* here; a token is reserved immediately before
        // the network action below. A missing backend costs nothing, while an
        // attempted react/reply consumes quota even if delivery later fails so
        // a broken endpoint cannot bypass the volume guard indefinitely.
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
    let session_persona = AppState::lock(&state.engine).session_persona(&scoped_channel);
    let initial_persona = session_persona.map_or_else(
        || persona_for(&enriched, &settings),
        |persona| persona_for_session(&enriched, &settings, Some(persona)),
    );
    let Some(backend) = &state.backend else {
        if !forced {
            // A policy that wants to speak with nothing to speak through is a
            // policy we cannot follow; treat it as silence without learning.
            return Outcome::Stayed;
        }
        let reply = OutboundMessage {
            text: ask::degraded_reply(initial_persona),
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
        reputation,
    );
    let reply_to = Some(event.native_message_id.clone());
    let generated = with_typing(out, &event.native_channel_id, async {
        // One local generation at a time; the typing indicator keeps going
        // while this turn waits for its slot. Tools are offered only when
        // someone addressed Abbey — budgeted policy replies stay single-shot.
        let _slot = state.acquire_generation().await?;
        let ask = Ask {
            scope: &scoped_channel,
            context: &context,
            user_input: &enriched,
            now,
        };
        let delivery = Some(Delivery {
            out,
            native_channel_id: &event.native_channel_id,
            reply_to: reply_to.as_deref(),
        });
        if forced {
            let mut host = crate::runtime::ToolScope {
                state,
                network: event.network,
                scoped_guild: scoped_guild.clone(),
                scoped_user: scoped_user.clone(),
                scoped_channel: scoped_channel.clone(),
                persona: initial_persona,
            };
            generate_with_tools(state, &mut host, &ask, delivery).await
        } else {
            generate_read_only(state, initial_persona, &ask, delivery).await
        }
    })
    .await;
    let (answer, already_sent, persona) = match generated {
        Ok(triple) => triple,
        Err(e) => {
            tracing::warn!(error = %e, backend = backend.label(), "reply generation failed");
            if forced {
                // Someone addressed Abbey and waited; dead air is worse than
                // the same honest failure line `/persona ask` already posts.
                let failure = OutboundMessage {
                    text: ask::render_failure(initial_persona, backend.label(), &e),
                    reply_to_native_message_id: reply_to.clone(),
                    ..OutboundMessage::default()
                };
                let _ = out.send(&event.native_channel_id, &failure).await;
            }
            return Outcome::ReplyFailed(e.to_string());
        }
    };
    // A tool may have switched the persona; the transcript keeps its history
    // and the next turn prepares with the new persona.
    if persona != initial_persona {
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

/// Channel summary + remembered facts + WDBX recollections + standing.
pub fn assemble_context(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    scoped_channel: &str,
    query: &str,
    reputation: f64,
) -> PersonaContext {
    state.memory_service().context_for(
        scoped_guild,
        scoped_user,
        scoped_channel,
        query,
        RECALL_K,
        reputation,
    )
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
        if let Ok(desc) = vision_client.describe(bytes).await {
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
        Err(e) => return Outcome::ReplyFailed(e.to_string()),
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

/// Test doubles shared with [`crate::generation`]'s tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::Mutex;

    /// A recording [`Outbound`]: every send, react, and edit is kept.
    #[derive(Default)]
    pub(crate) struct FakeOut {
        pub(crate) sent: Mutex<Vec<(String, OutboundMessage)>>,
        pub(crate) reacted: Mutex<Vec<(String, String, String)>>,
        pub(crate) edited: Mutex<Vec<(String, String, String)>>,
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
}

#[cfg(test)]
#[path = "pipeline/tests.rs"]
mod tests;
