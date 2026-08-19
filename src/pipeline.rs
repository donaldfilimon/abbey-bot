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
}

/// What the pipeline did with an event — for logs and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ignored(&'static str),
    Rewarded,
    Welcomed,
    Stayed,
    CooledDown,
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

    let action = if forced {
        BotAction::Reply
    } else {
        let mut brains = AppState::lock(&state.brains);
        let stores = AppState::lock(&state.stores);
        let brain = brains.brain(&scoped_guild, &*stores, now);
        if let Some(eps) = settings.epsilon_override {
            brain.set_epsilon(eps);
        }
        BotAction::from_index(brain.select_action(&encoded)).unwrap_or(BotAction::Stay)
    };

    if action == BotAction::Stay {
        AppState::lock(&state.brains).remember(
            &scoped_guild,
            crate::brain::reward::RewardCollector::silence_experience(encoded.to_vec()),
        );
        return Outcome::Stayed;
    }

    // Unsolicited output is rate-limited per channel; mentions and DMs bypass.
    if !forced {
        let permitted = AppState::lock(&state.cooldown).permitted(
            &scoped_channel,
            settings.reply_cooldown_seconds,
            now,
        );
        if !permitted {
            return Outcome::CooledDown;
        }
    }

    if action == BotAction::React {
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
        AppState::lock(&state.cooldown).record_reply(&scoped_channel, now);
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

    out.typing(&event.native_channel_id).await;
    let context = assemble_context(
        state,
        &scoped_guild,
        &scoped_user,
        &scoped_channel,
        &enriched,
    );
    let prepared =
        AppState::lock(&state.engine).prepare(&scoped_channel, persona, &context, &enriched, now);
    let answer = llm::chat_backend(
        &state.llm,
        backend,
        &prepared.system_prompt,
        &prepared.turns,
    )
    .await;
    let answer = match answer {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e.0, backend = backend.label(), "reply generation failed");
            return Outcome::ReplyFailed(e.0);
        }
    };
    AppState::lock(&state.engine).commit(&scoped_channel, &enriched, &answer, now);

    let reply = OutboundMessage {
        text: answer,
        reply_to_native_message_id: Some(event.native_message_id.clone()),
        title: None,
        accent_color: None,
    };
    let sent_id = match out.send(&event.native_channel_id, &reply).await {
        Ok(id) => id,
        Err(e) => return Outcome::ReplyFailed(e),
    };

    AppState::lock(&state.rewards).register_reply(
        encoded.to_vec(),
        BotAction::Reply.index(),
        sent_id,
        scoped_guild.clone(),
        now,
    );
    if !forced {
        AppState::lock(&state.cooldown).record_reply(&scoped_channel, now);
    }
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
    let text = match llm::ask_backend(&state.llm, backend, &system, "Say hello.").await {
        Ok(t) => t,
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
                    Outcome::Stayed | Outcome::Reacted | Outcome::CooledDown
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
