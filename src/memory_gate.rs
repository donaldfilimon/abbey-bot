//! Glue between the memory commands and the episode gate (amendment
//! 2026-09-06, §4): propose before writing, write only on `appended`, fail
//! closed and visibly. Every function is a no-op `Ok(None)` when no gate is
//! configured, which keeps the default deployment byte-identical.

use crate::episode_gate::{GateOutcome, MemoryCandidateRequest, MemoryClass, RetentionClass};
use crate::runtime::{self, AppState};

/// Why a memory write did not happen, in words the person can act on. Content-
/// free: only the gateway's closed labels ride along.
fn refusal(outcome: &GateOutcome) -> String {
    match outcome {
        GateOutcome::Rejected { detail } => {
            format!("Not stored: the constitutional memory gate refused it ({detail}).")
        }
        GateOutcome::Unavailable { detail } => format!(
            "Not stored: the constitutional memory gate could not be reached ({detail}). Nothing was changed."
        ),
        GateOutcome::Appended { .. } => String::new(),
    }
}

/// Propose a `fact` candidate for `fact`, superseding the receipt of
/// `replaces` when that fact has one. `Ok(None)` means no gate is configured;
/// `Ok(Some(digest_hex))` is the receipt to key the stored fact by; `Err` is
/// the message to show, and nothing may be written.
pub async fn admit_fact(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    fact: &str,
    replaces: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(gate) = state.episode_gate.as_ref() else {
        return Ok(None);
    };
    let service = state.memory_service();
    let supersedes =
        match replaces.and_then(|old| service.resolve_fact(scoped_guild, scoped_user, old)) {
            Some(old) => match service.receipt(scoped_guild, scoped_user, &old) {
                Some(hex) => crate::episode_gate::parse_digest(&hex),
                None => {
                    // The old fact predates the gate: its removal leaves no
                    // tombstone edge, so count it where `inspect_status` shows it.
                    gate.note_ungated_forget();
                    None
                }
            },
            None => None,
        };
    let request = MemoryCandidateRequest {
        scoped_guild: scoped_guild.to_owned(),
        class: MemoryClass::Fact,
        retention: RetentionClass::Durable,
        payload: fact.as_bytes().to_vec(),
        member_scoped: true,
        supersedes,
        forgets: None,
        now: runtime::now(),
        nonce: gate.next_nonce(),
    };
    match gate.record_memory_candidate(request).await {
        GateOutcome::Appended { digest_hex, .. } => Ok(Some(digest_hex)),
        other => Err(refusal(&other)),
    }
}

/// Propose a `forgets` candidate for the exact stored `fact` before it is
/// deleted locally. A fact with no receipt (stored before the gate existed)
/// is deleted without a ledger edge and counted as an ungated forget. The
/// receipt stays until the caller reports the local delete with
/// [`drop_receipt`], so a delete that fails after all keeps its join.
pub async fn admit_forget(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    fact: &str,
) -> Result<(), String> {
    let Some(gate) = state.episode_gate.as_ref() else {
        return Ok(());
    };
    let service = state.memory_service();
    let Some(target) = service
        .receipt(scoped_guild, scoped_user, fact)
        .and_then(|hex| crate::episode_gate::parse_digest(&hex))
    else {
        gate.note_ungated_forget();
        return Ok(());
    };
    let request = MemoryCandidateRequest {
        scoped_guild: scoped_guild.to_owned(),
        class: MemoryClass::Fact,
        retention: RetentionClass::Durable,
        payload: Vec::new(),
        member_scoped: true,
        supersedes: None,
        forgets: Some(target),
        now: runtime::now(),
        nonce: gate.next_nonce(),
    };
    match gate.record_memory_candidate(request).await {
        GateOutcome::Appended { .. } => Ok(()),
        other => Err(refusal(&other)),
    }
}

/// The local delete happened: the fact's receipt is no longer a live join.
pub fn drop_receipt(state: &AppState, scoped_guild: &str, scoped_user: &str, fact: &str) {
    state
        .memory_service()
        .take_receipt(scoped_guild, scoped_user, fact);
}

/// After a successful local write, key the stored fact by its receipt and
/// drop the receipt of anything it replaced.
pub fn settle_receipts(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    stored: &str,
    removed: Option<&str>,
    digest_hex: &str,
) {
    let service = state.memory_service();
    if let Some(removed) = removed {
        service.take_receipt(scoped_guild, scoped_user, removed);
    }
    service.record_receipt(scoped_guild, scoped_user, stored, digest_hex);
}
