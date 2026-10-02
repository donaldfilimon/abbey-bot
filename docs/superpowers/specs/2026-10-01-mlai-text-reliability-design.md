# MLAI Text Latency and Reliability Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Measure and improve text delivery while preserving bounded ownership, cancellation and effect-aware fallback.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Release baseline first; exposes stage meanings reused by voice and qualification.

## Current evidence

Two pacing regressions reproduced 100ms/1.1s intermediate spacing and now pass; generation suite passed 16. Provider attempt duration exists. Installed queue/first-visible/final distributions are unknown. Common Outbound currently returns String errors; provider work and outbound awaits share a select loop.

## Architecture

Keep one provider execution authority. Introduce closed stage measurements and typed outbound failures, then a joined provider owner with coalesced bounded delivery state. Tune capacity only after attributable measurements.

## Requirements

TEXT-RELIABILITY-R1: Measure queue wait from reserve attempt to admitted permit, provider-first-text from provider start to first nonempty delta, first-visible from request admission to successful first send, final-delivered from admission to successful final send/edit, and cancellation/failure outcomes.
TEXT-RELIABILITY-R2: Add closed EventCode values only; preserve OperationalEvent schema discipline. Logs carry duration, aggregate count and configured ProviderId, never prompt/reply/run identity/URL/recipient/channel or dynamic error text.
TEXT-RELIABILITY-R3: OutboundFailure has category Permission, RateLimited, Transport, Capacity or Internal; certainty NotSent or PossiblySent; optional retry_after_secs is bounded to 300. Redacted/static Debug and user copy; no arbitrary error string escapes.
TEXT-RELIABILITY-R4: Typed send/react/edit results replace String failure results across all Outbound implementations and fakes; fetch may keep its infrastructure error path until explicitly classified. Discord delivery failure must not increment provider failure/circuit evidence.
TEXT-RELIABILITY-R5: Provider execution stays under an observed join owner while delivery awaits. Coalesced accumulated UTF-8 text is bounded to 65,536 bytes; overflow cancels and observes the producer, renders a bounded honest failure and never executes/replays tools.
TEXT-RELIABILITY-R6: Progressive edits wait 2 seconds after successful delivery; final/failure replacement remains immediate. Effect marking precedes possibly accepted network delivery; no fallback after visible output, tool dispatch or uncertain send.
TEXT-RELIABILITY-R7: Baseline: 24 synthetic short-answer, 12 coding and 12 tool-turn probes on fixed provider/model/hardware, plus six operator-authorized low-volume Discord requests for actual visible-delivery receipts. Report incomplete probes and failures; do not substitute REST-only delivery timing for generation timing.
TEXT-RELIABILITY-R8: After tuning, controlled-case failures must not increase and first-visible p95 must be at most 1.10 times the same installed baseline. A 25% reduction is an optimization target, not a promise; failing the non-regression condition requires investigation or explicit revised acceptance.

## Interfaces and data ownership

Proposed `OutboundFailure { category: OutboundFailureCategory, certainty: DeliveryCertainty, retry_after_secs: Option<u16> }`. Proposed `TextStage::{QueueWait,ProviderFirstText,FirstVisible,FinalDelivered,Cancelled,DeliveryFailed}` maps to closed EventCode; no dynamic payload. Proposed `CoalescedText::push(&mut self, delta: &str) -> Result<(), StreamOverflow>` caps UTF-8 bytes at 65,536. Existing ProviderConversation/AttemptLease retain admission and effect authority.

## Global constraints

- Rust 1.98.0 stable, edition 2024; retain --locked on every Cargo command.
- Canonical abbey-bot checkout; preserve unrelated dirty work; no checkout, clone, worktree, commit, push or PR without separate authorization.
- No new production dependency, credential rotation, permission escalation or deployment configuration change without the required operator decision.
- Pure policy receives time and seeds from callers; no Discord, network, randomness or wall-clock reads in pure modules.
- Production and external test-only Rust modules remain below 1,000 lines; production modules over 800 lines require review.
- Keep explicit ToolScope; voice, unsolicited generation and summaries remain read-only.
- Every DM uses network:dm:<user_id>; scoped persistence keys use U+001F; never widen a private scope into a guild or another user.
- Canonical state is atomic abbey-state.json; wdbx.seg.0.jsonl remains a rebuildable projection; no ABI/WDBX crate dependency.
- No automatic quarantine, contradiction or resolve edges; those remain explicit administrative actions.
- Service owners close admission, join observed work, freeze and attempt final persistence at most once; cancellation request is not cleanup.
- Managed JSONL contains only closed content-free events; no prompts, replies, raw feedback, recipient IDs, channel IDs, URLs, credentials or dynamic errors.
- Voice stores no raw audio or transcripts; personal durable Local policy-1 receipts, current roster, permissions and media epoch gate processing.
- Activity Portal URL mapping and human-witnessed acceptance remain operator-gated; no second Discord gateway beside the managed service.
- Preserve pipeline guard, cooldown and hourly-budget ordering; no fallback replay after visible output or mutating/uncertain effects.
- Defer interactions before network work, clamp rendered replies, preserve classic Action Rows; Components V2 remains crate-blocked.
- Run ./check.sh for each completed source slice, python3 scripts/check-pages-liquid.py for docs/Activity, and npm --prefix activity test for Court.
- Source tests, installed hash, provider qualification, Discord behavior and human acceptance are separate evidence layers; unknown evidence is not completion.

## Failures and acceptance

- Possibly accepted send loses acknowledgement and must not replay. Test owner: implementation Task 2.
- Provider continues producing while outbound is blocked without unbounded memory. Test owner: implementation Task 3.
- Shutdown during queue wait returns every permit and observes producer completion. Test owner: implementation Task 3.
- Discord permission/rate-limit failure must not be reported as a provider outage. Test owner: implementation Task 2.
- A no-text or failed probe must appear in the baseline denominator. Test owner: implementation Task 4.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-text-reliability.md](../plans/2026-10-01-mlai-text-reliability.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.

## Reviewed stream and measurement interfaces

Task 3 owns `CoalescedText::append(&mut self, delta: &str) -> Result<(), BufferFull>` and `snapshot(&self) -> &str`, with65536-byte inclusive UTF-8 bound. `StreamOwner<T>` owns a `tokio::task::JoinHandle<T>` and `tokio_util::sync::CancellationToken`, both existing dependencies; `async fn cancel_and_join(self) -> Result<T, tokio::task::JoinError>` requests cancellation then observes completion. `async fn join(self) -> Result<T, tokio::task::JoinError>` observes normal completion. No Drop implementation may claim cleanup. Task3 owns service admission/retention and the parent `mod stream_owner` registration. Provider task alone owns the attempt lease, releases it on every terminal path, and publishes validated final ModelTurn once. BufferFull requests cancellation, joins, returns a closed Capacity failure and cannot replay. Tests explicitly block send with a barrier, advance producer to final turn, then cancel/unblock and assert observed join and permit return.

Task4's benchmark report separates the48 synthetic provider probes from the six authorized Discord delivery cases. For each population, count success/failure/incomplete/no-text across all attempts; only successfully observed stage durations enter that stage's distribution. Missing stages remain null with a reason; never assign zero. Compute p95 by nearest rank: sort n durations and take index ceil(0.95*n)-1, undefined for n=0. Six live successes therefore produce a maximum, explicitly labeled a small-sample witness. Compare identical populations/provider/hardware; failure/incomplete/no-text counts must each not increase, observed sample coverage must not decrease, and available same-stage p95 must be <=baseline*1.10. Synthetic provider-first-text is never labeled Discord first-visible latency.

