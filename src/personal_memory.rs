//! Personal fact provenance and generated-use consent. Ledger admission is not authorship.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub const PERSONAL_MEMORY_POLICY_VERSION: u32 = 1;
pub const MAX_SUBJECTS: usize = 10_000;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UseChoice {
    #[default]
    Off,
    On,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentStamp {
    pub revision: u64,
    pub consent_epoch: u64,
    pub exposure_epoch: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberProof {
    pub actor: String,
    pub subject: String,
    pub guild: String,
    pub interaction_id: String,
    pub platform: String,
    pub at: u64,
    pub policy_version: u32,
}
#[derive(Debug, Clone)]
pub struct SelfAuthorizedFactAction {
    pub(crate) proof: MemberProof,
    pub(crate) expected: ConsentStamp,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryConsentError {
    InvalidProof,
    Stale,
    Bounds,
    Persistence,
    UnverifiedFact,
    Blocked,
    NotFound,
    RequestConflict,
}
impl std::fmt::Display for MemoryConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Personal memory change refused: {:?}", self)
    }
}
impl std::error::Error for MemoryConsentError {}
impl SelfAuthorizedFactAction {
    pub fn new(proof: MemberProof, expected: ConsentStamp) -> Result<Self, MemoryConsentError> {
        if proof.actor != proof.subject
            || !valid_scope(&proof.actor)
            || !valid_scope(&proof.guild)
            || proof.interaction_id.is_empty()
            || proof.platform.is_empty()
            || proof.policy_version != PERSONAL_MEMORY_POLICY_VERSION
            || [
                &proof.actor,
                &proof.guild,
                &proof.interaction_id,
                &proof.platform,
            ]
            .iter()
            .any(|s| s.len() > 128 || s.contains('\u{1f}'))
        {
            return Err(MemoryConsentError::InvalidProof);
        }
        Ok(Self { proof, expected })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactAuthority {
    SelfAuthored,
    MemberConfirmed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactProof {
    pub authority: FactAuthority,
    pub member: MemberProof,
    pub fact_key: String,
    pub previous_revision: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalMemorySubject {
    #[serde(default)]
    pub schema: u32,
    pub revision: u64,
    pub consent_epoch: u64,
    pub choice: UseChoice,
    pub policy_version: u32,
    pub proofs: BTreeMap<String, FactProof>,
    pub outcomes: BTreeMap<String, CompletedRequest>,
    #[serde(default)]
    pub activation_pending: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExposureState {
    pub schema: u32,
    pub epoch: u64,
    pub cutoff: u64,
    #[serde(default)]
    pub scope_epochs: BTreeMap<String, u64>,
    #[serde(default)]
    pub receipts: Vec<ExposureReceipt>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentStatus {
    pub choice: UseChoice,
    pub stamp: ConsentStamp,
    pub policy_version: u32,
    pub eligible_facts: usize,
    pub unverified_facts: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedRequest {
    #[serde(default)]
    pub completed: bool,
    pub payload_digest: String,
    pub at: u64,
    pub result: ConsentStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExposureReceipt {
    pub epoch: u64,
    pub cutoff: u64,
    pub request_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Mutation {
    Choice(UseChoice),
    Confirm {
        key: String,
        exact: String,
    },
    Remember {
        text: String,
        receipt: Option<String>,
    },
    Correct {
        old: String,
        text: String,
        receipt: Option<String>,
    },
    Forget {
        exact: String,
    },
}
pub(crate) fn request_digest(
    action: &SelfAuthorizedFactAction,
    mutation: &Mutation,
) -> Result<String, MemoryConsentError> {
    let bytes = serde_json::to_vec(&(
        "personal-memory-request-v1",
        &action.proof,
        action.expected,
        mutation,
    ))
    .map_err(|_| MemoryConsentError::Bounds)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub fn valid_scope(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(|c| c.is_control())
}
pub fn valid_subject_key(key: &str) -> bool {
    let mut parts = key.split('\u{1f}');
    matches!((parts.next(),parts.next(),parts.next()),(Some(g),Some(u),None) if valid_scope(g)&&valid_scope(u))
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub(crate) fn status(stores: &crate::persist::Stores, guild: &str, user: &str) -> ConsentStatus {
    let fallback = PersonalMemorySubject::default();
    let s = stores
        .personal_memory
        .get(&subject_key(guild, user))
        .unwrap_or(&fallback);
    let facts = stores.memory.facts(guild, user);
    let eligible = facts.iter().filter(|f| s.eligible(guild, user, f)).count();
    ConsentStatus {
        choice: if s.activation_pending || s.outcomes.values().any(|r| !r.completed) {
            UseChoice::Off
        } else {
            s.choice
        },
        stamp: s.stamp(stores.personal_memory_exposure.epoch),
        policy_version: PERSONAL_MEMORY_POLICY_VERSION,
        eligible_facts: eligible,
        unverified_facts: facts.len() - eligible,
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryUsePermitSet {
    exposure_epoch: u64,
    contributors: BTreeMap<String, ConsentStamp>,
    #[serde(default)]
    scope: Option<String>,
    // A process-local service seal is never restored from serialized context.
    #[serde(skip)]
    context_digest: Option<String>,
}
fn context_digest(facts: &[String]) -> String {
    let mut h = Sha256::new();
    h.update(b"abbey-personal-context-v1");
    for fact in facts {
        h.update((fact.len() as u64).to_be_bytes());
        h.update(fact.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn subject_key(guild: &str, user: &str) -> String {
    format!("{guild}\u{1f}{user}")
}
pub fn fact_key(guild: &str, user: &str, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"abbey-personal-fact-v1");
    for s in [guild, user, text] {
        h.update((s.len() as u64).to_be_bytes());
        h.update(s.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn validate_metadata(
    subjects: &BTreeMap<String, PersonalMemorySubject>,
    exposure: &ExposureState,
) -> Result<(), MemoryConsentError> {
    if subjects.len() > MAX_SUBJECTS
        || exposure.schema > 1
        || exposure.epoch == u64::MAX
        || exposure.scope_epochs.len() > MAX_SUBJECTS
        || exposure.receipts.len() > 128
        || exposure
            .scope_epochs
            .iter()
            .any(|(key, epoch)| !valid_subject_key(key) || *epoch > exposure.epoch)
        || exposure.receipts.iter().any(|r| {
            !valid_digest(&r.request_digest)
                || r.epoch > exposure.epoch
                || r.cutoff > exposure.cutoff
        })
    {
        return Err(MemoryConsentError::Bounds);
    }
    let bytes =
        serde_json::to_vec(&(subjects, exposure)).map_err(|_| MemoryConsentError::Bounds)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(MemoryConsentError::Bounds);
    }
    for (key, s) in subjects {
        if !valid_subject_key(key)
            || s.schema > 1
            || s.revision == u64::MAX
            || s.consent_epoch == u64::MAX
            || (s.policy_version != 0 && s.policy_version != PERSONAL_MEMORY_POLICY_VERSION)
            || s.proofs.len() > 100
            || s.outcomes.len() > 128
            || (s.choice == UseChoice::On
                && (s.schema != 1 || s.policy_version != PERSONAL_MEMORY_POLICY_VERSION))
        {
            return Err(MemoryConsentError::Bounds);
        }
        for (digest, p) in &s.proofs {
            if digest.len() != 64
                || !valid_digest(digest)
                || p.fact_key != *digest
                || p.member.at == 0
                || p.previous_revision > s.revision
                || p.member.actor != p.member.subject
                || subject_key(&p.member.guild, &p.member.subject) != *key
                || SelfAuthorizedFactAction::new(p.member.clone(), ConsentStamp::default()).is_err()
            {
                return Err(MemoryConsentError::InvalidProof);
            }
        }
        if s.outcomes.iter().any(|(id, r)| {
            id.is_empty()
                || id.len() > 64
                || !valid_digest(&r.payload_digest)
                || r.at == 0
                || r.result.policy_version != PERSONAL_MEMORY_POLICY_VERSION
                || r.result.stamp.revision > s.revision
                || r.result.stamp.consent_epoch > s.consent_epoch
                || r.result.stamp.exposure_epoch > exposure.epoch
                || r.result
                    .eligible_facts
                    .checked_add(r.result.unverified_facts)
                    .is_none_or(|count| count > 100)
        }) {
            return Err(MemoryConsentError::Bounds);
        }
    }
    Ok(())
}
impl PersonalMemorySubject {
    pub fn stamp(&self, exposure_epoch: u64) -> ConsentStamp {
        ConsentStamp {
            revision: self.revision,
            consent_epoch: self.consent_epoch,
            exposure_epoch,
        }
    }
    pub fn eligible(&self, guild: &str, user: &str, text: &str) -> bool {
        if self.activation_pending
            || self.outcomes.values().any(|r| !r.completed)
            || self.schema != 1
            || self.choice != UseChoice::On
            || self.policy_version != PERSONAL_MEMORY_POLICY_VERSION
        {
            return false;
        }
        let key = fact_key(guild, user, text);
        self.proofs.get(&key).is_some_and(|p| {
            p.fact_key == key
                && p.member.actor == user
                && p.member.subject == user
                && p.member.guild == guild
                && p.member.policy_version == PERSONAL_MEMORY_POLICY_VERSION
                && !p.member.interaction_id.is_empty()
        })
    }
    pub fn advance(&mut self) -> Result<(), MemoryConsentError> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(MemoryConsentError::Bounds)?;
        let consent_epoch = self
            .consent_epoch
            .checked_add(1)
            .ok_or(MemoryConsentError::Bounds)?;
        self.revision = revision;
        self.consent_epoch = consent_epoch;
        Ok(())
    }
}
impl MemoryUsePermitSet {
    pub fn empty(exposure_epoch: u64) -> Self {
        Self {
            exposure_epoch,
            contributors: BTreeMap::new(),
            scope: None,
            context_digest: None,
        }
    }
    pub fn for_subject(
        guild: &str,
        user: &str,
        s: &PersonalMemorySubject,
        exposure_epoch: u64,
    ) -> Self {
        let mut value = Self::empty(exposure_epoch);
        value.scope = Some(subject_key(guild, user));
        if !s.activation_pending
            && s.outcomes.values().all(|r| r.completed)
            && s.schema == 1
            && s.choice == UseChoice::On
            && s.policy_version == PERSONAL_MEMORY_POLICY_VERSION
        {
            value
                .contributors
                .insert(subject_key(guild, user), s.stamp(exposure_epoch));
        }
        value
    }
    pub fn exposure_epoch(&self) -> u64 {
        self.exposure_epoch
    }
    pub fn scope_identity(&self) -> Option<&str> {
        self.scope.as_deref()
    }
    pub(crate) fn bind_context(&mut self, facts: &[String]) {
        self.context_digest = Some(context_digest(facts));
    }
    pub fn is_context_sealed(&self) -> bool {
        self.context_digest.is_some()
    }
    pub fn validates_context(&self, facts: &[String], summary: &str) -> bool {
        self.scope.is_some()
            && summary.is_empty()
            && self.context_digest.as_ref() == Some(&context_digest(facts))
    }
    pub fn authorizes_personal_memory(&self) -> bool {
        !self.contributors.is_empty()
    }
    pub fn validate(
        &self,
        subjects: &BTreeMap<String, PersonalMemorySubject>,
        exposure_epoch: u64,
    ) -> bool {
        self.exposure_epoch == exposure_epoch
            && self.contributors.iter().all(|(key, stamp)| {
                subjects.get(key).is_some_and(|s| {
                    !s.activation_pending
                        && s.outcomes.values().all(|r| r.completed)
                        && s.schema == 1
                        && s.choice == UseChoice::On
                        && s.policy_version == PERSONAL_MEMORY_POLICY_VERSION
                        && s.stamp(exposure_epoch) == *stamp
                })
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_is_off() {
        assert_eq!(PersonalMemorySubject::default().choice, UseChoice::Off)
    }
    #[test]
    fn exact_scopes_bound_keys() {
        assert_ne!(fact_key("a", "bc", "d"), fact_key("ab", "c", "d"));
        assert_ne!(fact_key("a", "b", "c"), fact_key("a", "b", "C"));
    }
    #[test]
    fn cross_subject_proof_refused() {
        assert!(
            SelfAuthorizedFactAction::new(
                MemberProof {
                    actor: "a".into(),
                    subject: "b".into(),
                    guild: "g".into(),
                    interaction_id: "i".into(),
                    platform: "discord".into(),
                    at: 1,
                    policy_version: 1
                },
                ConsentStamp::default()
            )
            .is_err()
        );
    }
    #[test]
    fn exposure_invalidates_empty() {
        assert!(!MemoryUsePermitSet::empty(1).validate(&BTreeMap::new(), 2));
    }
}
