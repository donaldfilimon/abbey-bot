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
use crate::brain::outcome::{self, ReplyOutcome};
use crate::brain::reward::ReplyTurn;
use crate::brain::state::{self, BotAction, StateInput};
use crate::engine;
use crate::generation::{Ask, Delivery, generate, with_typing};
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
    //
    // Two reward channels, deliberately separate. `human_replied` is the
    // existing untyped heuristic — "a human bothered to reply", +0.5 whatever
    // they said. `observe_*` feeds the delayed channel: *what* they said,
    // typed. A Discord reply-to legitimately feeds both; that is the blend, not
    // a double count. See `brain::reward`.
    //
    // Claim-honest: this is the only place real Discord observations reach the
    // delayed channel today. It covers a reply-to and a same-channel follow-up.
    // Reactions still land on the untyped path, and edits, threads, and voice
    // are not observed at all.
    {
        let mut rewards = AppState::lock(&state.rewards);
        if let Some(id) = reply_to {
            rewards.human_replied(id);
        }
        let prior_ask = match reply_to {
            Some(id) => rewards.open_ask(id),
            None => rewards.open_ask_in_scope(&scoped_channel, now),
        }
        .map(str::to_owned);
        // An unreadable or off-topic message means the human moved on without
        // engaging: weak evidence, worth exactly nothing, never a penalty.
        let observed =
            outcome::classify(&text, prior_ask.as_deref()).unwrap_or(ReplyOutcome::NoEngagement);
        let credited = match reply_to {
            Some(id) => rewards
                .observe_reply_to(id, observed)
                .then(|| id.to_owned()),
            None => rewards.observe_in_scope(&scoped_channel, &scoped_user, observed, now),
        };
        if let Some(turn) = credited {
            tracing::debug!(
                turn = %turn,
                outcome = ?observed,
                "delayed outcome attributed to an open turn"
            );
        }
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
    );
    let reply_to = Some(event.native_message_id.clone());
    let mut host = crate::runtime::ToolScope {
        state,
        scoped_guild: scoped_guild.clone(),
        scoped_user: scoped_user.clone(),
        scoped_channel: scoped_channel.clone(),
        persona: initial_persona,
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
                    text: ask::render_failure(initial_persona, backend.label(), &e.0),
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

    // The full turn: scope and ask travel with it so a later follow-up in this
    // channel — one with no reply-to pointer — can still be attributed back to
    // this exact action.
    AppState::lock(&state.rewards).register_turn(ReplyTurn {
        state: encoded.to_vec(),
        action: BotAction::Reply.index(),
        sent_native_message_id: sent_id,
        scope: scoped_channel.clone(),
        scoped_guild_id: scoped_guild.clone(),
        // The human's own words, not `enriched`: vision folds Abbey-written
        // image descriptions into the prompt text, and padding the ask with
        // them would depress every later topic-overlap ratio.
        ask: text.clone(),
        asker: scoped_user.clone(),
        now,
    });
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
) -> PersonaContext {
    let mut context =
        AppState::lock(&state.stores)
            .memory
            .context_for(scoped_guild, scoped_user, scoped_channel);
    let recalled =
        AppState::lock(&state.recall).recall_for_user(scoped_guild, scoped_user, query, RECALL_K);
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
mod tests {
    use super::testing::FakeOut;
    use super::*;
    use crate::platform::{EventKind, SocialNetwork};

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

    /// An open Abbey turn in `discord:c1`, the scope `message()` produces,
    /// answering `discord:u1` — the user the tests below speak as.
    fn open_turn(state: &AppState, now: u64) {
        AppState::lock(&state.rewards).register_turn(ReplyTurn {
            state: vec![0.0; 18],
            action: BotAction::Reply.index(),
            sent_native_message_id: "abbey-msg".into(),
            scope: "discord:c1".into(),
            scoped_guild_id: "discord:g".into(),
            ask: "how do I configure the voice gateway timeout?".into(),
            asker: "discord:u1".into(),
            now,
        });
    }

    fn only_settled_reward(state: &AppState, now: u64) -> f32 {
        let settled = AppState::lock(&state.rewards).settle_expired(now);
        assert_eq!(settled.len(), 1);
        settled[0].1.reward
    }

    #[tokio::test]
    async fn a_thanks_in_reply_to_abbey_reaches_the_delayed_channel() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        open_turn(&state, 0);
        // The guild has not opted in, so Abbey stays silent — but reward
        // bookkeeping for a turn she already took runs before the gates.
        let outcome = handle(
            &state,
            &out,
            message("thanks, that worked", Some("g"), "u1"),
            false,
            Some("abbey-msg"),
        )
        .await;
        assert_eq!(outcome, Outcome::Ignored("act off"));
        // −0.2 baseline + 0.5 untyped engagement + 1.0 typed thanks.
        let reward = only_settled_reward(&state, 10_000);
        assert!((reward - 1.3).abs() < 1e-6, "{reward}");
    }

    #[tokio::test]
    async fn a_correction_in_reply_to_abbey_costs_more_than_silence() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        open_turn(&state, 0);
        let _ = handle(
            &state,
            &out,
            message("no, that's not the right port", Some("g"), "u1"),
            false,
            Some("abbey-msg"),
        )
        .await;
        // −0.2 + 0.5 engaged − 1.0 typed correction: below the −0.2 a turn
        // nobody answered would have settled at.
        let reward = only_settled_reward(&state, 10_000);
        assert!((reward + 0.7).abs() < 1e-6, "{reward}");
        assert!(reward < -0.2);
    }

    #[tokio::test]
    async fn a_same_channel_follow_up_needs_no_reply_pointer() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let now = runtime::now();
        open_turn(&state, now);
        let _ = handle(
            &state,
            &out,
            message("does the gateway retry after a timeout?", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        // −0.2 + 0.4 topical follow-up. No untyped +0.5: there was no reply-to,
        // which is exactly the gap scope attribution exists to cover.
        let reward = only_settled_reward(
            &state,
            now + crate::brain::reward::SETTLEMENT_WINDOW_SECS + 1,
        );
        assert!((reward - 0.2).abs() < 1e-6, "{reward}");
    }

    #[tokio::test]
    async fn a_bystanders_thanks_in_the_same_channel_is_not_credited() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let now = runtime::now();
        open_turn(&state, now);
        // u2 was never answered by Abbey — this is "thanks Carol!", not
        // feedback, and it must not move the turn u1's ask earned.
        let _ = handle(
            &state,
            &out,
            message("thanks, that worked", Some("g"), "u2"),
            false,
            None,
        )
        .await;
        let reward = only_settled_reward(
            &state,
            now + crate::brain::reward::SETTLEMENT_WINDOW_SECS + 1,
        );
        assert_eq!(reward.to_bits(), (-0.2f32).to_bits(), "{reward}");
    }

    #[tokio::test]
    async fn unrelated_chatter_leaves_the_turn_exactly_as_it_was() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let now = runtime::now();
        open_turn(&state, now);
        let _ = handle(
            &state,
            &out,
            message("anyone up for lunch?", Some("g"), "u1"),
            false,
            None,
        )
        .await;
        let reward = only_settled_reward(
            &state,
            now + crate::brain::reward::SETTLEMENT_WINDOW_SECS + 1,
        );
        assert_eq!(
            reward.to_bits(),
            (-0.2f32).to_bits(),
            "an off-topic message is no engagement, and no engagement is free"
        );
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
        for i in 0..400 {
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
            "the policy never picked reply/react in 400 tries"
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

    #[tokio::test]
    async fn a_member_join_welcome_is_gated_like_any_unsolicited_speech() {
        let out = FakeOut::default();
        let join = |guild: &str| SocialEvent {
            kind: EventKind::MemberJoined,
            ..message("", Some(guild), "newbie")
        };
        // Quiet wins.
        let mut state = AppState::in_memory();
        std::sync::Arc::get_mut(&mut state).unwrap().quiet = true;
        assert_eq!(
            handle(&state, &out, join("g"), false, None).await,
            Outcome::Ignored("quiet")
        );
        // Not opted in.
        let state = AppState::in_memory();
        assert_eq!(
            handle(&state, &out, join("g"), false, None).await,
            Outcome::Ignored("act off")
        );
        // Opted in, no backend → honest silence, nothing sent.
        opt_in(&state, "discord:g", 6);
        assert_eq!(
            handle(&state, &out, join("g"), false, None).await,
            Outcome::Ignored("welcome needs a backend")
        );
        assert!(out.sent.lock().unwrap().is_empty());
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
    fn two_users_in_one_guild_never_share_semantic_recall() {
        let state = AppState::in_memory();
        AppState::lock(&state.recall).remember(
            "discord:g",
            "discord:alice",
            "alice's private editor preference is helix",
            1,
        );
        AppState::lock(&state.recall).remember(
            "discord:g",
            "discord:bob",
            "bob's private editor preference is vim",
            2,
        );
        let alice = assemble_context(
            &state,
            "discord:g",
            "discord:alice",
            "discord:c",
            "editor preference",
        );
        let bob = assemble_context(
            &state,
            "discord:g",
            "discord:bob",
            "discord:c",
            "editor preference",
        );
        assert_eq!(
            alice.user_facts,
            ["alice's private editor preference is helix"]
        );
        assert_eq!(bob.user_facts, ["bob's private editor preference is vim"]);
    }

    #[test]
    fn persona_routing_follows_the_canonical_abi_router() {
        let s = GuildSettings::default();
        assert_eq!(persona_for("hello there", &s), Persona::Abbey);
        assert_eq!(
            persona_for("execute the deploy quickly", &s),
            Persona::Aviva
        );
        assert_eq!(persona_for("ABI: review governance risk", &s), Persona::Abi);
        let aviva = GuildSettings {
            default_persona: Persona::Aviva,
            ..GuildSettings::default()
        };
        assert_eq!(persona_for("hello there", &aviva), Persona::Aviva);
        assert_eq!(persona_for("Abbey, help me", &aviva), Persona::Abbey);
        assert_eq!(
            persona_for_session("and what about tomorrow?", &s, Some(Persona::Aviva)),
            Persona::Aviva,
            "a neutral follow-up keeps a tool-selected persona"
        );
        assert_eq!(
            persona_for_session("Abbey, take this one", &s, Some(Persona::Aviva)),
            Persona::Abbey,
            "an explicit canonical name still overrides sticky state"
        );
    }
}
