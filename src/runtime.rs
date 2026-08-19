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
pub struct AppState {
    pub stores: Mutex<Stores>,
    pub guilds: Mutex<GuildRegistry>,
    pub brains: Mutex<BrainRegistry<DqnAgent>>,
    pub social: Mutex<SocialBrain>,
    pub rewards: Mutex<RewardCollector>,
    pub cooldown: Mutex<ReplyCooldown>,
    /// Per-guild hourly budget for unsolicited actions.
    pub budget: Mutex<Budget>,
    pub recall: Mutex<Recall>,
    pub engine: Mutex<Engine>,
    pub backend: Option<Backend>,
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
        Ok(Arc::new(Self {
            stores: Mutex::new(stores),
            guilds: Mutex::new(GuildRegistry::new()),
            brains: Mutex::new(BrainRegistry::new(fresh_brain, DEFAULT_EVICT_AFTER_SECS)),
            social: Mutex::new(SocialBrain::new()),
            rewards: Mutex::new(RewardCollector::new()),
            cooldown: Mutex::new(ReplyCooldown::new()),
            budget: Mutex::new(Budget::default()),
            recall: Mutex::new(recall),
            engine: Mutex::new(Engine::new()),
            backend: Backend::from_env(),
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
            recall: Mutex::new(Recall::new()),
            engine: Mutex::new(Engine::new()),
            backend: None,
            quiet: false,
            llm: HttpTransport::default(),
            vision: None,
            data_dir: None,
            self_ids: Mutex::new(Vec::new()),
        })
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
            let mut guilds = Self::lock(&self.guilds);
            let mut stores = Self::lock(&self.stores);
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
    }
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
            let mut brains = AppState::lock(&state.brains);
            let stores = AppState::lock(&state.stores);
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
