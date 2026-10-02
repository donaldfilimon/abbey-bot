# MLAI Voice Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Prove reliable consented server voice and eliminate lifecycle ambiguity without weakening consent.

**Architecture:** Use the existing consent/media epoch runtime and voice supervision. Make every preparation/listening owner observable by the existing service registry. Validate all-current roster/receipts/permissions before Decode and before unmute; keep Pass/self-deafen for presence and output-only reconnect.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-voice-completion-design.md](../specs/2026-10-01-mlai-voice-completion-design.md); read it and the parent design before execution.

**Dependencies:** Release baseline first; coordinate text telemetry semantics with text-reliability; human acceptance needs willing participants.

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

- Receipt persists across restart but changed policy/version fails coverage — pinned in Task 1.
- Delayed preparation completes after withdrawal or shutdown and must not open media — pinned in Task 2.
- Retired bot session event arrives after a new session and must be ignored — pinned in Task 2.
- Uncovered new arrival pauses a currently speaking call immediately — pinned in Task 3.
- Synthetic audition or muted presence must never tick human audible acceptance — pinned in Task 4.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Pin current upgrade and durable consent policy

**Files:**
- Modify: `src/commands_voice/auto_listen_gate.rs`
- Modify: `src/voice_consent.rs`
- Modify: `src/voice_consent_store.rs`
- Modify: `src/commands_voice/supervision.rs`

**Interfaces:** Produces decide_upgrade(UpgradeFacts) -> UpgradeDecision; consumes existing phase/roster/receipt facts. No second caller or new consent ledger.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `valid_receipt_survives_restart` pins:

```text
assert agrees(Local, policy1) == true
```

  Test `changed_policy_and_empty_roster_refuse` pins:

```text
assert decision == Noop
```

  Test `phase_matrix_only_upgrades_presence` pins:

```text
assert Eligible only for PresenceOnly + all required facts
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Reuse existing pure gate where its types suffice; add facts only if the current tests cannot express permissions/phase. Correct stale documentation about fresh-every-epoch consent.
- [ ] **Step 4: Run focused verification:** `cargo test --locked auto_listen; cargo test --locked voice_consent (separate commands, nonzero counts)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Retain and join upgrade/preparation owners

**Files:**
- Modify: `src/commands_voice/supervision.rs`
- Modify: `src/commands_voice/auto_listen.rs`
- Modify: `src/service.rs`
- Modify: `src/voice_session/ownership.rs`

**Interfaces:** Consumes Task 1 Eligible and existing service admission/retained joins. Produces tracked upgrade owner whose completion is observed before shutdown freeze.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `withdrawal_during_prepare_never_opens_decode` pins:

```text
assert decodeOpened == false; assert joined == true
```

  Test `retired_event_does_not_revoke_new_epoch` pins:

```text
assert currentEpoch == newEpoch
```

  Test `shutdown_observes_upgrade_owner` pins:

```text
assert liveOwners == 0; assert persistenceAttempts <= 1
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Audit the existing tokio::spawn join hook. Transfer ownership to the existing service/voice owner registry; keep generation checks around every await. Do not invent a second shutdown controller.
- [ ] **Step 4: Run focused verification:** `cargo test --locked voice_session; cargo test --locked service (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Extend the consent, interruption and reconnect matrix

**Files:**
- Modify: `src/commands_voice/supervision.rs`
- Modify: `src/voice_local.rs`
- Modify: `src/voice_session.rs`
- Modify: `src/observability.rs`
- Modify: `src/observability/tests.rs`

**Interfaces:** Consumes retained owner and existing revocation paths; produces closed voice timing categories and fake-media lifecycle evidence.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `uncovered_arrival_stops_speaking_before_later_frames` pins:

```text
assert processedFramesAfterRevoke == 0
```

  Test `output_only_reconnect_has_no_decode` pins:

```text
assert mode == Pass; assert selfDeaf == true
```

  Test `voice_events_are_content_free` pins:

```text
assert serializedKeys exclude utterance/member/channel/rawAudio
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Use existing fake PCM/media seams. Add timeout, barge-in, backend failure, bot-state adverse event and leave cases; confirm telemetry cannot retain human content.
- [ ] **Step 4: Run focused verification:** `cargo test --locked voice_local; cargo test --locked observability (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 4: Qualify the installed human conversation and watcher decision

**Files:**
- Modify: `README.md`
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`
- Modify: `deploy/watch-office-hours-auto-listen.sh`

**Interfaces:** Consumes exact installed hash, synthetic probe and Tasks 1–3. Produces witnessed case ledger and operator decision to retain/retire fallback watcher.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `human_case_ledger_requires_witness` pins:

```text
assert audibleCase has participantWitness && exactInstalledHash
```

  Test `consent_block_is_not_retryable_restart_reason` pins:

```text
assert missingReceipt never requests restart
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** First keep watcher unchanged and audit observed behavior. Obtain participant action rather than fabricate a receipt. Run controlled witnessed cases; prepare a narrow watcher change only if evidence proves it necessary, with separate operator direction and shell/fake launchctl tests wired into the gate.
- [ ] **Step 4: Run focused verification:** `RUN_ABBEY_BOT_OUT=<fresh-private-directory> .claude/skills/run-abbey-bot/smoke.sh voice; ./check.sh (separate source and live acceptance records)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.
