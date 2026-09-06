//! Glue between the memory commands and the episode gate (amendment
//! 2026-09-06, §4): propose before writing, write only on `appended`, fail
//! closed and visibly. Every function is a no-op `Ok(None)` when no gate is
//! configured, which keeps the default deployment byte-identical.

use crate::episode_gate::{GateOutcome, MemoryCandidateRequest, MemoryClass, RetentionClass};
use crate::memory;
use crate::runtime::{self, AppState, RememberOutcome};

/// Queued model-tool writes per process. The tool host is synchronous and
/// cannot propose, so it queues; a full queue refuses rather than grows.
pub const MAX_QUEUED: usize = 64;

/// One model-tool memory write waiting for the gate (Donald's choice
/// 2026-09-06: queue, do not refuse). Nothing is stored until it drains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFact {
    pub scoped_guild: String,
    pub scoped_user: String,
    /// Already validated by `memory::validated_fact`.
    pub fact: String,
    /// A model-proposed supersession; the old fact is never removed here.
    pub supersedes: Option<String>,
    pub queued_at: u64,
}

/// What one drain did, for logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drained {
    pub admitted: usize,
    pub refused: usize,
}

/// Queue a model-tool write for the next drain. `Err` is the message to give
/// the model; nothing is stored either way.
pub fn enqueue(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    fact: &str,
    supersedes: Option<&str>,
    now: u64,
) -> Result<String, String> {
    let fact = memory::validated_fact(fact).map_err(str::to_owned)?;
    if let Some(reason) = state
        .memory_service()
        .remember_blocked(scoped_guild, scoped_user, &fact)
    {
        return Err(format!("Not queued: {reason}."));
    }
    let mut queue = AppState::lock(&state.memory_queue);
    if queue.iter().any(|queued| {
        queued.scoped_guild == scoped_guild
            && queued.scoped_user == scoped_user
            && queued.fact == fact
    }) {
        return Err(
            "Already queued for the constitutional memory gate; nothing is on record yet.".into(),
        );
    }
    if queue.len() >= MAX_QUEUED {
        return Err(
            "Not queued: the memory gate queue is full; ask the person to run /remember.".into(),
        );
    }
    queue.push(QueuedFact {
        scoped_guild: scoped_guild.to_owned(),
        scoped_user: scoped_user.to_owned(),
        fact: fact.clone(),
        supersedes: supersedes.map(str::to_owned),
        queued_at: now,
    });
    Ok(format!(
        "Queued for the constitutional memory gate: {fact}. It is stored only once the gate admits it; nothing is on record yet."
    ))
}

/// Propose every queued write and store the admitted ones. Refusals are
/// logged and counted by the gate; the fact is dropped, not retried, so a
/// refused write cannot pile up behind an outage.
pub async fn drain(state: &AppState) -> Drained {
    let queued: Vec<QueuedFact> = std::mem::take(&mut *AppState::lock(&state.memory_queue));
    let mut drained = Drained::default();
    for item in queued {
        let receipt = match admit_fact(
            state,
            &item.scoped_guild,
            &item.scoped_user,
            &item.fact,
            None,
        )
        .await
        {
            Ok(receipt) => receipt,
            Err(message) => {
                drained.refused += 1;
                tracing::warn!(reason = %message, "episode gate: queued model memory write not admitted; nothing stored");
                continue;
            }
        };
        let service = state.memory_service();
        let outcome = match item.supersedes.as_deref() {
            Some(old) => service.remember_proposing(
                &item.scoped_guild,
                &item.scoped_user,
                &item.fact,
                old,
                item.queued_at,
            ),
            None => service.remember(
                &item.scoped_guild,
                &item.scoped_user,
                &item.fact,
                item.queued_at,
            ),
        };
        match (&outcome, &receipt) {
            (
                Ok(RememberOutcome::Stored(stored) | RememberOutcome::Proposed { stored, .. }),
                Some(digest_hex),
            ) => {
                settle_receipts(
                    state,
                    &item.scoped_guild,
                    &item.scoped_user,
                    stored,
                    None,
                    digest_hex,
                );
                drained.admitted += 1;
            }
            (Ok(RememberOutcome::Stored(_) | RememberOutcome::Proposed { .. }), None) => {
                drained.admitted += 1;
            }
            _ => {
                // Admitted (or ungated) but the local store refused after all:
                // a concurrent write beat the queue. The log is the record.
                tracing::warn!("episode gate: queued model memory write stored nothing locally");
                drained.refused += 1;
            }
        }
    }
    if drained.admitted + drained.refused > 0 {
        tracing::info!(
            admitted = drained.admitted,
            refused = drained.refused,
            "episode gate: model memory queue drained"
        );
    }
    drained
}

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
/// `replaces` when that fact has one. `Ok(None)` means no gate is configured
/// or this scope is not covered by it; `Ok(Some(digest_hex))` is the receipt
/// to key the stored fact by; `Err` is the message to show, and nothing may
/// be written.
pub async fn admit_fact(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    fact: &str,
    replaces: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(gate) = state.gate_for(scoped_guild) else {
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
    let Some(gate) = state.gate_for(scoped_guild) else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_gate::{EpisodeGate, EpisodeGateConfig};
    use crate::platform::SocialNetwork;
    use crate::runtime::ToolScope;
    use crate::tools::ToolHost as _;
    use std::sync::Arc;

    /// A gate whose `abi` binary does not exist: every proposal is
    /// `Unavailable`, which is the fail-closed path this module promises.
    fn dead_gate() -> Arc<EpisodeGate> {
        dead_gate_covering(None)
    }

    fn dead_gate_covering(guilds: Option<&[&str]>) -> Arc<EpisodeGate> {
        let temp = std::env::temp_dir();
        let mut json = serde_json::json!({
            "abi_cli": temp.join("abi-that-does-not-exist").display().to_string(),
            "endpoint": "http://127.0.0.1:50051",
            "token_file": temp.join("abbey-episode-token").display().to_string(),
            "policy_version": "policy_v1",
            "contract_revision": 2,
            "contract_digest": "01".repeat(32),
            "timeout_secs": 5,
        });
        if let Some(guilds) = guilds {
            json["guilds"] = serde_json::json!(guilds);
        }
        Arc::new(EpisodeGate::new(
            EpisodeGateConfig::from_json(&json.to_string()).unwrap(),
        ))
    }

    fn scope(state: &AppState) -> ToolScope<'_> {
        ToolScope {
            state,
            network: SocialNetwork::Discord,
            scoped_guild: "discord:123456789012345678".into(),
            scoped_user: "discord:42".into(),
            scoped_channel: "discord:c".into(),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        }
    }

    #[tokio::test]
    async fn with_a_gate_the_model_tool_queues_and_a_dead_gate_stores_nothing() {
        let mut state = AppState::in_memory();
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(dead_gate());
        let (g, u) = ("discord:123456789012345678", "discord:42");
        let mut host = scope(&state);
        let reply = host.remember_fact("likes compilers", None);
        assert!(reply.starts_with("Queued"), "{reply}");
        assert!(reply.contains("nothing is on record yet"));
        assert!(state.memory_service().facts(g, u).is_empty());
        assert_eq!(AppState::lock(&state.memory_queue).len(), 1);
        assert!(
            host.remember_fact("likes compilers", None)
                .starts_with("Already queued")
        );
        assert!(
            host.remember_fact("moved to zig", Some("likes compilers"))
                .starts_with("Queued")
        );
        assert_eq!(AppState::lock(&state.memory_queue).len(), 2);

        let drained = drain(&state).await;
        assert_eq!(
            drained,
            Drained {
                admitted: 0,
                refused: 2
            }
        );
        assert!(
            state.memory_service().facts(g, u).is_empty(),
            "nothing admitted, nothing stored"
        );
        assert!(
            AppState::lock(&state.memory_queue).is_empty(),
            "refused items are dropped"
        );
        let counters = state.episode_gate.as_ref().unwrap().counters();
        assert_eq!(counters.unavailable, 2);
        assert_eq!(counters.appended, 0);
        assert!(
            state
                .memory_service()
                .receipt(g, u, "likes compilers")
                .is_none()
        );
    }

    #[tokio::test]
    async fn without_a_gate_a_drained_queue_stores_locally_and_a_held_fact_is_not_queued() {
        let state = AppState::in_memory();
        let (g, u) = ("discord:123456789012345678", "discord:42");
        AppState::lock(&state.memory_queue).push(QueuedFact {
            scoped_guild: g.into(),
            scoped_user: u.into(),
            fact: "uses rust".into(),
            supersedes: None,
            queued_at: 7,
        });
        let drained = drain(&state).await;
        assert_eq!(
            drained,
            Drained {
                admitted: 1,
                refused: 0
            }
        );
        assert_eq!(
            state.memory_service().facts(g, u),
            vec!["uses rust".to_string()]
        );
        assert!(
            state.memory_service().receipt(g, u, "uses rust").is_none(),
            "no gate, no receipt"
        );
        assert_eq!(
            enqueue(&state, g, u, "uses rust", None, 8).unwrap_err(),
            "Not queued: already on record."
        );
        assert!(
            enqueue(&state, g, u, "   ", None, 8).is_err(),
            "validation still applies"
        );
        assert!(AppState::lock(&state.memory_queue).is_empty());
    }

    /// A dead gate that does not cover this scope must be invisible: the
    /// model tool stores immediately, the slash paths report "no gate", the
    /// queue stays empty, and no counter moves.
    #[tokio::test]
    async fn an_uncovered_scope_behaves_exactly_as_if_no_gate_were_configured() {
        let mut state = AppState::in_memory();
        Arc::get_mut(&mut state).unwrap().episode_gate =
            Some(dead_gate_covering(Some(&["discord:999"])));
        let (g, u) = ("discord:123456789012345678", "discord:42");
        assert!(state.gate_for(g).is_none());
        assert!(state.gate_for("discord:999").is_some());

        let mut host = scope(&state);
        let reply = host.remember_fact("likes compilers", None);
        assert!(reply.starts_with("Stored"), "{reply}");
        assert_eq!(
            state.memory_service().facts(g, u),
            vec!["likes compilers".to_string()]
        );
        assert!(AppState::lock(&state.memory_queue).is_empty());
        assert!(
            state
                .memory_service()
                .receipt(g, u, "likes compilers")
                .is_none()
        );

        assert_eq!(
            admit_fact(&state, g, u, "uses rust", None).await,
            Ok(None),
            "no receipt, nothing proposed"
        );
        assert_eq!(admit_forget(&state, g, u, "likes compilers").await, Ok(()));
        let counters = state.episode_gate.as_ref().unwrap().counters();
        assert_eq!(
            (
                counters.appended,
                counters.rejected,
                counters.unavailable,
                counters.ungated_forgets
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(counters.covered_guilds, Some(1));
    }
}
