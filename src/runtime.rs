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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
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
use crate::persist::{FsPersistenceSink, PersistReport, PersistenceSink, Stores};
use crate::platform::SocialNetwork;
#[cfg(test)]
use crate::provider::FoundationModels;
use crate::provider::ProviderRuntime;
use crate::vision::{VisionError, VisionRequest, VisionTransport};
use crate::wdbx::Recall;

mod memory_service;
mod provider_setup;
mod scheduler;
mod tool_scope;
mod vision_transport;
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

/// Everything the shells share. Construct once in `main`, clone the `Arc`.
///
/// **Lock order.** When more than one of these mutexes is held at once, take
/// them in field order: `stores` → `guilds` → `brains` → `social` → `rewards`
/// → `cooldown` → `ask_cooldown` → `budget` → `recall` → `engine`. The 5-minute persist tick
/// holds `stores` then `brains`; a message handler that took `brains` first
/// would deadlock against it (reported on PR #10, fixed after #16). `engine`
/// and `recall` are only ever taken alone or last.
pub struct AppState {
    operational_events: OnceLock<crate::service::telemetry::TelemetryRequests>,
    managed_status: OnceLock<Arc<crate::service::status::ManagedStatus>>,
    service: OnceLock<crate::service::OperationRegistry>,
    self_weak: OnceLock<Weak<Self>>,
    persistence_requests: OnceLock<crate::service::persistence::PersistenceRequests>,
    persistence_preparation: tokio::sync::Mutex<()>,
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
    pub memory_turn: Option<&'a crate::memory_gate::MemoryTurn>,
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
            operational_events: OnceLock::new(),
            managed_status: OnceLock::new(),
            service: OnceLock::new(),
            self_weak: OnceLock::new(),
            persistence_requests: OnceLock::new(),
            persistence_preparation: tokio::sync::Mutex::new(()),
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
            operational_events: OnceLock::new(),
            managed_status: OnceLock::new(),
            service: OnceLock::new(),
            self_weak: OnceLock::new(),
            persistence_requests: OnceLock::new(),
            persistence_preparation: tokio::sync::Mutex::new(()),
        })
    }

    pub fn attach_observability(
        &self,
        events: crate::service::telemetry::TelemetryRequests,
        status: Arc<crate::service::status::ManagedStatus>,
    ) {
        self.providers.attach_observability(events.clone());
        assert!(
            self.operational_events.set(events).is_ok(),
            "operational output attached once"
        );
        assert!(
            self.managed_status.set(status).is_ok(),
            "readiness attached once"
        );
    }
    pub fn operational_events(&self) -> Option<&crate::service::telemetry::TelemetryRequests> {
        self.operational_events.get()
    }
    pub fn managed_status(&self) -> Option<&Arc<crate::service::status::ManagedStatus>> {
        self.managed_status.get()
    }

    pub fn attach_service(
        self: &Arc<Self>,
        registry: crate::service::OperationRegistry,
    ) -> crate::service::persistence::PersistenceWriter {
        let writer = crate::service::persistence::PersistenceWriter::start(
            self.data_dir.clone(),
            self.persistence_sink.clone(),
        );
        if let Some(gate) = &self.episode_gate {
            gate.attach_service(registry.clone());
        }
        self.providers.attach_service(registry.clone());
        assert!(self.service.set(registry).is_ok(), "service attached once");
        assert!(
            self.self_weak.set(Arc::downgrade(self)).is_ok(),
            "state owner attached once"
        );
        assert!(
            self.persistence_requests.set(writer.requests()).is_ok(),
            "writer attached once"
        );
        writer
    }

    pub fn service_registry(&self) -> Option<&crate::service::OperationRegistry> {
        self.service.get()
    }
    pub fn owned_state(&self) -> Option<Arc<Self>> {
        self.self_weak.get().and_then(Weak::upgrade)
    }

    pub fn spawn_episode(&self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        if let Some(registry) = self.service.get() {
            let _ = registry.spawn_result(crate::service::OperationKind::Episode, work);
        }
    }

    pub fn final_snapshot(&self) -> crate::service::persistence::Snapshot {
        let (mut stores, recall) = self.take_snapshot(now());
        if let Some(gate) = &self.episode_gate {
            crate::checkpoint_gate::restrict_to_admitted(
                &mut stores,
                &Self::lock(&self.checkpoints),
                |guild| gate.covers(guild),
            );
        }
        crate::service::persistence::Snapshot { stores, recall }
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
        self.vision_for(false)
    }
    pub fn vision_for(&self, ocr: bool) -> Option<&ProviderRuntime> {
        self.providers
            .request_readiness(crate::provider::RequestClass::image(ocr))
            .is_ok()
            .then_some(&self.providers)
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
    pub async fn request_persistence(
        &self,
    ) -> Result<PersistReport, crate::service::persistence::RequestError> {
        use crate::service::persistence::RequestError;
        let Some(registry) = self.service.get() else {
            return Ok(self.persist_all_gated().await);
        };
        let state = self
            .self_weak
            .get()
            .and_then(Weak::upgrade)
            .ok_or(RequestError::WriterUnavailable)?;
        let result = registry
            .spawn_result(
                crate::service::OperationKind::PersistencePreparation,
                async move {
                    let _serial = state.persistence_preparation.lock().await;
                    let (stores, recall) = state.prepare_gated_snapshot().await;
                    state
                        .persistence_requests
                        .get()
                        .ok_or(RequestError::WriterUnavailable)?
                        .submit(crate::service::persistence::Snapshot { stores, recall })
                        .await
                        .inspect(|report| {
                            if let Some(status) = state.managed_status() {
                                status.persisted(*report);
                            }
                            if let Some(events) = state.operational_events() {
                                let _ = events.record(
                                    crate::observability::EventComponent::Persistence,
                                    crate::observability::EventCode::PersistenceAttempt,
                                    if report.overall == crate::persist::PersistOverall::Complete {
                                        crate::observability::EventOutcome::Succeeded
                                    } else {
                                        crate::observability::EventOutcome::Degraded
                                    },
                                    None,
                                );
                            }
                        })
                },
            )
            .map_err(|_| RequestError::Draining)?;
        result.await.map_err(|_| RequestError::WriterUnavailable)?
    }

    pub async fn persist_all_gated(&self) -> PersistReport {
        let snapshots = self.prepare_gated_snapshot().await;
        self.persist_snapshot(snapshots)
    }

    async fn prepare_gated_snapshot(&self) -> (Stores, Recall) {
        let Some(gate) = self.episode_gate.clone() else {
            return self.take_snapshot(now());
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
        snapshots
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
        crate::service::persistence::write_snapshot(
            self.data_dir.as_deref(),
            &*self.persistence_sink,
            crate::service::persistence::Snapshot {
                stores: snapshots.0,
                recall: snapshots.1,
            },
        )
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
mod tests;
