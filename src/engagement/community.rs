//! Public facilitation uses independent guild switches and scoped evidence.
use super::schedule::CandidateProposal;
use super::*;
pub(crate) mod assessment;
const DAY: u64 = 86_400;
pub const WEEK: u64 = DAY * 7;
/// Metadata only. The linked candidate owns the scope and terminal dedupe identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunityReceipt {
    pub policy_revision: u64,
    #[serde(default)]
    pub attempted_at: Option<u64>,
    pub evidence: CommunityEvidence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CommunityEvidence {
    Message,
    Join {
        member: u64,
        joined_at: u64,
    },
    Project {
        project: u64,
        revision: u64,
        actor: u64,
        audience: BTreeSet<u64>,
        content: BTreeSet<crate::work::WorkContentRef>,
    },
}
/// Shell-created proofs are transient; no source text or inferred consent persists.
#[derive(Debug, Clone)]
pub struct CommunityFact {
    pub kind: EngagementKind,
    pub scope: EngagementScope,
    pub source: Option<SourceRef>,
    pub evidence: CommunityEvidence,
    pub at: u64,
    pub useful: bool,
    pub current: bool,
}
#[derive(Debug, Clone, Default)]
pub struct CommunityFacts {
    pub rows: Vec<CommunityFact>,
    pub join_events_available: bool,
}
pub fn feature(kind: EngagementKind) -> Option<CommunityFeature> {
    match kind {
        EngagementKind::ConversationStarter => Some(CommunityFeature::Starters),
        EngagementKind::UnansweredQuestion => Some(CommunityFeature::Questions),
        EngagementKind::Welcome => Some(CommunityFeature::Welcomes),
        EngagementKind::ProjectCheckIn => Some(CommunityFeature::Projects),
        _ => None,
    }
}
pub fn enabled(store: &EngagementStore, scope: &EngagementScope, kind: EngagementKind) -> bool {
    let EngagementScope::Guild { guild, channel } = scope else {
        return false;
    };
    feature(kind).is_some_and(|f| {
        store.guild_features.get(guild).is_some_and(|p| {
            p.enabled.contains(&f) && p.channels.get(&f).is_some_and(|c| c.contains(channel))
        })
    })
}
fn valid_project_content(project: u64, content: &BTreeSet<crate::work::WorkContentRef>) -> bool {
    let bounded = !content.is_empty() && content.len() <= 8;
    let exact_ids = content.iter().all(|source| match source {
        crate::work::WorkContentRef::Task {
            project: source_project,
            id,
            revision,
        } => *source_project == project && *id != 0 && *revision != 0,
        _ => false,
    });
    bounded && exact_ids
}
fn fact_valid(f: &CommunityFact, facts: &CommunityFacts, now: u64) -> bool {
    if !f.current || !f.useful || f.at > now || f.scope.validate().is_err() {
        return false;
    }
    match (&f.evidence, f.kind, &f.source) {
        (
            CommunityEvidence::Message,
            EngagementKind::ConversationStarter | EngagementKind::UnansweredQuestion,
            Some(s),
        ) => {
            s.validate().is_ok()
                && s.scope == f.scope
                && s.at == f.at
                && f.at.checked_add(DAY).is_some_and(|t| t <= now)
        }
        (CommunityEvidence::Join { member, joined_at }, EngagementKind::Welcome, None) => {
            facts.join_events_available && *member != 0 && *joined_at == f.at && now - f.at < 3600
        }
        (
            CommunityEvidence::Project {
                project,
                revision,
                actor,
                audience,
                content,
            },
            EngagementKind::ProjectCheckIn,
            None,
        ) => {
            *project != 0
                && *revision != 0
                && *actor != 0
                && !audience.contains(&0)
                && audience.contains(actor)
                && audience.len() <= 1000
                && valid_project_content(*project, content)
        }
        _ => false,
    }
}
pub fn community_candidates(
    store: &EngagementStore,
    facts: &CommunityFacts,
    now: u64,
) -> Vec<CandidateProposal> {
    if crate::calendar::utc(now).is_err() || facts.rows.len() > 1000 {
        return Vec::new();
    }
    let mut proposals = Vec::new();
    for f in &facts.rows {
        if matches!(f.evidence, CommunityEvidence::Join { .. })
            && store.community_receipts.iter().any(|(id, r)| {
                r.evidence == f.evidence
                    && store
                        .candidates
                        .get(id)
                        .is_some_and(|c| match (&c.scope, &f.scope) {
                            (
                                EngagementScope::Guild { guild: a, .. },
                                EngagementScope::Guild { guild: b, .. },
                            ) => a == b,
                            _ => false,
                        })
            })
        {
            continue;
        }
        if !fact_valid(f, facts, now) || !enabled(store, &f.scope, f.kind) {
            continue;
        }
        let member = match &f.evidence {
            CommunityEvidence::Project { actor, .. } => Some(*actor),
            _ => None,
        };
        if member.is_some_and(|m| {
            store
                .member_policies
                .get(&m)
                .is_none_or(|p| !p.personalized_enabled())
        }) {
            continue;
        }
        let interval = if f.kind == EngagementKind::ProjectCheckIn {
            WEEK
        } else {
            DAY
        };
        if store.candidates.values().any(|c| {
            if c.scope != f.scope || c.kind != f.kind {
                return false;
            }
            let receipt = store.community_receipts.get(&c.id);
            let recent = receipt
                .and_then(|r| r.attempted_at)
                .unwrap_or(c.due_at)
                .saturating_add(interval)
                > now;
            match &f.evidence {
                CommunityEvidence::Join { .. } => receipt.is_some_and(|r| r.evidence == f.evidence),
                CommunityEvidence::Project { project, .. } => {
                    recent
                        && receipt.is_some_and(|r| match &r.evidence {
                            CommunityEvidence::Project { project: old, .. } => old == project,
                            _ => false,
                        })
                }
                CommunityEvidence::Message => {
                    c.source.as_ref().is_some_and(|s| {
                        f.source
                            .as_ref()
                            .is_some_and(|new| s.message == new.message)
                    }) || (f.kind == EngagementKind::ConversationStarter && recent)
                }
            }
        }) {
            continue;
        }
        if proposals
            .iter()
            .any(|p: &CandidateProposal| p.scope == f.scope && p.kind == f.kind)
        {
            continue;
        }
        proposals.push(CandidateProposal {
            kind: f.kind,
            source: f.source.clone(),
            member,
            scope: f.scope.clone(),
            due_at: now,
            introduction_id: None,
        });
    }
    proposals
}
impl EngagementStore {
    pub fn propose_community(
        &mut self,
        facts: &CommunityFacts,
        now: u64,
    ) -> Result<usize, WorkError> {
        let proposals = community_candidates(self, facts, now);
        let mut next = self.clone();
        let mut count = 0;
        for p in proposals {
            let fact = facts
                .rows
                .iter()
                .find(|f| {
                    f.kind == p.kind
                        && f.scope == p.scope
                        && f.source == p.source
                        && match (&f.evidence, p.member) {
                            (CommunityEvidence::Project { actor, .. }, Some(m)) => *actor == m,
                            (_, None) => true,
                            _ => false,
                        }
                })
                .ok_or(WorkError::Invalid)?;
            // Insert metadata alongside the prospective candidate, before validating it.
            let id = next.sequence.checked_add(1).ok_or(WorkError::Full)?;
            let EngagementScope::Guild { guild, .. } = p.scope else {
                return Err(WorkError::Invalid);
            };
            next.community_receipts.insert(
                id,
                CommunityReceipt {
                    policy_revision: next.guild_features[&guild].revision,
                    attempted_at: None,
                    evidence: fact.evidence.clone(),
                },
            );
            if next.propose(p, now)?.is_some() {
                count += 1;
            } else {
                next.community_receipts.remove(&id);
            }
        }
        next.validate()?;
        *self = next;
        Ok(count)
    }
    pub(crate) fn community_capacity(&self, c: &Candidate, now: u64) -> bool {
        let interval = match c.kind {
            EngagementKind::ConversationStarter => DAY,
            EngagementKind::ProjectCheckIn => WEEK,
            _ => return true,
        };
        self.community_receipts.iter().all(|(id, r)| {
            if *id == c.id
                || r.attempted_at
                    .is_none_or(|t| t.saturating_add(interval) <= now)
            {
                return true;
            }
            let Some(old) = self.candidates.get(id) else {
                return false;
            };
            if old.kind != c.kind || old.scope != c.scope {
                return true;
            }
            if c.kind == EngagementKind::ProjectCheckIn {
                match (
                    &r.evidence,
                    self.community_receipts.get(&c.id).map(|r| &r.evidence),
                ) {
                    (
                        CommunityEvidence::Project { project: a, .. },
                        Some(CommunityEvidence::Project { project: b, .. }),
                    ) => a != b,
                    _ => false,
                }
            } else {
                false
            }
        })
    }
    pub(crate) fn community_receipt_current(&self, c: &Candidate) -> bool {
        if !enabled(self, &c.scope, c.kind) {
            return false;
        }
        let EngagementScope::Guild { guild, .. } = c.scope else {
            return false;
        };
        self.community_receipts.get(&c.id).is_some_and(|r| {
            self.guild_features
                .get(&guild)
                .is_some_and(|p| p.revision == r.policy_revision)
        })
    }
    pub(super) fn validate_community(&self) -> Result<(), WorkError> {
        if let Some(cursor) = &self.community_cursor {
            cursor.validate()?;
            crate::calendar::utc(cursor.at)?;
            if !matches!(cursor.scope, EngagementScope::Guild { .. }) {
                return Err(WorkError::Invalid);
            }
        }

        if self.community_receipts.len() > 10_000 {
            return Err(WorkError::Full);
        }
        for (id, r) in &self.community_receipts {
            let c = self.candidates.get(id).ok_or(WorkError::Invalid)?;
            if feature(c.kind).is_none()
                || !matches!(c.scope, EngagementScope::Guild { .. })
                || c.destination != DestinationPreference::Origin
            {
                return Err(WorkError::Invalid);
            }
            let valid = match (&r.evidence, c.kind) {
                (
                    CommunityEvidence::Message,
                    EngagementKind::ConversationStarter | EngagementKind::UnansweredQuestion,
                ) => c.source.is_some() && c.member.is_none(),
                (CommunityEvidence::Join { member, joined_at }, EngagementKind::Welcome) => {
                    *member != 0
                        && *joined_at <= c.due_at
                        && c.due_at - *joined_at < 3600
                        && c.source.is_none()
                        && c.member.is_none()
                }
                (
                    CommunityEvidence::Project {
                        project,
                        revision,
                        actor,
                        audience,
                        content,
                    },
                    EngagementKind::ProjectCheckIn,
                ) => {
                    *project != 0
                        && *revision != 0
                        && *actor != 0
                        && audience.contains(actor)
                        && !audience.contains(&0)
                        && audience.len() <= 1000
                        && valid_project_content(*project, content)
                        && c.source.is_none()
                        && c.member == Some(*actor)
                }
                _ => false,
            };
            if r.attempted_at
                .is_some_and(|t| t < c.due_at || crate::calendar::utc(t).is_err())
                || (matches!(
                    c.state,
                    CandidateState::Reserved
                        | CandidateState::Sent
                        | CandidateState::ReviewRequired
                ) && r.attempted_at.is_none())
            {
                return Err(WorkError::Invalid);
            }
            if !valid {
                return Err(WorkError::Invalid);
            }
        }
        // Old unsupported public records stay loadable, but cannot reserve without proof.
        Ok(())
    }
    pub(crate) fn cancel_public_origin(&mut self, scope: &EngagementScope, after: u64) -> usize {
        let mut n = 0;
        for c in self.candidates.values_mut() {
            if &c.scope == scope
                && matches!(
                    c.kind,
                    EngagementKind::ConversationStarter | EngagementKind::UnansweredQuestion
                )
                && c.source.as_ref().is_some_and(|s| s.at < after)
                && matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
            {
                c.state = CandidateState::Cancelled;
                n += 1;
            }
        }
        n
    }
}
#[cfg(test)]
mod tests;
