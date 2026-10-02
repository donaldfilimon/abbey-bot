# MLAI Release and Completion Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Prove source, installed artifact and live capability acceptance across the entire completion program.

**Architecture:** Use the existing transactional installer/readiness rollback and dated acceptance ledger. Freeze source identity before gates and installation. Maintain one row per requirement with its proper proof layer; an unavailable witness/host stays Blocked or Partial, never silently removed from scope.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-release-qualification-design.md](../specs/2026-10-01-mlai-release-qualification-design.md); read it and the parent design before execution.

**Dependencies:** Task 1 begins immediately after planning approval; final acceptance depends on all seven other workstreams.

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

- Moving source after gate invalidates attribution before install — pinned in Task 1.
- Installer timeout with live handle is a wait, not a failed transaction to restart — pinned in Task 1.
- Green source tests must not promote human voice/new iframe to Current — pinned in Task 3.
- Missing external fixture or hardware must stay a named proof gap — pinned in Task 2.
- Cleanup/rollback may retain recovery state; preserve it and inspect before retry — pinned in Task 1.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Close exact current source-to-installed baseline

**Files:**
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`

**Interfaces:** Consumes current frozen source, release/installed hashes and existing installer; produces ReleaseReceipt. This is an operator action, not a new code subsystem.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `baseline_receipt_is_exact` pins:

```text
assert gatedSource == installedBuildSource; assert releaseSha == installedSha
```

  Test `unknown_transaction_is_not_restarted` pins:

```text
assert terminalObservedBeforeRetry == true
```

  Test `rollback_retains_recovery` pins:

```text
assert retainedRecoveryNotDeleted == true
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Revalidate source/owners and readiness. If unchanged, use existing gate result; rerun gate if source changed. With operator execution authorization, transactionally install only when hashes differ and inspect the same live handle until terminal, then status/hash. No config/model changes bundled in this task.
- [ ] **Step 4: Run focused verification:** `python3 -I deploy/service-status.py; shasum -a 256 target/release/abbey-bot ~/.local/libexec/abbey-bot/abbey-bot; ./deploy/install-launchd.sh (separate commands; installer only after operator direction)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Qualify routes, isolation and affected platform contracts

**Files:**
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`
- Modify: `README.md`

**Interfaces:** Consumes each completed source slice and exact installed identity; produces route/contract/platform acceptance rows with explicit unavailable layers.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `qualification_requires_exact_identity` pins:

```text
assert reportedModel == observedQualifiedModel
```

  Test `missing_fixture_is_not_conformance_pass` pins:

```text
assert strictGateMissingFixture == Blocked
```

  Test `platform_scope_is_explicit` pins:

```text
assert macGateDoesNotClaimWindowsRuntime == true
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Run fresh token-free synthetic provider/voice probes in fresh output directories. Run strict WDBX fixture gate for affected contracts, and platform-specific gates on available hosts for portability changes. Never run live episode acceptance except exact operator-authorized ignored scratch-gateway test.
- [ ] **Step 4: Run focused verification:** `./check.sh; ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh when contract/persistence affected; ./check.ps1 on Windows when claiming Windows qualification (separate terminal records)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Audit all deliverables and close only evidenced requirements

**Files:**
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`
- Modify: `tasks/goals.md`
- Modify: `tasks/todo.md`

**Interfaces:** Consumes all seven project acceptance records; produces completion matrix and append-only ledger correction. No historical goal rewrite.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `all_spec_requirements_have_evidence_rows` pins:

```text
assert uncoveredRequirements == 0
```

  Test `human_and_public_layers_have_actual_witnesses` pins:

```text
assert voiceAudibleWitness && newCourtIframeWitness
```

  Test `external_blockers_remain_visible` pins:

```text
assert missingEvidenceNeverMarkedCurrent == true
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Read whole relevant ledger sections and verify artifacts before appending. Audit every named feature/command/invariant/test/platform/receipt against current evidence. Report Current/Partial/Blocked separately; close implementation goal only when all required rows are proven.
- [ ] **Step 4: Run focused verification:** `python3 scripts/check-pages-liquid.py; git diff --check; final read-only identity/status and requirement audit (each result recorded)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.
