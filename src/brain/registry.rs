//! One policy per guild (`docs/spec/adaptivelearning.md`, "BrainRegistry" /
//! "BrainState").
//!
//! Generic over the brain so this file compiles with no knowledge of the DQN:
//! the orchestrator implements [`Brain`] for `DqnAgent` and hands a
//! constructor closure in. Persistence goes through [`BrainStore`] (the
//! `brain_states` table: one JSON snapshot + experience count per scoped
//! guild). Time is injected as unix seconds.

use std::collections::HashMap;

/// Idle brains unload after this long; their snapshot persists.
pub const DEFAULT_EVICT_AFTER_SECS: u64 = 6 * 3600;

/// What the registry needs from a per-guild policy.
///
/// `Experience` is an associated type so this module never names the replay
/// buffer's struct; the orchestrator binds it to `crate::brain::replay::Experience`.
pub trait Brain {
    type Experience;

    fn remember(&mut self, exp: Self::Experience);
    fn learn(&mut self);
    /// JSON-encoded `BrainSnapshot`.
    fn export_json(&self) -> String;
    /// Returns `false` when `json` does not decode; the brain is left as it was.
    fn import_json(&mut self, json: &str) -> bool;
}

/// The `brain_states` row store, keyed by scoped guild id.
pub trait BrainStore {
    /// `(snapshot_json, experience_count)` if a row exists.
    fn load(&self, scoped_guild_id: &str) -> Option<(String, u64)>;
    fn save(&mut self, scoped_guild_id: &str, snapshot_json: &str, experience_count: u64);
}

/// HashMap-backed store: the test double and the default backend until a
/// durable one is wired in.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct InMemoryBrainStore {
    pub rows: HashMap<String, (String, u64)>,
}

#[cfg(test)]
impl InMemoryBrainStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl BrainStore for InMemoryBrainStore {
    fn load(&self, scoped_guild_id: &str) -> Option<(String, u64)> {
        self.rows.get(scoped_guild_id).cloned()
    }

    fn save(&mut self, scoped_guild_id: &str, snapshot_json: &str, experience_count: u64) {
        self.rows.insert(
            scoped_guild_id.to_owned(),
            (snapshot_json.to_owned(), experience_count),
        );
    }
}

struct Loaded<B> {
    brain: B,
    experience_count: u64,
    last_touched: u64,
}

/// Lazily-loaded, per-guild brains with snapshot persistence and idle eviction.
pub struct BrainRegistry<B: Brain> {
    make: Box<dyn Fn() -> B + Send + Sync>,
    brains: HashMap<String, Loaded<B>>,
    evict_after_secs: u64,
}

impl<B: Brain> BrainRegistry<B> {
    /// `make` builds a fresh (random-weight) brain for a never-seen guild.
    pub fn new(make: impl Fn() -> B + Send + Sync + 'static, evict_after_secs: u64) -> Self {
        Self {
            make: Box::new(make),
            brains: HashMap::new(),
            evict_after_secs,
        }
    }

    /// The guild's brain, created on first access (restored from `store` if a
    /// snapshot exists). Every call refreshes the idle clock.
    pub fn brain(&mut self, scoped_guild_id: &str, store: &dyn BrainStore, now: u64) -> &mut B {
        let entry = self
            .brains
            .entry(scoped_guild_id.to_owned())
            .or_insert_with(|| {
                let mut brain = (self.make)();
                let mut experience_count = 0;
                if let Some((json, count)) = store.load(scoped_guild_id)
                    && brain.import_json(&json)
                {
                    experience_count = count;
                }
                Loaded {
                    brain,
                    experience_count,
                    last_touched: now,
                }
            });
        entry.last_touched = now;
        &mut entry.brain
    }

    /// Hand an experience to a *loaded* brain. Per spec, an unloaded guild
    /// drops it: the act path always calls [`Self::brain`] first, so a miss
    /// here means the guild was evicted mid-flight and the sample is stale.
    pub fn remember(&mut self, scoped_guild_id: &str, exp: B::Experience) {
        if let Some(loaded) = self.brains.get_mut(scoped_guild_id) {
            loaded.brain.remember(exp);
            loaded.experience_count += 1;
        }
    }

    /// One learning step on every loaded brain whose guild has learning on.
    pub fn learn_all(&mut self, is_learning_enabled: impl Fn(&str) -> bool) {
        for (guild, loaded) in &mut self.brains {
            if is_learning_enabled(guild) {
                loaded.brain.learn();
            }
        }
    }

    /// Snapshot every loaded brain, then unload those idle longer than the
    /// eviction window.
    pub fn persist_all(&mut self, store: &mut dyn BrainStore, now: u64) {
        for (guild, loaded) in &self.brains {
            store.save(guild, &loaded.brain.export_json(), loaded.experience_count);
        }
        let cutoff = now.saturating_sub(self.evict_after_secs);
        self.brains
            .retain(|_, loaded| loaded.last_touched >= cutoff);
    }

    /// Snapshot one guild's brain (if loaded) and unload it.
    pub fn persist_and_evict(&mut self, scoped_guild_id: &str, store: &mut dyn BrainStore) {
        if let Some(loaded) = self.brains.remove(scoped_guild_id) {
            store.save(
                scoped_guild_id,
                &loaded.brain.export_json(),
                loaded.experience_count,
            );
        }
    }

    /// Experiences seen by this guild's brain (restored count + live adds);
    /// `None` when the guild is not loaded.
    pub fn experience_count(&self, scoped_guild_id: &str) -> Option<u64> {
        self.brains
            .get(scoped_guild_id)
            .map(|loaded| loaded.experience_count)
    }

    /// Scoped ids of every loaded brain, sorted for stable output.
    pub fn loaded_guilds(&self) -> Vec<String> {
        let mut guilds: Vec<String> = self.brains.keys().cloned().collect();
        guilds.sort();
        guilds
    }

    /// Read-only view for `/admin` observability; does not touch the idle clock.
    pub fn get(&self, scoped_guild_id: &str) -> Option<&B> {
        self.brains.get(scoped_guild_id).map(|loaded| &loaded.brain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A brain whose "weights" are one integer, so restore/export are checkable.
    #[derive(Debug, Default, PartialEq)]
    struct Counter {
        weight: i64,
        remembered: Vec<u8>,
        learn_calls: u32,
    }

    impl Brain for Counter {
        type Experience = u8;

        fn remember(&mut self, exp: u8) {
            self.remembered.push(exp);
        }
        fn learn(&mut self) {
            self.learn_calls += 1;
        }
        fn export_json(&self) -> String {
            self.weight.to_string()
        }
        fn import_json(&mut self, json: &str) -> bool {
            match json.parse() {
                Ok(w) => {
                    self.weight = w;
                    true
                }
                Err(_) => false,
            }
        }
    }

    const A: &str = "discord:1";
    const B_: &str = "discord:2";

    fn registry() -> BrainRegistry<Counter> {
        BrainRegistry::new(Counter::default, DEFAULT_EVICT_AFTER_SECS)
    }

    #[test]
    fn brain_is_created_lazily_and_reused() {
        let store = InMemoryBrainStore::new();
        let mut reg = registry();
        assert!(reg.loaded_guilds().is_empty());
        reg.brain(A, &store, 0).weight = 42;
        assert_eq!(reg.brain(A, &store, 1).weight, 42);
        assert_eq!(reg.loaded_guilds(), vec![A.to_owned()]);
        assert_eq!(reg.experience_count(A), Some(0));
    }

    #[test]
    fn first_access_restores_from_store() {
        let mut store = InMemoryBrainStore::new();
        store.save(A, "7", 123);
        let mut reg = registry();
        assert_eq!(reg.brain(A, &store, 0).weight, 7);
        assert_eq!(reg.experience_count(A), Some(123));
    }

    #[test]
    fn undecodable_snapshot_yields_fresh_brain_and_zero_count() {
        let mut store = InMemoryBrainStore::new();
        store.save(A, "not a number", 123);
        let mut reg = registry();
        assert_eq!(reg.brain(A, &store, 0).weight, 0);
        assert_eq!(reg.experience_count(A), Some(0));
    }

    #[test]
    fn remember_is_ignored_for_unloaded_guild() {
        let store = InMemoryBrainStore::new();
        let mut reg = registry();
        reg.remember(A, 9);
        assert!(reg.get(A).is_none());
        assert_eq!(reg.experience_count(A), None);

        reg.brain(A, &store, 0);
        reg.remember(A, 9);
        reg.remember(A, 3);
        assert_eq!(reg.get(A).unwrap().remembered, vec![9, 3]);
        assert_eq!(reg.experience_count(A), Some(2));
    }

    #[test]
    fn learn_all_honours_predicate() {
        let store = InMemoryBrainStore::new();
        let mut reg = registry();
        reg.brain(A, &store, 0);
        reg.brain(B_, &store, 0);
        reg.learn_all(|g| g == A);
        assert_eq!(reg.get(A).unwrap().learn_calls, 1);
        assert_eq!(reg.get(B_).unwrap().learn_calls, 0);
    }

    #[test]
    fn persist_all_saves_every_brain_and_evicts_only_idle() {
        let mut store = InMemoryBrainStore::new();
        let mut reg = registry();
        reg.brain(A, &store, 0).weight = 1;
        reg.remember(A, 1);
        reg.brain(B_, &store, 100).weight = 2;

        // Just under the window for A: nothing evicted, both saved.
        let now = DEFAULT_EVICT_AFTER_SECS - 1;
        reg.persist_all(&mut store, now);
        assert_eq!(store.load(A), Some(("1".to_owned(), 1)));
        assert_eq!(store.load(B_), Some(("2".to_owned(), 0)));
        assert_eq!(reg.loaded_guilds(), vec![A.to_owned(), B_.to_owned()]);

        // Exactly at the boundary A is still fresh (touched >= cutoff).
        reg.persist_all(&mut store, DEFAULT_EVICT_AFTER_SECS);
        assert_eq!(reg.loaded_guilds(), vec![A.to_owned(), B_.to_owned()]);

        // One second past: A goes, B (touched at 100) stays.
        reg.persist_all(&mut store, DEFAULT_EVICT_AFTER_SECS + 1);
        assert_eq!(reg.loaded_guilds(), vec![B_.to_owned()]);

        // A re-hydrates from its snapshot.
        assert_eq!(reg.brain(A, &store, DEFAULT_EVICT_AFTER_SECS + 2).weight, 1);
        assert_eq!(reg.experience_count(A), Some(1));
    }

    #[test]
    fn persist_and_evict_removes_and_saves() {
        let mut store = InMemoryBrainStore::new();
        let mut reg = registry();
        reg.brain(A, &store, 0).weight = 5;
        reg.persist_and_evict(A, &mut store);
        assert!(reg.get(A).is_none());
        assert_eq!(store.load(A), Some(("5".to_owned(), 0)));

        // Evicting an unloaded guild is a no-op, not a panic, and writes nothing.
        reg.persist_and_evict(B_, &mut store);
        assert_eq!(store.load(B_), None);
    }

    #[test]
    fn get_does_not_refresh_idle_clock() {
        let mut store = InMemoryBrainStore::new();
        let mut reg = registry();
        reg.brain(A, &store, 0);
        assert!(reg.get(A).is_some());
        reg.persist_all(&mut store, DEFAULT_EVICT_AFTER_SECS + 1);
        assert!(reg.get(A).is_none());
    }
}
