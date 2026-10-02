//! Independent owner-only pending proposal store and exact approval publication.
//! All mutation authority arrives as typed fresh proofs, never model metadata.
use super::community_ops::{Lease, load_policy};
use super::owned_file::OwnedFile;
use crate::community_ops::proposals::*;
use crate::community_ops::{Action, Mode, Policy};
use std::{
    fs::{self},
    path::{Path, PathBuf},
};
const MAX_STORE_BYTES: u64 = 8 * 1024 * 1024;

fn private_file(path: &Path) -> Result<(), &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| "proposal file unavailable")?;
    if !meta.is_file() || meta.len() > MAX_STORE_BYTES {
        return Err("unsafe proposal file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 || meta.uid() != rustix::process::geteuid().as_raw() {
            return Err("proposal file must be owner-only");
        }
    }
    Ok(())
}
fn lock(path: PathBuf) -> Result<OwnedFile, &'static str> {
    let (file, guard) =
        OwnedFile::create(&path).map_err(|_| "proposal lease or pending file requires review")?;
    guard
        .sync_all(&file)
        .map_err(|_| "proposal lease sync failed")?;
    Ok(guard)
}
fn directory(data: &Path) -> Result<PathBuf, &'static str> {
    if !data.is_absolute() {
        return Err("proposal data path must be absolute");
    }
    let dir = data.join("community-operations");
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err("unsafe proposal directory"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&dir).map_err(|_| "proposal directory creation failed")?;
        }
        Err(_) => return Err("proposal directory unavailable"),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = fs::symlink_metadata(&dir).map_err(|_| "proposal directory unavailable")?;
        if meta.uid() != rustix::process::geteuid().as_raw() {
            return Err("proposal directory owner mismatch");
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "proposal directory privacy failed")?;
    }
    Ok(dir)
}
fn publish_bytes(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err("proposal publication exceeds bound");
    }
    let pending = path.with_extension("proposal-pending");
    let (mut file, mut owned) = OwnedFile::create(&pending)
        .map_err(|_| "proposal lease or pending file requires review")?;
    owned
        .write_all(&mut file, bytes)
        .map_err(|_| "proposal write failed")?;
    owned.sync_all(&file).map_err(|_| "proposal sync failed")?;
    fs::rename(&pending, path).map_err(|_| "proposal rename failed")?;
    owned.published();
    OwnedFile::sync_directory(path.parent().ok_or("proposal parent missing")?)
        .map_err(|_| "proposal directory sync failed")?;
    private_file(path)?;
    if fs::read(path).map_err(|_| "proposal readback failed")? != bytes {
        return Err("proposal readback mismatch");
    }
    Ok(())
}
fn load_at(path: &Path) -> Result<ProposalStore, &'static str> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ProposalStore::default()),
        Err(_) => return Err("proposal store unavailable"),
        Ok(_) => {}
    }
    private_file(path)?;
    let store: ProposalStore =
        serde_json::from_slice(&fs::read(path).map_err(|_| "proposal store read failed")?)
            .map_err(|_| "invalid proposal store")?;
    store.validate()?;
    Ok(store)
}
fn save_at(path: &Path, store: &mut ProposalStore) -> Result<(), &'static str> {
    store.revision = store
        .revision
        .checked_add(1)
        .ok_or("proposal revision exhausted")?;
    store.validate()?;
    publish_bytes(
        path,
        &serde_json::to_vec_pretty(store).map_err(|_| "proposal store encode failed")?,
    )
}
pub fn load(data: &Path) -> Result<ProposalStore, &'static str> {
    load_at(&directory(data)?.join("proposals.json"))
}
/// The command boundary verifies the live guild owner before this short
/// inspection transaction. Only unapproved Pending rows can become Stale;
/// prepared and finalized approval journals remain reconciliation evidence.
pub fn inspect_pending(
    data: &Path,
    policy: &Policy,
    now: u64,
) -> Result<ProposalStore, &'static str> {
    policy.validate()?;
    let immutable = scope_digest(policy, &policy.assessment)?;
    let dir = directory(data)?;
    let _lock = lock(dir.join("proposals.lock"))?;
    let path = dir.join("proposals.json");
    let mut store = load_at(&path)?;
    let mut changed = false;
    for p in store.proposals.values_mut() {
        if p.status == ProposalStatus::Pending
            && (p.guild != policy.guild
                || p.policy_owner != policy.owner
                || p.created_at > now
                || p.expires_at <= now
                || p.source.scope_digest != immutable)
        {
            p.revision = p
                .revision
                .checked_add(1)
                .ok_or("proposal revision exhausted")?;
            p.status = ProposalStatus::Stale;
            changed = true;
        }
    }
    if changed {
        save_at(&path, &mut store)?;
    }
    Ok(store)
}

#[derive(Debug)]
pub struct AssessmentReservation {
    id: String,
    guild: u64,
    owner: u64,
    scope_digest: String,
    policy_digest: String,
}
impl AssessmentReservation {
    pub fn id(&self) -> &str {
        &self.id
    }
}
pub fn begin_assessment(
    data: &Path,
    policy: &Policy,
    scope: &AssessmentScope,
    policy_digest: &str,
    now: u64,
) -> Result<AssessmentReservation, &'static str> {
    policy.validate()?;
    if policy.mode == Mode::Stopped || !scope.enabled || scope.source_channels.is_empty() {
        return Err("assessment is disabled or stopped");
    }
    let dir = directory(data)?;
    let _lock = lock(dir.join("proposals.lock"))?;
    let path = dir.join("proposals.json");
    let mut store = load_at(&path)?;
    if store
        .last_attempt_at
        .is_some_and(|last| last > now || now - last < 86400)
    {
        return Err("assessment already charged or clock regressed");
    }
    if store.proposals.len() >= MAX_PENDING {
        return Err("pending store is full");
    }
    // A crash never replays a Started request. Preserve its charge and mark the
    // abandoned attempt when a later eligible daily reservation is made.
    for a in &mut store.attempts {
        if a.outcome == AssessmentOutcome::Started {
            a.outcome = AssessmentOutcome::Cancelled;
            a.finished_at = Some(now.max(a.started_at));
        }
    }
    if store.attempts.len() == 100 {
        store.attempts.remove(0);
    }
    let id = digest(&(policy.guild, now, policy_digest))?;
    store.attempts.push(AssessmentAttempt {
        id: id.clone(),
        started_at: now,
        finished_at: None,
        source_digest: None,
        policy_digest: policy_digest.into(),
        outcome: AssessmentOutcome::Started,
    });
    store.last_attempt_at = Some(now);
    save_at(&path, &mut store)?;
    Ok(AssessmentReservation {
        id,
        guild: policy.guild,
        owner: policy.owner,
        scope_digest: scope_digest(policy, scope)?,
        policy_digest: policy_digest.into(),
    })
}
pub fn finish_assessment(
    data: &Path,
    reservation: &AssessmentReservation,
    policy: &Policy,
    current_policy_digest: &str,
    proposals: Vec<PendingProposal>,
    outcome: AssessmentOutcome,
    now: u64,
) -> Result<(), &'static str> {
    let scope = &policy.assessment;
    let harmless_failure = proposals.is_empty()
        && matches!(
            outcome,
            AssessmentOutcome::Cancelled
                | AssessmentOutcome::LocalUnavailable
                | AssessmentOutcome::SourceIncomplete
                | AssessmentOutcome::InvalidOutput
                | AssessmentOutcome::PersistenceReviewRequired
        );
    if policy.guild != reservation.guild
        || policy.owner != reservation.owner
        || (!harmless_failure
            && (policy.mode == Mode::Stopped
                || scope_digest(policy, scope)? != reservation.scope_digest
                || current_policy_digest != reservation.policy_digest))
    {
        return Err("assessment policy changed");
    }
    if matches!(outcome, AssessmentOutcome::Started)
        || (proposals.is_empty() == (outcome == AssessmentOutcome::PendingPublished))
    {
        return Err("invalid assessment settlement");
    }
    let dir = directory(data)?;
    let _lock = lock(dir.join("proposals.lock"))?;
    let path = dir.join("proposals.json");
    let mut store = load_at(&path)?;
    let attempt = store
        .attempts
        .iter()
        .find(|a| a.id == reservation.id)
        .ok_or("assessment reservation missing")?;
    if attempt.outcome != AssessmentOutcome::Started || now < attempt.started_at {
        return Err("assessment already settled or clock regressed");
    }
    let mut targets = store.active_targets();
    if store.proposals.len() + proposals.len() > MAX_PENDING {
        return Err("pending store is full");
    }
    for p in &proposals {
        p.validate()?;
        if p.guild != policy.guild
            || p.policy_owner != policy.owner
            || p.status != ProposalStatus::Pending
            || p.decision.is_some()
            || p.source.assessment_id != reservation.id
            || p.source.scope_digest != reservation.scope_digest
            || p.source.policy_digest != reservation.policy_digest
            || p.created_at < attempt.started_at
            || p.created_at > now
            || store.proposals.contains_key(&p.id)
            || !targets.insert(p.draft.operation.target())
        {
            return Err("pending publication identity or conflict drift");
        }
    }
    let source_digest = proposals.first().map(|p| p.source.inventory_digest.clone());
    for p in proposals {
        store.proposals.insert(p.id.clone(), p);
    }
    let attempt = store
        .attempts
        .iter_mut()
        .find(|a| a.id == reservation.id)
        .ok_or("assessment reservation missing")?;
    attempt.finished_at = Some(now);
    attempt.source_digest = source_digest;
    attempt.outcome = outcome;
    save_at(&path, &mut store)
}

pub struct ApprovalRequest<'a> {
    pub policy_path: &'a Path,
    pub data: &'a Path,
    pub scope: &'a AssessmentScope,
    pub expected_policy_digest: &'a str,
    pub expected_store_revision: u64,
    pub proposal_id: &'a str,
    pub reviewed_hash: &'a str,
    pub owner: FreshOwnerProof,
    pub operation: FreshOperationProof,
}
fn check_pending(
    p: &PendingProposal,
    policy: &Policy,
    scope: &AssessmentScope,
    hash: &str,
    now: u64,
) -> Result<(), &'static str> {
    p.validate()?;
    if p.status != ProposalStatus::Pending
        || p.hash != hash
        || p.guild != policy.guild
        || p.policy_owner != policy.owner
        || p.created_at > now
        || p.expires_at <= now
        || p.source.scope_digest != scope_digest(policy, scope)?
    {
        return Err("pending proposal is stale or not approveable");
    }
    Ok(())
}
pub fn approve(
    request: ApprovalRequest<'_>,
    clock: impl Fn() -> u64,
) -> Result<Action, &'static str> {
    let _policy_lock = lock(request.policy_path.with_extension("mode-lock"))?;
    let dir = directory(request.data)?;
    let _proposal_lock = lock(dir.join("proposals.lock"))?;
    let execution_lease = Lease::acquire(request.data)?;
    let execution = execution_lease.load()?;
    let (mut policy, current_digest) = load_policy(request.policy_path)?;
    if current_digest != request.expected_policy_digest {
        return Err("approval policy CAS failed");
    }
    let path = dir.join("proposals.json");
    let mut store = load_at(&path)?;
    if store.revision != request.expected_store_revision {
        return Err("approval store CAS failed");
    }
    let now = clock();
    let actor = request.owner.check(&policy, request.scope, now)?;
    let p = store
        .proposals
        .get(request.proposal_id)
        .ok_or("proposal unavailable")?;
    check_pending(p, &policy, request.scope, request.reviewed_hash, now)?;
    request.operation.check(p, now)?;
    let action = p.action();
    if policy.actions.iter().any(|a| a.key == action.key)
        || active_inventory_targets(&policy, &execution).contains(&action.operation.target())
    {
        return Err("approval conflicts with existing inventory");
    }
    // Authorization uses a temporary Apply copy only for budget and target
    // eligibility. The persisted mode is preserved exactly, including Stopped.
    let mut eligibility = policy.clone();
    eligibility.mode = Mode::Apply;
    eligibility.actions.push(action.clone());
    execution.authorize(&eligibility, &action, now)?;
    let immutable = scope_digest(&policy, request.scope)?;
    policy.actions.push(action.clone());
    policy.validate()?;
    if scope_digest(&policy, request.scope)? != immutable {
        return Err("approval widened owner scope");
    }
    let bytes =
        serde_json::to_vec_pretty(&policy).map_err(|_| "approval policy encoding failed")?;
    let planned_digest = hash_bytes(&bytes)?;
    let now = clock();
    request.owner.check(&policy, request.scope, now)?;
    request.operation.check(p, now)?;
    check_pending(p, &policy, request.scope, request.reviewed_hash, now)?;
    let p = store
        .proposals
        .get_mut(request.proposal_id)
        .ok_or("proposal unavailable")?;
    p.status = ProposalStatus::ApprovalPrepared;
    p.revision = p
        .revision
        .checked_add(1)
        .ok_or("proposal revision exhausted")?;
    p.decision = Some(OwnerDecision {
        actor,
        decided_at: now,
        reviewed_hash: p.hash.clone(),
        previous_policy_digest: current_digest.clone(),
        published_policy_digest: None,
        approved_action_key: Some(action.key.clone()),
        journal: Some(ApprovalJournal {
            action: action.clone(),
            reviewed_hash: p.hash.clone(),
            scope_digest: immutable,
            previous_policy_digest: current_digest.clone(),
            planned_policy_digest: planned_digest.clone(),
            stage: ApprovalStage::Prepared,
        }),
        recovery_audit: None,
    });
    save_at(&path, &mut store)?;
    let publication = (|| {
        let commit_now = clock();
        request.owner.check(&policy, request.scope, commit_now)?;
        let pending = store
            .proposals
            .get(request.proposal_id)
            .ok_or("proposal unavailable")?;
        if pending.created_at > commit_now || pending.expires_at <= commit_now {
            return Err("proposal expired during approval publication");
        }
        request.operation.check(
            store
                .proposals
                .get(request.proposal_id)
                .ok_or("proposal unavailable")?,
            commit_now,
        )?;
        if load_policy(request.policy_path)?.1 != current_digest {
            return Err("policy changed during approval publication");
        }
        publish_bytes(request.policy_path, &bytes)?;
        let (observed, digest) = load_policy(request.policy_path)?;
        if observed != policy || digest != planned_digest {
            return Err("approved policy readback mismatch");
        }
        Ok(())
    })();
    if publication.is_err() {
        mark_review(&mut store, request.proposal_id)?;
        save_at(&path, &mut store)?;
        return Err("approval publication uncertain; owner reconciliation required");
    }
    finalize(&mut store, request.proposal_id, &planned_digest)?;
    save_at(&path, &mut store)?;
    Ok(action)
}
fn mark_review(store: &mut ProposalStore, id: &str) -> Result<(), &'static str> {
    let p = store.proposals.get_mut(id).ok_or("proposal unavailable")?;
    p.status = ProposalStatus::ReviewRequired;
    p.revision = p
        .revision
        .checked_add(1)
        .ok_or("proposal revision exhausted")?;
    p.decision
        .as_mut()
        .and_then(|d| d.journal.as_mut())
        .ok_or("approval journal missing")?
        .stage = ApprovalStage::ReconciliationRequired;
    Ok(())
}
fn finalize(store: &mut ProposalStore, id: &str, digest: &str) -> Result<(), &'static str> {
    let p = store.proposals.get_mut(id).ok_or("proposal unavailable")?;
    p.status = ProposalStatus::Approved;
    p.revision = p
        .revision
        .checked_add(1)
        .ok_or("proposal revision exhausted")?;
    let decision = p.decision.as_mut().ok_or("approval decision missing")?;
    decision.published_policy_digest = Some(digest.into());
    decision
        .journal
        .as_mut()
        .ok_or("approval journal missing")?
        .stage = ApprovalStage::Finalized;
    Ok(())
}
/// Reconcile only an already-published exact action. Missing/mismatched actions
/// never cause speculative republishing or receipt clearing.
pub fn reconcile(
    data: &Path,
    policy_path: &Path,
    scope: &AssessmentScope,
) -> Result<(), &'static str> {
    let _policy_lock = lock(policy_path.with_extension("mode-lock"))?;
    let dir = directory(data)?;
    let _proposal_lock = lock(dir.join("proposals.lock"))?;
    let path = dir.join("proposals.json");
    let mut store = load_at(&path)?;
    let (policy, policy_digest) = load_policy(policy_path)?;
    let immutable = scope_digest(&policy, scope)?;
    let ids: Vec<_> = store
        .proposals
        .values()
        .filter(|p| {
            matches!(
                p.status,
                ProposalStatus::ApprovalPrepared | ProposalStatus::ReviewRequired
            )
        })
        .map(|p| p.id.clone())
        .collect();
    let mut unresolved = false;
    for id in &ids {
        let p = store.proposals.get(id).ok_or("proposal unavailable")?;
        let journal = p
            .decision
            .as_ref()
            .and_then(|d| d.journal.as_ref())
            .ok_or("approval journal missing")?;
        if p.guild == policy.guild
            && p.policy_owner == policy.owner
            && journal.scope_digest == immutable
            && policy
                .actions
                .iter()
                .filter(|a| a.key == journal.action.key)
                .count()
                == 1
            && policy.actions.contains(&journal.action)
        {
            finalize(&mut store, id, &policy_digest)?;
        } else {
            mark_review(&mut store, id)?;
            unresolved = true;
        }
    }
    if !ids.is_empty() {
        save_at(&path, &mut store)?;
    }
    if unresolved {
        Err("approval reconciliation requires owner review")
    } else {
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
pub fn reject(
    data: &Path,
    policy_path: &Path,
    scope: &AssessmentScope,
    policy_digest: &str,
    revision: u64,
    id: &str,
    reviewed_hash: &str,
    owner: FreshOwnerProof,
    clock: impl Fn() -> u64,
) -> Result<(), &'static str> {
    let _policy_lock = lock(policy_path.with_extension("mode-lock"))?;
    let dir = directory(data)?;
    let _proposal_lock = lock(dir.join("proposals.lock"))?;
    let (policy, digest) = load_policy(policy_path)?;
    if digest != policy_digest {
        return Err("rejection policy CAS failed");
    }
    let now = clock();
    let actor = owner.check(&policy, scope, now)?;
    let path = dir.join("proposals.json");
    let mut store = load_at(&path)?;
    if store.revision != revision {
        return Err("rejection store CAS failed");
    }
    let p = store.proposals.get_mut(id).ok_or("proposal unavailable")?;
    check_pending(p, &policy, scope, reviewed_hash, now)?;
    let commit_now = clock();
    owner.check(&policy, scope, commit_now)?;
    check_pending(p, &policy, scope, reviewed_hash, commit_now)?;
    p.status = ProposalStatus::Rejected;
    p.revision = p
        .revision
        .checked_add(1)
        .ok_or("proposal revision exhausted")?;
    p.decision = Some(OwnerDecision {
        actor,
        decided_at: commit_now,
        reviewed_hash: p.hash.clone(),
        previous_policy_digest: digest,
        published_policy_digest: None,
        approved_action_key: None,
        journal: None,
        recovery_audit: None,
    });
    save_at(&path, &mut store)
}

#[cfg(test)]
mod tests;
