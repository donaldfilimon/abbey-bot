//! Complete retained personal-memory transactions. No Stores lock crosses disk IO.
use super::AppState;
use crate::{
    persist::{self, personal_memory as journal},
    personal_memory::*,
};
use std::sync::atomic::Ordering;
mod mutation;
mod reconciliation;
use mutation::{
    apply_mutation, exact_readback, memory_rows, receipt_key, subject_epoch, validate_action,
};
pub(super) use reconciliation::ReconciliationState;
use reconciliation::{MarkerRetirement, install_fact_rows};
pub(super) struct PendingRequest {
    digest: String,
    result: std::sync::Mutex<Option<Result<ConsentStatus, MemoryConsentError>>>,
    changed: tokio::sync::Notify,
    #[cfg(test)]
    preparation_queued: std::sync::atomic::AtomicBool,
}
impl PendingRequest {
    async fn wait(&self) -> Result<ConsentStatus, MemoryConsentError> {
        loop {
            let changed = self.changed.notified();
            if let Some(result) = AppState::lock(&self.result).clone() {
                return result;
            }
            changed.await;
        }
    }
}
struct PendingOwner {
    state: std::sync::Arc<AppState>,
    key: String,
    pending: std::sync::Arc<PendingRequest>,
}
impl Drop for PendingOwner {
    fn drop(&mut self) {
        self.state.settle_personal_pending(
            &self.key,
            &self.pending,
            Err(MemoryConsentError::Persistence),
        );
    }
}
impl AppState {
    pub fn personal_memory_exposure_epoch(&self) -> u64 {
        Self::lock(&self.stores).personal_memory_exposure.epoch
    }
    pub fn personal_memory_status(&self, guild: &str, user: &str) -> ConsentStatus {
        let stores = Self::lock(&self.stores);
        let mut result = status(&stores, guild, user);
        if self.personal_memory_blocked.load(Ordering::Acquire) {
            result.choice = UseChoice::Off;
            result.unverified_facts += result.eligible_facts;
            result.eligible_facts = 0;
        }
        result
    }
    pub fn personal_memory_permits(&self, guild: &str, user: &str) -> MemoryUsePermitSet {
        let stores = Self::lock(&self.stores);
        if self.personal_memory_blocked.load(Ordering::Acquire) {
            return MemoryUsePermitSet::empty(stores.personal_memory_exposure.epoch);
        }
        let fallback = PersonalMemorySubject::default();
        MemoryUsePermitSet::for_subject(
            guild,
            user,
            stores
                .personal_memory
                .get(&subject_key(guild, user))
                .unwrap_or(&fallback),
            stores.personal_memory_exposure.epoch,
        )
    }
    pub fn validate_personal_memory_permits(&self, permits: &MemoryUsePermitSet) -> bool {
        !self.personal_memory_blocked.load(Ordering::Acquire) && {
            let stores = Self::lock(&self.stores);
            permits.validate(
                &stores.personal_memory,
                stores.personal_memory_exposure.epoch,
            )
        }
    }
    pub async fn set_personal_memory_use(
        &self,
        action: SelfAuthorizedFactAction,
        choice: UseChoice,
        request_id: String,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        self.commit_personal_memory(action, Mutation::Choice(choice), request_id)
            .await
    }
    pub async fn confirm_personal_memory_fact(
        &self,
        action: SelfAuthorizedFactAction,
        key: String,
        exact: String,
        request_id: String,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        self.commit_personal_memory(action, Mutation::Confirm { key, exact }, request_id)
            .await
    }
    pub async fn remember_personal_memory_fact(
        &self,
        action: SelfAuthorizedFactAction,
        text: String,
        request_id: String,
        receipt: Option<String>,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        self.commit_personal_memory(action, Mutation::Remember { text, receipt }, request_id)
            .await
    }
    pub async fn correct_personal_memory_fact(
        &self,
        action: SelfAuthorizedFactAction,
        old: String,
        text: String,
        request_id: String,
        receipt: Option<String>,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        self.commit_personal_memory(action, Mutation::Correct { old, text, receipt }, request_id)
            .await
    }
    pub async fn forget_personal_memory_fact(
        &self,
        action: SelfAuthorizedFactAction,
        exact: String,
        request_id: String,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        self.commit_personal_memory(action, Mutation::Forget { exact }, request_id)
            .await
    }
    async fn commit_personal_memory(
        &self,
        action: SelfAuthorizedFactAction,
        mutation: Mutation,
        request_id: String,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        self.commit_personal_request(action, mutation, request_id)
            .await
    }
    fn settle_personal_pending(
        &self,
        key: &str,
        pending: &std::sync::Arc<PendingRequest>,
        result: Result<ConsentStatus, MemoryConsentError>,
    ) {
        let mut slot = Self::lock(&pending.result);
        if slot.is_none() {
            *slot = Some(result);
        }
        drop(slot);
        let mut inflight = Self::lock(&self.personal_memory_pending);
        if inflight
            .get(key)
            .is_some_and(|existing| std::sync::Arc::ptr_eq(existing, pending))
        {
            inflight.remove(key);
        }
        drop(inflight);
        pending.changed.notify_waiters();
    }
    async fn commit_personal_request(
        &self,
        action: SelfAuthorizedFactAction,
        mutation: Mutation,
        request_id: String,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        if request_id.is_empty() || request_id.len() > 64 {
            return Err(MemoryConsentError::Bounds);
        }
        let digest = request_digest(&action, &mutation)?;
        let pending_key = format!(
            "{}\u{1f}{}",
            subject_key(&action.proof.guild, &action.proof.subject),
            request_id
        );
        {
            let stores = Self::lock(&self.stores);
            if let Some(record) = stores
                .personal_memory
                .get(&subject_key(&action.proof.guild, &action.proof.subject))
                .and_then(|s| s.outcomes.get(&request_id))
                && record.completed
            {
                return if record.payload_digest == digest {
                    Ok(record.result.clone())
                } else {
                    Err(MemoryConsentError::RequestConflict)
                };
            }
        }
        let (pending, first) = {
            let mut table = Self::lock(&self.personal_memory_pending);
            if let Some(existing) = table.get(&pending_key) {
                if existing.digest != digest {
                    return Err(MemoryConsentError::RequestConflict);
                }
                (existing.clone(), false)
            } else {
                if table.len() >= MAX_SUBJECTS {
                    if matches!(mutation, Mutation::Choice(UseChoice::Off)) {
                        validate_action(&action, super::now())?;
                        let mut stores = Self::lock(&self.stores);
                        if status(&stores, &action.proof.guild, &action.proof.subject).stamp
                            != action.expected
                        {
                            return Err(MemoryConsentError::Stale);
                        }
                        self.personal_memory_blocked.store(true, Ordering::Release);
                        if let Some(subject) = stores
                            .personal_memory
                            .get_mut(&subject_key(&action.proof.guild, &action.proof.subject))
                        {
                            subject.choice = UseChoice::Off;
                            subject.activation_pending = false;
                            subject.advance()?;
                        }
                        stores.personal_memory_exposure.epoch = stores
                            .personal_memory_exposure
                            .epoch
                            .checked_add(1)
                            .ok_or(MemoryConsentError::Bounds)?;
                    }
                    return Err(MemoryConsentError::Bounds);
                }
                let pending = std::sync::Arc::new(PendingRequest {
                    digest: digest.clone(),
                    result: std::sync::Mutex::new(None),
                    changed: tokio::sync::Notify::new(),
                    #[cfg(test)]
                    preparation_queued: std::sync::atomic::AtomicBool::new(false),
                });
                table.insert(pending_key.clone(), pending.clone());
                (pending, true)
            }
        };
        if !first {
            return pending.wait().await;
        }
        let preparation = (|| {
            let guild = &action.proof.guild;
            let user = &action.proof.subject;
            let key = subject_key(guild, user);
            let mut allowed = action.expected;
            {
                let mut stores = Self::lock(&self.stores);
                if let Some(record) = stores
                    .personal_memory
                    .get(&key)
                    .and_then(|s| s.outcomes.get(&request_id))
                {
                    if !record.completed {
                        if record.payload_digest != digest {
                            return Err(MemoryConsentError::RequestConflict);
                        }
                    } else {
                        return if record.payload_digest == digest {
                            Ok((None, Some(record.result.clone())))
                        } else {
                            Err(MemoryConsentError::RequestConflict)
                        };
                    }
                }
                let current = status(&stores, guild, user);
                if current.stamp == action.expected
                    && matches!(mutation, Mutation::Choice(UseChoice::Off))
                {
                    validate_action(&action, super::now())?;
                    self.personal_memory_blocked.store(true, Ordering::Release);
                    if !stores.personal_memory.contains_key(&key)
                        && stores.personal_memory.len() >= MAX_SUBJECTS
                    {
                        return Err(MemoryConsentError::Bounds);
                    }
                    let s = stores.personal_memory.entry(key.clone()).or_default();
                    s.choice = UseChoice::Off;
                    s.activation_pending = false;
                    s.advance()?;
                    stores.personal_memory_exposure.epoch = stores
                        .personal_memory_exposure
                        .epoch
                        .checked_add(1)
                        .ok_or(MemoryConsentError::Bounds)?;
                    stores.personal_memory_exposure.cutoff = stores
                        .personal_memory_exposure
                        .cutoff
                        .max(super::now().max(1));
                    allowed = status(&stores, guild, user).stamp;
                    Self::lock(&self.personal_memory_reconciliation).deny(
                        key.clone(),
                        request_id.clone(),
                        allowed,
                        stores.personal_memory_exposure.cutoff,
                    );
                }
            }
            Ok((Some(allowed), None))
        })();
        let (allowed, completed) = match preparation {
            Ok(value) => value,
            Err(error) => {
                self.settle_personal_pending(&pending_key, &pending, Err(error.clone()));
                return Err(error);
            }
        };
        if let Some(result) = completed {
            self.settle_personal_pending(&pending_key, &pending, Ok(result.clone()));
            return Ok(result);
        }
        let allowed = allowed.ok_or(MemoryConsentError::Persistence)?;
        let state = match self.owned_state() {
            Some(state) => state,
            None => {
                self.settle_personal_pending(
                    &pending_key,
                    &pending,
                    Err(MemoryConsentError::Persistence),
                );
                return Err(MemoryConsentError::Persistence);
            }
        };
        let Some(registry) = self.service.get() else {
            self.settle_personal_pending(
                &pending_key,
                &pending,
                Err(MemoryConsentError::Persistence),
            );
            return Err(MemoryConsentError::Persistence);
        };
        let owner = PendingOwner {
            state: state.clone(),
            key: pending_key.clone(),
            pending: pending.clone(),
        };
        let retained = match registry.blocking_result(
            crate::service::OperationKind::PersistencePreparation,
            move || {
                #[cfg(not(test))]
                let _serial = state.persistence_preparation.blocking_lock();
                #[cfg(test)]
                let _serial = tokio::runtime::Handle::current().block_on(async {
                    let lock = state.persistence_preparation.lock();
                    tokio::pin!(lock);
                    std::future::poll_fn(|cx| {
                        let result = std::future::Future::poll(lock.as_mut(), cx);
                        // Signal after polling: a pending result is already in Tokio's FIFO.
                        owner
                            .pending
                            .preparation_queued
                            .store(true, Ordering::Release);
                        result
                    })
                    .await
                });
                let result = state
                    .personal_memory_transaction(action, mutation, request_id, digest, allowed);
                state.settle_personal_pending(&owner.key, &owner.pending, result.clone());
                result
            },
        ) {
            Ok(result) => result,
            Err(_) => {
                self.settle_personal_pending(
                    &pending_key,
                    &pending,
                    Err(MemoryConsentError::Persistence),
                );
                return Err(MemoryConsentError::Persistence);
            }
        };
        retained
            .await
            .map_err(|_| MemoryConsentError::Persistence)?
    }
    fn qualify_personal_projection(
        &self,
        candidate: &persist::Stores,
        dir: &std::path::Path,
    ) -> Result<crate::wdbx::Recall, MemoryConsentError> {
        let mut projection = Self::lock(&self.recall).clone();
        projection.reconcile_memory_facts(
            candidate
                .memory
                .fact_records()
                .into_iter()
                .map(|f| (f.guild, f.user, f.text, f.at)),
        );
        persist::persist_projection(&*self.persistence_sink, dir, &projection)
            .map_err(|_| MemoryConsentError::Persistence)?;
        let disk = crate::wdbx::Recall::load(&persist::Stores::wdbx_path(dir))
            .map_err(|_| MemoryConsentError::Persistence)?;
        if disk.all_memory_facts() != projection.all_memory_facts() {
            return Err(MemoryConsentError::Persistence);
        }
        Ok(projection)
    }
    fn install_personal_candidate(
        &self,
        candidate: &persist::Stores,
        expected: &persist::Stores,
        clear_barrier: bool,
    ) -> Result<(), MemoryConsentError> {
        let mut live = Self::lock(&self.stores);
        if live.personal_memory != expected.personal_memory
            || live.personal_memory_exposure != expected.personal_memory_exposure
            || candidate.personal_memory_exposure.epoch < live.personal_memory_exposure.epoch
            || candidate.personal_memory_exposure.cutoff < live.personal_memory_exposure.cutoff
        {
            return Err(MemoryConsentError::Stale);
        }
        live.canonical_base.set(candidate.canonical_base.get());
        install_fact_rows(&mut live, candidate);
        live.memory_receipts = candidate.memory_receipts.clone();
        live.personal_memory = candidate.personal_memory.clone();
        live.personal_memory_exposure = candidate.personal_memory_exposure.clone();
        let mut reconciliation = Self::lock(&self.personal_memory_reconciliation);
        reconciliation.retire_denials(candidate);
        reconciliation.publication = None;
        if clear_barrier && reconciliation.denials_cleared() {
            self.personal_memory_blocked.store(false, Ordering::Release);
        }
        Ok(())
    }
    fn personal_memory_transaction(
        &self,
        action: SelfAuthorizedFactAction,
        mutation: Mutation,
        request_id: String,
        digest: String,
        allowed: ConsentStamp,
    ) -> Result<ConsentStatus, MemoryConsentError> {
        let _mutation = Self::lock(&self.personal_memory_mutation);
        let dir = self
            .data_dir
            .as_ref()
            .ok_or(MemoryConsentError::Persistence)?;
        let owner = journal::lease(dir)?;
        let guild = &action.proof.guild;
        let user = &action.proof.subject;
        let key = subject_key(guild, user);
        let mut marker = journal::load(dir)?;
        let disk = persist::Stores::load(dir).map_err(|_| MemoryConsentError::Persistence)?;
        let existing = disk
            .personal_memory
            .get(&key)
            .and_then(|s| s.outcomes.get(&request_id));
        let denial = marker
            .withdrawals
            .get(&key)
            .filter(|w| w.request_id == request_id);
        let activation = marker
            .activations
            .get(&key)
            .filter(|w| w.request_id == request_id);
        if existing.is_some() || denial.is_some() || activation.is_some() {
            self.personal_memory_blocked.store(true, Ordering::Release);
            if existing.is_some_and(|r| r.payload_digest != digest)
                || denial.is_some_and(|w| w.payload_digest != digest)
                || activation.is_some_and(|w| w.payload_digest != digest)
            {
                return Err(MemoryConsentError::RequestConflict);
            }
            let live = { Self::lock(&self.stores).clone() };
            // An unchanged snapshot may already include a newer immediate withdrawal.
            // Its authority survives request failure and may not be consumed by this replay.
            if Self::lock(&self.personal_memory_reconciliation).has_other_denial(&key, &request_id)
            {
                return Err(MemoryConsentError::Stale);
            }
            // Only our exact retained publication may contribute an uninstalled delta.
            let known = Self::lock(&self.personal_memory_reconciliation)
                .publication
                .clone();
            let mut repaired = if let Some(known) = known {
                if !known.matches(&live, &disk) {
                    return Err(MemoryConsentError::Stale);
                }
                let mut reconciled = live.clone();
                known.rebase(&mut reconciled, &disk)?;
                reconciled
            } else {
                // Restart/lost-ack replay may repair its own delta, never unrelated facts.
                if live.canonical_base.get() != disk.canonical_base.get()
                    && memory_rows(&live) != memory_rows(&disk)
                {
                    let mut intended = live.clone();
                    intended.personal_memory.entry(key.clone()).or_default();
                    apply_mutation(&mut intended, &action, &mutation)?;
                    if memory_rows(&intended) != memory_rows(&disk) {
                        return Err(MemoryConsentError::Stale);
                    }
                }
                disk
            };
            let aborted_on = activation.is_some()
                || (matches!(mutation, Mutation::Choice(UseChoice::On)) && denial.is_some());
            journal::fold_denials(&mut repaired, &marker)?;
            if aborted_on {
                self.publish_personal_replay(&owner, dir, &mut repaired, &live, marker, &action)?;
                return Err(MemoryConsentError::Blocked);
            }
            if !repaired
                .personal_memory
                .get(&key)
                .is_some_and(|s| s.outcomes.contains_key(&request_id))
            {
                if !matches!(mutation, Mutation::Choice(UseChoice::Off)) {
                    return Err(MemoryConsentError::Blocked);
                }
                let result = status(&repaired, guild, user);
                let subject = repaired
                    .personal_memory
                    .get_mut(&key)
                    .ok_or(MemoryConsentError::Persistence)?;
                if subject.outcomes.len() >= 128 {
                    return Err(MemoryConsentError::Bounds);
                }
                subject.outcomes.insert(
                    request_id.clone(),
                    CompletedRequest {
                        completed: false,
                        payload_digest: digest.clone(),
                        at: action.proof.at,
                        result,
                    },
                );
            }
            let result = repaired
                .personal_memory
                .get(&key)
                .and_then(|subject| subject.outcomes.get(&request_id))
                .ok_or(MemoryConsentError::Persistence)?
                .result
                .clone();
            self.publish_personal_replay(&owner, dir, &mut repaired, &live, marker, &action)?;
            return Ok(result);
        }
        validate_action(&action, super::now())?;
        let mut candidate = { Self::lock(&self.stores).clone() };
        if status(&candidate, guild, user).stamp != allowed {
            return Err(MemoryConsentError::Stale);
        }
        let live_base = candidate.clone();
        if disk.canonical_base.get() != candidate.canonical_base.get() {
            let known = Self::lock(&self.personal_memory_reconciliation)
                .publication
                .clone();
            if matches!(mutation, Mutation::Choice(UseChoice::Off))
                && let Some(known) = known.filter(|known| known.matches(&candidate, &disk))
            {
                known.rebase(&mut candidate, &disk)?;
            } else {
                if matches!(mutation, Mutation::Choice(UseChoice::Off)) {
                    journal::add_withdrawal(
                        &mut marker,
                        guild,
                        user,
                        journal::Withdrawal {
                            guild: guild.clone(),
                            user: user.clone(),
                            epoch: allowed.consent_epoch.max(1),
                            minimum_revision: allowed.revision.max(1),
                            exposure_epoch: allowed.exposure_epoch.max(1),
                            cutoff: candidate
                                .personal_memory_exposure
                                .cutoff
                                .max(super::now().max(1)),
                            request_id: request_id.clone(),
                            payload_digest: digest.clone(),
                            at: action.proof.at,
                        },
                    )?;
                    journal::publish_owned(&owner, dir, marker.revision, marker)?;
                }
                return Err(MemoryConsentError::Stale);
            }
        }
        validate_metadata(
            &candidate.personal_memory,
            &candidate.personal_memory_exposure,
        )?;
        if !candidate.personal_memory.contains_key(&key)
            && candidate.personal_memory.len() >= MAX_SUBJECTS
        {
            return Err(MemoryConsentError::Bounds);
        }
        let is_off = matches!(mutation, Mutation::Choice(UseChoice::Off));
        let is_on = matches!(mutation, Mutation::Choice(UseChoice::On));
        if !is_off
            && (!marker.withdrawals.is_empty()
                || !marker.activations.is_empty()
                || !Self::lock(&self.personal_memory_reconciliation).denials_cleared())
        {
            return Err(MemoryConsentError::Blocked);
        }
        candidate.personal_memory.entry(key.clone()).or_default();
        apply_mutation(&mut candidate, &action, &mutation)?;
        let s = candidate
            .personal_memory
            .get_mut(&key)
            .ok_or(MemoryConsentError::Persistence)?;
        s.schema = 1;
        s.policy_version = PERSONAL_MEMORY_POLICY_VERSION;
        s.advance()?;
        candidate.personal_memory_exposure.schema = 1;
        candidate.personal_memory_exposure.epoch = candidate
            .personal_memory_exposure
            .epoch
            .checked_add(1)
            .ok_or(MemoryConsentError::Bounds)?;
        candidate.personal_memory_exposure.cutoff = candidate
            .personal_memory_exposure
            .cutoff
            .max(super::now().max(1));
        let exposure = &mut candidate.personal_memory_exposure;
        if !exposure.scope_epochs.contains_key(&key) && exposure.scope_epochs.len() >= MAX_SUBJECTS
        {
            return Err(MemoryConsentError::Bounds);
        }
        exposure.scope_epochs.insert(key.clone(), exposure.epoch);
        exposure.receipts.push(ExposureReceipt {
            epoch: exposure.epoch,
            cutoff: exposure.cutoff,
            request_digest: digest.clone(),
        });
        if exposure.receipts.len() > 128 {
            exposure.receipts.remove(0);
        }
        let mut result = status(&candidate, guild, user);
        let record = CompletedRequest {
            completed: false,
            payload_digest: digest.clone(),
            at: super::now(),
            result: result.clone(),
        };
        let subject = candidate
            .personal_memory
            .get_mut(&key)
            .ok_or(MemoryConsentError::Persistence)?;
        if subject.outcomes.len() >= 128
            && let Some(oldest) = subject
                .outcomes
                .iter()
                .min_by_key(|(id, r)| (r.at, *id))
                .map(|(id, _)| id.clone())
        {
            subject.outcomes.remove(&oldest);
        }
        subject.outcomes.insert(request_id.clone(), record);
        validate_metadata(
            &candidate.personal_memory,
            &candidate.personal_memory_exposure,
        )?;
        let w = journal::Withdrawal {
            guild: guild.clone(),
            user: user.clone(),
            epoch: subject_epoch(&candidate, &key)?,
            minimum_revision: result.stamp.revision,
            exposure_epoch: result.stamp.exposure_epoch,
            cutoff: candidate.personal_memory_exposure.cutoff,
            request_id: request_id.clone(),
            payload_digest: digest,
            at: action.proof.at,
        };
        if is_off {
            journal::add_withdrawal(&mut marker, guild, user, w.clone())?;
            marker = journal::publish_owned(&owner, dir, marker.revision, marker)?;
            journal::fold_denials(&mut candidate, &marker)?;
            result = status(&candidate, guild, user);
            candidate
                .personal_memory
                .get_mut(&key)
                .ok_or(MemoryConsentError::Persistence)?
                .outcomes
                .get_mut(&request_id)
                .ok_or(MemoryConsentError::Persistence)?
                .result = result.clone();
        }
        if is_on {
            if marker.withdrawals.len() + marker.activations.len() >= MAX_SUBJECTS {
                return Err(MemoryConsentError::Bounds);
            }
            marker.activations.insert(key.clone(), w.clone());
            marker = journal::publish_owned(&owner, dir, marker.revision, marker)?;
            candidate
                .personal_memory
                .get_mut(&key)
                .ok_or(MemoryConsentError::Persistence)?
                .activation_pending = true;
        }
        validate_metadata(
            &candidate.personal_memory,
            &candidate.personal_memory_exposure,
        )?;
        let publication = (|| {
            let qualified = self.publish_personal_candidate(
                reconciliation::PublicationContext {
                    owner: &owner,
                    dir,
                    expected: &live_base,
                    action: &action,
                },
                &mut candidate,
                &mut marker,
                MarkerRetirement::Subject {
                    key: &key,
                    off: is_off,
                    on: is_on,
                    allowed,
                },
            )?;
            self.install_qualified_personal(&candidate, &live_base, qualified, false)
        })();
        if let Err(error) = publication {
            self.personal_memory_blocked.store(true, Ordering::Release);
            // Even a final publication/readback uncertainty retains a durable denial.
            if is_on || is_off {
                let mut fresh = journal::load(dir)?;
                journal::add_withdrawal(&mut fresh, guild, user, w)?;
                journal::publish_owned(&owner, dir, fresh.revision, fresh)?;
            }
            // No success-shaped live fact mutation was published. Off's immediate fence remains.
            return Err(error);
        }
        if marker.withdrawals.is_empty() && marker.activations.is_empty() {
            let live = Self::lock(&self.stores);
            if live.personal_memory_exposure.epoch == candidate.personal_memory_exposure.epoch
                && Self::lock(&self.personal_memory_reconciliation).denials_cleared()
            {
                self.personal_memory_blocked.store(false, Ordering::Release);
            }
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
