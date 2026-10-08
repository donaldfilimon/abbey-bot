//! Recomputable replay safety commitments; no source text or native IDs retained.
use super::*;
use sha2::{Digest, Sha256};
fn digest(value: impl Serialize) -> String {
    let bytes = serde_json::to_vec(&value).expect("closed engagement identity is serializable");
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn invitation(interaction: u64) -> String {
    digest(("erased-invitation-v1", interaction))
}
pub(super) fn introduction(scope: &EngagementScope, mut members: [u64; 2]) -> String {
    members.sort_unstable();
    digest(("erased-introduction-v1", scope.learning_scope(), members))
}
pub(super) fn proposal(p: &schedule::CandidateProposal) -> String {
    digest((
        "erased-candidate-v1",
        p.kind,
        &p.scope,
        p.member,
        p.source.as_ref().map(|s| s.message),
        if p.kind == EngagementKind::WeeklyCheckIn {
            p.due_at
        } else {
            0
        },
    ))
}
pub(super) fn candidate(c: &Candidate) -> String {
    proposal(&schedule::CandidateProposal {
        kind: c.kind,
        scope: c.scope.clone(),
        member: c.member,
        source: c.source.clone(),
        due_at: c.due_at,
        introduction_id: c.introduction_id,
    })
}
pub(super) fn community(
    scope: &EngagementScope,
    kind: EngagementKind,
    evidence: &community::CommunityEvidence,
    source: Option<&SourceRef>,
) -> String {
    digest((
        "erased-community-v1",
        if matches!(evidence, community::CommunityEvidence::Join { .. }) {
            scope.learning_scope()
        } else {
            String::from(scope.clone())
        },
        kind,
        evidence,
        source.map(|s| s.message),
    ))
}

pub(super) fn project(project: u64) -> String {
    digest(("erased-project-budget-v1", project))
}

/// Native Work scope/task/revision replay protection survives candidate erasure.
pub(super) fn task_follow_up(
    scope: &crate::work::WorkScope,
    task: &crate::work::WorkContentRef,
) -> String {
    digest(("erased-task-follow-up-v1", scope, task))
}
