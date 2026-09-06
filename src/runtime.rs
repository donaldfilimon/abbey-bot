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

use std::collections::BTreeMap;
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
use crate::llm::Backend;
use crate::persist::{
    FsPersistenceSink, PersistComponentOutcome, PersistReport, PersistenceSink, Stores,
    persist_canonical, persist_projection,
};
use crate::platform::SocialNetwork;
#[cfg(test)]
use crate::provider::FoundationModels;
use crate::provider::ProviderRuntime;
use crate::vision::{VisionError, VisionRequest, VisionTransport};
use crate::wdbx::Recall;

mod memory_service;
mod provider_setup;
pub use memory_service::{MemoryService, RememberOutcome, SupersessionOutcome};

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
    now_millis() / 1000
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
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

fn attachment_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static attachment client configuration is valid")
}

/// The live vision transport — reqwest behind the same seam the tests fake.
pub struct HttpVisionTransport {
    remote_client: reqwest::Client,
    loopback_client: reqwest::Client,
}

impl Default for HttpVisionTransport {
    fn default() -> Self {
        let client = |no_proxy| {
            let builder = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none());
            let builder = if no_proxy {
                builder.no_proxy()
            } else {
                builder
            };
            builder
                .build()
                .expect("static vision transport client configuration is valid")
        };
        Self {
            remote_client: client(false),
            // Images destined for a local VLM must never transit a process-
            // wide HTTP proxy, matching the local text and speech boundary.
            loopback_client: client(true),
        }
    }
}

impl HttpVisionTransport {
    fn client_for(&self, raw_url: &str) -> &reqwest::Client {
        if reqwest::Url::parse(raw_url).is_ok_and(|url| crate::llm::url_is_loopback(&url)) {
            &self.loopback_client
        } else {
            &self.remote_client
        }
    }
}

impl VisionTransport for HttpVisionTransport {
    fn post(
        &self,
        request: &VisionRequest,
    ) -> impl std::future::Future<Output = Result<String, VisionError>> + Send {
        let mut builder = self
            .client_for(&request.url)
            .post(&request.url)
            .json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value);
        }
        async move {
            let response = builder.send().await.map_err(|e| {
                VisionError::classified(
                    "the vision transport failed",
                    if e.is_timeout() {
                        crate::provider::ProviderFailureKind::Timeout
                    } else {
                        crate::provider::ProviderFailureKind::TransportUnavailable
                    },
                )
            })?;
            let status = response.status();
            let rejection = crate::llm::LlmError::http(
                status,
                response.headers().get(reqwest::header::RETRY_AFTER),
            );
            if status.is_success()
                && response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER)
            {
                return Err(VisionError::classified(
                    "incompatible provider delay metadata",
                    crate::provider::ProviderFailureKind::ProtocolDrift,
                ));
            }
            let body = crate::http_body::read_capped(response, 2 * 1024 * 1024)
                .await
                .map_err(|e| VisionError::internal(e.to_string()))?;
            if !status.is_success() {
                drop(body);
                return Err(VisionError::from_llm(rejection));
            }
            String::from_utf8(body).map_err(|_| {
                VisionError::internal("the vision provider returned non-UTF-8 response bytes")
            })
        }
    }
}

/// Everything the shells share. Construct once in `main`, clone the `Arc`.
///
/// **Lock order.** When more than one of these mutexes is held at once, take
/// them in field order: `stores` → `guilds` → `brains` → `social` → `rewards`
/// → `cooldown` → `ask_cooldown` → `budget` → `recall` → `engine`. The 5-minute persist tick
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
    /// Atomic per-user reservation for `/persona ask` cost control.
    pub ask_cooldown: Mutex<ReplyCooldown>,
    /// Per-guild hourly budget for unsolicited actions.
    pub budget: Mutex<Budget>,
    pub providers: ProviderRuntime,
    pub recall: Mutex<Recall>,
    pub engine: Mutex<Engine>,
    /// `ABBEY_QUIET=1`: never speak unsolicited, anywhere. Mentions, DMs, and
    /// commands still answer. The guard for running a many-guild token while
    /// the policy is untrained.
    pub quiet: bool,
    /// Shared, timeout-bounded client for Discord attachment downloads.
    pub attachments: reqwest::Client,
    pub data_dir: Option<PathBuf>,
    persistence_sink: Arc<dyn PersistenceSink>,
    /// The bot's own user id per platform (`"discord:123"`), filled in at
    /// ready time; needed to tell a mention from a message and to ignore
    /// Abbey's own traffic.
    pub self_ids: Mutex<Vec<String>>,
    /// Guild-keyed coarse voice lifecycle published for the Inspect pack.
    /// This carries no participant, consent, media, provider, or timestamp
    /// detail and is never held while another process-state lock is held.
    pub voice_inspect: Arc<crate::inspect::VoiceInspectRegistry>,
    /// `ABBEY_EPISODE_GATE_CONFIG`: the constitutional episode gate client.
    /// `None` (the default) means no ledger write is ever attempted.
    pub episode_gate: Option<Arc<crate::episode_gate::EpisodeGate>>,
    /// The last brain checkpoint the ledger admitted per guild (or the row
    /// loaded before the gate existed). Only consulted when `episode_gate` is
    /// configured; see `checkpoint_gate`.
    pub checkpoints: Mutex<BTreeMap<String, crate::checkpoint_gate::AdmittedCheckpoint>>,
    /// Model-tool memory writes waiting for the episode gate; drained after
    /// each tool turn and before each gated persist. Empty unless a gate is
    /// configured.
    pub memory_queue: Mutex<Vec<crate::memory_gate::QueuedFact>>,
}

/// Default wait for a generation slot before answering "busy".
pub const DEFAULT_QUEUE_SECS: u64 = 90;
/// Live Discord voice waits at least this long for the same one-slot
/// semaphore. A short text reply must not fail-close an in-channel turn.
/// Still one permit — local gemma4:12b is one-at-a-time.
pub const DEFAULT_VOICE_QUEUE_SECS: u64 = 180;

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

/// Voice never waits less than the text queue, and at least
/// [`DEFAULT_VOICE_QUEUE_SECS`].
pub fn voice_queue_secs(text_queue_secs: u64) -> u64 {
    text_queue_secs.max(DEFAULT_VOICE_QUEUE_SECS)
}

/// The runtime's [`crate::tools::ToolHost`]: one conversation's scope, over
/// `AppState`. Each method takes the locks it needs, briefly, in the
/// documented order, and returns the short plain string the model reads.
pub struct ToolScope<'a> {
    pub state: &'a AppState,
    /// Network of the conversation. Explicit native ids supplied to tools are
    /// scoped with this value; they must never default to Discord.
    pub network: SocialNetwork,
    pub scoped_guild: String,
    pub scoped_user: String,
    pub scoped_channel: String,
    /// One timestamp captured by the caller for the complete tool conversation.
    pub now: u64,
    /// The persona now answering; `switch_persona` changes it and the caller
    /// rebuilds the system prompt from it.
    pub persona: crate::persona::Persona,
}

impl crate::tools::ToolHost for ToolScope<'_> {
    fn remember_fact(&mut self, fact: &str, supersedes: Option<&str>) -> String {
        // A model may PROPOSE that a new fact replaces an old one, but never
        // apply it. `remember_proposing` stores the new fact and queues the
        // proposal; the old fact survives until a human confirms. There is no
        // model-callable path to `remember_replacing` — a model must not be
        // able to confirm its own contested claim.
        // With the episode gate configured every memory write is proposed to
        // the ledger first, and this trait is synchronous, so the model path
        // cannot propose here. It queues instead (Donald's choice 2026-09-06):
        // the write is proposed and stored by the next drain, and the model is
        // told nothing is on record yet. A supersession stays a proposal the
        // person confirms; the old fact is never removed by the model.
        if self.state.gate_for(&self.scoped_guild).is_some() {
            return match crate::memory_gate::enqueue(
                self.state,
                &self.scoped_guild,
                &self.scoped_user,
                fact,
                supersedes,
                self.now,
            ) {
                Ok(message) | Err(message) => message,
            };
        }
        let service = self.state.memory_service();
        let outcome = match supersedes {
            Some(old) => service.remember_proposing(
                &self.scoped_guild,
                &self.scoped_user,
                fact,
                old,
                self.now,
            ),
            None => service.remember(&self.scoped_guild, &self.scoped_user, fact, self.now),
        };
        match outcome {
            Ok(RememberOutcome::Stored(fact)) => format!("Stored: {fact}"),
            Ok(RememberOutcome::Proposed { stored, proposed }) => format!(
                "Stored: {stored}. Proposed to replace {proposed:?}, which is unchanged until the person confirms."
            ),
            Ok(RememberOutcome::Superseded { stored, removed }) => {
                format!("Stored: {stored}. Replaced: {removed}")
            }
            Ok(RememberOutcome::Unchanged) => {
                "Already on record (or the fact list is full).".to_string()
            }
            Err(message) => message.to_string(),
        }
    }

    fn lookup_reputation(&mut self, user_id: Option<&str>) -> String {
        let user = match user_id {
            Some(id) => crate::guild::scoped_user_id(
                self.network.as_str(),
                id.trim_start_matches(['<', '@', '!']).trim_end_matches('>'),
            ),
            None => self.scoped_user.clone(),
        };
        let rep = self.state.reputation_snapshot(&self.scoped_guild, &user);
        format!("Reputation {rep:.2} (0 = poor, 1 = excellent).")
    }

    fn recall(&mut self, query: &str) -> String {
        let facts =
            self.state
                .memory_service()
                .recall(&self.scoped_guild, &self.scoped_user, query, 5);
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

    fn inspect_status(&mut self, aspect: crate::tools::InspectAspect) -> String {
        let runtime = crate::inspect::RuntimeInspect {
            generation_configured: self.state.generation_label().is_some(),
            tools_on: self.state.providers.tools_enabled(),
            vision_on: self.state.providers.vision_available(),
            quiet: self.state.quiet,
            data: self.state.data_dir.is_some(),
            gate: self.state.episode_gate.as_ref().map(|gate| gate.counters()),
        };
        let guild_line = if matches!(
            aspect,
            crate::tools::InspectAspect::Guild | crate::tools::InspectAspect::All
        ) {
            let stores = AppState::lock(&self.state.stores);
            let guilds = AppState::lock(&self.state.guilds);
            let settings = guilds.lookup(&self.scoped_guild, &*stores);
            drop(guilds);
            drop(stores);
            settings.map(|settings| {
                let left = AppState::lock(&self.state.budget).tokens_left(
                    &self.scoped_guild,
                    settings.unsolicited_per_hour,
                    self.now,
                );
                crate::inspect::render_guild_body(&settings, left)
            })
        } else {
            None
        };
        let voice = if matches!(
            aspect,
            crate::tools::InspectAspect::Voice | crate::tools::InspectAspect::All
        ) {
            self.state.voice_inspect.state_for(&self.scoped_guild)
        } else {
            crate::inspect::VoiceInspectState::Off
        };
        let providers = if matches!(
            aspect,
            crate::tools::InspectAspect::Provider | crate::tools::InspectAspect::All
        ) {
            self.state.provider_inspect()
        } else {
            Vec::new()
        };
        crate::inspect::render_status(aspect, &runtime, guild_line.as_deref(), voice, &providers)
    }

    fn list_facts(&mut self) -> String {
        let service = self.state.memory_service();
        let (facts, pending) = service.subject_snapshot(&self.scoped_guild, &self.scoped_user);
        crate::inspect::render_facts(&facts, &pending)
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
        let (stores, recall) =
            memory_service::reconcile_loaded(stores, recall).map_err(StartupError)?;
        let backend = Backend::from_env();
        if let Some(backend) = &backend {
            backend
                .validate()
                .map_err(|e| StartupError(e.to_string()))?;
        }
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
        if let Some(fallback) = &fallback {
            fallback
                .validate()
                .map_err(|e| StartupError(e.to_string()))?;
        }
        let tools_enabled = !std::env::var("ABBEY_BOT_LLM_TOOLS")
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case("off"));
        let provider_setup = provider_setup::from_env(backend.as_ref(), tools_enabled)?;
        let mut providers = ProviderRuntime::legacy(
            backend.clone(),
            fallback,
            provider_setup.foundation_models,
            provider_setup.vision,
            tools_enabled,
            concurrency_from_env(backend.as_ref()),
            queue_secs_from_value(std::env::var("ABBEY_BOT_LLM_QUEUE_SECS").ok()),
        );
        let provider_config = crate::provider::ProviderConfig::from_iter(std::env::vars_os())
            .map_err(|error| StartupError(error.to_string()))?;
        let block_directory = provider_config
            .state_dir
            .clone()
            .or_else(|| {
                data_dir
                    .as_ref()
                    .map(|directory| directory.join("provider-runtime"))
            })
            .or_else(|| {
                #[cfg(windows)]
                let home = std::env::var_os("LOCALAPPDATA");
                #[cfg(not(windows))]
                let home = std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local/share").into_os_string());
                home.map(|home| PathBuf::from(home).join("abbey-bot/provider-runtime"))
            });
        if (providers.generation_label().is_some() || providers.vision_available())
            && block_directory.is_none()
        {
            return Err(StartupError(
                "configured providers require an operational state directory".into(),
            ));
        }
        providers.apply_configuration(provider_config);
        if let Some(path) = std::env::var_os("ABBEY_FM_CAPABILITY_MANIFEST") {
            providers
                .apply_fm_qualification(std::path::Path::new(&path))
                .map_err(StartupError)?;
        }
        if let Some(directory) = block_directory.as_ref() {
            providers
                .restore_blocks(directory.join("provider-blocks.json"))
                .map_err(StartupError)?;
        }
        let episode_gate = crate::episode_gate::EpisodeGateConfig::from_env()
            .map_err(StartupError)?
            .map(|config| Arc::new(crate::episode_gate::EpisodeGate::new(config)));
        // Rows already on disk are never dropped by a gate decision.
        let checkpoints = if episode_gate.is_some() {
            crate::checkpoint_gate::seed(&stores.brains)
        } else {
            BTreeMap::new()
        };
        Ok(Arc::new(Self {
            stores: Mutex::new(stores),
            guilds: Mutex::new(GuildRegistry::new()),
            brains: Mutex::new(BrainRegistry::new(fresh_brain, DEFAULT_EVICT_AFTER_SECS)),
            social: Mutex::new(SocialBrain::new()),
            rewards: Mutex::new(rewards),
            cooldown: Mutex::new(ReplyCooldown::new()),
            ask_cooldown: Mutex::new(ReplyCooldown::new()),
            budget: Mutex::new(Budget::default()),
            providers,
            recall: Mutex::new(recall),
            engine: Mutex::new(Engine::new()),
            quiet: std::env::var("ABBEY_QUIET").is_ok_and(|v| v.trim() == "1"),
            attachments: attachment_client(),
            data_dir,
            persistence_sink: Arc::new(FsPersistenceSink),
            self_ids: Mutex::new(Vec::new()),
            voice_inspect: Arc::new(crate::inspect::VoiceInspectRegistry::default()),
            episode_gate,
            checkpoints: Mutex::new(checkpoints),
            memory_queue: Mutex::new(Vec::new()),
        }))
    }

    /// An empty state with no backend, no vision, and no data directory — what
    /// the pipeline tests run against, and what `from_env` degrades to when
    /// nothing is configured.
    pub fn in_memory() -> Arc<Self> {
        Self::in_memory_with_persistence(None, Arc::new(FsPersistenceSink))
    }

    fn in_memory_with_persistence(
        data_dir: Option<PathBuf>,
        persistence_sink: Arc<dyn PersistenceSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            stores: Mutex::new(Stores::default()),
            guilds: Mutex::new(GuildRegistry::new()),
            brains: Mutex::new(BrainRegistry::new(fresh_brain, DEFAULT_EVICT_AFTER_SECS)),
            social: Mutex::new(SocialBrain::new()),
            rewards: Mutex::new(RewardCollector::new()),
            cooldown: Mutex::new(ReplyCooldown::new()),
            ask_cooldown: Mutex::new(ReplyCooldown::new()),
            budget: Mutex::new(Budget::default()),
            providers: ProviderRuntime::empty(),
            recall: Mutex::new(Recall::new()),
            engine: Mutex::new(Engine::new()),
            quiet: false,
            attachments: attachment_client(),
            data_dir,
            persistence_sink,
            self_ids: Mutex::new(Vec::new()),
            voice_inspect: Arc::new(crate::inspect::VoiceInspectRegistry::default()),
            episode_gate: None,
            checkpoints: Mutex::new(BTreeMap::new()),
            memory_queue: Mutex::new(Vec::new()),
        })
    }

    pub async fn chat(
        &self,
        system_prompt: &str,
        turns: &[crate::llm::ChatTurn],
    ) -> Result<(String, &'static str), crate::llm::LlmError> {
        self.providers.chat(system_prompt, turns).await
    }
    pub fn generation_label(&self) -> Option<&'static str> {
        self.providers.generation_label()
    }
    pub fn vision(&self) -> Option<&ProviderRuntime> {
        self.providers.vision_available().then_some(&self.providers)
    }
    fn provider_inspect(&self) -> Vec<crate::inspect::ProviderRouteInspect> {
        self.providers.inspect_snapshot()
    }

    /// Lock helper: a poisoned mutex means a panic elsewhere already took the
    /// process off the rails; recovering the guard keeps the bot answering
    /// rather than cascading every command into an error.
    pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The canonical coordinator for plain memory and semantic WDBX recall.
    /// The episode gate for a scope: `None` when no gate is configured or
    /// the config's `guilds` list does not cover this scoped guild, in which
    /// case every memory path behaves exactly as with no gate.
    pub fn gate_for(&self, scoped_guild: &str) -> Option<&Arc<crate::episode_gate::EpisodeGate>> {
        self.episode_gate
            .as_ref()
            .filter(|gate| gate.covers(scoped_guild))
    }

    pub fn memory_service(&self) -> MemoryService<'_> {
        MemoryService::new(&self.stores, &self.recall)
    }

    /// One coherent standing snapshot from the canonical SocialBrain/store
    /// authority, always taking the process-wide `stores -> social` order.
    pub fn reputation_snapshot(&self, scoped_guild: &str, scoped_user: &str) -> f64 {
        let stores = Self::lock(&self.stores);
        Self::lock(&self.social).reputation(scoped_user, scoped_guild, &*stores)
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

    /// Snapshot every brain, flush reputation, evict idle sessions, and report
    /// exactly what reached each durable authority. A failed persist does not
    /// take the gateway down, but it is never represented as success.
    ///
    /// Synchronous, so it proposes nothing: with the episode gate configured
    /// a brain row that the ledger has not admitted is replaced on disk by
    /// the last admitted one (`checkpoint_gate::restrict_to_admitted`). The
    /// scheduled task and `/admin flush` use [`Self::persist_all_gated`].
    pub fn persist_all(&self) -> PersistReport {
        self.persist_all_at(now())
    }

    fn persist_all_at(&self, t: u64) -> PersistReport {
        let mut snapshots = self.take_snapshot(t);
        if let Some(gate) = &self.episode_gate {
            let substituted = crate::checkpoint_gate::restrict_to_admitted(
                &mut snapshots.0,
                &Self::lock(&self.checkpoints),
                |guild| gate.covers(guild),
            );
            if !substituted.is_empty() {
                tracing::warn!(
                    guilds = substituted.len(),
                    "persist: brain checkpoints not yet admitted by the episode gate were not written; last admitted rows kept"
                );
            }
        }
        self.persist_snapshot(snapshots)
    }

    /// Propose every changed brain checkpoint to the episode gate as an
    /// `experience` memory candidate (one per guild, superseding the last
    /// admitted one), then persist with refused checkpoints substituted. With
    /// no gate configured this is exactly [`Self::persist_all`].
    pub async fn persist_all_gated(&self) -> PersistReport {
        let Some(gate) = self.episode_gate.clone() else {
            return self.persist_all();
        };
        crate::memory_gate::drain(self).await;
        let t = now();
        let mut snapshots = self.take_snapshot(t);
        let proposals = crate::checkpoint_gate::plan(
            &snapshots.0.brains,
            &Self::lock(&self.checkpoints),
            |guild| gate.covers(guild),
        );
        let mut outcomes = Vec::with_capacity(proposals.len());
        for proposal in proposals {
            let request = crate::episode_gate::MemoryCandidateRequest {
                scoped_guild: proposal.guild.clone(),
                class: crate::episode_gate::MemoryClass::Experience,
                retention: crate::episode_gate::RetentionClass::Operational,
                payload: proposal.payload.clone(),
                member_scoped: false,
                supersedes: proposal.supersedes,
                forgets: None,
                now: t,
                nonce: gate.next_nonce(),
            };
            let outcome = gate.record_memory_candidate(request).await;
            outcomes.push((proposal, outcome));
        }
        let settlement = crate::checkpoint_gate::settle(
            &mut snapshots.0,
            &mut Self::lock(&self.checkpoints),
            outcomes,
        );
        if !settlement.refused.is_empty() {
            tracing::warn!(
                refused = settlement.refused.len(),
                admitted = settlement.admitted.len(),
                "persist: brain checkpoints not admitted by the episode gate; last admitted rows persisted instead"
            );
        }
        self.persist_snapshot(snapshots)
    }

    fn take_snapshot(&self, t: u64) -> (Stores, Recall) {
        Self::lock(&self.engine).evict_idle(t, SESSION_IDLE_SECS);
        self.memory_service().consistent_snapshot_after(|stores| {
            Self::lock(&self.brains).persist_all(stores, t);
            Self::lock(&self.social).flush(stores);
            stores.pending_rewards = Self::lock(&self.rewards).export_pending();
        })
    }

    fn persist_snapshot(&self, snapshots: (Stores, Recall)) -> PersistReport {
        let Some(dir) = &self.data_dir else {
            return PersistReport::memory_only();
        };
        if let Err(category) = persist_canonical(&*self.persistence_sink, dir, &snapshots.0) {
            return PersistReport::from_components(
                PersistComponentOutcome::Failed(category),
                PersistComponentOutcome::SkippedCanonicalFailure,
            );
        }
        let projection = persist_projection(&*self.persistence_sink, dir, &snapshots.1)
            .map_or_else(PersistComponentOutcome::Failed, |()| {
                PersistComponentOutcome::Committed
            });
        PersistReport::from_components(PersistComponentOutcome::Committed, projection)
    }

    /// Rolling channel summaries — the spec's "rolling 2k-token summary
    /// compressed via ABI". For every channel whose count is
    /// [`crate::memory::SUMMARY_EVERY_MESSAGES`] past its last summary, and
    /// whose guild has opted in (`/admin act on`) or is a DM, ask the backend
    /// for a summary of the recent lines and store it as the channel's
    /// context. One generation at a time, through the usual slot, so it never
    /// starves a live reply. Returns how many channels were summarised.
    pub async fn refresh_summaries(&self) -> usize {
        if !self.providers.generation_available() {
            return 0;
        }
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
                    tracing::warn!(channel = %scoped_channel, error = %e, "rolling summary failed");
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
        let persistence_state = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PERSIST_EVERY);
            tick.tick().await;
            loop {
                tick.tick().await;
                let report = persistence_state.persist_all_gated().await;
                crate::persist::log_report("scheduled", &report);
            }
        });
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
    use crate::brain::social::ReputationStore;
    use crate::guild::GuildSettings;
    use crate::persist::{PersistComponentOutcome, PersistErrorCategory, PersistOverall};

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

    #[test]
    fn process_persistence_reports_memory_only_without_calling_a_sink() {
        let sink = crate::persist::tests::RuntimeRecordingSink::success();
        let state = AppState::in_memory_with_persistence(None, Arc::new(sink.clone()));
        let report = state.persist_all_at(42);
        assert_eq!(report.overall, PersistOverall::MemoryOnly);
        assert_eq!(
            report.canonical_state,
            PersistComponentOutcome::NotConfigured
        );
        assert_eq!(
            report.wdbx_projection,
            PersistComponentOutcome::NotConfigured
        );
        assert!(sink.attempts().is_empty());
    }

    #[test]
    fn canonical_failure_skips_the_wdbx_projection() {
        let sink = crate::persist::tests::RuntimeRecordingSink::fail_canonical(
            PersistErrorCategory::SyncTemporary,
        );
        let state = AppState::in_memory_with_persistence(
            Some(PathBuf::from("/injected/state")),
            Arc::new(sink.clone()),
        );
        let report = state.persist_all_at(42);
        assert_eq!(report.overall, PersistOverall::Failed);
        assert_eq!(
            report.canonical_state,
            PersistComponentOutcome::Failed(PersistErrorCategory::SyncTemporary)
        );
        assert_eq!(
            report.wdbx_projection,
            PersistComponentOutcome::SkippedCanonicalFailure
        );
        assert_eq!(sink.attempts(), ["canonical"]);
    }

    #[test]
    fn both_durable_components_committing_is_complete() {
        let sink = crate::persist::tests::RuntimeRecordingSink::success();
        let state = AppState::in_memory_with_persistence(
            Some(PathBuf::from("/injected/state")),
            Arc::new(sink.clone()),
        );
        let report = state.persist_all_at(42);
        assert_eq!(report.overall, PersistOverall::Complete);
        assert_eq!(report.canonical_state, PersistComponentOutcome::Committed);
        assert_eq!(report.wdbx_projection, PersistComponentOutcome::Committed);
        assert_eq!(sink.attempts(), ["canonical", "wdbx"]);
    }

    #[test]
    fn projection_failure_is_partial_after_a_canonical_commit() {
        let sink = crate::persist::tests::RuntimeRecordingSink::fail_projection(
            PersistErrorCategory::SyncDirectory,
        );
        let state = AppState::in_memory_with_persistence(
            Some(PathBuf::from("/injected/state")),
            Arc::new(sink.clone()),
        );
        let report = state.persist_all_at(42);
        assert_eq!(report.overall, PersistOverall::Partial);
        assert_eq!(report.canonical_state, PersistComponentOutcome::Committed);
        assert_eq!(
            report.wdbx_projection,
            PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory)
        );
        assert_eq!(sink.attempts(), ["canonical", "wdbx"]);
    }

    #[test]
    fn queue_and_concurrency_parse_with_fallbacks() {
        assert_eq!(queue_secs_from_value(None), DEFAULT_QUEUE_SECS);
        assert_eq!(queue_secs_from_value(Some("30".into())), 30);
        assert_eq!(queue_secs_from_value(Some("0".into())), DEFAULT_QUEUE_SECS);
    }

    #[test]
    fn voice_queue_never_shorter_than_text_and_has_a_floor() {
        assert_eq!(voice_queue_secs(1), DEFAULT_VOICE_QUEUE_SECS);
        assert_eq!(
            voice_queue_secs(DEFAULT_QUEUE_SECS),
            DEFAULT_VOICE_QUEUE_SECS
        );
        assert_eq!(
            voice_queue_secs(DEFAULT_VOICE_QUEUE_SECS),
            DEFAULT_VOICE_QUEUE_SECS
        );
        assert_eq!(voice_queue_secs(240), 240);
    }

    #[test]
    fn vision_transport_routes_every_loopback_shape_to_the_no_proxy_client() {
        let transport = HttpVisionTransport::default();
        for endpoint in [
            "http://127.0.0.1:11434/v1/chat/completions",
            "http://localhost:8080/v1/chat/completions",
            "http://[::1]:8181/v1/chat/completions",
        ] {
            assert!(std::ptr::eq(
                transport.client_for(endpoint),
                &transport.loopback_client
            ));
        }
        assert!(std::ptr::eq(
            transport.client_for("https://vision.example.com/v1/chat/completions"),
            &transport.remote_client
        ));
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
    fn voice_inspect_is_exact_guild_only_and_dm_safe() {
        let state = AppState::in_memory();
        state
            .voice_inspect
            .publish("discord:g", crate::inspect::VoiceInspectState::Active);
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };

        assert_eq!(
            crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Voice),
            "voice: active"
        );
        scope.scoped_guild = "discord:other".into();
        assert_eq!(
            crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Voice),
            "voice: off"
        );
        scope.scoped_guild = "discord:dm:u".into();
        assert_eq!(
            crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Voice),
            "voice: off"
        );
    }

    #[test]
    fn configured_but_ineligible_fm_routes_publish_no_capabilities() {
        let mut state = AppState::in_memory();
        Arc::get_mut(&mut state)
            .expect("unique state")
            .providers
            .set_fm(Some(FoundationModels::new(
                crate::provider::FmConfig {
                    mode: crate::provider::FmMode::System,
                    endpoint: Some("http://127.0.0.1:8899".into()),
                    cli: PathBuf::from("/usr/bin/fm"),
                    fallback: false,
                    timeout_secs: 30,
                },
                None,
                true,
            )));
        let rendered = crate::inspect::render_provider(&state.provider_inspect());

        assert!(
            rendered.contains("foundation-models-server: routable no"),
            "{rendered}"
        );
        assert!(
            rendered.contains("foundation-models-cli: routable no"),
            "{rendered}"
        );
        assert_eq!(
            rendered
                .matches("text no · tools no · vision no · ocr no")
                .count(),
            2,
            "{rendered}"
        );
        assert!(!rendered.contains("127.0.0.1"), "{rendered}");
        assert!(!rendered.contains("/usr/bin"), "{rendered}");
    }

    #[test]
    fn unknown_guild_inspect_is_non_provisioning() {
        let state = AppState::in_memory();
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:missing".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };

        let rendered =
            crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Guild);

        assert_eq!(rendered, "No guild settings on record.");
        assert!(AppState::lock(&state.stores).guilds.is_empty());
        assert!(!AppState::lock(&state.guilds).is_cached("discord:missing"));
    }

    #[test]
    fn durable_guild_inspect_does_not_fill_the_cache() {
        let state = AppState::in_memory();
        AppState::lock(&state.stores).guilds.insert(
            "discord:g".into(),
            GuildSettings {
                default_persona: crate::persona::Persona::Abi,
                ..GuildSettings::default()
            },
        );
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };

        let rendered =
            crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Guild);

        assert!(rendered.contains("persona: abi"), "{rendered}");
        assert!(!AppState::lock(&state.guilds).is_cached("discord:g"));
    }

    #[test]
    fn cached_guild_inspect_uses_the_recorded_settings_and_injected_time() {
        let state = AppState::in_memory();
        {
            let mut stores = AppState::lock(&state.stores);
            let mut guilds = AppState::lock(&state.guilds);
            guilds.update("discord:g", &mut *stores, |settings| {
                settings.default_persona = crate::persona::Persona::Aviva;
                settings.unsolicited_per_hour = 6;
            });
        }
        {
            let mut budget = AppState::lock(&state.budget);
            for _ in 0..6 {
                assert!(budget.try_take("discord:g", 6, 10_000));
            }
        }
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10_600,
            persona: crate::persona::Persona::Abbey,
        };

        let rendered =
            crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Guild);

        assert!(rendered.contains("persona: aviva"), "{rendered}");
        assert!(rendered.contains("(1.0 left)"), "{rendered}");
        assert!(AppState::lock(&state.guilds).is_cached("discord:g"));
    }

    #[test]
    fn all_tool_memory_writes_use_the_scope_timestamp() {
        let state = AppState::in_memory();
        state
            .memory_service()
            .remember("discord:g", "discord:u", "uses rust", 1)
            .expect("seed");
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 4_242,
            persona: crate::persona::Persona::Abbey,
        };

        crate::tools::ToolHost::remember_fact(&mut scope, "moved to zig", Some("uses rust"));
        crate::tools::ToolHost::remember_fact(&mut scope, "likes compilers", None);

        let stores = AppState::lock(&state.stores);
        let memory = stores
            .memory
            .user("discord:g", "discord:u")
            .expect("subject memory");
        assert_eq!(memory.updated_at, 4_242);
        assert_eq!(memory.pending_supersessions.len(), 1);
        assert_eq!(memory.pending_supersessions[0].at, 4_242);
    }

    #[test]
    fn list_facts_isolated_to_the_exact_canonical_subject() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service
            .remember("discord:g", "discord:u", "own fact", 1)
            .expect("own fact");
        service
            .remember_proposing("discord:g", "discord:u", "own replacement", "own fact", 2)
            .expect("own pending");
        service
            .remember("discord:g", "discord:other", "other user fact", 3)
            .expect("other user");
        service
            .remember("discord:other", "discord:u", "other guild fact", 4)
            .expect("other guild");
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };

        let rendered = crate::tools::ToolHost::list_facts(&mut scope);

        assert!(rendered.contains("own fact"), "{rendered}");
        assert!(rendered.contains("own replacement"), "{rendered}");
        assert!(!rendered.contains("other user fact"), "{rendered}");
        assert!(!rendered.contains("other guild fact"), "{rendered}");
    }

    /// The safety property at its real integration point: a model calling
    /// `remember_fact` with `supersedes` must PROPOSE, never delete. Verified
    /// here through the actual `ToolHost` impl rather than by reading the
    /// routing — `remember_proposing` being correct in isolation would not
    /// prove `ToolScope` routes to it instead of `remember_replacing`.
    #[test]
    fn a_model_supersedes_argument_proposes_and_never_deletes() {
        let state = AppState::in_memory();
        state
            .memory_service()
            .remember("discord:g", "discord:u", "uses rust", 1)
            .expect("seed");
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };

        let reply =
            crate::tools::ToolHost::remember_fact(&mut scope, "moved to zig", Some("uses rust"));
        assert!(reply.contains("Proposed to replace"), "{reply}");

        // BOTH facts must survive. The model does not get to delete.
        let facts = state.memory_service().facts("discord:g", "discord:u");
        assert!(facts.contains(&"uses rust".to_string()), "{facts:?}");
        assert!(facts.contains(&"moved to zig".to_string()), "{facts:?}");
        assert_eq!(
            state
                .memory_service()
                .pending_supersessions("discord:g", "discord:u")
                .len(),
            1
        );
    }

    #[test]
    fn tool_memory_uses_the_shared_fact_validator() {
        let state = AppState::in_memory();
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:g".into(),
            scoped_user: "discord:u".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };
        assert_eq!(
            crate::tools::ToolHost::remember_fact(&mut scope, "  Donald\nlikes\tRust.  ", None),
            "Stored: Donald likes Rust."
        );
        assert_eq!(
            state.memory_service().facts("discord:g", "discord:u"),
            ["Donald likes Rust."]
        );
        assert_eq!(
            crate::tools::ToolHost::remember_fact(
                &mut scope,
                &"🦀".repeat(crate::memory::MAX_FACT_CHARS + 1),
                None
            ),
            "Keep one remembered fact to 300 characters or fewer."
        );
        assert_eq!(
            state.memory_service().facts("discord:g", "discord:u").len(),
            1
        );
    }

    #[test]
    fn explicit_reputation_ids_are_scoped_to_the_conversation_network() {
        for (network, expected) in [
            (SocialNetwork::Discord, 0.61),
            (SocialNetwork::Telegram, 0.72),
            (SocialNetwork::Slack, 0.83),
        ] {
            let state = AppState::in_memory();
            let guild = format!("{}:g", network.as_str());
            let user = format!("{}:42", network.as_str());
            AppState::lock(&state.stores).store_reputation(&guild, &user, expected, 1);
            let mut scope = ToolScope {
                state: &state,
                network,
                scoped_guild: guild,
                scoped_user: format!("{}:self", network.as_str()),
                scoped_channel: format!("{}:c", network.as_str()),
                now: 10,
                persona: crate::persona::Persona::Abbey,
            };
            let native_id = if network == SocialNetwork::Discord {
                "<@42>"
            } else {
                "42"
            };
            assert_eq!(
                crate::tools::ToolHost::lookup_reputation(&mut scope, Some(native_id)),
                format!("Reputation {expected:.2} (0 = poor, 1 = excellent).")
            );
        }
    }

    #[test]
    fn conflicting_scoped_reputation_id_cannot_escape_the_current_network() {
        let state = AppState::in_memory();
        AppState::lock(&state.stores).store_reputation("telegram:g", "discord:42", 0.99, 1);
        let mut scope = ToolScope {
            state: &state,
            network: SocialNetwork::Telegram,
            scoped_guild: "telegram:g".into(),
            scoped_user: "telegram:self".into(),
            scoped_channel: "telegram:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };
        assert_eq!(
            crate::tools::ToolHost::lookup_reputation(&mut scope, Some("discord:42")),
            "Reputation 0.50 (0 = poor, 1 = excellent)."
        );
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
