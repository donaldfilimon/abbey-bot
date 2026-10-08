//! Pure confirmed continuity domain. Time, boot entropy and current canonical
//! Work authority are supplied by callers. Proposals/grants stay transient.
use super::{WorkAccess, WorkContentRef, WorkError, WorkScope, WorkStore};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CARDS: usize = 256;
const MAX_PROPOSALS: usize = 256;
const MAX_TEXT_BYTES: usize = 1600;
const MAX_SOURCES: usize = 8;
const CARD_LIFETIME: u64 = 604800;
const PROPOSAL_LIFETIME: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProposalId {
    boot_nonce: [u8; 16],
    sequence: u64,
}

impl ProposalId {
    pub fn encode(self) -> String {
        let nonce: String = self
            .boot_nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("{nonce}:{}", self.sequence)
    }

    pub fn decode(encoded: &str) -> Result<Self, WorkError> {
        let (nonce, sequence) = encoded.split_once(':').ok_or(WorkError::Invalid)?;
        if nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(WorkError::Invalid);
        }
        let sequence: u64 = sequence.parse().map_err(|_| WorkError::Invalid)?;
        let mut boot_nonce = [0; 16];
        for (index, byte) in boot_nonce.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&nonce[index * 2..index * 2 + 2], 16)
                .map_err(|_| WorkError::Invalid)?;
        }
        let id = Self {
            boot_nonce,
            sequence,
        };
        if sequence == 0 || id.encode() != encoded {
            return Err(WorkError::Invalid);
        }
        Ok(id)
    }
}

/// Input is a proposal, never durable authority. Exact text is not trimmed.
#[derive(Debug)]
pub struct ContinuityDraft {
    pub scope: WorkScope,
    pub base_revision: u64,
    pub presented_text: String,
    pub source_refs: BTreeSet<WorkContentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuityProposal {
    pub id: ProposalId,
    pub actor: u64,
    pub scope: WorkScope,
    pub base_revision: u64,
    pub presented_text: String,
    pub source_refs: BTreeSet<WorkContentRef>,
    pub expires_at: u64,
}

/// Only registry resolution can construct this one-use, nonserializable grant.
/// The native human shell owns that registry; models receive no registry API.
#[derive(Debug)]
pub struct ResolvedConfirmation {
    proposal: ContinuityProposal,
}
impl ResolvedConfirmation {
    pub(crate) fn expires_at(&self) -> u64 {
        self.proposal.expires_at
    }
    pub(crate) fn scope(&self) -> &WorkScope {
        &self.proposal.scope
    }
}

#[derive(Debug)]
pub struct ProposalRegistry {
    boot_nonce: [u8; 16],
    sequence: u64,
    proposals: BTreeMap<ProposalId, ContinuityProposal>,
}

impl ProposalRegistry {
    /// The infrastructure caller supplies a fresh OS-random boot nonce.
    pub fn new(boot_nonce: [u8; 16]) -> Self {
        Self {
            boot_nonce,
            sequence: 0,
            proposals: BTreeMap::new(),
        }
    }

    pub fn propose(
        &mut self,
        draft: ContinuityDraft,
        access: &WorkAccess,
        work: &WorkStore,
        now: u64,
    ) -> Result<ContinuityProposal, WorkError> {
        authorize_scope(&draft.scope, access, work, false)?;
        validate_content(&draft.presented_text, &draft.source_refs)?;
        validate_sources(&draft.scope, &draft.source_refs, work)?;
        let expires_at = now
            .checked_add(PROPOSAL_LIFETIME)
            .ok_or(WorkError::Invalid)?;
        let sequence = self.sequence.checked_add(1).ok_or(WorkError::Full)?;
        if self
            .proposals
            .values()
            .filter(|p| p.expires_at > now)
            .count()
            >= MAX_PROPOSALS
        {
            return Err(WorkError::Full);
        }
        let proposal = ContinuityProposal {
            id: ProposalId {
                boot_nonce: self.boot_nonce,
                sequence,
            },
            actor: access.actor,
            scope: draft.scope,
            base_revision: draft.base_revision,
            presented_text: draft.presented_text,
            source_refs: draft.source_refs,
            expires_at,
        };
        self.proposals.retain(|_, p| p.expires_at > now);
        self.sequence = sequence;
        self.proposals.insert(proposal.id, proposal.clone());
        Ok(proposal)
    }

    pub fn resolve_confirmation(
        &mut self,
        id: ProposalId,
        actor: u64,
        scope: &WorkScope,
        now: u64,
    ) -> Result<ResolvedConfirmation, WorkError> {
        let proposal = self.proposals.get(&id).ok_or(WorkError::Missing)?;
        if actor != proposal.actor || scope != &proposal.scope {
            return Err(WorkError::Denied);
        }
        if !within_lifetime(proposal.expires_at, PROPOSAL_LIFETIME, now) {
            if now >= proposal.expires_at {
                self.proposals.remove(&id);
            }
            return Err(WorkError::Stale);
        }
        let proposal = self.proposals.remove(&id).ok_or(WorkError::Missing)?;
        Ok(ResolvedConfirmation { proposal })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuityCard {
    pub schema_version: u8,
    pub scope: WorkScope,
    pub confirmed_by: u64,
    pub revision: u64,
    pub confirmed_text: String,
    pub source_refs: BTreeSet<WorkContentRef>,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_receipt: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContinuityStore {
    cards: BTreeMap<WorkScope, ContinuityCard>,
}

// Array wire form supports structured scope keys and rejects duplicate scopes.
// Runtime/canonical integration and expiry pruning on restore belong to Task 3.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadedContinuity {
    cards: Vec<ContinuityCard>,
}

impl<'de> Deserialize<'de> for ContinuityStore {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let loaded = LoadedContinuity::deserialize(deserializer)?;
        if loaded.cards.len() > MAX_CARDS {
            return Err(serde::de::Error::custom("continuity card limit"));
        }
        let mut cards = BTreeMap::new();
        for card in loaded.cards {
            validate_card(&card).map_err(serde::de::Error::custom)?;
            if cards.insert(card.scope.clone(), card).is_some() {
                return Err(serde::de::Error::custom("duplicate continuity scope"));
            }
        }
        Ok(Self { cards })
    }
}

impl Serialize for ContinuityStore {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct SavedContinuity<'a> {
            cards: Vec<&'a ContinuityCard>,
        }
        SavedContinuity {
            cards: self.cards.values().collect(),
        }
        .serialize(serializer)
    }
}

/// Private construction prevents raw loaded/proposed text becoming prompt authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedContinuity {
    card: ContinuityCard,
    actor: u64,
    channel: u64,
}
impl AuthorizedContinuity {
    pub fn text(&self) -> &str {
        &self.card.confirmed_text
    }
    pub(crate) fn card(&self) -> &ContinuityCard {
        &self.card
    }
    pub(crate) fn actor(&self) -> u64 {
        self.actor
    }
    pub(crate) fn channel(&self) -> u64 {
        self.channel
    }
    /// Canonical checks supplement fresh transport access; they never mint it.
    pub(crate) fn current(&self, cards: &ContinuityStore, work: &WorkStore, now: u64) -> bool {
        let access = WorkAccess {
            actor: self.actor,
            channel: self.channel,
            can_view: true,
            can_manage: false,
            guild: match self.card.scope {
                WorkScope::Personal { .. } => None,
                WorkScope::Team { guild, .. } => Some(guild),
            },
        };
        cards
            .context(&self.card.scope, &access, work, now)
            .is_some_and(|current| current == *self)
    }
}

impl ContinuityStore {
    pub(crate) fn replace_exact(
        &mut self,
        scope: &WorkScope,
        expected: Option<&ContinuityCard>,
        replacement: Option<&ContinuityCard>,
    ) -> Result<(), WorkError> {
        let current = self.cards.get(scope);
        if current != expected && current != replacement {
            return Err(WorkError::Stale);
        }
        if let Some(card) = replacement {
            if card.scope != *scope {
                return Err(WorkError::Invalid);
            }
            validate_card(card)?;
            self.cards.insert(scope.clone(), card.clone());
        } else {
            self.cards.remove(scope);
        }
        Ok(())
    }
    pub(crate) fn card(&self, scope: &WorkScope) -> Option<&ContinuityCard> {
        self.cards.get(scope)
    }
    pub(crate) fn install_receipt(
        &mut self,
        card: &ContinuityCard,
        receipt: String,
    ) -> Result<ContinuityCard, WorkError> {
        if self.cards.get(&card.scope) != Some(card) {
            return Err(WorkError::Stale);
        }
        let mut admitted = card.clone();
        admitted.episode_receipt = Some(receipt);
        validate_card(&admitted)?;
        self.cards.insert(admitted.scope.clone(), admitted.clone());
        Ok(admitted)
    }
    pub(crate) fn remove_exact(&mut self, card: &ContinuityCard) -> Result<(), WorkError> {
        if self.cards.get(&card.scope) != Some(card) {
            return Err(WorkError::Stale);
        }
        self.cards.remove(&card.scope);
        Ok(())
    }
    /// Infrastructure supplies current time after decoding the canonical file.
    /// Sources are native joins, never reconstructed from a projection/cache.
    pub(crate) fn prune_current(&mut self, work: &WorkStore, now: u64) {
        self.cards.retain(|scope, card| {
            card.episode_receipt.is_some()
                || (within_lifetime(card.expires_at, CARD_LIFETIME, now)
                    && validate_sources(scope, &card.source_refs, work).is_ok()
                    && work
                        .projects
                        .values()
                        .any(|project| &project.scope == scope))
        });
    }

    pub(crate) fn clear(
        &mut self,
        scope: &WorkScope,
        access: &WorkAccess,
        work: &WorkStore,
    ) -> Result<Option<ContinuityCard>, WorkError> {
        authorize_scope(scope, access, work, true)?;
        Ok(self.cards.remove(scope))
    }

    pub(crate) fn erasure_targets(
        &self,
        scope: &str,
        member: Option<u64>,
        work: &WorkStore,
    ) -> Vec<ContinuityCard> {
        self.cards
            .values()
            .filter(|card| {
                let native_scope = match card.scope {
                    WorkScope::Personal { owner } => format!("discord:dm:{owner}"),
                    WorkScope::Team { guild, .. } => format!("discord:{guild}"),
                };
                native_scope == scope && card.linked_to(member, work)
            })
            .cloned()
            .collect()
    }

    pub fn confirm(
        &mut self,
        grant: ResolvedConfirmation,
        access: &WorkAccess,
        work: &WorkStore,
        now: u64,
    ) -> Result<ContinuityCard, WorkError> {
        let proposal = grant.proposal;
        if proposal.actor != access.actor {
            return Err(WorkError::Denied);
        }
        authorize_scope(&proposal.scope, access, work, true)?;
        if !within_lifetime(proposal.expires_at, PROPOSAL_LIFETIME, now) {
            return Err(WorkError::Stale);
        }
        validate_content(&proposal.presented_text, &proposal.source_refs)?;
        validate_sources(&proposal.scope, &proposal.source_refs, work)?;
        let current_revision = self
            .cards
            .get(&proposal.scope)
            .map_or(0, |card| card.revision);
        if current_revision != proposal.base_revision {
            return Err(WorkError::Stale);
        }
        let revision = current_revision.checked_add(1).ok_or(WorkError::Full)?;
        let expires_at = now.checked_add(CARD_LIFETIME).ok_or(WorkError::Invalid)?;
        let active_elsewhere = self
            .cards
            .values()
            .filter(|card| {
                card.scope != proposal.scope
                    && (card.expires_at > now || card.episode_receipt.is_some())
            })
            .count();
        if active_elsewhere >= MAX_CARDS {
            return Err(WorkError::Full);
        }
        let card = ContinuityCard {
            schema_version: 1,
            scope: proposal.scope,
            confirmed_by: proposal.actor,
            revision,
            confirmed_text: proposal.presented_text,
            source_refs: proposal.source_refs,
            expires_at,
            episode_receipt: None,
        };
        // Rejections above never alter live cards; prune only on successful commit.
        self.cards
            .retain(|_, card| card.expires_at > now || card.episode_receipt.is_some());
        self.cards.insert(card.scope.clone(), card.clone());
        Ok(card)
    }

    pub fn context(
        &self,
        scope: &WorkScope,
        access: &WorkAccess,
        work: &WorkStore,
        now: u64,
    ) -> Option<AuthorizedContinuity> {
        let card = self.cards.get(scope)?;
        authorize_scope(scope, access, work, false).ok()?;
        validate_card(card).ok()?;
        if !within_lifetime(card.expires_at, CARD_LIFETIME, now) {
            return None;
        }
        validate_sources(scope, &card.source_refs, work).ok()?;
        Some(AuthorizedContinuity {
            card: card.clone(),
            actor: access.actor,
            channel: access.channel,
        })
    }
}

impl ContinuityCard {
    pub(crate) fn linked_to(&self, member: Option<u64>, work: &WorkStore) -> bool {
        member.is_none_or(|member| {
            self.confirmed_by == member
                || self.source_refs.iter().any(|source| match source {
                    WorkContentRef::Task { project, id, .. } => {
                        work.tasks.get(id).is_some_and(|t| {
                            t.id == *id
                                && t.project_id == *project
                                && (t.owner == member || t.assignee == Some(member))
                        })
                    }
                    WorkContentRef::Decision { project, id, .. } => {
                        work.decisions.get(id).is_some_and(|d| {
                            d.id == *id && d.project_id == *project && d.author == member
                        })
                    }
                })
        })
    }
}

fn valid_scope(scope: &WorkScope) -> bool {
    match scope {
        WorkScope::Personal { owner } => *owner != 0,
        WorkScope::Team { guild, channel } => *guild != 0 && *channel != 0,
    }
}

fn authorize_scope(
    scope: &WorkScope,
    access: &WorkAccess,
    work: &WorkStore,
    manager: bool,
) -> Result<(), WorkError> {
    if !valid_scope(scope)
        || access.actor == 0
        || access.channel == 0
        || !access.can_view
        || scope != &access.scope()
    {
        return Err(WorkError::Denied);
    }
    work.scope_projects(scope, *access, manager)?;
    Ok(())
}

fn validate_content(text: &str, refs: &BTreeSet<WorkContentRef>) -> Result<(), WorkError> {
    if text.trim().is_empty()
        || text.len() > MAX_TEXT_BYTES
        || text.chars().any(|c| c.is_control() && c != '\n')
        || refs.len() > MAX_SOURCES
    {
        return Err(WorkError::Invalid);
    }
    for source in refs {
        let valid = match source {
            WorkContentRef::Task { project, id, .. } => *project != 0 && *id != 0,
            WorkContentRef::Decision {
                project,
                id,
                revision,
            } => *project != 0 && *id != 0 && *revision == 1,
        };
        if !valid {
            return Err(WorkError::Invalid);
        }
    }
    Ok(())
}

fn validate_card(card: &ContinuityCard) -> Result<(), WorkError> {
    if card.schema_version != 1
        || card.revision == 0
        || card.confirmed_by == 0
        || !valid_scope(&card.scope)
        || card.expires_at.checked_sub(CARD_LIFETIME).is_none()
        || matches!(card.scope, WorkScope::Personal { owner } if owner != card.confirmed_by)
    {
        return Err(WorkError::Invalid);
    }
    if let Some(receipt) = &card.episode_receipt
        && (receipt.len() != 64
            || !receipt
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        return Err(WorkError::Invalid);
    }
    validate_content(&card.confirmed_text, &card.source_refs)
}

fn within_lifetime(expires_at: u64, lifetime: u64, now: u64) -> bool {
    expires_at
        .checked_sub(lifetime)
        .is_some_and(|created| created <= now && now < expires_at)
}

fn validate_sources(
    scope: &WorkScope,
    refs: &BTreeSet<WorkContentRef>,
    work: &WorkStore,
) -> Result<(), WorkError> {
    for source in refs {
        let (project, native_matches) = match source {
            WorkContentRef::Task {
                project,
                id,
                revision,
            } => (
                *project,
                work.tasks.get(id).is_some_and(|task| {
                    task.id == *id && task.project_id == *project && task.revision == *revision
                }),
            ),
            WorkContentRef::Decision {
                project,
                id,
                revision,
            } => (
                *project,
                work.decisions.get(id).is_some_and(|decision| {
                    decision.id == *id && decision.project_id == *project && *revision == 1
                }),
            ),
        };
        if !native_matches
            || work
                .projects
                .get(&project)
                .is_none_or(|p| p.id != project || &p.scope != scope)
        {
            return Err(WorkError::Stale);
        }
    }
    Ok(())
}

#[cfg(test)]
mod erasure_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod tests;
