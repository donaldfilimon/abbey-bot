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
    /// The replacement fact this proposal was premised on is itself gone, so
    /// confirming would remove the old fact and leave the user holding
    /// NEITHER. Refused, and the stale proposal is cleared. Nothing removed.
    PremiseGone { old_fact: String, new_fact: String },
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
            // it to a request that stored nothing. This restore cannot itself
            // fail: `selected` was just removed from a de-duplicated list and
            // the list is one under the cap.
            stores.memory.remember(guild, user, &selected, now);
            // Reconcile even though the fact SET is unchanged. The
            // forget-then-restore moved `selected` to the end of the Vec and
            // bumped `updated_at`, which `fact_records` stamps onto every
            // projected row for this user — skipping this leaves WDBX
            // timestamps stale until the next unrelated write. Every mutating
            // path in this module reconciles before returning; this one is not
            // an exception.
            let mut recall = AppState::lock(self.recall);
            reconcile_projection(&stores, &mut recall);
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
        self.remember_admitted(guild, user, fact, Some(supersedes), now, None)
    }

    /// Publish a fact, optional proposed replacement, and its admission receipt
    /// under the same canonical lock used by persistence snapshots.
    pub fn remember_admitted(
        &self,
        guild: &str,
        user: &str,
        fact: &str,
        supersedes: Option<&str>,
        now: u64,
        receipt: Option<&str>,
    ) -> Result<RememberOutcome, &'static str> {
        let fact = memory::validated_fact(fact)?;
        let mut stores = AppState::lock(self.stores);
        if !stores.memory.remember(guild, user, &fact, now) {
            return Ok(RememberOutcome::Unchanged);
        }
        let proposed = supersedes
            .and_then(|old| fact_for_deletion(stores.memory.facts(guild, user), old))
            .filter(|candidate| candidate != &fact);
        let outcome = match proposed {
            Some(old) => {
                stores
                    .memory
                    .propose_supersession(guild, user, &fact, &old, now);
                RememberOutcome::Proposed {
                    stored: fact.clone(),
                    proposed: old,
                }
            }
            None => RememberOutcome::Stored(fact.clone()),
        };
        if let Some(receipt) = receipt {
            stores
                .memory_receipts
                .insert(Self::receipt_key(guild, user, &fact), receipt.to_owned());
        }
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
        // A proposal is a claim that `new_fact` REPLACES `old_fact`. If the
        // replacement has since been removed — a bare `/forget` on it, say —
        // that claim no longer holds, and confirming would delete the old
        // fact too and leave the person with neither. Nothing clears a
        // proposal when its `new_fact` goes away, so the stale entry can
        // outlive its own premise and `/pending list` would still render it.
        // Refuse and clear it rather than complete a destructive action on a
        // premise the display no longer matches.
        if !stores
            .memory
            .facts(guild, user)
            .iter()
            .any(|fact| fact == &pending.new_fact)
        {
            stores
                .memory
                .drop_supersession(guild, user, &pending.old_fact);
            let mut recall = AppState::lock(self.recall);
            reconcile_projection(&stores, &mut recall);
            return SupersessionOutcome::PremiseGone {
                old_fact: pending.old_fact,
                new_fact: pending.new_fact,
            };
        }
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

    /// Clone one subject's canonical facts and pending replacements while
    /// holding the store mutex once, so the two views describe one state.
    pub fn subject_snapshot(
        &self,
        guild: &str,
        user: &str,
    ) -> (Vec<String>, Vec<memory::PendingSupersession>) {
        let stores = AppState::lock(self.stores);
        (
            stores.memory.facts(guild, user).to_vec(),
            stores.memory.pending_supersessions(guild, user).to_vec(),
        )
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
    /// The exact stored fact a `/forget` or `replaces` request names, if any.
    /// Read-only: lets a caller propose a ledger edge before deleting.
    pub fn resolve_fact(&self, guild: &str, user: &str, requested: &str) -> Option<String> {
        let stores = AppState::lock(self.stores);
        fact_for_deletion(stores.memory.facts(guild, user), requested)
    }

    /// Read-only precondition for a gated `remember`: `Some(reason)` when the
    /// local store would refuse the fact anyway (already held, or at the cap),
    /// so no candidate is proposed for a write that would not happen.
    pub fn remember_blocked(&self, guild: &str, user: &str, fact: &str) -> Option<&'static str> {
        let stores = AppState::lock(self.stores);
        let facts = stores.memory.facts(guild, user);
        if facts.iter().any(|held| held == fact) {
            Some("already on record")
        } else if facts.len() >= memory::MAX_FACTS {
            Some("the fact list is full")
        } else {
            None
        }
    }

    /// Read-only precondition for a gated `/pending confirm`: true only when a
    /// proposal names `old_fact` and both it and its replacement are still
    /// stored, i.e. when `confirm_supersession` would actually remove
    /// something. Otherwise no tombstone may be proposed for it.
    pub fn confirm_would_remove(&self, guild: &str, user: &str, old_fact: &str) -> bool {
        let stores = AppState::lock(self.stores);
        let facts = stores.memory.facts(guild, user);
        stores
            .memory
            .pending_supersessions(guild, user)
            .iter()
            .any(|pending| {
                pending.old_fact == old_fact
                    && facts.iter().any(|held| held == old_fact)
                    && facts.iter().any(|held| held == &pending.new_fact)
            })
    }

    fn receipt_key(guild: &str, user: &str, fact: &str) -> String {
        format!("{guild}\u{1f}{user}\u{1f}{fact}")
    }

    /// The ledger receipt (episode digest, hex) recorded for a fact, if any.
    pub fn receipt(&self, guild: &str, user: &str, fact: &str) -> Option<String> {
        AppState::lock(self.stores)
            .memory_receipts
            .get(&Self::receipt_key(guild, user, fact))
            .cloned()
    }

    pub fn record_receipt(&self, guild: &str, user: &str, fact: &str, digest_hex: &str) {
        AppState::lock(self.stores)
            .memory_receipts
            .insert(Self::receipt_key(guild, user, fact), digest_hex.to_owned());
    }

    pub fn take_receipt(&self, guild: &str, user: &str, fact: &str) -> Option<String> {
        AppState::lock(self.stores)
            .memory_receipts
            .remove(&Self::receipt_key(guild, user, fact))
    }

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
#[path = "memory_service/tests.rs"]
mod tests;
