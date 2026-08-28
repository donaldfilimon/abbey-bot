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
    /// An explicit `replaces` removed the named old fact and stored the new
    /// one atomically. Carries the removed text so the caller can report it.
    Superseded { stored: String, removed: String },
    /// The new fact was stored, and a model-proposed supersession of the named
    /// old fact was queued for explicit confirmation. The old fact is
    /// untouched — this outcome never removes anything.
    Proposed { stored: String, proposed: String },
}

/// Result of acting on one queued supersession.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupersessionOutcome {
    /// The old fact was removed and the proposal cleared.
    Confirmed(String),
    /// The proposal existed but its old fact was already gone (a bare
    /// `/forget`, or the cap). The proposal is cleared; nothing was removed.
    AlreadyGone(String),
    /// No proposal names that old fact for this user.
    NotPending,
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

    /// Store `fact`, replacing `replaces` in the same lock boundary.
    ///
    /// This is the AUTHORITATIVE path: the caller supplied an explicit signal
    /// naming exactly what to replace, so no confirmation is required. Order
    /// matters and is deliberate — validate first so a rejected fact can never
    /// trigger a deletion, then forget, then remember. Forgetting first is
    /// also what lets a user sitting at `MAX_FACTS` supersede at all, since it
    /// frees the slot the new fact needs.
    pub fn remember_replacing(
        &self,
        guild: &str,
        user: &str,
        fact: &str,
        replaces: &str,
        now: u64,
    ) -> Result<RememberOutcome, &'static str> {
        let fact = memory::validated_fact(fact)?;
        let mut stores = AppState::lock(self.stores);
        let Some(selected) = fact_for_deletion(stores.memory.facts(guild, user), replaces) else {
            return Err("No remembered fact matches what you asked to replace.");
        };
        if selected == fact {
            // Replacing a fact with itself would delete then re-store the same
            // text; report it as a no-op rather than churning both stores.
            return Ok(RememberOutcome::Unchanged);
        }
        if !stores.memory.forget(guild, user, &selected) {
            return Err("No remembered fact matches what you asked to replace.");
        }
        if !stores.memory.remember(guild, user, &fact, now) {
            // The slot was just freed and the text differs from what was
            // removed, so the only way this fails is an exact duplicate of
            // another held fact. Put the removed fact back rather than losing
            // it to a request that stored nothing.
            stores.memory.remember(guild, user, &selected, now);
            return Ok(RememberOutcome::Unchanged);
        }
        // Any queued proposal naming the now-removed fact is moot.
        stores.memory.drop_supersession(guild, user, &selected);
        let mut recall = AppState::lock(self.recall);
        reconcile_projection(&stores, &mut recall);
        Ok(RememberOutcome::Superseded {
            stored: fact,
            removed: selected,
        })
    }

    /// Store `fact` and QUEUE a model-proposed supersession of `supersedes`.
    ///
    /// Never removes anything. The new fact is stored on its own merits; the
    /// old one survives until a human explicitly confirms via
    /// [`Self::confirm_supersession`]. If the proposed old fact does not
    /// exist, the new fact is still stored and no proposal is queued — there
    /// is nothing to contest.
    pub fn remember_proposing(
        &self,
        guild: &str,
        user: &str,
        fact: &str,
        supersedes: &str,
        now: u64,
    ) -> Result<RememberOutcome, &'static str> {
        let fact = memory::validated_fact(fact)?;
        let mut stores = AppState::lock(self.stores);
        if !stores.memory.remember(guild, user, &fact, now) {
            return Ok(RememberOutcome::Unchanged);
        }
        let proposed = fact_for_deletion(stores.memory.facts(guild, user), supersedes)
            .filter(|candidate| candidate != &fact);
        let outcome = match proposed {
            Some(old) => {
                stores
                    .memory
                    .propose_supersession(guild, user, &fact, &old, now);
                RememberOutcome::Proposed {
                    stored: fact,
                    proposed: old,
                }
            }
            None => RememberOutcome::Stored(fact),
        };
        let mut recall = AppState::lock(self.recall);
        reconcile_projection(&stores, &mut recall);
        Ok(outcome)
    }

    /// Apply one queued supersession after an explicit human decision.
    ///
    /// Re-checks that the old fact still exists: by the time someone confirms,
    /// a bare `/forget` or the `MAX_FACTS` cap may already have removed it.
    /// That is reported distinctly rather than silently succeeding.
    pub fn confirm_supersession(
        &self,
        guild: &str,
        user: &str,
        old_fact: &str,
    ) -> SupersessionOutcome {
        let mut stores = AppState::lock(self.stores);
        let pending = stores
            .memory
            .pending_supersessions(guild, user)
            .iter()
            .find(|entry| entry.old_fact == old_fact)
            .cloned();
        let Some(pending) = pending else {
            return SupersessionOutcome::NotPending;
        };
        let removed = stores.memory.forget(guild, user, &pending.old_fact);
        stores
            .memory
            .drop_supersession(guild, user, &pending.old_fact);
        let mut recall = AppState::lock(self.recall);
        reconcile_projection(&stores, &mut recall);
        if removed {
            SupersessionOutcome::Confirmed(pending.old_fact)
        } else {
            SupersessionOutcome::AlreadyGone(pending.old_fact)
        }
    }

    /// Drop one queued proposal without touching either fact.
    pub fn dismiss_supersession(&self, guild: &str, user: &str, old_fact: &str) -> bool {
        let mut stores = AppState::lock(self.stores);
        stores.memory.drop_supersession(guild, user, old_fact)
    }

    pub fn pending_supersessions(
        &self,
        guild: &str,
        user: &str,
    ) -> Vec<memory::PendingSupersession> {
        AppState::lock(self.stores)
            .memory
            .pending_supersessions(guild, user)
            .to_vec()
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

    /// The explicit path is authoritative — it removes the named fact without
    /// any confirmation step, because the caller already gave the signal.
    #[test]
    fn explicit_replaces_supersedes_atomically() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        let outcome = service
            .remember_replacing("g", "u", "moved to zig", "uses rust", 2)
            .expect("supersede");
        assert_eq!(
            outcome,
            RememberOutcome::Superseded {
                stored: "moved to zig".to_string(),
                removed: "uses rust".to_string(),
            }
        );
        assert_eq!(service.facts("g", "u"), vec!["moved to zig".to_string()]);
        assert!(service.pending_supersessions("g", "u").is_empty());
    }

    /// A rejected fact must never cause a deletion. Validation runs before
    /// the forget, so the old fact is still there afterwards.
    #[test]
    fn a_rejected_replacement_never_removes_the_old_fact() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        let too_long = "x".repeat(memory::MAX_FACT_CHARS + 1);
        assert!(
            service
                .remember_replacing("g", "u", &too_long, "uses rust", 2)
                .is_err()
        );
        assert_eq!(service.facts("g", "u"), vec!["uses rust".to_string()]);
    }

    /// Naming a fact that is not held must not store the new one under a
    /// false pretense, and must not remove anything.
    #[test]
    fn replacing_an_absent_fact_is_refused_without_storing() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        assert!(
            service
                .remember_replacing("g", "u", "moved to zig", "plays banjo", 2)
                .is_err()
        );
        assert_eq!(service.facts("g", "u"), vec!["uses rust".to_string()]);
    }

    /// Superseding must work at the cap: the forget frees the slot the new
    /// fact needs. Without forget-before-remember this silently fails.
    #[test]
    fn superseding_works_at_the_fact_cap() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        for index in 0..memory::MAX_FACTS {
            service
                .remember("g", "u", &format!("fact number {index}"), 1)
                .expect("seed");
        }
        assert_eq!(service.facts("g", "u").len(), memory::MAX_FACTS);
        // A plain remember at the cap is rejected...
        assert_eq!(
            service.remember("g", "u", "one more", 2).expect("capped"),
            RememberOutcome::Unchanged
        );
        // ...but an explicit supersession still succeeds.
        let outcome = service
            .remember_replacing("g", "u", "one more", "fact number 0", 3)
            .expect("supersede at cap");
        assert_eq!(
            outcome,
            RememberOutcome::Superseded {
                stored: "one more".to_string(),
                removed: "fact number 0".to_string(),
            }
        );
        let facts = service.facts("g", "u");
        assert_eq!(facts.len(), memory::MAX_FACTS);
        assert!(facts.contains(&"one more".to_string()));
        assert!(!facts.contains(&"fact number 0".to_string()));
    }

    /// THE core invariant of this feature: a model proposal stores the new
    /// fact but must never remove the old one.
    #[test]
    fn a_model_proposal_stores_without_removing_anything() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        let outcome = service
            .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
            .expect("propose");
        assert_eq!(
            outcome,
            RememberOutcome::Proposed {
                stored: "moved to zig".to_string(),
                proposed: "uses rust".to_string(),
            }
        );
        let facts = service.facts("g", "u");
        assert!(facts.contains(&"uses rust".to_string()), "{facts:?}");
        assert!(facts.contains(&"moved to zig".to_string()), "{facts:?}");
        assert_eq!(service.pending_supersessions("g", "u").len(), 1);
    }

    /// Proposing against a fact that is not held still stores the new fact —
    /// it stands on its own merits — but queues nothing to contest.
    #[test]
    fn proposing_against_an_absent_fact_stores_without_queuing() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        let outcome = service
            .remember_proposing("g", "u", "moved to zig", "never said this", 1)
            .expect("propose");
        assert_eq!(outcome, RememberOutcome::Stored("moved to zig".to_string()));
        assert!(service.pending_supersessions("g", "u").is_empty());
    }

    #[test]
    fn confirming_a_proposal_removes_the_old_fact_exactly_once() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        service
            .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
            .expect("propose");
        assert_eq!(
            service.confirm_supersession("g", "u", "uses rust"),
            SupersessionOutcome::Confirmed("uses rust".to_string())
        );
        assert_eq!(service.facts("g", "u"), vec!["moved to zig".to_string()]);
        assert!(service.pending_supersessions("g", "u").is_empty());
        // Confirming again must not resurrect or re-remove anything.
        assert_eq!(
            service.confirm_supersession("g", "u", "uses rust"),
            SupersessionOutcome::NotPending
        );
    }

    /// The race the plan called out: by the time someone confirms, a bare
    /// `/forget` may already have removed the old fact. That must report
    /// distinctly rather than silently succeeding.
    #[test]
    fn confirming_after_the_old_fact_is_already_gone_reports_it() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        service
            .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
            .expect("propose");
        assert!(service.forget("g", "u", "uses rust"));
        assert_eq!(
            service.confirm_supersession("g", "u", "uses rust"),
            SupersessionOutcome::AlreadyGone("uses rust".to_string())
        );
        assert!(service.pending_supersessions("g", "u").is_empty());
    }

    #[test]
    fn dismissing_a_proposal_keeps_both_facts() {
        let state = AppState::in_memory();
        let service = state.memory_service();
        service.remember("g", "u", "uses rust", 1).expect("store");
        service
            .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
            .expect("propose");
        assert!(service.dismiss_supersession("g", "u", "uses rust"));
        let facts = service.facts("g", "u");
        assert!(facts.contains(&"uses rust".to_string()));
        assert!(facts.contains(&"moved to zig".to_string()));
        assert!(service.pending_supersessions("g", "u").is_empty());
    }

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
        assert!(
            state
                .memory_service()
                .forget("g", "u", " Donald   likes Rust. "),
            "a whitespace variant selects the normalized canonical fact"
        );
        assert!(
            state.memory_service().facts("g", "u").is_empty(),
            "canonical JSON fact is deleted"
        );
        assert!(
            AppState::lock(&state.recall)
                .facts_for_user("g", "u")
                .is_empty(),
            "WDBX projection is reconciled in the same operation"
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
