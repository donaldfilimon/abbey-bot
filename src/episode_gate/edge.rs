//! Memory-edge episodes (amendment 2026-09-16): a guild moderator's review of
//! a stored fact.
//!
//! The first and only emitter is `/admin quarantine` / `/admin resolve`
//! (Donald's decision, 2026-09-16). There is no automatic policy: nothing here
//! quarantines on its own, including on a failed signature.
//!
//! Who the ledger records differs by kind, because WDBX enforces it: a
//! quarantine is recorded by the bot's service principal (flagging is the
//! protective direction), and a resolution by the *invoking human*, as the
//! same keyed, content-free principal a learning-toggle proposal uses. This
//! is the only write this bot records under a human principal.
//!
//! Transcribed from `abi-wdbx::v3::episode::{MemoryEdge, MemoryEdgeKind,
//! EdgeReason}` and pinned by `tests/fixtures/episode_write_memory_edge.json`.
//! The bot never records `contradicts` yet; the variant exists so the wire
//! type matches the canonical one and the fixture parses.

use serde::{Deserialize, Serialize};

use super::{
    ActorKind, ActorRef, EpisodeEvent, EpisodeGate, EpisodeGateConfig, EpisodeSource, EpisodeWrite,
    GateOutcome, TOKEN_COST, guild_ref_for, keyed_id, requester_principal,
};

const EDGE_OPERATION_SEED: u64 = 0x6d65_6d65_6467_6f70; // "memedgop"
const EDGE_REQUEST_SEED: u64 = 0x6d65_6d65_6467_7271; // "memedgrq"

/// Closed edge kind. Transcribed from `MemoryEdgeKind`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEdgeKind {
    Quarantines,
    Contradicts,
    Resolves,
}

/// Closed, content-free reason. Transcribed from `EdgeReason`; WDBX checks
/// that it fits the kind.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeReason {
    SourceUntrusted,
    SignatureInvalid,
    PolicyViolation,
    OperatorReport,
    ConflictingObservation,
    SupersededEvidence,
    ReviewedValid,
    ReviewedInvalid,
}

impl EdgeReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::SourceUntrusted => "source_untrusted",
            Self::SignatureInvalid => "signature_invalid",
            Self::PolicyViolation => "policy_violation",
            Self::OperatorReport => "operator_report",
            Self::ConflictingObservation => "conflicting_observation",
            Self::SupersededEvidence => "superseded_evidence",
            Self::ReviewedValid => "reviewed_valid",
            Self::ReviewedInvalid => "reviewed_invalid",
        }
    }
}

/// Content-free edge over memory-candidate episodes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MemoryEdge {
    pub kind: MemoryEdgeKind,
    pub target: [u8; 32],
    pub counterpart: Option<[u8; 32]>,
    pub reason: EdgeReason,
}

/// The governance role of the human resolving an edge. Each maps to an actor
/// kind WDBX admits for `resolves`; a member without one of these never
/// reaches this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reviewer {
    Owner,
    Administrator,
    Manager,
}

impl Reviewer {
    const fn kind(self) -> ActorKind {
        match self {
            Self::Owner => ActorKind::GuildOwner,
            Self::Administrator => ActorKind::GuildAdministrator,
            Self::Manager => ActorKind::GuildManager,
        }
    }
}

/// What a review command hands over: plain values, no Discord types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryEdgeRequest {
    /// Flag the fact whose ledger receipt is `target`.
    Quarantine {
        scoped_guild: String,
        target: [u8; 32],
        reason: EdgeReason,
        now: u64,
        nonce: u64,
    },
    /// Close the open edge episode `edge`, as `reviewer`.
    Resolve {
        scoped_guild: String,
        scoped_user: String,
        reviewer: Reviewer,
        edge: [u8; 32],
        valid: bool,
        now: u64,
        nonce: u64,
    },
}

/// Build the memory-edge write. `Err` names the reason nothing can be
/// proposed; nothing was sent. WDBX still decides whether the edge is
/// admitted (the target must be live, the edge open, and so on).
pub fn memory_edge_write(
    config: &EpisodeGateConfig,
    request: &MemoryEdgeRequest,
) -> Result<EpisodeWrite, String> {
    let (scoped_guild, recorded_by, edge, now, nonce) = match request {
        MemoryEdgeRequest::Quarantine {
            scoped_guild,
            target,
            reason,
            now,
            nonce,
        } => {
            if !matches!(
                reason,
                EdgeReason::SourceUntrusted
                    | EdgeReason::SignatureInvalid
                    | EdgeReason::PolicyViolation
                    | EdgeReason::OperatorReport
                    | EdgeReason::SupersededEvidence
            ) {
                return Err("that reason does not describe a quarantine".into());
            }
            let recorded_by = ActorRef {
                principal_id: config.service_principal.clone(),
                kind: ActorKind::Service,
            };
            let edge = MemoryEdge {
                kind: MemoryEdgeKind::Quarantines,
                target: *target,
                counterpart: None,
                reason: *reason,
            };
            (scoped_guild, recorded_by, edge, *now, *nonce)
        }
        MemoryEdgeRequest::Resolve {
            scoped_guild,
            scoped_user,
            reviewer,
            edge,
            valid,
            now,
            nonce,
        } => {
            let principal_id = requester_principal(scoped_guild, scoped_user);
            if principal_id == config.service_principal {
                return Err("reviewer principal collides with the service principal".into());
            }
            let recorded_by = ActorRef {
                principal_id,
                kind: reviewer.kind(),
            };
            let edge = MemoryEdge {
                kind: MemoryEdgeKind::Resolves,
                target: *edge,
                counterpart: None,
                reason: if *valid {
                    EdgeReason::ReviewedValid
                } else {
                    EdgeReason::ReviewedInvalid
                },
            };
            (scoped_guild, recorded_by, edge, *now, *nonce)
        }
    };
    if edge.target == [0; 32] {
        return Err("an edge must name a nonzero digest".into());
    }
    let guild_ref = guild_ref_for(scoped_guild)
        .ok_or_else(|| "scoped guild id does not map to a ledger guild reference".to_string())?;
    let kind = match edge.kind {
        MemoryEdgeKind::Quarantines => "quarantines",
        MemoryEdgeKind::Contradicts => "contradicts",
        MemoryEdgeKind::Resolves => "resolves",
    };
    Ok(EpisodeWrite {
        request_id: keyed_id("req", EDGE_REQUEST_SEED, &guild_ref, now, nonce),
        operation_id: keyed_id(
            &format!("memory-edge-{kind}"),
            EDGE_OPERATION_SEED,
            &guild_ref,
            now,
            nonce,
        ),
        contract_revision: config.contract_revision,
        contract_digest: config.contract_digest,
        guild_ref,
        consent_epoch: None,
        source_type: EpisodeSource::DiscordGuild,
        policy_version: config.policy_version.clone(),
        evidence_level: config.evidence_level,
        event: EpisodeEvent::MemoryEdge { recorded_by, edge },
        token_cost: TOKEN_COST,
        expected_commitment: None,
        quiet: false,
    })
}

impl EpisodeGate {
    /// Propose one review edge. The reply names the returned digest, which is
    /// what a later resolution passes back. Logs are content-free.
    pub async fn record_memory_edge(&self, request: MemoryEdgeRequest) -> GateOutcome {
        let kind = match &request {
            MemoryEdgeRequest::Quarantine { .. } => "quarantines",
            MemoryEdgeRequest::Resolve { .. } => "resolves",
        };
        let outcome = match memory_edge_write(self.config(), &request) {
            Ok(write) => self.propose(&write).await,
            Err(detail) => GateOutcome::Unavailable { detail },
        };
        self.count(&outcome);
        match &outcome {
            GateOutcome::Appended { .. } => {
                tracing::info!(kind, outcome = %outcome.summary(), "episode gate: memory edge recorded");
            }
            GateOutcome::Rejected { .. } | GateOutcome::Unavailable { .. } => {
                tracing::warn!(kind, outcome = %outcome.summary(), "episode gate: memory edge not recorded");
            }
        }
        outcome
    }
}
