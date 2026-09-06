//! Glue between the memory commands and the episode gate (amendment
//! 2026-09-06, §4): propose before writing, write only on `appended`, fail
//! closed and visibly. Every function is a no-op `Ok(None)` when no gate is
//! configured, which keeps the default deployment byte-identical.

mod turn;
pub use turn::{Decision, MemoryTurn};

use crate::episode_gate::{GateOutcome, MemoryCandidateRequest, MemoryClass, RetentionClass};
use crate::memory;
use crate::runtime::{self, AppState, RememberOutcome};

/// Queued model-tool writes per process. The tool host is synchronous and
/// cannot propose, so it queues; a full queue refuses rather than grows.
pub const MAX_QUEUED: usize = 64;

/// One model-tool memory write waiting for the gate (Donald's choice
/// 2026-09-06: queue, do not refuse). Nothing is stored until it drains.
#[derive(Debug)]
pub struct QueuedFact {
    pub completion: Option<turn::Completion>,
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
    turn: Option<&MemoryTurn>,
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
        completion: turn.map(MemoryTurn::completion),
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
    if let Some(registry) = state.service_registry() {
        let Some(owned) = state.owned_state() else {
            return cancel_queued(state, None);
        };
        return match registry.spawn_result(crate::service::OperationKind::MemoryDrain, async move {
            drain_owned(&owned).await
        }) {
            Ok(result) => match result.await {
                Ok(drained) => drained,
                Err(_) => cancel_queued(state, None),
            },
            Err(_) => cancel_queued(state, None),
        };
    }
    drain_owned(state).await
}

/// Cancel facts that have not entered a drain. `turn=None` is the terminal
/// managed-service admission failure; a specific turn is cancelled after its
/// original response cannot be delivered. Items already taken by an admitted
/// drain are absent from this queue and keep their original completion owner.
pub fn cancel_pending(state: &AppState, turn: &MemoryTurn) -> Drained {
    cancel_queued(state, Some(turn))
}

fn cancel_queued(state: &AppState, turn: Option<&MemoryTurn>) -> Drained {
    let mut queue = AppState::lock(&state.memory_queue);
    let mut kept = Vec::with_capacity(queue.len());
    let mut refused = 0;
    for mut item in std::mem::take(&mut *queue) {
        let selected = match turn {
            None => true,
            Some(turn) => item
                .completion
                .as_ref()
                .is_some_and(|completion| completion.belongs_to(turn)),
        };
        if selected {
            if let Some(completion) = item.completion.take() {
                completion.finish(Decision::Cancelled);
            }
            refused += 1;
        } else {
            kept.push(item);
        }
    }
    *queue = kept;
    Drained {
        admitted: 0,
        refused,
    }
}

/// Deliver every terminal outcome and record one closed operational failure
/// per failed response. The decisions are consumed, so observing a delivery
/// failure cannot re-propose or replay the underlying mutation.
pub async fn deliver_notices<E, F, Fut>(
    state: &AppState,
    component: crate::observability::EventComponent,
    turn: MemoryTurn,
    send: F,
) -> turn::DeliveryReport
where
    F: FnMut(Decision) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    let report = turn.deliver(send).await;
    for _ in 0..report.failed() {
        observe_delivery_failure(state, component);
    }
    if report.attempted() > 0 {
        tracing::info!(
            attempted = report.attempted(),
            failed = report.failed(),
            "memory decision notifications completed"
        );
    }
    report
}

/// Record a content-free response failure for the adapter that attempted it.
pub fn observe_delivery_failure(state: &AppState, component: crate::observability::EventComponent) {
    tracing::warn!(?component, "response delivery unavailable");
    if let Some(events) = state.operational_events() {
        let _ = events.record(
            component,
            crate::observability::EventCode::ResponseDelivery,
            crate::observability::EventOutcome::Failed,
            Some(crate::observability::OperationalErrorCategory::Unavailable),
        );
    }
}

async fn drain_owned(state: &AppState) -> Drained {
    let queued: Vec<QueuedFact> = std::mem::take(&mut *AppState::lock(&state.memory_queue));
    let mut drained = Drained::default();
    for item in queued {
        let receipt = match admit_fact_decision(
            state,
            &item.scoped_guild,
            &item.scoped_user,
            &item.fact,
            None,
        )
        .await
        {
            Ok(receipt) => receipt,
            Err(decision) => {
                if let Some(completion) = item.completion {
                    completion.finish(decision);
                }
                drained.refused += 1;
                tracing::warn!(
                    reason = decision.message(),
                    "episode gate: queued model memory write not admitted; nothing stored"
                );
                continue;
            }
        };
        let service = state.memory_service();
        let outcome = service.remember_admitted(
            &item.scoped_guild,
            &item.scoped_user,
            &item.fact,
            item.supersedes.as_deref(),
            item.queued_at,
            receipt.as_deref(),
        );
        let decision = match &outcome {
            Ok(RememberOutcome::Stored(_)) => Decision::Stored,
            Ok(RememberOutcome::Proposed { .. }) => Decision::Proposed,
            _ => Decision::LocalRefused,
        };
        if let Some(completion) = item.completion {
            completion.finish(decision);
        }
        match (&outcome, &receipt) {
            (Ok(RememberOutcome::Stored(_) | RememberOutcome::Proposed { .. }), Some(_)) => {
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
        GateOutcome::Rejected { .. } => "Not stored: the constitutional memory gate refused it. Nothing was stored locally.".into(),
        GateOutcome::Unavailable { .. } => "Not stored locally: admission is unknown because the constitutional memory gate did not return a valid receipt.".into(),
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
    admit_fact_decision(state, scoped_guild, scoped_user, fact, replaces)
        .await
        .map_err(|decision| decision.message().to_owned())
}

async fn admit_fact_decision(
    state: &AppState,
    scoped_guild: &str,
    scoped_user: &str,
    fact: &str,
    replaces: Option<&str>,
) -> Result<Option<String>, Decision> {
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
        GateOutcome::Rejected { .. } => Err(Decision::Rejected),
        GateOutcome::Unavailable { .. } => Err(Decision::Unknown),
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
            memory_turn: None,
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
        let turn = MemoryTurn::default();
        let mut host = scope(&state);
        host.memory_turn = Some(&turn);
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
        assert_eq!(
            turn.decisions().await,
            [Decision::Unknown, Decision::Unknown]
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
            completion: None,
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
            enqueue(&state, g, u, "uses rust", None, 8, None).unwrap_err(),
            "Not queued: already on record."
        );
        assert!(
            enqueue(&state, g, u, "   ", None, 8, None).is_err(),
            "validation still applies"
        );
        assert!(AppState::lock(&state.memory_queue).is_empty());
    }

    #[tokio::test]
    async fn periodic_drain_keeps_decisions_with_the_turn_that_enqueued_them() {
        let state = AppState::in_memory();
        let stored_turn = MemoryTurn::default();
        let refused_turn = MemoryTurn::default();
        state
            .memory_service()
            .remember("g", "u", "already held", 1)
            .expect("seed held fact");
        AppState::lock(&state.memory_queue).extend([
            QueuedFact {
                completion: Some(stored_turn.completion()),
                scoped_guild: "g".into(),
                scoped_user: "u".into(),
                fact: "new fact".into(),
                supersedes: None,
                queued_at: 2,
            },
            QueuedFact {
                completion: Some(refused_turn.completion()),
                scoped_guild: "g".into(),
                scoped_user: "u".into(),
                fact: "already held".into(),
                supersedes: None,
                queued_at: 3,
            },
        ]);

        assert_eq!(
            drain(&state).await,
            Drained {
                admitted: 1,
                refused: 1
            }
        );
        assert_eq!(stored_turn.decisions().await, [Decision::Stored]);
        assert_eq!(refused_turn.decisions().await, [Decision::LocalRefused]);
    }

    #[tokio::test]
    async fn closed_service_admission_cancels_queued_turn_without_hanging_or_writing() {
        let state = AppState::in_memory();
        let mut supervisor = crate::service::ServiceSupervisor::new();
        supervisor.finish_startup();
        let _writer = state.attach_service(supervisor.operations());
        supervisor.begin_draining(
            crate::service::ShutdownReason::Signal,
            tokio::time::Instant::now(),
        );
        let turn = MemoryTurn::default();
        AppState::lock(&state.memory_queue).push(QueuedFact {
            completion: Some(turn.completion()),
            scoped_guild: "g".into(),
            scoped_user: "u".into(),
            fact: "never proposed".into(),
            supersedes: None,
            queued_at: 4,
        });

        assert_eq!(
            drain(&state).await,
            Drained {
                admitted: 0,
                refused: 1
            }
        );
        let decisions = tokio::time::timeout(std::time::Duration::from_secs(1), turn.decisions())
            .await
            .expect("closed admission must terminate the turn");
        assert_eq!(decisions, [Decision::Cancelled]);
        assert!(state.memory_service().facts("g", "u").is_empty());
        assert!(AppState::lock(&state.memory_queue).is_empty());
    }

    #[tokio::test]
    async fn cancelling_one_failed_response_leaves_another_turn_for_its_drain() {
        let state = AppState::in_memory();
        let cancelled = MemoryTurn::default();
        let retained = MemoryTurn::default();
        AppState::lock(&state.memory_queue).extend([
            QueuedFact {
                completion: Some(cancelled.completion()),
                scoped_guild: "g".into(),
                scoped_user: "u".into(),
                fact: "cancel me".into(),
                supersedes: None,
                queued_at: 4,
            },
            QueuedFact {
                completion: Some(retained.completion()),
                scoped_guild: "g".into(),
                scoped_user: "u".into(),
                fact: "keep me".into(),
                supersedes: None,
                queued_at: 5,
            },
        ]);

        assert_eq!(
            cancel_pending(&state, &cancelled),
            Drained {
                admitted: 0,
                refused: 1
            }
        );
        assert_eq!(cancelled.decisions().await, [Decision::Cancelled]);
        assert_eq!(AppState::lock(&state.memory_queue).len(), 1);
        assert_eq!(
            drain(&state).await,
            Drained {
                admitted: 1,
                refused: 0
            }
        );
        assert_eq!(retained.decisions().await, [Decision::Stored]);
        assert_eq!(state.memory_service().facts("g", "u"), ["keep me"]);
    }

    #[tokio::test]
    async fn failed_notice_attempts_the_remainder_without_replaying_queued_memory() {
        let state = AppState::in_memory();
        AppState::lock(&state.memory_queue).push(QueuedFact {
            completion: None,
            scoped_guild: "g".into(),
            scoped_user: "u".into(),
            fact: "background fact".into(),
            supersedes: None,
            queued_at: 1,
        });
        let turn = MemoryTurn::default();
        turn.completion().finish(Decision::Stored);
        turn.completion().finish(Decision::Rejected);
        let mut attempts = Vec::new();

        let report = deliver_notices(
            &state,
            crate::observability::EventComponent::Discord,
            turn,
            |decision| {
                attempts.push(decision);
                std::future::ready(if decision == Decision::Stored {
                    Err("first notice failed")
                } else {
                    Ok(())
                })
            },
        )
        .await;

        assert_eq!(attempts, [Decision::Stored, Decision::Rejected]);
        assert_eq!(report.attempted(), 2);
        assert_eq!(report.failed(), 1);
        assert_eq!(AppState::lock(&state.memory_queue).len(), 1);
        assert!(state.memory_service().facts("g", "u").is_empty());
    }

    #[tokio::test]
    async fn dropped_completion_reports_unobserved_without_claiming_no_local_write() {
        let turn = MemoryTurn::default();
        drop(turn.completion());

        let decisions = turn.decisions().await;
        assert_eq!(decisions, [Decision::Unobserved]);
        assert!(
            !decisions[0]
                .message()
                .contains("Nothing was stored locally")
        );
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

        let turn = MemoryTurn::default();
        let mut host = scope(&state);
        host.memory_turn = Some(&turn);
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
        assert!(
            turn.decisions().await.is_empty(),
            "an uncovered scope publishes no gate outcome notice"
        );
    }
    #[test]
    fn unknown_admission_does_not_claim_no_gateway_change_or_expose_diagnostics() {
        let message = refusal(&GateOutcome::Unavailable {
            detail: "private-path/token timed out".into(),
        });
        assert!(message.contains("unknown"));
        assert!(message.contains("locally"));
        assert!(!message.contains("private-path"));
        assert!(!message.contains("Nothing was changed"));
    }
}
