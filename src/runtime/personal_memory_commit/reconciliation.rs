//! Runtime denial authority and three-way reconciliation of our uninstalled publication.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct UninstalledPublication {
    base: persist::Stores,
    identity: Option<String>,
}
struct RuntimeDenial {
    request_id: String,
    stamp: ConsentStamp,
    cutoff: u64,
}
#[derive(Default)]
pub(in crate::runtime) struct ReconciliationState {
    pub(super) publication: Option<UninstalledPublication>,
    // One entry per admitted subject, bounded by canonical subject admission.
    // Unlike the request table this survives an owner's failed completion.
    denials: BTreeMap<String, RuntimeDenial>,
}
impl ReconciliationState {
    pub(super) fn deny(
        &mut self,
        key: String,
        request_id: String,
        stamp: ConsentStamp,
        cutoff: u64,
    ) {
        self.denials.insert(
            key,
            RuntimeDenial {
                request_id,
                stamp,
                cutoff,
            },
        );
    }
    pub(super) fn has_other_denial(&self, key: &str, request_id: &str) -> bool {
        self.denials
            .iter()
            .any(|(scope, denial)| scope != key || denial.request_id != request_id)
    }
    pub(super) fn denials_cleared(&self) -> bool {
        self.denials.is_empty()
    }
    pub(super) fn retire_denials(&mut self, qualified: &persist::Stores) {
        self.denials.retain(|key, denial| {
            !qualified.personal_memory.get(key).is_some_and(|subject| {
                subject.choice == UseChoice::Off
                    && !subject.activation_pending
                    && subject.revision >= denial.stamp.revision
                    && subject.consent_epoch >= denial.stamp.consent_epoch
                    && qualified.personal_memory_exposure.epoch >= denial.stamp.exposure_epoch
                    && qualified.personal_memory_exposure.cutoff >= denial.cutoff
            })
        });
    }
    pub(super) fn published(&mut self, base: &persist::Stores, published: &persist::Stores) {
        self.publication = Some(UninstalledPublication {
            base: base.clone(),
            identity: published.canonical_base.get(),
        });
    }
}
impl UninstalledPublication {
    pub(super) fn matches(&self, live: &persist::Stores, disk: &persist::Stores) -> bool {
        self.identity.is_some()
            && self.identity == disk.canonical_base.get()
            && self.base.canonical_base.get() == live.canonical_base.get()
    }
    pub(super) fn rebase(
        &self,
        live: &mut persist::Stores,
        disk: &persist::Stores,
    ) -> Result<(), MemoryConsentError> {
        // Apply only the base -> published changes. Facts subsequently removed from
        // live but unchanged on disk stay removed; unrelated live additions stay put.
        let keys: BTreeSet<_> = self
            .base
            .memory
            .users
            .keys()
            .chain(disk.memory.users.keys())
            .collect();
        for key in keys {
            let empty = crate::memory::UserMemory::default();
            let before = self.base.memory.users.get(key).unwrap_or(&empty);
            let after = disk.memory.users.get(key).unwrap_or(&empty);
            if before.facts == after.facts
                && before.pending_supersessions == after.pending_supersessions
            {
                continue;
            }
            let target = live.memory.users.entry(key.clone()).or_default();
            merge_rows(&mut target.facts, &before.facts, &after.facts);
            if target.facts.len() > crate::memory::MAX_FACTS {
                return Err(MemoryConsentError::Bounds);
            }
            merge_rows(
                &mut target.pending_supersessions,
                &before.pending_supersessions,
                &after.pending_supersessions,
            );
            target.updated_at = target.updated_at.max(after.updated_at);
        }
        merge_map(
            &mut live.memory_receipts,
            &self.base.memory_receipts,
            &disk.memory_receipts,
        );
        for (key, published) in &disk.personal_memory {
            let empty = PersonalMemorySubject::default();
            let before = self.base.personal_memory.get(key).unwrap_or(&empty);
            let target = live.personal_memory.entry(key.clone()).or_default();
            if target == before {
                *target = published.clone();
            } else {
                merge_map(&mut target.proofs, &before.proofs, &published.proofs);
                merge_map(&mut target.outcomes, &before.outcomes, &published.outcomes);
                target.schema = target.schema.max(published.schema);
                target.policy_version = target.policy_version.max(published.policy_version);
                target.revision = target.revision.max(published.revision);
                target.consent_epoch = target.consent_epoch.max(published.consent_epoch);
                // A concurrently admitted denial always wins over an uninstalled grant.
                if target.choice == UseChoice::Off || published.choice == UseChoice::Off {
                    target.choice = UseChoice::Off;
                    target.activation_pending = false;
                } else {
                    target.activation_pending |= published.activation_pending;
                }
            }
        }
        let facts = live.memory.fact_records();
        let fact_keys: BTreeSet<_> = facts
            .iter()
            .map(|f| fact_key(&f.guild, &f.user, &f.text))
            .collect();
        let receipt_keys: BTreeSet<_> = facts
            .iter()
            .map(|f| receipt_key(&f.guild, &f.user, &f.text))
            .collect();
        for subject in live.personal_memory.values_mut() {
            subject.proofs.retain(|key, _| fact_keys.contains(key));
        }
        live.memory_receipts
            .retain(|key, _| receipt_keys.contains(key));
        let exposure = &mut live.personal_memory_exposure;
        let published = &disk.personal_memory_exposure;
        exposure.schema = exposure.schema.max(published.schema);
        exposure.epoch = exposure.epoch.max(published.epoch);
        exposure.cutoff = exposure.cutoff.max(published.cutoff);
        for (key, epoch) in &published.scope_epochs {
            let current = exposure.scope_epochs.entry(key.clone()).or_default();
            *current = (*current).max(*epoch);
        }
        for receipt in &published.receipts {
            if !exposure.receipts.contains(receipt) {
                exposure.receipts.push(receipt.clone());
            }
        }
        exposure
            .receipts
            .sort_by_key(|receipt| (receipt.epoch, receipt.cutoff));
        if exposure.receipts.len() > 128 {
            exposure.receipts.drain(..exposure.receipts.len() - 128);
        }
        live.canonical_base.set(disk.canonical_base.get());
        validate_metadata(&live.personal_memory, &live.personal_memory_exposure)
    }
}
fn merge_rows<T: Clone + PartialEq>(live: &mut Vec<T>, before: &[T], after: &[T]) {
    live.retain(|row| !before.contains(row) || after.contains(row));
    for row in after {
        if !before.contains(row) && !live.contains(row) {
            live.push(row.clone());
        }
    }
}
fn merge_map<T: Clone + PartialEq>(
    live: &mut BTreeMap<String, T>,
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
) {
    for key in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        // A newer live modification or deletion wins a same-key conflict.
        if live.get(key) == before.get(key) && before.get(key) != after.get(key) {
            if let Some(value) = after.get(key) {
                live.insert(key.clone(), value.clone());
            } else {
                live.remove(key);
            }
        }
    }
}

// The mutation owner excludes ordinary fact writes during qualification/installation.
// Copy only fact-bearing fields: channel and other observation state may still advance.
pub(super) fn install_fact_rows(live: &mut persist::Stores, candidate: &persist::Stores) {
    for (scope, source) in &candidate.memory.users {
        let target = live.memory.users.entry(scope.clone()).or_default();
        target.facts = source.facts.clone();
        target.pending_supersessions = source.pending_supersessions.clone();
        target.updated_at = target.updated_at.max(source.updated_at);
    }
}
// Called only after the complete canonical image and its projection have exact readback.
// Keep the original request result/digest; completion is qualification, not a new grant.
pub(super) fn complete_qualified_outcomes(candidate: &mut persist::Stores) {
    for subject in candidate.personal_memory.values_mut() {
        for outcome in subject.outcomes.values_mut() {
            outcome.completed = true;
        }
    }
}
/// Only this receipt permits live installation; all durable phases precede it.
pub(super) struct QualifiedCommit {
    pub projection: crate::wdbx::Recall,
}
pub(super) enum MarkerRetirement<'a> {
    Replay,
    Subject {
        key: &'a str,
        off: bool,
        on: bool,
        allowed: ConsentStamp,
    },
}
pub(super) struct PublicationContext<'a> {
    pub owner: &'a journal::Lease,
    pub dir: &'a std::path::Path,
    pub expected: &'a persist::Stores,
    pub action: &'a SelfAuthorizedFactAction,
}
impl AppState {
    pub(super) fn publish_personal_candidate(
        &self,
        context: PublicationContext<'_>,
        candidate: &mut persist::Stores,
        marker: &mut journal::Marker,
        retirement: MarkerRetirement<'_>,
    ) -> Result<QualifiedCommit, MemoryConsentError> {
        let PublicationContext {
            owner,
            dir,
            expected,
            action,
        } = context;
        let guild = &action.proof.guild;
        let user = &action.proof.subject;
        validate_metadata(
            &candidate.personal_memory,
            &candidate.personal_memory_exposure,
        )?;
        self.overlay_continuity(&mut candidate.continuity)
            .map_err(|_| MemoryConsentError::Persistence)?;
        persist::persist_canonical_owned(owner, &*self.persistence_sink, dir, candidate)
            .map_err(|_| MemoryConsentError::Persistence)?;
        candidate
            .canonical_base
            .set(exact_readback(dir, candidate, guild, user)?);
        Self::lock(&self.personal_memory_reconciliation).published(expected, candidate);
        let projection = self.qualify_personal_projection(candidate, dir)?;
        if let MarkerRetirement::Subject { key, .. } = retirement {
            candidate
                .personal_memory
                .get_mut(key)
                .ok_or(MemoryConsentError::Persistence)?
                .activation_pending = false;
        }
        complete_qualified_outcomes(candidate);
        self.overlay_continuity(&mut candidate.continuity)
            .map_err(|_| MemoryConsentError::Persistence)?;
        persist::persist_canonical_owned(owner, &*self.persistence_sink, dir, candidate)
            .map_err(|_| MemoryConsentError::Persistence)?;
        candidate
            .canonical_base
            .set(exact_readback(dir, candidate, guild, user)?);
        Self::lock(&self.personal_memory_reconciliation).published(expected, candidate);
        match retirement {
            MarkerRetirement::Replay => {
                journal::verify_minima(candidate, marker)?;
                if !marker.withdrawals.is_empty() || !marker.activations.is_empty() {
                    marker.withdrawals.clear();
                    marker.activations.clear();
                    *marker = journal::publish_owned(owner, dir, marker.revision, marker.clone())?;
                }
            }
            MarkerRetirement::Subject {
                key,
                off,
                on,
                allowed,
            } => {
                if off {
                    journal::verify_minima(candidate, marker)?;
                }
                if status(&Self::lock(&self.stores), guild, user).stamp != allowed {
                    return Err(MemoryConsentError::Stale);
                }
                if off {
                    marker.withdrawals.remove(key);
                }
                if off || on {
                    marker.activations.remove(key);
                    *marker = journal::publish_owned(owner, dir, marker.revision, marker.clone())?;
                }
            }
        }
        Ok(QualifiedCommit { projection })
    }
    pub(super) fn publish_personal_replay(
        &self,
        owner: &journal::Lease,
        dir: &std::path::Path,
        candidate: &mut persist::Stores,
        expected: &persist::Stores,
        mut marker: journal::Marker,
        action: &SelfAuthorizedFactAction,
    ) -> Result<(), MemoryConsentError> {
        let qualified = self.publish_personal_candidate(
            PublicationContext {
                owner,
                dir,
                expected,
                action,
            },
            candidate,
            &mut marker,
            MarkerRetirement::Replay,
        )?;
        self.install_qualified_personal(candidate, expected, qualified, true)
    }
    pub(super) fn install_qualified_personal(
        &self,
        candidate: &persist::Stores,
        expected: &persist::Stores,
        qualified: QualifiedCommit,
        clear_barrier: bool,
    ) -> Result<(), MemoryConsentError> {
        self.install_personal_candidate(candidate, expected, clear_barrier)?;
        *Self::lock(&self.recall) = qualified.projection;
        Ok(())
    }
}
