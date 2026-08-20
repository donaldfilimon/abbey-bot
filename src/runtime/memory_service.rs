//! Canonical coordination for Abbey's two durable-memory representations.
//!
//! Plain memory is persisted in `abbey-state.json`; semantic memory is the
//! adjacent WDBX segment. They are separate file formats, but one logical
//! service: JSON memory is canonical, while WDBX is a semantic projection.
//! Every mutation takes `stores` and then `recall`, so the projection is
//! reconciled before another observer can see the canonical change.

use std::sync::Mutex;

use crate::memory::{self, PersonaContext};
use crate::persist::Stores;
use crate::wdbx::{Recall, RecalledFact};

use super::AppState;

/// Version 1 makes `Stores.memory` the sole fact authority and WDBX `mem:*`
/// rows a projection rebuilt at startup. Version 0 is migrated by first
/// unioning recoverable WDBX-only facts into JSON memory.
pub(super) const MEMORY_PROJECTION_VERSION: u32 = 1;

/// Reconcile loaded state before it becomes observable through `AppState`.
/// A successful subsequent JSON save atomically publishes version 1; crashes
/// before that simply repeat the idempotent legacy union next startup. A
/// future projection is rejected before either in-memory document is mutated:
/// an older binary cannot safely infer which representation that version made
/// authoritative, so starting would risk erasing facts during reconciliation.
pub(super) fn reconcile_loaded(
    mut stores: Stores,
    mut recall: Recall,
) -> Result<(Stores, Recall), String> {
    if stores.memory_projection_version > MEMORY_PROJECTION_VERSION {
        return Err(format!(
            "state uses unsupported memory projection version {}; this binary supports up to {}",
            stores.memory_projection_version, MEMORY_PROJECTION_VERSION
        ));
    }
    stores.memory.migrate_legacy_user_keys();
    if stores.memory_projection_version < MEMORY_PROJECTION_VERSION {
        for (guild, fact) in recall.all_memory_facts() {
            stores
                .memory
                .remember(&guild, &fact.user, &fact.text, fact.at);
        }
        stores.memory_projection_version = MEMORY_PROJECTION_VERSION;
    }
    reconcile_projection(&stores, &mut recall);
    Ok((stores, recall))
}

fn reconcile_projection(stores: &Stores, recall: &mut Recall) {
    recall.reconcile_memory_facts(
        stores
            .memory
            .fact_records()
            .into_iter()
            .map(|fact| (fact.guild, fact.user, fact.text, fact.at)),
    );
}

/// Result of a validated remember operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RememberOutcome {
    /// A previously unknown fact was stored in both representations.
    Stored(String),
    /// The fact was already represented, or canonical memory was at its cap.
    Unchanged,
}

/// One logical memory service over the JSON and WDBX stores.
///
/// The service borrows [`AppState`]'s mutexes rather than adding a third owner
/// or a second cache. Its invariant is simple: whenever both locks are needed,
/// `stores` is acquired before `recall`, matching the process-wide lock order.
pub struct MemoryService<'a> {
    stores: &'a Mutex<Stores>,
    recall: &'a Mutex<Recall>,
}

impl<'a> MemoryService<'a> {
    pub(super) const fn new(stores: &'a Mutex<Stores>, recall: &'a Mutex<Recall>) -> Self {
        Self { stores, recall }
    }

    /// Validate and write canonical JSON memory, then reconcile the semantic
    /// projection before releasing either lock.
    pub fn remember(
        &self,
        guild: &str,
        user: &str,
        fact: &str,
        now: u64,
    ) -> Result<RememberOutcome, &'static str> {
        let fact = memory::validated_fact(fact)?;
        let mut stores = AppState::lock(self.stores);
        if !stores.memory.remember(guild, user, &fact, now) {
            return Ok(RememberOutcome::Unchanged);
        }
        let mut recall = AppState::lock(self.recall);
        reconcile_projection(&stores, &mut recall);
        Ok(RememberOutcome::Stored(fact))
    }

    /// Facts from canonical JSON memory. Legacy WDBX-only rows are recovered
    /// once in `reconcile_loaded`, before `AppState` becomes observable.
    pub fn facts(&self, guild: &str, user: &str) -> Vec<String> {
        AppState::lock(self.stores)
            .memory
            .facts(guild, user)
            .to_vec()
    }

    /// Remove one exact or whitespace-normalized fact from both stores under a
    /// single lock boundary. This also removes duplicate WDBX records left by
    /// older write paths.
    pub fn forget(&self, guild: &str, user: &str, requested: &str) -> bool {
        let mut stores = AppState::lock(self.stores);
        let Some(selected) = fact_for_deletion(stores.memory.facts(guild, user), requested) else {
            return false;
        };
        if !stores.memory.forget(guild, user, &selected) {
            return false;
        }
        let mut recall = AppState::lock(self.recall);
        reconcile_projection(&stores, &mut recall);
        true
    }

    /// Semantic lookup for one person. Keeping this behind the service makes
    /// ToolScope use the same subject boundary as slash commands.
    pub fn recall(&self, guild: &str, user: &str, query: &str, limit: usize) -> Vec<RecalledFact> {
        AppState::lock(self.recall).recall_for_user(guild, user, query, limit)
    }

    /// Assemble plain channel context and semantic matches from one consistent
    /// in-memory boundary, without widening the guild/user privacy scope.
    pub fn context_for(
        &self,
        guild: &str,
        user: &str,
        channel: &str,
        query: &str,
        recall_limit: usize,
        reputation: f64,
    ) -> PersonaContext {
        let stores = AppState::lock(self.stores);
        let recall = AppState::lock(self.recall);
        let mut context = stores.memory.context_for(guild, user, channel);
        // SocialBrain is the live standing authority. MemoryBank's legacy
        // field may be stale, so the caller supplies its already-consistent
        // social snapshot rather than taking another lock here.
        context.reputation = reputation;
        for fact in recall.recall_for_user(guild, user, query, recall_limit) {
            if !context.user_facts.contains(&fact.text) {
                context.user_facts.push(fact.text);
            }
        }
        context
    }

    /// Apply the non-memory state updates that belong in `Stores`, then clone
    /// both persistence documents while holding the canonical lock pair. The
    /// JSON publishes first; WDBX is saved only after that succeeds and is
    /// repaired from JSON at the next startup after any intervening crash.
    pub(super) fn consistent_snapshot_after(
        &self,
        update_stores: impl FnOnce(&mut Stores),
    ) -> (Stores, Recall) {
        let mut stores = AppState::lock(self.stores);
        update_stores(&mut stores);
        stores.memory_projection_version = MEMORY_PROJECTION_VERSION;
        let mut recall = AppState::lock(self.recall);
        reconcile_projection(&stores, &mut recall);
        (stores.clone(), recall.clone())
    }
}

/// Preserve exact matching for legacy facts written before normalization, but
/// let a manually entered whitespace variant find a normalized fact.
fn fact_for_deletion(facts: &[String], requested: &str) -> Option<String> {
    facts
        .iter()
        .find(|fact| fact.as_str() == requested)
        .or_else(|| {
            let normalized = memory::normalize_fact_text(requested);
            facts.iter().find(|fact| fact.as_str() == normalized)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn remember_validates_once_and_updates_both_representations() {
        let state = AppState::in_memory();
        assert_eq!(
            state
                .memory_service()
                .remember("g", "u", "  Donald\nlikes\tRust.  ", 7),
            Ok(RememberOutcome::Stored("Donald likes Rust.".into()))
        );
        assert_eq!(
            AppState::lock(&state.stores).memory.facts("g", "u"),
            ["Donald likes Rust."]
        );
        assert_eq!(
            AppState::lock(&state.recall)
                .facts_for_user("g", "u")
                .into_iter()
                .map(|fact| fact.text)
                .collect::<Vec<_>>(),
            ["Donald likes Rust."]
        );
        assert_eq!(
            state
                .memory_service()
                .remember("g", "u", &"🦀".repeat(memory::MAX_FACT_CHARS + 1), 8),
            Err("Keep one remembered fact to 300 characters or fewer.")
        );
    }

    #[test]
    fn runtime_reads_only_canonical_facts_after_migration() {
        let state = AppState::in_memory();
        AppState::lock(&state.stores)
            .memory
            .remember("g", "u", "plain only", 1);
        AppState::lock(&state.recall).remember("g", "u", "semantic only", 2);

        assert_eq!(state.memory_service().facts("g", "u"), ["plain only"]);
        assert!(!state.memory_service().forget("g", "u", "semantic only"));
    }

    #[test]
    fn concurrent_writes_and_snapshots_never_observe_one_sided_facts() {
        let state = AppState::in_memory();
        let writer = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || {
                for i in 0..memory::MAX_FACTS {
                    let fact = format!("fact {i}");
                    let _ = state.memory_service().remember("g", "u", &fact, i as u64);
                }
            })
        };

        while !writer.is_finished() {
            let (stores, recall) = state.memory_service().consistent_snapshot_after(|_| {});
            let plain = stores.memory.facts("g", "u").to_vec();
            let semantic: Vec<String> = recall
                .facts_for_user("g", "u")
                .into_iter()
                .map(|fact| fact.text)
                .collect();
            assert_eq!(plain, semantic);
        }
        writer.join().expect("writer did not panic");
        // The writer can finish before this thread is first scheduled, so the
        // test must assert at least one snapshot independent of the race.
        let (stores, recall) = state.memory_service().consistent_snapshot_after(|_| {});
        assert_eq!(
            stores.memory.facts("g", "u"),
            recall
                .facts_for_user("g", "u")
                .into_iter()
                .map(|fact| fact.text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn first_load_recovers_legacy_wdbx_only_facts_then_marks_json_canonical() {
        let mut stores = Stores::default();
        stores.memory.remember("g", "u", "plain", 1);
        let mut recall = Recall::new();
        recall.remember("g", "u", "semantic legacy", 2);

        let (stores, recall) = reconcile_loaded(stores, recall).expect("legacy migration");
        assert_eq!(stores.memory_projection_version, MEMORY_PROJECTION_VERSION);
        assert_eq!(stores.memory.facts("g", "u"), ["plain", "semantic legacy"]);
        assert_eq!(
            recall
                .facts_for_user("g", "u")
                .into_iter()
                .map(|fact| fact.text)
                .collect::<Vec<_>>(),
            ["plain", "semantic legacy"]
        );
    }

    #[test]
    fn canonical_json_repairs_a_crash_stale_wdbx_without_resurrecting_deletions() {
        let mut stores = Stores {
            memory_projection_version: MEMORY_PROJECTION_VERSION,
            ..Stores::default()
        };
        stores.memory.remember("g", "u", "survives", 2);
        let mut recall = Recall::new();
        recall.remember("g", "u", "deleted before crash", 1);

        let (stores, recall) = reconcile_loaded(stores, recall).expect("canonical repair");
        assert_eq!(stores.memory.facts("g", "u"), ["survives"]);
        assert_eq!(
            recall
                .facts_for_user("g", "u")
                .into_iter()
                .map(|fact| fact.text)
                .collect::<Vec<_>>(),
            ["survives"]
        );
    }

    #[test]
    fn context_uses_the_callers_live_social_standing_not_legacy_memory() {
        let state = AppState::in_memory();
        AppState::lock(&state.stores)
            .memory
            .user_mut("g", "u")
            .reputation = 0.11;

        let context = state
            .memory_service()
            .context_for("g", "u", "c", "anything", 3, 0.83);

        assert_eq!(context.reputation, 0.83);
    }

    #[test]
    fn a_future_projection_version_fails_closed_before_reconciliation() {
        let mut stores = Stores {
            memory_projection_version: MEMORY_PROJECTION_VERSION + 1,
            ..Stores::default()
        };
        stores.memory.remember("g", "u", "future canonical fact", 2);
        let mut recall = Recall::new();
        recall.remember("g", "u", "future semantic fact", 3);

        let error = reconcile_loaded(stores, recall).expect_err("future schema must not start");

        assert!(error.contains("unsupported memory projection version 2"));
        assert!(error.contains("supports up to 1"));
    }
}
