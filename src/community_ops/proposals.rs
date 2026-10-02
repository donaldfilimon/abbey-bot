//! Untrusted model drafts, public proofs, and owner-bound pending proposals.
//! This module cannot perform I/O or grant execution authority.
use super::{Action, Ledger, Mode, Operation, Policy, Status};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SOURCE_BYTES: usize = 32 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;
pub const PROOF_TTL: u64 = 60;
pub const PROPOSAL_TTL: u64 = 7 * 86400;
pub const MAX_PENDING: usize = 100;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentKind {
    Topic,
    Move,
    CreateText,
    CreateInterestRole,
    RetireInterestRole,
    Archive,
}
impl AssessmentKind {
    fn of(operation: &Operation) -> Option<Self> {
        Some(match operation {
            Operation::Topic { .. } => Self::Topic,
            Operation::Move { .. } => Self::Move,
            Operation::CreateText { .. } => Self::CreateText,
            Operation::CreateInterestRole { .. } => Self::CreateInterestRole,
            Operation::RetireInterestRole { .. } => Self::RetireInterestRole,
            Operation::Archive { .. } => Self::Archive,
            _ => return None,
        })
    }
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssessmentScope {
    pub enabled: bool,
    pub allowed_kinds: BTreeSet<AssessmentKind>,
    pub source_channels: BTreeSet<u64>,
    pub review_channel: Option<u64>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelProposalBatch {
    pub version: u32,
    pub proposals: Vec<ModelProposal>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelProposal {
    pub operation: Operation,
    pub reason: String,
}
impl ModelProposalBatch {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err("proposal output exceeds bound");
        }
        let batch: Self = serde_json::from_slice(bytes).map_err(|_| "invalid proposal JSON")?;
        if batch.version != 1 || batch.proposals.len() > 5 {
            return Err("invalid proposal version or count");
        }
        Ok(batch)
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicChannelMetadata {
    pub id: u64,
    pub name: String,
    pub kind: String,
    pub parent: u64,
    pub topic: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NamedMetadata {
    pub id: u64,
    pub name: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssessmentSource {
    pub version: u32,
    pub assessment_id: String,
    pub guild: u64,
    pub scope_digest: String,
    pub policy_digest: String,
    pub captured_at: u64,
    pub inventory_digest: String,
    pub channels: Vec<PublicChannelMetadata>,
    pub categories: Vec<NamedMetadata>,
    pub roles: Vec<NamedMetadata>,
}
/// Constructed by fresh Discord observation; never deserialize permission proof
/// from a model, a dashboard field, or stored source metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelProof {
    pub metadata: PublicChannelMetadata,
    pub public: bool,
    pub archive_safe: bool,
    pub permissions_digest: String,
    pub before_digest: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct RoleProof {
    pub metadata: NamedMetadata,
    pub permissions: u64,
    pub entitlement_free: bool,
    pub before_digest: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct PublicProofs {
    pub guild: u64,
    pub checked_at: u64,
    pub channels: BTreeMap<u64, ChannelProof>,
    pub categories: BTreeMap<u64, String>,
    pub roles: BTreeMap<u64, RoleProof>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostingChange {
    Preserve,
    CloseOrdinaryPosting,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExpectedAccess {
    pub target_before_digest: Option<String>,
    pub category_before_digest: Option<String>,
    pub permissions_before_digest: Option<String>,
    pub view_invariant: bool,
    pub posting_change: PostingChange,
    pub role_permissions: Option<u64>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProposalSource {
    pub assessment_id: String,
    pub captured_at: u64,
    pub inventory_digest: String,
    pub scope_digest: String,
    pub policy_digest: String,
    pub source_channel_ids: BTreeSet<u64>,
    pub source_category_ids: BTreeSet<u64>,
    pub source_role_ids: BTreeSet<u64>,
    pub locality: SourceLocality,
    pub request_class: SourceClass,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceLocality {
    SameHost,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceClass {
    TextReadOnly,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    ApprovalPrepared,
    Approved,
    Rejected,
    Stale,
    ReviewRequired,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStage {
    Prepared,
    PolicyVerified,
    Finalized,
    ReconciliationRequired,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApprovalJournal {
    pub action: Action,
    pub reviewed_hash: String,
    pub scope_digest: String,
    pub previous_policy_digest: String,
    pub planned_policy_digest: String,
    pub stage: ApprovalStage,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAudit {
    pub receipt_key: String,
    pub receipt_digest: String,
    pub original_access_digest: String,
    pub archived_access_digest: String,
    pub reason: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerDecision {
    pub actor: u64,
    pub decided_at: u64,
    pub reviewed_hash: String,
    pub previous_policy_digest: String,
    pub published_policy_digest: Option<String>,
    pub approved_action_key: Option<String>,
    pub journal: Option<ApprovalJournal>,
    pub recovery_audit: Option<RecoveryAudit>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingProposal {
    pub id: String,
    pub revision: u64,
    pub hash: String,
    pub guild: u64,
    pub policy_owner: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub draft: ModelProposal,
    pub source: ProposalSource,
    pub expected_access: ExpectedAccess,
    pub status: ProposalStatus,
    pub decision: Option<OwnerDecision>,
}
impl PendingProposal {
    pub fn action(&self) -> Action {
        Action {
            key: format!("proposal-{}", self.id),
            reason: self.draft.reason.clone(),
            operation: self.draft.operation.clone(),
        }
    }
    pub fn reviewed_hash(&self) -> Result<String, &'static str> {
        digest(&(
            self.id.as_str(),
            self.guild,
            self.policy_owner,
            self.created_at,
            self.expires_at,
            &self.draft,
            &self.source,
            &self.expected_access,
        ))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.len() != 64
            || !self.id.bytes().all(|c| c.is_ascii_hexdigit())
            || self.hash != self.reviewed_hash()?
            || self.revision == 0
            || self.guild == 0
            || self.policy_owner == 0
            || self.expires_at != self.created_at.saturating_add(PROPOSAL_TTL)
            || self.source.captured_at > self.created_at
            || self.draft.reason.trim().is_empty()
            || self.draft.reason.len() > 512
            || AssessmentKind::of(&self.draft.operation).is_none()
        {
            return Err("invalid pending proposal integrity");
        }
        if self.status == ProposalStatus::Pending && self.decision.is_some() {
            return Err("pending proposal already has a decision");
        }
        if matches!(
            self.status,
            ProposalStatus::Approved
                | ProposalStatus::ApprovalPrepared
                | ProposalStatus::ReviewRequired
        ) {
            let decision = self.decision.as_ref().ok_or("approval decision missing")?;
            let journal = decision
                .journal
                .as_ref()
                .ok_or("approval journal missing")?;
            if decision.actor != self.policy_owner
                || decision.reviewed_hash != self.hash
                || journal.action != self.action()
                || journal.reviewed_hash != self.hash
                || journal.scope_digest != self.source.scope_digest
                || journal.previous_policy_digest != decision.previous_policy_digest
                || decision.approved_action_key.as_ref() != Some(&journal.action.key)
                || (self.status == ProposalStatus::Approved
                    && journal.stage != ApprovalStage::Finalized)
            {
                return Err("approval journal integrity mismatch");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentOutcome {
    Started,
    LocalUnavailable,
    SourceIncomplete,
    InvalidOutput,
    NoChange,
    PendingPublished,
    Cancelled,
    PersistenceReviewRequired,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssessmentAttempt {
    pub id: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub source_digest: Option<String>,
    pub policy_digest: String,
    pub outcome: AssessmentOutcome,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProposalStore {
    pub version: u32,
    pub revision: u64,
    pub last_attempt_at: Option<u64>,
    pub attempts: Vec<AssessmentAttempt>,
    pub proposals: BTreeMap<String, PendingProposal>,
}
impl Default for ProposalStore {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            last_attempt_at: None,
            attempts: Vec::new(),
            proposals: BTreeMap::new(),
        }
    }
}
impl ProposalStore {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || self.proposals.len() > MAX_PENDING || self.attempts.len() > 100 {
            return Err("invalid proposal store limits");
        }
        for (id, p) in &self.proposals {
            if id != &p.id {
                return Err("proposal identity mismatch");
            }
            p.validate()?;
        }
        let mut ids = BTreeSet::new();
        for a in &self.attempts {
            if !ids.insert(&a.id) || a.finished_at.is_some_and(|t| t < a.started_at) {
                return Err("invalid assessment attempt");
            }
        }
        Ok(())
    }
    pub fn active_targets(&self) -> BTreeSet<String> {
        self.proposals
            .values()
            .filter(|p| {
                matches!(
                    p.status,
                    ProposalStatus::Pending
                        | ProposalStatus::ApprovalPrepared
                        | ProposalStatus::ReviewRequired
                )
            })
            .map(|p| p.draft.operation.target())
            .collect()
    }
    pub fn executable(&self, action: &Action) -> bool {
        if !action.key.starts_with("proposal-") {
            return true;
        }
        self.proposals.values().any(|p| {
            p.status == ProposalStatus::Approved && p.action() == *action && p.validate().is_ok()
        })
    }
}

pub fn digest(value: &impl Serialize) -> Result<String, &'static str> {
    hash_bytes(&serde_json::to_vec(value).map_err(|_| "proposal encoding failed")?)
}
pub fn hash_bytes(bytes: &[u8]) -> Result<String, &'static str> {
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub fn active_inventory_targets(policy: &Policy, execution: &Ledger) -> BTreeSet<String> {
    policy
        .actions
        .iter()
        .filter(|a| {
            execution.receipts.get(&a.key).is_none_or(|r| {
                r.action != **a || !matches!(r.status, Status::Verified | Status::Rejected)
            })
        })
        .map(|a| a.operation.target())
        .collect()
}
pub fn scope_digest(policy: &Policy, scope: &AssessmentScope) -> Result<String, &'static str> {
    if scope != &policy.assessment {
        return Err("assessment scope is not owner policy");
    }
    let mut v = serde_json::to_value(policy).map_err(|_| "policy projection failed")?;
    let map = v.as_object_mut().ok_or("invalid policy projection")?;
    map.remove("actions");
    map.remove("mode");
    digest(&(v, scope))
}
fn fresh(checked: u64, now: u64) -> Result<(), &'static str> {
    if checked > now || now - checked > PROOF_TTL {
        Err("stale proof")
    } else {
        Ok(())
    }
}
pub fn expected_access(
    policy: &Policy,
    scope: &AssessmentScope,
    source: &ProposalSource,
    proofs: &PublicProofs,
    operation: &Operation,
    now: u64,
) -> Result<ExpectedAccess, &'static str> {
    fresh(proofs.checked_at, now)?;
    if proofs.guild != policy.guild
        || !scope.enabled
        || source.scope_digest != scope_digest(policy, scope)?
    {
        return Err("assessment scope mismatch");
    }
    let kind = AssessmentKind::of(operation).ok_or("operation is not model-proposable")?;
    if !scope.allowed_kinds.contains(&kind) {
        return Err("operation kind lacks owner scope");
    }
    let mut expected = ExpectedAccess {
        target_before_digest: None,
        category_before_digest: None,
        permissions_before_digest: None,
        view_invariant: true,
        posting_change: PostingChange::Preserve,
        role_permissions: None,
    };
    let channel_id = match operation {
        Operation::Topic { channel, .. }
        | Operation::Move { channel, .. }
        | Operation::Archive { channel, .. } => Some(*channel),
        _ => None,
    };
    if let Some(id) = channel_id {
        let c = proofs.channels.get(&id).ok_or("target unavailable")?;
        if !source.source_channel_ids.contains(&id)
            || !scope.source_channels.contains(&id)
            || !c.public
            || policy.protected_channels.contains(&id)
            || !policy.public_categories.contains(&c.metadata.parent)
        {
            return Err("target outside public source scope");
        }
        expected.target_before_digest = Some(c.before_digest.clone());
        expected.permissions_before_digest = Some(c.permissions_digest.clone());
        if let Operation::Topic { topic, .. } = operation
            && c.metadata.topic.as_deref() == Some(topic)
        {
            return Err("proposal is a no-op");
        }
        if let Operation::Move { category, .. } = operation
            && c.metadata.parent == *category
        {
            return Err("proposal is a no-op");
        }
    }
    let category = match operation {
        Operation::Move { category, .. }
        | Operation::Archive { category, .. }
        | Operation::CreateText { category, .. } => Some(*category),
        _ => None,
    };
    if let Some(id) = category {
        if !policy.public_categories.contains(&id) || !source.source_category_ids.contains(&id) {
            return Err("destination outside owner source scope");
        }
        expected.category_before_digest = Some(
            proofs
                .categories
                .get(&id)
                .ok_or("category public proof unavailable")?
                .clone(),
        );
    }
    match operation {
        Operation::Archive { channel, .. } => {
            if !proofs.channels.get(channel).is_some_and(|c| c.archive_safe) {
                return Err("archive posting closure lacks proof");
            }
            expected.posting_change = PostingChange::CloseOrdinaryPosting;
        }
        Operation::CreateText { category, name, .. } => {
            if proofs
                .channels
                .values()
                .any(|c| c.metadata.parent == *category && c.metadata.name == *name)
            {
                return Err("creation collision");
            }
            expected.permissions_before_digest = expected.category_before_digest.clone();
        }
        Operation::CreateInterestRole { name } => {
            if proofs.roles.values().any(|r| r.metadata.name == *name) {
                return Err("role name collision");
            }
            expected.role_permissions = Some(0);
        }
        Operation::RetireInterestRole { role, name } => {
            let r = proofs.roles.get(role).ok_or("role unavailable")?;
            if !policy.ordinary_roles.contains(role)
                || !source.source_role_ids.contains(role)
                || r.permissions != 0
                || !r.entitlement_free
                || r.metadata.name == *name
                || proofs.roles.values().any(|r| r.metadata.name == *name)
            {
                return Err("role is not ordinary or name collides");
            }
            expected.target_before_digest = Some(r.before_digest.clone());
            expected.role_permissions = Some(0);
        }
        _ => {}
    }
    Ok(expected)
}

pub struct ProposalInventory<'a> {
    pub execution: &'a Ledger,
    pub pending: &'a ProposalStore,
}
pub fn validate_drafts(
    policy: &Policy,
    scope: &AssessmentScope,
    source: &AssessmentSource,
    proofs: &PublicProofs,
    inventory: ProposalInventory<'_>,
    now: u64,
    batch: ModelProposalBatch,
) -> Result<Vec<PendingProposal>, &'static str> {
    let ProposalInventory { execution, pending } = inventory;
    policy.validate()?;
    pending.validate()?;
    fresh(source.captured_at, now)?;
    if policy.mode == Mode::Stopped
        || !scope.enabled
        || batch.version != 1
        || batch.proposals.len() > 5
        || source.version != 1
        || source.guild != policy.guild
        || source.scope_digest != scope_digest(policy, scope)?
        || source.channels.len() > 100
        || source.categories.len() > 20
        || source.roles.len() > 50
        || serde_json::to_vec(source)
            .map_err(|_| "source encoding failed")?
            .len()
            > MAX_SOURCE_BYTES
    {
        return Err("invalid or stopped assessment source");
    }
    let channel_ids: BTreeSet<_> = source.channels.iter().map(|c| c.id).collect();
    let category_ids: BTreeSet<_> = source.categories.iter().map(|c| c.id).collect();
    let role_ids: BTreeSet<_> = source.roles.iter().map(|c| c.id).collect();
    if channel_ids != scope.source_channels
        || channel_ids.len() != source.channels.len()
        || category_ids.len() != source.categories.len()
        || role_ids.len() != source.roles.len()
    {
        return Err("source inventory incomplete or duplicated");
    }
    for c in &source.channels {
        if proofs
            .channels
            .get(&c.id)
            .is_none_or(|p| !p.public || p.metadata != *c)
        {
            return Err("source public proof mismatch");
        }
    }
    for c in &source.categories {
        if !policy.public_categories.contains(&c.id) || !proofs.categories.contains_key(&c.id) {
            return Err("category source mismatch");
        }
    }
    for r in &source.roles {
        if !policy.ordinary_roles.contains(&r.id)
            || proofs
                .roles
                .get(&r.id)
                .is_none_or(|p| p.metadata != *r || p.permissions != 0 || !p.entitlement_free)
        {
            return Err("role source mismatch");
        }
    }
    if source.inventory_digest != digest(&(&source.channels, &source.categories, &source.roles))? {
        return Err("inventory digest mismatch");
    }
    let source_ref = ProposalSource {
        assessment_id: source.assessment_id.clone(),
        captured_at: source.captured_at,
        inventory_digest: source.inventory_digest.clone(),
        scope_digest: source.scope_digest.clone(),
        policy_digest: source.policy_digest.clone(),
        source_channel_ids: channel_ids,
        source_category_ids: category_ids,
        source_role_ids: role_ids,
        locality: SourceLocality::SameHost,
        request_class: SourceClass::TextReadOnly,
    };
    let mut targets = pending.active_targets();
    targets.extend(active_inventory_targets(policy, execution));
    let mut shadow = execution.clone();
    let mut result = Vec::new();
    for draft in batch.proposals {
        if draft.reason.trim().is_empty()
            || draft.reason.len() > 512
            || !targets.insert(draft.operation.target())
        {
            return Err("invalid or conflicting proposal");
        }
        let expected = expected_access(policy, scope, &source_ref, proofs, &draft.operation, now)?;
        let id = digest(&(policy.guild, &source_ref, &draft))?;
        let mut p = PendingProposal {
            id,
            revision: 1,
            hash: String::new(),
            guild: policy.guild,
            policy_owner: policy.owner,
            created_at: now,
            expires_at: now.saturating_add(PROPOSAL_TTL),
            draft,
            source: source_ref.clone(),
            expected_access: expected,
            status: ProposalStatus::Pending,
            decision: None,
        };
        p.hash = p.reviewed_hash()?;
        p.validate()?;
        let action = p.action();
        let mut candidate = policy.clone();
        candidate.mode = Mode::Apply;
        candidate.actions.push(action.clone());
        shadow.authorize(&candidate, &action, now)?;
        shadow.receipts.insert(
            action.key.clone(),
            super::Receipt {
                action,
                policy_digest: source.policy_digest.clone(),
                at: now,
                status: Status::Reserved,
                before: serde_json::Value::Null,
                observed: None,
                detail: "validation only; not authority".into(),
            },
        );
        result.push(p);
    }
    if pending.proposals.len() + result.len() > MAX_PENDING {
        return Err("pending store is full");
    }
    Ok(result)
}

/// Fresh authority is a non-deserializable capability constructed by the
/// command boundary after Discord owner/origin verification.
pub struct FreshOwnerProof {
    guild: u64,
    owner: u64,
    actor: u64,
    origin: u64,
    checked_at: u64,
}
impl FreshOwnerProof {
    pub fn verified(
        guild: u64,
        owner: u64,
        actor: u64,
        origin: u64,
        checked_at: u64,
    ) -> Result<Self, &'static str> {
        if guild == 0 || owner == 0 || actor != owner || origin == 0 {
            return Err("owner proof denied");
        }
        Ok(Self {
            guild,
            owner,
            actor,
            origin,
            checked_at,
        })
    }
    pub fn check(
        &self,
        policy: &Policy,
        scope: &AssessmentScope,
        now: u64,
    ) -> Result<u64, &'static str> {
        fresh(self.checked_at, now)?;
        if self.guild != policy.guild
            || self.owner != policy.owner
            || self.actor != policy.owner
            || scope.review_channel.is_some_and(|c| c != self.origin)
        {
            return Err("owner scope denied");
        }
        Ok(self.actor)
    }
}
pub struct FreshOperationProof {
    reviewed_hash: String,
    expected: ExpectedAccess,
    checked_at: u64,
}
impl FreshOperationProof {
    pub fn verified(
        policy: &Policy,
        scope: &AssessmentScope,
        proposal: &PendingProposal,
        proofs: &PublicProofs,
        now: u64,
    ) -> Result<Self, &'static str> {
        proposal.validate()?;
        let expected = expected_access(
            policy,
            scope,
            &proposal.source,
            proofs,
            &proposal.draft.operation,
            now,
        )?;
        if expected != proposal.expected_access {
            return Err("expected access drift");
        }
        Ok(Self {
            reviewed_hash: proposal.hash.clone(),
            expected,
            checked_at: proofs.checked_at,
        })
    }
    pub fn check(&self, p: &PendingProposal, now: u64) -> Result<(), &'static str> {
        fresh(self.checked_at, now)?;
        if self.reviewed_hash != p.hash || self.expected != p.expected_access {
            return Err("operation proof mismatch");
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;
