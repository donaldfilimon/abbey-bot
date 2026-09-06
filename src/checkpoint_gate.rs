//! DQN replay checkpoints as memory candidates (amendment 2026-09-06, §4.4).
//!
//! A `brain::replay::Experience` is per decision and would exhaust any sane
//! token budget, so the `experience` class is proposed once per persisted
//! replay checkpoint: the guild's serialized `BrainRow`. The rule this module
//! keeps is "only an admitted checkpoint is persisted". When the gate is
//! configured, a guild's brain row reaches the state file only after the
//! ledger appended a candidate committing to those exact bytes; a refused or
//! unreachable gate leaves the last admitted row (or the row loaded from disk
//! before the gate existed) in its place, so a gate outage never holds facts,
//! settings, or the other guilds' brains hostage and never loses a checkpoint
//! that was already on disk.
//!
//! Coverage: `plan` and `restrict_to_admitted` take the gate's `covers`
//! predicate, so a guild outside the config's `guilds` list is neither
//! proposed nor substituted; its row persists exactly as with no gate.
//!
//! Every function here is pure over plain values; `runtime` does the locking
//! and the proposing.

use std::collections::BTreeMap;

use sha2::{Digest as _, Sha256};

use crate::episode_gate::{GateOutcome, parse_digest};
use crate::persist::{BrainRow, Stores};

/// The last checkpoint the ledger admitted for a guild, or the row loaded
/// from disk before the gate existed (`episode_digest` is then `None`, so the
/// first gated checkpoint opens a fresh chain instead of superseding it).
#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedCheckpoint {
    pub commitment: [u8; 32],
    pub episode_digest: Option<[u8; 32]>,
    pub row: BrainRow,
}

/// One checkpoint to propose: the bytes, their commitment, and the admitted
/// checkpoint it supersedes.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub guild: String,
    pub row: BrainRow,
    pub payload: Vec<u8>,
    pub commitment: [u8; 32],
    pub supersedes: Option<[u8; 32]>,
}

/// Which guilds' checkpoints were admitted and which were substituted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settlement {
    pub admitted: Vec<String>,
    pub refused: Vec<String>,
}

/// The canonical payload: the row's JSON. `BrainRow` serializes
/// deterministically (two fields, no maps).
pub fn payload(row: &BrainRow) -> Vec<u8> {
    serde_json::to_vec(row).unwrap_or_default()
}

pub fn commitment(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(payload).into()
}

/// Rows loaded from disk before any gate decision: treated as admitted so
/// they are never dropped, with no ledger digest to chain from.
pub fn seed(rows: &BTreeMap<String, BrainRow>) -> BTreeMap<String, AdmittedCheckpoint> {
    rows.iter()
        .map(|(guild, row)| {
            (
                guild.clone(),
                AdmittedCheckpoint {
                    commitment: commitment(&payload(row)),
                    episode_digest: None,
                    row: row.clone(),
                },
            )
        })
        .collect()
}

/// Every covered guild whose row differs from its last admitted checkpoint.
/// An unchanged row is not re-proposed: each proposal charges its full
/// payload against the guild's storage budget.
pub fn plan(
    rows: &BTreeMap<String, BrainRow>,
    admitted: &BTreeMap<String, AdmittedCheckpoint>,
    covers: impl Fn(&str) -> bool,
) -> Vec<Proposal> {
    rows.iter()
        .filter(|(guild, _)| covers(guild))
        .filter_map(|(guild, row)| {
            let payload = payload(row);
            let commitment = commitment(&payload);
            let previous = admitted.get(guild);
            if previous.is_some_and(|previous| previous.commitment == commitment) {
                return None;
            }
            Some(Proposal {
                guild: guild.clone(),
                row: row.clone(),
                payload,
                commitment,
                supersedes: previous.and_then(|previous| previous.episode_digest),
            })
        })
        .collect()
}

/// Apply the gate's answers: admitted rows are recorded with their receipt
/// digest; refused or unanswered rows are replaced in `stores` by the last
/// admitted row, or removed when the guild never had one.
pub fn settle(
    stores: &mut Stores,
    admitted: &mut BTreeMap<String, AdmittedCheckpoint>,
    outcomes: Vec<(Proposal, GateOutcome)>,
) -> Settlement {
    let mut settlement = Settlement::default();
    for (proposal, outcome) in outcomes {
        match outcome {
            GateOutcome::Appended { digest_hex, .. } => {
                admitted.insert(
                    proposal.guild.clone(),
                    AdmittedCheckpoint {
                        commitment: proposal.commitment,
                        episode_digest: parse_digest(&digest_hex),
                        row: proposal.row,
                    },
                );
                settlement.admitted.push(proposal.guild);
            }
            GateOutcome::Rejected { .. } | GateOutcome::Unavailable { .. } => {
                substitute(stores, admitted, &proposal.guild);
                settlement.refused.push(proposal.guild);
            }
        }
    }
    settlement
}

/// The synchronous persist path (shutdown) proposes nothing: every covered
/// row that is not the admitted checkpoint is substituted, so an unadmitted
/// checkpoint never reaches disk. Returns the guilds substituted.
pub fn restrict_to_admitted(
    stores: &mut Stores,
    admitted: &BTreeMap<String, AdmittedCheckpoint>,
    covers: impl Fn(&str) -> bool,
) -> Vec<String> {
    let changed: Vec<String> = stores
        .brains
        .iter()
        .filter(|(guild, _)| covers(guild))
        .filter(|(guild, row)| {
            admitted
                .get(*guild)
                .is_none_or(|previous| previous.commitment != commitment(&payload(row)))
        })
        .map(|(guild, _)| guild.clone())
        .collect();
    for guild in &changed {
        substitute(stores, admitted, guild);
    }
    changed
}

fn substitute(stores: &mut Stores, admitted: &BTreeMap<String, AdmittedCheckpoint>, guild: &str) {
    match admitted.get(guild) {
        Some(previous) => {
            stores
                .brains
                .insert(guild.to_string(), previous.row.clone());
        }
        None => {
            stores.brains.remove(guild);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(json: &str, count: u64) -> BrainRow {
        BrainRow {
            snapshot_json: json.into(),
            experience_count: count,
        }
    }

    fn stores_with(rows: &[(&str, BrainRow)]) -> Stores {
        let mut stores = Stores::default();
        for (guild, row) in rows {
            stores.brains.insert((*guild).into(), row.clone());
        }
        stores
    }

    #[test]
    fn unchanged_rows_are_not_re_proposed_and_changed_rows_supersede_their_receipt() {
        let loaded = stores_with(&[("g1", row("{\"a\":1}", 1)), ("g2", row("{\"b\":2}", 2))]);
        let mut admitted = seed(&loaded.brains);
        assert!(
            plan(&loaded.brains, &admitted, |_| true).is_empty(),
            "seeded rows are admitted"
        );

        let mut next = loaded.clone();
        next.brains.insert("g1".into(), row("{\"a\":2}", 3));
        next.brains.insert("g3".into(), row("{\"c\":1}", 1));
        let proposals = plan(&next.brains, &admitted, |_| true);
        assert_eq!(
            proposals
                .iter()
                .map(|p| p.guild.as_str())
                .collect::<Vec<_>>(),
            ["g1", "g3"]
        );
        assert_eq!(
            proposals[0].supersedes, None,
            "a pre-gate row has no digest to supersede"
        );
        assert_eq!(
            proposals[0].commitment,
            commitment(&payload(&next.brains["g1"]))
        );

        let digest = "ab".repeat(32);
        let outcomes = proposals
            .into_iter()
            .map(|proposal| {
                let outcome = GateOutcome::Appended {
                    digest_hex: digest.clone(),
                    sequence: "1".into(),
                };
                (proposal, outcome)
            })
            .collect();
        let settlement = settle(&mut next, &mut admitted, outcomes);
        assert_eq!(settlement.admitted, ["g1", "g3"]);
        assert!(settlement.refused.is_empty());
        assert_eq!(admitted["g1"].episode_digest, Some([0xab; 32]));
        assert_eq!(
            next.brains["g1"],
            row("{\"a\":2}", 3),
            "admitted rows persist as proposed"
        );

        // The next change supersedes the admitted checkpoint's digest.
        next.brains.insert("g1".into(), row("{\"a\":3}", 4));
        let again = plan(&next.brains, &admitted, |_| true);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].supersedes, Some([0xab; 32]));
    }

    #[test]
    fn a_refused_checkpoint_is_replaced_by_the_last_admitted_row_or_dropped() {
        let loaded = stores_with(&[("g1", row("{\"a\":1}", 1))]);
        let mut admitted = seed(&loaded.brains);
        let mut next = loaded.clone();
        next.brains.insert("g1".into(), row("{\"a\":2}", 2));
        next.brains.insert("new".into(), row("{\"n\":1}", 1));
        let proposals = plan(&next.brains, &admitted, |_| true);
        let outcomes = proposals
            .into_iter()
            .map(|proposal| {
                let outcome = if proposal.guild == "g1" {
                    GateOutcome::Rejected {
                        detail: "FailedPrecondition: episode_storage_budget_exhausted".into(),
                    }
                } else {
                    GateOutcome::Unavailable {
                        detail: "timed out".into(),
                    }
                };
                (proposal, outcome)
            })
            .collect();
        let settlement = settle(&mut next, &mut admitted, outcomes);
        assert_eq!(settlement.refused, ["g1", "new"]);
        assert_eq!(
            next.brains["g1"],
            row("{\"a\":1}", 1),
            "the on-disk row survives"
        );
        assert!(
            !next.brains.contains_key("new"),
            "never admitted, never persisted"
        );
        assert_eq!(admitted["g1"].episode_digest, None, "nothing was admitted");
        assert!(!admitted.contains_key("new"));
    }

    #[test]
    fn the_synchronous_path_persists_only_admitted_checkpoints() {
        let loaded = stores_with(&[("g1", row("{\"a\":1}", 1))]);
        let admitted = seed(&loaded.brains);
        let mut next = loaded.clone();
        next.brains.insert("g1".into(), row("{\"a\":9}", 9));
        next.brains.insert("g2".into(), row("{\"b\":1}", 1));
        next.guilds.insert("g2".into(), Default::default());
        let substituted = restrict_to_admitted(&mut next, &admitted, |_| true);
        assert_eq!(substituted, ["g1", "g2"]);
        assert_eq!(next.brains["g1"], row("{\"a\":1}", 1));
        assert!(!next.brains.contains_key("g2"));
        assert!(next.guilds.contains_key("g2"), "only brain rows are gated");
        assert!(restrict_to_admitted(&mut next, &admitted, |_| true).is_empty());
    }

    #[test]
    fn an_uncovered_guild_is_neither_proposed_nor_substituted() {
        let loaded = stores_with(&[("g1", row("{\"a\":1}", 1)), ("g2", row("{\"b\":1}", 1))]);
        let admitted = seed(&loaded.brains);
        let mut next = loaded.clone();
        next.brains.insert("g1".into(), row("{\"a\":2}", 2));
        next.brains.insert("g2".into(), row("{\"b\":2}", 2));
        next.brains.insert("g3".into(), row("{\"c\":1}", 1));
        let covers = |guild: &str| guild == "g1";

        let proposals = plan(&next.brains, &admitted, covers);
        assert_eq!(
            proposals
                .iter()
                .map(|p| p.guild.as_str())
                .collect::<Vec<_>>(),
            ["g1"],
            "only the covered guild is proposed"
        );

        let substituted = restrict_to_admitted(&mut next, &admitted, covers);
        assert_eq!(substituted, ["g1"]);
        assert_eq!(next.brains["g1"], row("{\"a\":1}", 1));
        assert_eq!(
            next.brains["g2"],
            row("{\"b\":2}", 2),
            "an uncovered row persists as written"
        );
        assert_eq!(
            next.brains["g3"],
            row("{\"c\":1}", 1),
            "an uncovered new row is never dropped"
        );
    }
}
