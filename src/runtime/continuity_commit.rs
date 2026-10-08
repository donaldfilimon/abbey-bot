//! Retained canonical continuity mutation owners. Native human grants stay transient.
use super::*;
use crate::{
    episode_gate::{
        GateOutcome, MemoryCandidateRequest, MemoryClass, RetentionClass,
        continuity::MemoryCandidateState,
    },
    work::{
        WorkError, WorkScope,
        continuity::{ContinuityCard, ContinuityStore, ResolvedConfirmation},
    },
};
#[derive(Clone)]
pub(super) struct PendingPublication {
    forget_memo: Option<String>,
    expected: Option<ContinuityCard>,
    desired: Option<ContinuityCard>,
}
impl AppState {
    pub(crate) async fn confirm_continuity(
        &self,
        grant: ResolvedConfirmation,
        generation: u64,
        actor: u64,
        channel: u64,
    ) -> Result<ContinuityCard, WorkError> {
        if self.data_dir.is_none() || self.persistence_requests.get().is_none() {
            return Err(WorkError::Persistence);
        }
        let state = self.owned_state().ok_or(WorkError::Persistence)?;
        let owner = self
            .service_registry()
            .ok_or(WorkError::Persistence)?
            .spawn_result(
                crate::service::OperationKind::PersistencePreparation,
                async move {
                    let _serial = state.persistence_preparation.lock().await;
                    let scope = grant.scope().clone();
                    let grant_expiry = grant.expires_at();
                    state.reserve_continuity_recovery(&scope)?;
                    if state.continuity_generation(&scope) != Some(generation) {
                        return Err(WorkError::Stale);
                    }
                    let access = state
                        .fresh_continuity_access(&scope, actor, channel)
                        .await?;
                    if state.continuity_generation(&scope) != Some(generation) {
                        return Err(WorkError::Stale);
                    }
                    let mut snapshot = state.snapshot_without_proposals();
                    if snapshot.stores.canonical_preparation_failed {
                        return Err(WorkError::Persistence);
                    }
                    let old = snapshot.stores.continuity.card(&scope).cloned();
                    let mut card = snapshot.stores.continuity.confirm(
                        grant,
                        &access,
                        &snapshot.stores.work,
                        now(),
                    )?;
                    let native = continuity_scope(&scope);
                    if let Some(gate) = state.gate_for(&native) {
                        let supersedes = if let Some(receipt) =
                            old.as_ref().and_then(|c| c.episode_receipt.as_ref())
                        {
                            if gate.verify_memory(&native, receipt).await
                                != MemoryCandidateState::Live
                            {
                                return Err(WorkError::Stale);
                            }
                            Some(
                                crate::episode_gate::parse_digest(receipt)
                                    .ok_or(WorkError::Invalid)?,
                            )
                        } else {
                            None
                        };
                        if supersedes.is_some() {
                            let access = state
                                .fresh_continuity_access(&scope, actor, channel)
                                .await?;
                            if now() >= grant_expiry
                                || state.continuity_generation(&scope) != Some(generation)
                            {
                                return Err(WorkError::Stale);
                            }
                            let stores = Self::lock(&state.stores);
                            if stores.continuity.card(&scope) != old.as_ref() {
                                return Err(WorkError::Stale);
                            }
                            stores.work.scope_projects(&scope, access, true)?;
                            snapshot
                                .stores
                                .continuity
                                .context(&scope, &access, &stores.work, now())
                                .ok_or(WorkError::Stale)?;
                        }
                        let request = MemoryCandidateRequest {
                            scoped_guild: native,
                            class: MemoryClass::Summary,
                            retention: RetentionClass::Durable,
                            payload: serde_json::to_vec(&card).map_err(|_| WorkError::Invalid)?,
                            member_scoped: matches!(scope, WorkScope::Personal { .. }),
                            supersedes,
                            forgets: None,
                            now: now(),
                            nonce: gate.next_nonce(),
                        };
                        let GateOutcome::Appended { digest_hex, .. } =
                            gate.record_memory_candidate(request).await
                        else {
                            return Err(WorkError::Persistence);
                        };
                        card = snapshot
                            .stores
                            .continuity
                            .install_receipt(&card, digest_hex)?;
                    } else if old.as_ref().is_some_and(|c| c.episode_receipt.is_some()) {
                        return Err(WorkError::Persistence);
                    }
                    let access = state.fresh_continuity_access(&scope, actor, channel).await;
                    let final_check = access.and_then(|access| {
                        if now() >= grant_expiry
                            || state.continuity_generation(&scope) != Some(generation)
                        {
                            return Err(WorkError::Stale);
                        }
                        let stores = Self::lock(&state.stores);
                        if stores.continuity.card(&scope) != old.as_ref() {
                            return Err(WorkError::Stale);
                        }
                        stores.work.scope_projects(&scope, access, true)?;
                        snapshot
                            .stores
                            .continuity
                            .context(&scope, &access, &stores.work, now())
                            .ok_or(WorkError::Stale)?;
                        Ok(())
                    });
                    if let Err(error) = final_check {
                        if card.episode_receipt.is_some() {
                            state.fence_continuity(std::slice::from_ref(&card))?;
                            Self::lock(&state.continuity_safety)
                                .orphans
                                .insert(scope, card);
                        }
                        return Err(error);
                    }
                    state.stage_continuity(&scope, old, Some(card.clone()), None)?;
                    state.publish_continuity(snapshot, &scope).await?;
                    Ok(card)
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        owner.await.map_err(|_| WorkError::Persistence)?
    }
    pub(crate) async fn clear_continuity(
        &self,
        scope: WorkScope,
        actor: u64,
        channel: u64,
    ) -> Result<usize, WorkError> {
        if self.data_dir.is_none() || self.persistence_requests.get().is_none() {
            return Err(WorkError::Persistence);
        }
        let state = self.owned_state().ok_or(WorkError::Persistence)?;
        let owner = self
            .service_registry()
            .ok_or(WorkError::Persistence)?
            .spawn_result(
                crate::service::OperationKind::PersistencePreparation,
                async move {
                    let _serial = state.persistence_preparation.lock().await;
                    let access = state
                        .fresh_continuity_access(&scope, actor, channel)
                        .await?;
                    {
                        let stores = Self::lock(&state.stores);
                        stores.work.scope_projects(&scope, access, true)?;
                    }
                    let reconciled = state.reconcile_continuity(&scope).await?;
                    state
                        .erase_continuity_orphan(&scope, Some((actor, channel)))
                        .await?;
                    let access = state
                        .fresh_continuity_access(&scope, actor, channel)
                        .await?;
                    let card = {
                        let stores = Self::lock(&state.stores);
                        let mut check = stores.continuity.clone();
                        check.clear(&scope, &access, &stores.work)?
                    };
                    let targets: Vec<_> = card.into_iter().collect();
                    let removed = state
                        .erase_continuity_targets(&targets, Some((actor, channel)))
                        .await?;
                    state.unfence_continuity(&scope);
                    Ok(removed + reconciled)
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        owner.await.map_err(|_| WorkError::Persistence)?
    }
    /// Caller already owns persistence_preparation and an observed service task.
    pub(super) async fn erase_continuity_owned(
        &self,
        scope: &str,
        member: Option<u64>,
    ) -> Result<usize, WorkError> {
        let work = Self::lock(&self.stores).work.clone();
        let orphan_scopes: Vec<_> = {
            let safety = Self::lock(&self.continuity_safety);
            safety
                .orphans
                .values()
                .filter(|c| continuity_scope(&c.scope) == scope && c.linked_to(member, &work))
                .map(|c| c.scope.clone())
                .collect()
        };
        for target in &orphan_scopes {
            self.erase_continuity_orphan(target, None).await?;
        }
        let pending_scopes: Vec<_> = {
            let safety = Self::lock(&self.continuity_safety);
            safety
                .pending
                .iter()
                .filter(|(s, op)| {
                    continuity_scope(s) == scope
                        && op
                            .expected
                            .iter()
                            .chain(op.desired.iter())
                            .any(|card| card.linked_to(member, &work))
                })
                .map(|(scope, _)| scope.clone())
                .collect()
        };
        let mut reconciled = 0;
        for target in pending_scopes {
            reconciled += self.reconcile_continuity(&target).await?;
        }
        let cards = {
            let stores = Self::lock(&self.stores);
            stores
                .continuity
                .erasure_targets(scope, member, &stores.work)
        };
        let count = self.erase_continuity_targets(&cards, None).await?;
        for target in orphan_scopes {
            self.unfence_continuity(&target);
        }
        Ok(count + reconciled)
    }
    async fn erase_continuity_targets(
        &self,
        cards: &[ContinuityCard],
        native_actor: Option<(u64, u64)>,
    ) -> Result<usize, WorkError> {
        self.fence_continuity(cards)?;
        let mut removed = 0;
        for card in cards {
            if self
                .snapshot_without_proposals()
                .stores
                .canonical_preparation_failed
            {
                return Err(WorkError::Persistence);
            }
            let memo = self.admit_continuity_forget(card, native_actor).await?;
            if let Some((actor, channel)) = native_actor {
                let access = self
                    .fresh_continuity_access(&card.scope, actor, channel)
                    .await?;
                Self::lock(&self.stores)
                    .work
                    .scope_projects(&card.scope, access, true)?;
            }
            let mut snapshot = self.snapshot_without_proposals();
            if snapshot.stores.canonical_preparation_failed {
                return Err(WorkError::Persistence);
            }
            snapshot.stores.continuity.remove_exact(card)?;
            self.stage_continuity(&card.scope, Some(card.clone()), None, memo.clone())?;
            self.publish_continuity(snapshot, &card.scope).await?;
            let mut safety = Self::lock(&self.continuity_safety);
            if let Some(key) = memo {
                safety.forgets.remove(&key);
            }
            removed += 1;
        }
        Ok(removed)
    }
    fn fence_continuity(&self, cards: &[ContinuityCard]) -> Result<(), WorkError> {
        let mut safety = Self::lock(&self.continuity_safety);
        safety.generation = safety.generation.and_then(|g| g.checked_add(1));
        if safety.generation.is_none() {
            return Err(WorkError::Full);
        }
        for card in cards {
            safety.blocked.insert(card.scope.clone());
        }
        if safety.blocked.len() > 256 {
            safety.generation = None;
            return Err(WorkError::Full);
        }
        Ok(())
    }
    async fn admit_continuity_forget(
        &self,
        card: &ContinuityCard,
        native_actor: Option<(u64, u64)>,
    ) -> Result<Option<String>, WorkError> {
        use sha2::{Digest as _, Sha256};
        let native = continuity_scope(&card.scope);
        let Some(receipt) = &card.episode_receipt else {
            if let Some(gate) = self.gate_for(&native) {
                gate.note_ungated_forget();
            }
            return Ok(None);
        };
        let gate = self.gate_for(&native).ok_or(WorkError::Persistence)?;
        let encoded = serde_json::to_vec(card).map_err(|_| WorkError::Invalid)?;
        let fingerprint: String = Sha256::digest(encoded)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let key = format!("{fingerprint}:{:p}", Arc::as_ptr(gate));
        {
            let mut safety = Self::lock(&self.continuity_safety);
            if safety.forgets.get(&key).is_some_and(Option::is_some) {
                return Ok(Some(key));
            }
            if !safety.forgets.contains_key(&key) && safety.forgets.len() >= 256 {
                return Err(WorkError::Full);
            }
            safety.forgets.entry(key.clone()).or_insert(None);
        }
        match gate.verify_memory(&native, receipt).await {
            MemoryCandidateState::Forgotten => Ok(Some(key)),
            MemoryCandidateState::Unknown => Err(WorkError::Persistence),
            MemoryCandidateState::Live => {
                if let Some((actor, channel)) = native_actor {
                    let access = self
                        .fresh_continuity_access(&card.scope, actor, channel)
                        .await?;
                    Self::lock(&self.stores)
                        .work
                        .scope_projects(&card.scope, access, true)?;
                }
                let request = MemoryCandidateRequest {
                    scoped_guild: native,
                    class: MemoryClass::Summary,
                    retention: RetentionClass::Durable,
                    payload: Vec::new(),
                    member_scoped: matches!(card.scope, WorkScope::Personal { .. }),
                    supersedes: None,
                    forgets: Some(
                        crate::episode_gate::parse_digest(receipt).ok_or(WorkError::Invalid)?,
                    ),
                    now: now(),
                    nonce: gate.next_nonce(),
                };
                let result = gate.record_memory_candidate(request).await;
                if !matches!(result, GateOutcome::Appended { .. }) {
                    return Err(WorkError::Persistence);
                }
                Self::lock(&self.continuity_safety)
                    .forgets
                    .insert(key.clone(), Some(result));
                Ok(Some(key))
            }
        }
    }
    async fn publish_continuity(
        &self,
        snapshot: crate::service::persistence::Snapshot,
        scope: &WorkScope,
    ) -> Result<(), WorkError> {
        if snapshot.stores.canonical_preparation_failed {
            return Err(WorkError::Persistence);
        }
        let dir = self.data_dir.clone().ok_or(WorkError::Persistence)?;
        let expected = serde_json::to_vec(&snapshot.stores).map_err(|_| WorkError::Persistence)?;
        let desired = snapshot.stores.continuity.card(scope).cloned();
        let report = self
            .persistence_requests
            .get()
            .ok_or(WorkError::Persistence)?
            .submit(snapshot)
            .await
            .map_err(|_| WorkError::Persistence)?;
        let exact = tokio::task::spawn_blocking(move || {
            if std::fs::read(Stores::state_path(&dir)).map_err(|_| WorkError::Persistence)?
                != expected
            {
                return Err(WorkError::Persistence);
            }
            Stores::load(&dir).map_err(|_| WorkError::Persistence)
        })
        .await
        .map_err(|_| WorkError::Persistence)??;
        Self::lock(&self.stores)
            .canonical_base
            .set(exact.canonical_base.get());
        let pending = Self::lock(&self.continuity_safety)
            .pending
            .get(scope)
            .cloned()
            .ok_or(WorkError::Stale)?;
        Self::lock(&self.stores).continuity.replace_exact(
            scope,
            pending.expected.as_ref(),
            desired.as_ref(),
        )?;
        {
            let mut safety = Self::lock(&self.continuity_safety);
            safety.pending.remove(scope);
            if let Some(key) = pending.forget_memo {
                safety.forgets.remove(&key);
            }
        }
        self.unfence_continuity(scope);
        if report.canonical_state != crate::persist::PersistComponentOutcome::Committed {
            return Err(WorkError::Persistence);
        }
        Ok(())
    }
    fn reserve_continuity_recovery(&self, scope: &WorkScope) -> Result<(), WorkError> {
        let safety = Self::lock(&self.continuity_safety);
        if safety.pending.contains_key(scope) || safety.orphans.contains_key(scope) {
            return Err(WorkError::Stale);
        }
        if safety.pending.len() + safety.orphans.len() >= 256
            || (safety.blocked.len() >= 256 && !safety.blocked.contains(scope))
        {
            return Err(WorkError::Full);
        }
        Ok(())
    }
    fn stage_continuity(
        &self,
        scope: &WorkScope,
        expected: Option<ContinuityCard>,
        desired: Option<ContinuityCard>,
        forget_memo: Option<String>,
    ) -> Result<(), WorkError> {
        let mut safety = Self::lock(&self.continuity_safety);
        if !safety.pending.contains_key(scope) && safety.pending.len() + safety.orphans.len() >= 256
        {
            return Err(WorkError::Full);
        }
        safety.generation = safety.generation.and_then(|g| g.checked_add(1));
        safety.blocked.insert(scope.clone());
        safety.pending.insert(
            scope.clone(),
            PendingPublication {
                expected,
                desired,
                forget_memo,
            },
        );
        if safety.generation.is_none() {
            return Err(WorkError::Full);
        }
        Ok(())
    }
    pub(super) fn overlay_continuity(&self, cards: &mut ContinuityStore) -> Result<(), WorkError> {
        let mut safety = Self::lock(&self.continuity_safety);
        let mut candidate = cards.clone();
        for (scope, op) in &safety.pending {
            if candidate
                .replace_exact(scope, op.expected.as_ref(), op.desired.as_ref())
                .is_err()
            {
                safety.generation = None;
                return Err(WorkError::Persistence);
            }
        }
        *cards = candidate;
        Ok(())
    }
    async fn reconcile_continuity(&self, scope: &WorkScope) -> Result<usize, WorkError> {
        let operation = Self::lock(&self.continuity_safety)
            .pending
            .get(scope)
            .cloned();
        if let Some(operation) = operation {
            self.publish_continuity(self.snapshot_without_proposals(), scope)
                .await?;
            return Ok(usize::from(
                operation.expected.is_some() && operation.desired.is_none(),
            ));
        }
        Ok(0)
    }
    async fn erase_continuity_orphan(
        &self,
        scope: &WorkScope,
        native_actor: Option<(u64, u64)>,
    ) -> Result<(), WorkError> {
        let orphan = Self::lock(&self.continuity_safety)
            .orphans
            .get(scope)
            .cloned();
        if let Some(card) = orphan {
            let memo = self.admit_continuity_forget(&card, native_actor).await?;
            if let Some((actor, channel)) = native_actor {
                let access = self.fresh_continuity_access(scope, actor, channel).await?;
                Self::lock(&self.stores)
                    .work
                    .scope_projects(scope, access, true)?;
            }
            let mut safety = Self::lock(&self.continuity_safety);
            if safety.orphans.get(scope) != Some(&card) {
                return Err(WorkError::Stale);
            }
            safety.orphans.remove(scope);
            if let Some(key) = memo {
                safety.forgets.remove(&key);
            }
        }
        Ok(())
    }
    fn unfence_continuity(&self, scope: &WorkScope) {
        let mut safety = Self::lock(&self.continuity_safety);
        if !safety.pending.contains_key(scope) && !safety.orphans.contains_key(scope) {
            safety.blocked.remove(scope);
        }
    }
}
pub(super) fn continuity_scope(scope: &WorkScope) -> String {
    match scope {
        WorkScope::Personal { owner } => format!("discord:dm:{owner}"),
        WorkScope::Team { guild, .. } => format!("discord:{guild}"),
    }
}
#[cfg(test)]
mod tests;
