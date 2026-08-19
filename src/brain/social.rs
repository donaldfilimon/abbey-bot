//! Reputation engine (`docs/spec/brain.md`, "SocialBrain.swift").
//!
//! A per-guild, per-user standing score in `0.0..=1.0`, kept in memory and
//! written through to a [`ReputationStore`]. The Swift original kept scores
//! purely in memory and lost them on every restart; this port hydrates on
//! read (miss → store → default `0.5`) and flushes dirty scores back, so a
//! restart no longer resets a guild's reputation while the audit trail keeps
//! growing.
//!
//! Pure: no serenity, no poise, no clock. Time is injected as unix seconds.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Standing a never-seen member starts with.
pub const DEFAULT_REPUTATION: f64 = 0.5;

/// EMA retention for [`SocialBrain::record_interaction`] — slow decay, fast reward.
const EMA_RETAIN: f64 = 0.95;
/// EMA weight of the newest interaction's quality.
const EMA_GAIN: f64 = 0.05;
/// Flat deduction applied by [`SocialBrain::penalize`].
const PENALTY: f64 = 0.1;

/// One append-only audit row: how a member's standing moved and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReputationEvent {
    pub user_id: String,
    pub guild_id: String,
    pub delta: f64,
    pub reason: String,
    /// Unix seconds, injected by the caller.
    pub at: u64,
}

/// Where the queryable current value and the audit trail live.
///
/// `store_reputation` is the write-back of one dirty score (the Swift
/// `UserMemory.reputation` upsert, with `interactionCount += delta`);
/// `append_event` is the append-only `ReputationEvent` table.
pub trait ReputationStore {
    fn load_reputation(&self, guild: &str, user: &str) -> Option<f64>;
    fn store_reputation(
        &mut self,
        guild: &str,
        user: &str,
        value: f64,
        interaction_count_delta: u32,
    );
    fn append_event(&mut self, event: ReputationEvent);
}

/// Cache key. Scoped guild ids contain a colon (`discord:123`), so the
/// Swift `"guild:user"` string key — split on the first `:` at flush time —
/// would mis-split here. A tuple key cannot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    guild: String,
    user: String,
}

impl Key {
    fn new(guild: &str, user: &str) -> Self {
        Self {
            guild: guild.to_owned(),
            user: user.to_owned(),
        }
    }
}

/// In-memory reputation cache with read-through and write-back.
#[derive(Debug, Default)]
pub struct SocialBrain {
    scores: HashMap<Key, f64>,
    dirty: HashSet<Key>,
}

impl SocialBrain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current standing, reading through to `store` on a cache miss and
    /// caching whatever comes back (or [`DEFAULT_REPUTATION`]).
    pub fn reputation(&mut self, user: &str, guild: &str, store: &dyn ReputationStore) -> f64 {
        let key = Key::new(guild, user);
        if let Some(cached) = self.scores.get(&key) {
            return *cached;
        }
        let value = store
            .load_reputation(guild, user)
            .unwrap_or(DEFAULT_REPUTATION);
        self.scores.insert(key, value);
        value
    }

    /// Fold one interaction of the given `quality` (`0.0..=1.0`) into the
    /// member's standing and append an `"interaction"` audit event.
    /// Returns the updated score.
    pub fn record_interaction(
        &mut self,
        user: &str,
        guild: &str,
        quality: f64,
        now: u64,
        store: &mut dyn ReputationStore,
    ) -> f64 {
        let current = self.reputation(user, guild, store);
        let updated = current * EMA_RETAIN + quality * EMA_GAIN;
        self.set(guild, user, updated);
        store.append_event(ReputationEvent {
            user_id: user.to_owned(),
            guild_id: guild.to_owned(),
            delta: updated - current,
            reason: "interaction".to_owned(),
            at: now,
        });
        updated
    }

    /// Knock `0.1` off the member's standing (floored at zero) and append an
    /// audit event carrying `reason`. Returns the updated score.
    pub fn penalize(
        &mut self,
        user: &str,
        guild: &str,
        reason: &str,
        now: u64,
        store: &mut dyn ReputationStore,
    ) -> f64 {
        let current = self.reputation(user, guild, store);
        let updated = (current - PENALTY).max(0.0);
        self.set(guild, user, updated);
        store.append_event(ReputationEvent {
            user_id: user.to_owned(),
            guild_id: guild.to_owned(),
            delta: -PENALTY,
            reason: reason.to_owned(),
            at: now,
        });
        updated
    }

    /// Write every dirty score through to `store` and clear the dirty set.
    /// Call on an interval and once at shutdown.
    pub fn flush(&mut self, store: &mut dyn ReputationStore) {
        let mut dirty: Vec<Key> = self.dirty.drain().collect();
        dirty.sort_by(|a, b| (&a.guild, &a.user).cmp(&(&b.guild, &b.user)));
        for key in dirty {
            if let Some(value) = self.scores.get(&key) {
                store.store_reputation(&key.guild, &key.user, *value, 1);
            }
        }
    }

    /// How many scores are waiting to be flushed.
    pub fn dirty_len(&self) -> usize {
        self.dirty.len()
    }

    fn set(&mut self, guild: &str, user: &str, value: f64) {
        let key = Key::new(guild, user);
        self.scores.insert(key.clone(), value);
        self.dirty.insert(key);
    }
}

/// HashMap-backed store: the test double and the default backend until a
/// durable one is wired in.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct InMemoryReputationStore {
    pub reputations: HashMap<(String, String), f64>,
    pub interaction_counts: HashMap<(String, String), u32>,
    pub events: Vec<ReputationEvent>,
}

#[cfg(test)]
impl InMemoryReputationStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl ReputationStore for InMemoryReputationStore {
    fn load_reputation(&self, guild: &str, user: &str) -> Option<f64> {
        self.reputations
            .get(&(guild.to_owned(), user.to_owned()))
            .copied()
    }

    fn store_reputation(
        &mut self,
        guild: &str,
        user: &str,
        value: f64,
        interaction_count_delta: u32,
    ) {
        let key = (guild.to_owned(), user.to_owned());
        self.reputations.insert(key.clone(), value);
        *self.interaction_counts.entry(key).or_insert(0) += interaction_count_delta;
    }

    fn append_event(&mut self, event: ReputationEvent) {
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: &str = "discord:100";
    const U: &str = "discord:7";

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    fn key(guild: &str, user: &str) -> (String, String) {
        (guild.to_owned(), user.to_owned())
    }

    #[test]
    fn unknown_member_defaults_to_half() {
        let store = InMemoryReputationStore::new();
        let mut brain = SocialBrain::new();
        assert!(close(brain.reputation(U, G, &store), 0.5));
    }

    #[test]
    fn read_through_hits_store_once_then_caches() {
        let mut store = InMemoryReputationStore::new();
        store.store_reputation(G, U, 0.9, 0);
        let mut brain = SocialBrain::new();
        assert!(close(brain.reputation(U, G, &store), 0.9));
        // Change the store behind the cache: the cached value must win.
        store.store_reputation(G, U, 0.1, 0);
        assert!(close(brain.reputation(U, G, &store), 0.9));
        // Reading never dirties.
        assert_eq!(brain.dirty_len(), 0);
    }

    #[test]
    fn interaction_is_exact_ema() {
        let mut store = InMemoryReputationStore::new();
        let mut brain = SocialBrain::new();
        let updated = brain.record_interaction(U, G, 0.8, 1_000, &mut store);
        assert!(close(updated, 0.515), "got {updated}");
        assert!(close(brain.reputation(U, G, &store), 0.515));
        assert_eq!(brain.dirty_len(), 1);
    }

    #[test]
    fn interaction_appends_event_with_delta() {
        let mut store = InMemoryReputationStore::new();
        let mut brain = SocialBrain::new();
        brain.record_interaction(U, G, 0.8, 1_234, &mut store);
        assert_eq!(store.events.len(), 1);
        let ev = &store.events[0];
        assert_eq!(ev.user_id, U);
        assert_eq!(ev.guild_id, G);
        assert_eq!(ev.reason, "interaction");
        assert_eq!(ev.at, 1_234);
        assert!(close(ev.delta, 0.015), "delta {}", ev.delta);
    }

    #[test]
    fn penalize_subtracts_tenth_and_logs_reason() {
        let mut store = InMemoryReputationStore::new();
        let mut brain = SocialBrain::new();
        let updated = brain.penalize(U, G, "spam", 5, &mut store);
        assert!(close(updated, 0.4));
        let ev = &store.events[0];
        assert_eq!(ev.reason, "spam");
        assert!(close(ev.delta, -0.1));
        assert_eq!(ev.at, 5);
    }

    #[test]
    fn penalize_floors_at_zero() {
        let mut store = InMemoryReputationStore::new();
        store.store_reputation(G, U, 0.05, 0);
        let mut brain = SocialBrain::new();
        assert!(close(brain.penalize(U, G, "x", 0, &mut store), 0.0));
        assert!(close(brain.penalize(U, G, "x", 0, &mut store), 0.0));
    }

    #[test]
    fn flush_writes_only_dirty_keys_and_clears() {
        let mut store = InMemoryReputationStore::new();
        store.store_reputation(G, "discord:clean", 0.7, 0);
        let mut brain = SocialBrain::new();
        // Read-only touch: cached, not dirty.
        brain.reputation("discord:clean", G, &store);
        brain.record_interaction(U, G, 1.0, 0, &mut store);
        brain.flush(&mut store);

        assert!(close(store.reputations[&key(G, U)], 0.525));
        assert_eq!(store.interaction_counts[&key(G, U)], 1);
        // The clean member's count was never bumped.
        assert_eq!(store.interaction_counts[&key(G, "discord:clean")], 0);
        assert_eq!(brain.dirty_len(), 0);

        // A second flush with nothing dirty writes nothing.
        brain.flush(&mut store);
        assert_eq!(store.interaction_counts[&key(G, U)], 1);
    }

    #[test]
    fn guilds_do_not_bleed() {
        let mut store = InMemoryReputationStore::new();
        let mut brain = SocialBrain::new();
        brain.penalize(U, "discord:a", "x", 0, &mut store);
        assert!(close(brain.reputation(U, "discord:b", &store), 0.5));
    }

    #[test]
    fn event_round_trips_through_serde() {
        let ev = ReputationEvent {
            user_id: U.into(),
            guild_id: G.into(),
            delta: -0.1,
            reason: "spam".into(),
            at: 9,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: ReputationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }
}
