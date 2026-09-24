//! Exact existing episode policy for work evidence. This adapter never changes
//! accounting, wire vocabulary or receipt ownership. Command consumption follows.
#[cfg(test)]
use crate::{
    episode_gate::*,
    work::{WorkError, recall::*},
};
#[cfg(test)]
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::{future::Future, pin::Pin, sync::Arc};

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Admitted(WorkAdmission),
    Rejected,
    Unknown,
}

/// Injectable transport only. Real coverage is selected by AppState::gate_for;
/// fakes observe the exact request passed to the existing episode builder.
#[cfg(test)]
pub trait Gate: Send + Sync {
    fn fingerprint(&self) -> String;
    fn nonce(&self) -> u64;
    fn ungated_forget(&self);
    fn propose(
        &self,
        request: MemoryCandidateRequest,
    ) -> Pin<Box<dyn Future<Output = GateOutcome> + Send + '_>>;
}
#[cfg(test)]
impl Gate for EpisodeGate {
    fn fingerprint(&self) -> String {
        // Configuration includes paths but never the bearer token contents.
        Sha256::digest(format!("{:?}", self.config()).as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    fn nonce(&self) -> u64 {
        self.next_nonce()
    }
    fn ungated_forget(&self) {
        self.note_ungated_forget();
    }
    fn propose(
        &self,
        request: MemoryCandidateRequest,
    ) -> Pin<Box<dyn Future<Output = GateOutcome> + Send + '_>> {
        Box::pin(self.record_memory_candidate(request))
    }
}
#[cfg(test)]
pub fn selected(
    state: &crate::runtime::AppState,
    payload: &WorkEvidencePayload,
) -> Option<Arc<dyn Gate>> {
    state
        .gate_for(&payload.scope.recall_gate_scope().0)
        .map(|g| g.clone() as Arc<dyn Gate>)
}
#[cfg(test)]
pub fn identity(gate: Option<&Arc<dyn Gate>>) -> (u64, String) {
    gate.map_or_else(|| (0, "0".repeat(64)), |g| (g.nonce(), g.fingerprint()))
}
#[cfg(test)]
fn receipt(
    row: &AdmittedWorkEvidence,
    payload: &WorkEvidencePayload,
) -> Result<Option<[u8; 32]>, WorkError> {
    if row.payload.source != payload.source || row.payload.scope != payload.scope {
        return Err(WorkError::Invalid);
    }
    let scope = payload.scope.recall_gate_scope().0;
    match &row.admission {
        WorkAdmission::Appended {
            digest_hex,
            scoped_guild,
        } if scoped_guild == &scope => parse_digest(digest_hex).map(Some).ok_or(WorkError::Invalid),
        WorkAdmission::Uncovered { scoped_guild } if scoped_guild == &scope => Ok(None),
        _ => Err(WorkError::Invalid),
    }
}
#[cfg(test)]
pub async fn admit(
    gate: Option<Arc<dyn Gate>>,
    payload: &WorkEvidencePayload,
    previous: Option<&AdmittedWorkEvidence>,
    forget: bool,
    at: u64,
    nonce: u64,
) -> Result<Outcome, WorkError> {
    let (scoped_guild, member_scoped) = payload.scope.recall_gate_scope();
    let prior = previous
        .map(|row| receipt(row, payload))
        .transpose()?
        .flatten();
    if forget && previous.is_none() {
        return Err(WorkError::Invalid);
    }
    let Some(gate) = gate else {
        return Ok(Outcome::Admitted(WorkAdmission::Uncovered { scoped_guild }));
    };
    if previous.is_some() && prior.is_none() {
        gate.ungated_forget();
        if forget {
            return Ok(Outcome::Admitted(WorkAdmission::Uncovered { scoped_guild }));
        }
    }
    let request = MemoryCandidateRequest {
        scoped_guild: scoped_guild.clone(),
        class: MemoryClass::Fact,
        retention: RetentionClass::Durable,
        payload: if forget {
            Vec::new()
        } else {
            payload.encoded()?
        },
        member_scoped,
        supersedes: if forget { None } else { prior },
        forgets: if forget { prior } else { None },
        now: at,
        nonce,
    };
    Ok(match gate.propose(request).await {
        GateOutcome::Appended { digest_hex, .. } if parse_digest(&digest_hex).is_some() => {
            Outcome::Admitted(WorkAdmission::Appended {
                digest_hex,
                scoped_guild,
            })
        }
        GateOutcome::Rejected { .. } => Outcome::Rejected,
        _ => Outcome::Unknown,
    })
}
