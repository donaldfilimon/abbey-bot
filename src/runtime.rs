//! The process-wide state behind every command and gateway event, and the
//! background heartbeat that learns, flushes, and persists it.
//!
//! This is the Rust shape of the spec's actor singletons (`BrainRegistry.shared`,
//! `SocialBrain.shared`, `GuildRegistry.shared`, `AbbeyScheduler`): one
//! [`AppState`] behind an `Arc`, each registry behind its own `Mutex`, locked
//! briefly and never across an `.await` that touches the network. The pure
//! modules never see a lock — they take `&mut self` and an injected `now`.
//!
//! Nothing here imports serenity or poise either; the Discord and Telegram
//! shells hand events in ([`crate::pipeline`]) and read state out.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::brain::budget::Budget;
use crate::brain::dqn::{BrainSnapshot, DqnAgent};
use crate::brain::registry::{Brain, BrainRegistry, DEFAULT_EVICT_AFTER_SECS};
use crate::brain::replay::Experience;
use crate::brain::reward::RewardCollector;
use crate::brain::social::SocialBrain;
use crate::brain::state::{BotAction, STATE_DIMENSIONS};
use crate::engine::Engine;
use crate::guild::{GuildRegistry, ReplyCooldown};
use crate::llm::{Backend, HttpTransport};
use crate::persist::Stores;
use crate::vision::{RemoteVision, VisionConfig, VisionError, VisionRequest, VisionTransport};
use crate::wdbx::Recall;

/// Hidden-layer widths per `docs/spec/adaptivelearning.md`: `[18, 64, 32, 3]`.
pub const TOPOLOGY: [usize; 4] = [STATE_DIMENSIONS, 64, 32, BotAction::ALL.len()];
/// Replay capacity per guild, per `docs/spec/brain.md`.
pub const REPLAY_CAPACITY: usize = 10_000;

/// Heartbeat intervals from the spec's `AbbeyScheduler`.
pub const LEARN_EVERY: Duration = Duration::from_secs(30);
pub const FLUSH_EVERY: Duration = Duration::from_secs(60);
pub const PERSIST_EVERY: Duration = Duration::from_secs(300);
pub const SETTLE_EVERY: Duration = Duration::from_secs(30);
/// Idle conversation sessions are dropped after this long.
pub const SESSION_IDLE_SECS: u64 = 6 * 3600;
/// How often the rolling channel summariser looks for due channels.
pub const SUMMARIZE_EVERY: Duration = Duration::from_secs(600);

/// Unix seconds now. The single place the wall clock is read; everything pure
/// takes the value as a parameter.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Local hour of day for the state encoder. UTC — the host's zone is not the
/// guild's, and a consistent clock matters more to the policy than a correct one.
pub fn hour_of_day(unix_secs: u64) -> u32 {
    u32::try_from((unix_secs / 3600) % 24).unwrap_or(0)
}

impl Brain for DqnAgent {
    type Experience = Experience;

    fn remember(&mut self, exp: Experience) {
        Self::remember(self, exp);
    }

    fn learn(&mut self) {
        Self::learn(self);
    }

    fn export_json(&self) -> String {
        serde_json::to_string(&self.export_weights()).unwrap_or_default()
    }

    fn import_json(&mut self, json: &str) -> bool {
        serde_json::from_str::<BrainSnapshot>(json)
            .ok()
            .is_some_and(|snapshot| self.import_weights(&snapshot).is_ok())
    }
}

/// A fresh per-guild policy. The seed comes from the clock so two guilds
/// created in the same second still diverge through their own experience.
pub fn fresh_brain() -> DqnAgent {
    DqnAgent::new(&TOPOLOGY, REPLAY_CAPACITY, now() ^ 0x5eed_ab13)
}

/// The live vision transport — reqwest behind the same seam the tests fake.
pub struct HttpVisionTransport {
    client: reqwest::Client,
}

impl Default for HttpVisionTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl VisionTransport for HttpVisionTransport {
    fn post(
        &self,
        request: &VisionRequest,
    ) -> impl std::future::Future<Output = Result<String, VisionError>> + Send {
        let mut builder = self.client.post(&request.url).json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value);
        }
        async move {
            let response = builder
                .send()
                .await
                .map_err(|e| VisionError(format!("the request failed: {e}")))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| VisionError(format!("reading the response failed: {e}")))?;
            if !status.is_success() {
                let brief: String = body.chars().take(300).collect();
                return Err(VisionError(format!("HTTP {status}: {brief}")));
            }
            Ok(body)
        }
    }
}

/// Everything the shells share. Construct once in `main`, clone the `Arc`.
///
/// **Lock order.** When more than one of these mutexes is held at once, take
/// them in field order: `stores` → `guilds` → `brains` → `social` → `rewards`
/// → `cooldown` → `budget` → `recall` → `engine`. The 5-minute persist tick
/// holds `stores` then `brains`; a message handler that took `brains` first
/// would deadlock against it (reported on PR #10, fixed after #16). `engine`
/// and `recall` are only ever taken alone or last.
pub struct AppState {
    pub stores: Mutex<Stores>,
    pub guilds: Mutex<GuildRegistry>,
    pub brains: Mutex<BrainRegistry<DqnAgent>>,
    pub social: Mutex<SocialBrain>,
    pub rewards: Mutex<RewardCollector>,
    pub cooldown: Mutex<ReplyCooldown>,
    /// Per-guild hourly budget for unsolicited actions.
    pub budget: Mutex<Budget>,
    /// Generation slots. A local endpoint (ollama) wedged under concurrent
    /// requests on 2026-08-19, so the local path defaults to one at a time;
    /// Anthropic defaults to four. `ABBEY_BOT_LLM_CONCURRENCY` overrides.
    pub generation: tokio::sync::Semaphore,
    /// How long a turn waits for a slot before answering "busy".
    pub queue_secs: u64,
    pub recall: Mutex<Recall>,
    pub engine: Mutex<Engine>,
    pub backend: Option<Backend>,
    /// The local backend kept as a one-shot fallback when Anthropic is primary
    /// and `ABBEY_BOT_LLM_ENDPOINT` is also set. `None` otherwise.
    pub fallback: Option<Backend>,
    /// `ABBEY_BOT_LLM_TOOLS`: `off` disables tool calling; anything else
    /// (default `auto`) offers Abbey's tools on mention/DM replies and
    /// `/persona ask`. Flips to false for the process if the backend rejects
    /// a tooled request (4xx), so a model without tool support degrades once
    /// and then stays plain.
    pub tools_enabled: std::sync::atomic::AtomicBool,
    /// `ABBEY_QUIET=1`: never speak unsolicited, anywhere. Mentions, DMs, and
    /// commands still answer. The guard for running a many-guild token while
    /// the policy is untrained.
    pub quiet: bool,
    pub llm: HttpTransport,
    pub vision: Option<RemoteVision<HttpVisionTransport>>,
    pub data_dir: Option<PathBuf>,
    /// The bot's own user id per platform (`"discord:123"`), filled in at
    /// ready time; needed to tell a mention from a message and to ignore
    /// Abbey's own traffic.
    pub self_ids: Mutex<Vec<String>>,
}

/// Default wait for a generation slot before answering "busy".
pub const DEFAULT_QUEUE_SECS: u64 = 90;

/// Concurrency for the configured backend: 1 for a local endpoint, 4 for
/// Anthropic, `ABBEY_BOT_LLM_CONCURRENCY` if set (blank/garbage/zero ignored).
pub fn concurrency_from_env(backend: Option<&Backend>) -> usize {
    let default = match backend {
        Some(Backend::Anthropic { .. }) => 4,
        _ => 1,
    };
    std::env::var("ABBEY_BOT_LLM_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// Parse the queue wait; blank/garbage/zero fall back to the default.
pub fn queue_secs_from_value(value: Option<String>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_QUEUE_SECS)
}

/// The honest copy when no slot frees up in time.
pub const BUSY_REPLY: &str = "the model is busy answering someone else; try again in a minute";

/// The runtime's [`crate::tools::ToolHost`]: one conversation's scope, over
/// `AppState`. Each method takes the locks it needs, briefly, in the
/// documented order, and returns the short plain string the model reads.
pub struct ToolScope<'a> {
    pub state: &'a AppState,
    pub scoped_guild: String,
    pub scoped_user: String,
    pub scoped_channel: String,
    /// The persona now answering; `switch_persona` changes it and the caller
    /// rebuilds the system prompt from it.
    pub persona: crate::persona::Persona,
}

impl crate::tools::ToolHost for ToolScope<'_> {
    fn remember_fact(&mut self, fact: &str) -> String {
        let t = now();
        let stored = AppState::lock(&self.state.stores).memory.remember(
            &self.scoped_guild,
            &self.scoped_user,
            fact,
            t,
        );
        if stored {
            AppState::lock(&self.state.recall).remember(
                &self.scoped_guild,
                &self.scoped_user,
                fact,
                t,
            );
            format!("Stored: {fact}")
        } else {
            "Already on record (or the fact list is full).".to_string()
        }
    }

    fn lookup_reputation(&mut self, user_id: Option<&str>) -> String {
        let user = match user_id {
            Some(id) => crate::guild::scoped_user_id(
                "discord",
                id.trim_start_matches(['<', '@', '!']).trim_end_matches('>'),
            ),
            None => self.scoped_user.clone(),
        };
        let stores = AppState::lock(&self.state.stores);
        let rep =
            AppState::lock(&self.state.social).reputation(&user, &self.scoped_guild, &*stores);
        format!("Reputation {rep:.2} (0 = poor, 1 = excellent).")
    }

    fn recall(&mut self, query: &str) -> String {
        let facts = AppState::lock(&self.state.recall).recall(&self.scoped_guild, query, 5);
        if facts.is_empty() {
            return "Nothing on record.".to_string();
        }
        facts
            .into_iter()
            .map(|f| format!("• {}", f.text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn switch_persona(&mut self, persona: crate::persona::Persona) -> String {
        self.persona = persona;
        format!("Switched to {persona}; continue the conversation as {persona}.")
    }

    fn recent_messages(&mut self, limit: usize) -> String {
        let text = AppState::lock(&self.state.stores)
            .memory
            .channel_mut(&self.scoped_channel)
            .render_recent(limit);
        if text.trim().is_empty() {
            "No recent messages on record for this channel.".to_string()
        } else {
            text
        }
    }
}

/// Why startup could not build the state.
#[derive(Debug)]
pub struct StartupError(pub String);

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StartupError {}

impl AppState {
    /// Build from the environment: `ABBEY_DATA_DIR` (optional) decides whether
    /// anything survives a restart; the LLM and vision backends come from
    /// their own variables. A corrupt state file is a startup error, not a
    /// silent fresh start — see [`Stores::load`].
    pub fn from_env() -> Result<Arc<Self>, StartupError> {
        let data_dir = std::env::var("ABBEY_DATA_DIR")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let (stores, recall) = match &data_dir {
            Some(dir) => {
                let stores = Stores::load(dir).map_err(|e| StartupError(e.to_string()))?;
                let recall = Recall::load(&Stores::wdbx_path(dir))
                    .map_err(|e| StartupError(e.to_string()))?;
                (stores, recall)
            }
            None => (Stores::default(), Recall::new()),
        };
        let vision = VisionConfig::from_env().map(|config| RemoteVision {
            config,
            transport: HttpVisionTransport::default(),
        });
        let backend = Backend::from_env();
        let mut rewards = RewardCollector::new();
        rewards.restore(stores.pending_rewards.clone());
        let fallback = match &backend {
            Some(Backend::Anthropic { .. }) => Backend::from_values(
                None,
                std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
                std::env::var("ABBEY_BOT_LLM_MODEL").ok(),
            ),
            _ => None,
        };
        Ok(Arc::new(Self {
            stores: Mutex::new(stores),
            guilds: Mutex::new(GuildRegistry::new()),
            brains: Mutex::new(BrainRegistry::new(fresh_brain, DEFAULT_EVICT_AFTER_SECS)),
            social: Mutex::new(SocialBrain::new()),
            rewards: Mutex::new(rewards),
            cooldown: Mutex::new(ReplyCooldown::new()),
            budget: Mutex::new(Budget::default()),
            generation: tokio::sync::Semaphore::new(concurrency_from_env(backend.as_ref())),
            queue_secs: queue_secs_from_value(std::env::var("ABBEY_BOT_LLM_QUEUE_SECS").ok()),
            recall: Mutex::new(recall),
            engine: Mutex::new(Engine::new()),
            backend,
            fallback,
            tools_enabled: std::sync::atomic::AtomicBool::new(
                !std::env::var("ABBEY_BOT_LLM_TOOLS")
                    .is_ok_and(|v| v.trim().eq_ignore_ascii_case("off")),
            ),
            quiet: std::env::var("ABBEY_QUIET").is_ok_and(|v| v.trim() == "1"),
            llm: HttpTransport::default(),
            vision,
            data_dir,
            self_ids: Mutex::new(Vec::new()),
        }))
    }

    /// An empty state with no backend, no vision, and no data directory — what
    /// the pipeline tests run against, and what `from_env` degrades to when
    /// nothing is configured.
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            stores: Mutex::new(Stores::default()),
            guilds: Mutex::new(GuildRegistry::new()),
            brains: Mutex::new(BrainRegistry::new(fresh_brain, DEFAULT_EVICT_AFTER_SECS)),
            social: Mutex::new(SocialBrain::new()),
            rewards: Mutex::new(RewardCollector::new()),
            cooldown: Mutex::new(ReplyCooldown::new()),
            budget: Mutex::new(Budget::default()),
            generation: tokio::sync::Semaphore::new(1),
            queue_secs: DEFAULT_QUEUE_SECS,
            recall: Mutex::new(Recall::new()),
            engine: Mutex::new(Engine::new()),
            backend: None,
            fallback: None,
            tools_enabled: std::sync::atomic::AtomicBool::new(true),
            quiet: false,
            llm: HttpTransport::default(),
            vision: None,
            data_dir: None,
            self_ids: Mutex::new(Vec::new()),
        })
    }

    /// Wait for a generation slot, up to `queue_secs`. `Err` is the
    /// user-facing reason (already honest copy) — callers render it with
    /// `ask::render_failure`.
    pub async fn acquire_generation(&self) -> Result<tokio::sync::SemaphorePermit<'_>, String> {
        match tokio::time::timeout(
            Duration::from_secs(self.queue_secs),
            self.generation.acquire(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err("the generation queue is closed".to_string()),
            Err(_) => Err(BUSY_REPLY.to_string()),
        }
    }

    /// Multi-turn generation through the primary backend, falling back to the
    /// local one once when Anthropic is primary and fails. Returns the text
    /// and the label of the backend that actually answered. The caller holds
    /// the generation slot.
    pub async fn chat(
        &self,
        system_prompt: &str,
        turns: &[crate::llm::ChatTurn],
    ) -> Result<(String, &'static str), crate::llm::LlmError> {
        let Some(primary) = &self.backend else {
            return Err(crate::llm::LlmError(
                "no generation backend is configured".into(),
            ));
        };
        match crate::llm::chat_backend(&self.llm, primary, system_prompt, turns).await {
            Ok(text) => Ok((text, primary.label())),
            Err(e) => match &self.fallback {
                Some(local) => {
                    tracing::warn!(error = %e.0, "primary backend failed; falling back to the local endpoint");
                    crate::llm::chat_backend(&self.llm, local, system_prompt, turns)
                        .await
                        .map(|text| (text, local.label()))
                }
                None => Err(e),
            },
        }
    }

    /// Lock helper: a poisoned mutex means a panic elsewhere already took the
    /// process off the rails; recovering the guard keeps the bot answering
    /// rather than cascading every command into an error.
    pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether `scoped_user_id` is one of Abbey's own accounts.
    pub fn is_self(&self, scoped_user_id: &str) -> bool {
        Self::lock(&self.self_ids)
            .iter()
            .any(|id| id == scoped_user_id)
    }

    pub fn register_self(&self, scoped_user_id: String) {
        let mut ids = Self::lock(&self.self_ids);
        if !ids.contains(&scoped_user_id) {
            ids.push(scoped_user_id);
        }
    }

    /// Settle expired rewards into their guilds' replay buffers.
    pub fn settle_rewards(&self) {
        let settled = Self::lock(&self.rewards).settle_expired(now());
        if settled.is_empty() {
            return;
        }
        let mut brains = Self::lock(&self.brains);
        for (guild, exp) in settled {
            let loaded = brains.get(&guild).is_some();
            tracing::info!(
                guild = %guild,
                reward = exp.reward,
                action = exp.action,
                loaded,
                "reward settled into the replay buffer"
            );
            if let Some(stats) = brains.stats_mut(&guild) {
                stats.record_reward(exp.reward);
            }
            brains.remember(&guild, exp);
        }
    }

    /// One learning step on every loaded brain whose guild has learning on.
    pub fn learn_all(&self) {
        let enabled: Vec<String> = {
            let mut stores = Self::lock(&self.stores);
            let mut guilds = Self::lock(&self.guilds);
            Self::lock(&self.brains)
                .loaded_guilds()
                .into_iter()
                .filter(|g| guilds.config(g, &mut *stores).learning_enabled)
                .collect()
        };
        Self::lock(&self.brains).learn_all(|g| enabled.iter().any(|e| e == g));
    }

    /// Write reputation through to the store.
    pub fn flush_social(&self) {
        let mut stores = Self::lock(&self.stores);
        Self::lock(&self.social).flush(&mut *stores);
    }

    /// Snapshot every brain, flush reputation, evict idle sessions, and write
    /// the data directory (if configured). Errors are logged, never fatal: a
    /// failed persist must not take the gateway down.
    pub fn persist_all(&self) {
        let t = now();
        {
            let mut stores = Self::lock(&self.stores);
            Self::lock(&self.brains).persist_all(&mut *stores, t);
            Self::lock(&self.social).flush(&mut *stores);
            stores.pending_rewards = Self::lock(&self.rewards).export_pending();
        }
        Self::lock(&self.engine).evict_idle(t, SESSION_IDLE_SECS);
        let Some(dir) = &self.data_dir else { return };
        if let Err(e) = Self::lock(&self.stores).save(dir) {
            tracing::error!(error = %e, "persisting state failed");
        }
        if let Err(e) = Self::lock(&self.recall).save(&Stores::wdbx_path(dir)) {
            tracing::error!(error = %e, "persisting the WDBX segment failed");
        }
    }

    /// Rolling channel summaries — the spec's "rolling 2k-token summary
    /// compressed via ABI". For every channel whose count is
    /// [`crate::memory::SUMMARY_EVERY_MESSAGES`] past its last summary, and
    /// whose guild has opted in (`/admin act on`) or is a DM, ask the backend
    /// for a summary of the recent lines and store it as the channel's
    /// context. One generation at a time, through the usual slot, so it never
    /// starves a live reply. Returns how many channels were summarised.
    pub async fn refresh_summaries(&self) -> usize {
        let Some(_) = &self.backend else { return 0 };
        let due: Vec<String> = Self::lock(&self.stores).memory.channels_due_for_summary();
        let mut done = 0;
        for scoped_channel in due {
            // Only where Abbey has been invited to pay attention.
            let Some(guild) = guild_of_channel(&Self::lock(&self.stores), &scoped_channel) else {
                continue;
            };
            let invited = guild.contains(":dm:") || {
                let mut stores = Self::lock(&self.stores);
                Self::lock(&self.guilds)
                    .config(&guild, &mut *stores)
                    .unsolicited
            };
            if !invited {
                continue;
            }
            let (transcript, count) = {
                let mut stores = Self::lock(&self.stores);
                let ctx = stores.memory.channel_mut(&scoped_channel);
                (
                    ctx.render_recent(crate::memory::RECENT_CAP),
                    ctx.recent.len(),
                )
            };
            if transcript.trim().is_empty() {
                continue;
            }
            let (system, user) =
                crate::engine::summarize_prompt(crate::persona::Persona::Abbey, &transcript, count);
            let Ok(_slot) = self.acquire_generation().await else {
                break;
            };
            match self
                .chat(&system, &[crate::llm::ChatTurn::user(user)])
                .await
            {
                Ok((summary, _)) => {
                    let summary = crate::ask::tidy_reply(crate::persona::Persona::Abbey, &summary);
                    let mut stores = Self::lock(&self.stores);
                    let ctx = stores.memory.channel_mut(&scoped_channel);
                    ctx.summary = summary;
                    ctx.summarized_at_count = ctx.message_count;
                    done += 1;
                    tracing::info!(channel = %scoped_channel, "rolling summary refreshed");
                }
                Err(e) => {
                    tracing::warn!(channel = %scoped_channel, error = %e.0, "rolling summary failed");
                    break;
                }
            }
        }
        done
    }

    /// Start the heartbeat: learn / flush / persist / settle on their
    /// intervals until the process exits. Returns nothing to hold — the tasks
    /// are detached, and [`AppState::persist_all`] at shutdown is the flush.
    pub fn start_scheduler(self: &Arc<Self>) {
        let spawn = |every: Duration, f: fn(&AppState)| {
            let state = Arc::clone(self);
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(every);
                tick.tick().await; // the first tick fires immediately; skip it
                loop {
                    tick.tick().await;
                    f(&state);
                }
            });
        };
        spawn(LEARN_EVERY, Self::learn_all);
        spawn(FLUSH_EVERY, Self::flush_social);
        spawn(PERSIST_EVERY, Self::persist_all);
        spawn(SETTLE_EVERY, Self::settle_rewards);
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SUMMARIZE_EVERY);
            tick.tick().await;
            loop {
                tick.tick().await;
                state.refresh_summaries().await;
            }
        });
    }
}

/// The scoped guild a channel's traffic belongs to, recovered from the most
/// recent message's stored guild tag. Channels are keyed by
/// `"{platform}:{channel}"` only, so the guild is stored beside the messages.
fn guild_of_channel(stores: &Stores, scoped_channel: &str) -> Option<String> {
    stores
        .memory
        .channels
        .get(scoped_channel)
        .and_then(|c| c.guild.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dqn_round_trips_through_the_brain_trait() {
        let mut a = fresh_brain();
        let json = Brain::export_json(&a);
        assert!(json.contains("\"topology\""));
        assert!(Brain::import_json(&mut a, &json));
        assert!(!Brain::import_json(&mut a, "{not json"));
        assert!(
            !Brain::import_json(
                &mut a,
                "{\"topology\":[1,2],\"layers\":[],\"epsilon\":0.1,\"step_count\":0}"
            ),
            "topology drift is rejected, not silently accepted"
        );
    }

    #[tokio::test]
    async fn generation_slots_are_bounded_and_time_out_honestly() {
        let mut state = AppState::in_memory();
        std::sync::Arc::get_mut(&mut state).unwrap().queue_secs = 1;
        let first = state.acquire_generation().await.expect("first slot");
        let started = std::time::Instant::now();
        let second = state.acquire_generation().await;
        assert_eq!(second.unwrap_err(), BUSY_REPLY);
        assert!(
            started.elapsed().as_millis() >= 900,
            "waited for the queue window"
        );
        drop(first);
        assert!(state.acquire_generation().await.is_ok(), "slot freed");
    }

    #[test]
    fn queue_and_concurrency_parse_with_fallbacks() {
        assert_eq!(queue_secs_from_value(None), DEFAULT_QUEUE_SECS);
        assert_eq!(queue_secs_from_value(Some("30".into())), 30);
        assert_eq!(queue_secs_from_value(Some("0".into())), DEFAULT_QUEUE_SECS);
    }

    #[tokio::test]
    async fn rolling_summaries_do_nothing_without_a_backend_and_keep_channels_due() {
        let state = AppState::in_memory();
        {
            let mut stores = AppState::lock(&state.stores);
            for i in 0..30 {
                stores
                    .memory
                    .record_message("discord:c", "a", &format!("m{i}"), i);
            }
            stores.memory.channel_mut("discord:c").guild = Some("discord:g".into());
        }
        assert_eq!(state.refresh_summaries().await, 0);
        assert_eq!(
            AppState::lock(&state.stores)
                .memory
                .channels_due_for_summary(),
            ["discord:c"],
            "still due — nothing consumed the marker"
        );
    }

    #[test]
    fn hour_of_day_wraps_at_24() {
        assert_eq!(hour_of_day(0), 0);
        assert_eq!(hour_of_day(3600 * 25), 1);
        assert_eq!(hour_of_day(3600 * 23 + 59), 23);
    }

    #[test]
    fn topology_matches_the_spec() {
        assert_eq!(TOPOLOGY, [18, 64, 32, 3]);
    }
    #[test]
    fn settled_rewards_reach_the_guild_stats() {
        let state = AppState::in_memory();
        {
            let stores = AppState::lock(&state.stores);
            let mut brains = AppState::lock(&state.brains);
            brains.brain("discord:g", &*stores, 0);
        }
        AppState::lock(&state.rewards).register_reply(vec![0.0; 18], 1, "m1", "discord:g", 0);
        AppState::lock(&state.rewards).reaction("👍", "m1", true);
        // settle_rewards reads the real clock; the entry is 150 s+ old by any clock.
        state.settle_rewards();
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").expect("loaded");
        assert_eq!(stats.settled_total, 1);
        assert!((stats.mean_recent_reward().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(brains.get("discord:g").unwrap().buffer_len(), 1);
        drop(brains);
        assert!(AppState::lock(&state.budget).try_take("discord:g", 6, 0));
    }
}
