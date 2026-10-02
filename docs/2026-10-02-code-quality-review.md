# Strict code quality review — 2026-10-02

**Verdict: REQUEST CHANGES.** Scope: current uncommitted tracked changes and new Rust/Activity surfaces relative to HEAD. This is a shared, moving checkout; findings apply to the inspected implementations, not a frozen release. No implementation fixes, deployment, or publication were performed.

## Findings

### 1. P2 — Consolidate the personal-memory publication state machine

Locations: `src/runtime/personal_memory_commit.rs:427`, `src/runtime/personal_memory_commit/reconciliation.rs:217`, `src/service/persistence.rs:268`.

The new transaction owner implements canonical publication, lineage readback, projection qualification, outcome completion, a second canonical publication, marker removal, live installation, and denial retirement. Replay implements substantially the same publication sequence separately. The ordinary writer has its own marker/lease admission sequence. Correctness consequently depends on several paths preserving the same order, while AppState carries pending requests, a mutation lock, reconciliation state, a global barrier, and persistence preparation.

This is a structural blocker, not a request to relax consent or crash recovery. Keep the immediate withdrawal fence, but represent preparation/replay as typed transaction inputs to one publication owner. That owner should return a qualified commit receipt and perform one shared installation/retirement sequence. Keep canonical lineage outside payload equality rather than making `CanonicalBase::eq` always return true. Merely splitting the 793-line file would retain the underlying complexity.

Validation: preserve existing lost-ack, restart, immediate-off, cross-subject denial, stale-base, projection failure, and interrupted-publication tests; run each failure phase against both fresh and replay requests.

### 2. P2 — Remove unrelated jobs from WorkDeliveryTransport

Locations: `src/runtime/work_delivery.rs:10`, `src/runtime/scheduler.rs:92`, `src/gateway/work_delivery.rs:12`.

Work's fakeable delivery boundary now exposes engagement scheduling and autonomous community maintenance, both defaulting to silent success. The scheduler routes engagement through this unrelated transport and executes maintenance only after Work delivery, under `OperationKind::WorkDelivery`. A slow reminder batch delays independent maintenance, and the operation registry cannot distinguish server mutations from reminder delivery. Fake Work transports silently omit the two new behaviors.

Give the scheduler explicit job dependencies for Work, engagement, and community operations; admit each through its own retained operation owner. Keep WorkDeliveryTransport responsible for Work authorization and delivery. Retain the existing per-job single-flight and shutdown guarantees. Do not parallelize stages within a mutation transaction.

Validation: hold Work delivery pending and verify an independently admitted maintenance job can progress; verify cancellation/join attribution per job and no silent default implementation of required scheduling behavior.

### 3. P2 — Replace engagement's feature switchboard with a typed delivery plan

Location: `src/runtime/engagement_delivery.rs:358`.

One shared attempt loop branches on candidate kind, optional source, optional response, community classification, and optional introduction. The same distinctions recur during hydration, generation, source validation, currentness checks, readiness, and final authorization. Optional combinations encode implicit invariants the reader must reconstruct repeatedly. Adding another engagement feature expands several unrelated parts of this loop.

Resolve the candidate once into a closed delivery-plan enum, with introduction, source/exchange, community, and invitation variants carrying their required evidence. Variant-specific preparation should produce the body and final proof; the common loop should own reservation, fresh recipient authorization, send certainty, and settlement. Preserve the final recipient check after external proofs.

The pre-reservation helper also erases cancellation and timeout into `WorkError::Denied` (`bounded`), and the wildcard authorization branch permanently rejects the candidate and clears introduction approvals. Use a typed preflight outcome so explicit denial, unavailable proof, cancellation, and timeout cannot accidentally acquire identical policy meaning. Preserve the agreed no-send behavior and make terminal versus deferred outcomes deliberate.

Validation: cover every plan variant and distinguish explicit denial from cancellation/timeout before reservation, including introduction approval retention or deliberate retirement. Preserve existing no-replay tests after an attempted send.

### 4. P2 — Acquire cleanup ownership immediately after creating a lock

Locations: `src/persist/community_ops.rs:64`, `src/persist/community_ops.rs:149`.

`set_mode` creates the control lock and calls `sync_all` before constructing its cleanup guard. `Lease::acquire` creates the execution lock and performs both `write_all` and `sync_all` before constructing Lease. An ordinary I/O failure returns while leaving a lock that all future calls interpret as an occupied/crashed operation. A failed control-lock sync can block Stop even though no mode transaction was admitted.

The neighboring proposal-store `lock` helper already constructs its owned-file guard before syncing. Reuse a common owned-lock primitive, constructing ownership immediately after successful create_new. Separate execution metadata from file cleanup ownership; retain genuinely crash-left evidence and never remove pre-existing locks. Apply the same deliberate temporary-file ownership policy to ledger publication.

Validation: inject write/sync failures after lock creation and verify only this invocation's lock is removed; verify pre-existing locks and genuinely published/recovery files remain intact.

### 5. P2 — Move community filesystem transactions off async executor threads

Locations: `src/gateway/community_ops.rs:38`, `src/commands_brain/dashboard/operations.rs:102`, `src/gateway/community_ops/assessment.rs:318`.

The policy watcher synchronously stats, reads, parses, validates, and hashes the policy every 250 ms while polling HTTP. Policy files may be up to 8 MiB. Async command handlers also call set_mode directly, including file and directory fsync; assessment directly runs proposal-store read/write transactions. Disk stalls block the executor thread and can delay cancellation, command acknowledgements, and unrelated gateway work. The timeout around the HTTP future cannot preempt a blocking load.

Use the existing retained blocking-operation infrastructure for complete filesystem transactions. Let the async watcher consume observed policy results while preserving the initial pre-dispatch check, 250 ms stop observation requirement, conservative uncertainty handling, and owner join. Do not turn publication into detached spawn_blocking work or hold application locks across an await.

Validation: inject a blocked filesystem operation and verify unrelated executor tasks and cancellation remain responsive, with the filesystem owner still retained through observed completion.

## Verification and limits

- Rust module-size/suppression checker: passed. No inspected Rust file crosses the 1,000-line cap; several existing production modules still require review above 800 lines.
- Activity test command: exit 0; 20 passed, zero failed.
- `git diff --check`: exit 0.
- Full strict gate: result recorded below when terminal completion is observed.
- These findings are source/control-flow evidence and structural review judgments. Failure-injection remedies above are requested coverage, not tests claimed to have run. Installed identity, public iframe, provider qualification, live Discord, and human voice were not qualified by this review.
