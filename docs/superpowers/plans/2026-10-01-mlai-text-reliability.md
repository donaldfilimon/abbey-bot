# MLAI Text Latency and Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Measure and improve text delivery while preserving bounded ownership, cancellation and effect-aware fallback.

**Architecture:** Keep one provider execution authority. Introduce closed stage measurements and typed outbound failures, then a joined provider owner with coalesced bounded delivery state. Tune capacity only after attributable measurements.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-text-reliability-design.md](../specs/2026-10-01-mlai-text-reliability-design.md); read it and the parent design before execution.

**Dependencies:** Release baseline first; exposes stage meanings reused by voice and qualification.

## Global Constraints

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

## Review Focus

- Possibly accepted send loses acknowledgement and must not replay — pinned in Task 2.
- Provider continues producing while outbound is blocked without unbounded memory — pinned in Task 3.
- Shutdown during queue wait returns every permit and observes producer completion — pinned in Task 3.
- Discord permission/rate-limit failure must not be reported as a provider outage — pinned in Task 2.
- A no-text or failed probe must appear in the baseline denominator — pinned in Task 4.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Instrument closed request stages

**Files:**
- Modify: `src/observability.rs`
- Modify: `src/observability/tests.rs`
- Modify: `src/provider/runtime/conversation.rs`
- Modify: `src/generation.rs`

**Interfaces:** Produces TextStage→EventCode mappings and bounded durations at the defined ownership points; consumes existing OperationalEvent.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `stages_emit_once_without_private_fields` pins:

```text
assert count(firstVisible) <= 1; assert keys exclude prompt/response/id/url
```

  Test `empty_delta_is_not_first_text` pins:

```text
assert providerFirstTextCount == 0
```

  Test `cancelled_wait_has_terminal_outcome` pins:

```text
assert terminalOutcomes == 1
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Keep request-local clocks in infrastructure; pure event values receive durations. Update privacy/schema tests and existing Python gates by explicit name where required.
- [ ] **Step 4: Run focused verification:** `cargo test --locked observability; cargo test --locked generation::tests (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Preserve typed outbound failure and certainty

**Files:**
- Modify: `src/pipeline.rs`
- Modify: `src/generation.rs`
- Modify: `src/gateway/interaction_outcomes.rs`
- Modify: `src/gateway/discord.rs`
- Modify: `src/gateway/slack.rs`
- Modify: `src/gateway/telegram.rs`
- Modify: `src/generation/tests.rs`
- Modify: `src/pipeline/tests/memory_outcomes.rs`
- Modify: `src/provider/runtime/tests.rs`
- Re-enumerate `impl Outbound` sites before mutation because concurrent writers may add more; Task2 owns migration of newly found adapters/fakes too.

**Interfaces:** Produces OutboundFailure and Result<T,OutboundFailure> for send/react/edit; consumes existing adapters and effect marking.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `lost_ack_is_possibly_sent_and_never_replayed` pins:

```text
assert certainty == PossiblySent; assert fallbackCalls == 0
```

  Test `discord_403_does_not_charge_provider_circuit` pins:

```text
assert category == Permission; assert providerFailures == before
```

  Test `retry_after_is_bounded` pins:

```text
assert retryAfter <= 300
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Map typed adapter errors at the shell boundary, discard raw dynamic diagnostic strings after mapping to closed error categories; retain no new raw-error store and preserve static degraded copy. Enumerate all Outbound implementations in the task receipt; do not leave generic String paths for delivery.
- [ ] **Step 4: Run focused verification:** `cargo test --locked generation::tests; cargo test --locked provider::runtime (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Separate joined producer from bounded delivery

**Files:**
- Create: `src/generation/stream_owner.rs`
- Modify: `src/generation.rs`
- Modify: `src/generation/tests.rs`
- Modify: `src/service/framework.rs`

**Interfaces:** Produces CoalescedText and retained stream producer completion; consumes ProviderConversation/AttemptLease plus Task 2 failure certainty.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `blocked_delivery_keeps_producer_polled` pins:

```text
assert producerCompletedWhileSendBlocked == true
```

  Test `utf8_buffer_bound` pins:

```text
assert accept(65536) == true; assert accept(65537) == false
```

  Test `cancelled_queue_and_producer_are_joined` pins:

```text
assert permitsReturned == true; assert liveOwners == 0
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Use existing service owner registration instead of detached tasks. Coalesce deltas rather than retaining an unbounded queue. Preserve final ModelTurn validation, tool-call ordering and no-op pacing; split modules before caps.
- [ ] **Step 4: Run focused verification:** `cargo test --locked generation::tests; cargo test --locked service (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 4: Measure installed baseline and tune only attributable limits

**Files:**
- Modify: `src/provider_self_test.rs`
- Modify: `src/runtime.rs`
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`

**Interfaces:** Consumes Tasks 1–3 and fixed installed identity; produces aggregate benchmark receipt with 48 synthetic probes and six authorized Discord cases.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `benchmark_counts_incomplete_and_empty_results` pins:

```text
assert total == succeeded + failed + incomplete; assert noTextIncluded == true
```

  Test `fixed_workload_non_regression` pins:

```text
assert failureCount <= baseline.failureCount; assert firstVisibleP95 <= baseline.firstVisibleP95 * 1.10
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Use fake fixtures for classifier/aggregation tests and token-free provider mode for synthetic requests. Compare identical workload/provider/hardware; report queue versus generation versus Discord costs. Change concurrency/timeouts only from evidence and verify voice contention.
- [ ] **Step 4: Run focused verification:** `cargo test --locked provider_self_test; ./check.sh (separate commands; actual provider/live probes are a later acceptance record)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.

## Reviewed stream and measurement interfaces

Task 3 owns `CoalescedText::append(&mut self, delta: &str) -> Result<(), BufferFull>` and `snapshot(&self) -> &str`, with65536-byte inclusive UTF-8 bound. `StreamOwner<T>` owns a `tokio::task::JoinHandle<T>` and `tokio_util::sync::CancellationToken`, both existing dependencies; `async fn cancel_and_join(self) -> Result<T, tokio::task::JoinError>` requests cancellation then observes completion. `async fn join(self) -> Result<T, tokio::task::JoinError>` observes normal completion. No Drop implementation may claim cleanup. Task3 owns service admission/retention and the parent `mod stream_owner` registration. Provider task alone owns the attempt lease, releases it on every terminal path, and publishes validated final ModelTurn once. BufferFull requests cancellation, joins, returns a closed Capacity failure and cannot replay. Tests explicitly block send with a barrier, advance producer to final turn, then cancel/unblock and assert observed join and permit return.

Task4's benchmark report separates the48 synthetic provider probes from the six authorized Discord delivery cases. For each population, count success/failure/incomplete/no-text across all attempts; only successfully observed stage durations enter that stage's distribution. Missing stages remain null with a reason; never assign zero. Compute p95 by nearest rank: sort n durations and take index ceil(0.95*n)-1, undefined for n=0. Six live successes therefore produce a maximum, explicitly labeled a small-sample witness. Compare identical populations/provider/hardware; failure/incomplete/no-text counts must each not increase, observed sample coverage must not decrease, and available same-stage p95 must be <=baseline*1.10. Synthetic provider-first-text is never labeled Discord first-visible latency.

