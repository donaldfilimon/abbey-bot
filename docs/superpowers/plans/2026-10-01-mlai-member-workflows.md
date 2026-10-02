# MLAI Member Workflows and Server Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Make the redesigned server and bot discoverable and usable by ordinary members across complete task flows.

**Architecture:** Keep the additive MLAI organization and catalog-owned authorization. Use classic controls and current REST permissions. Connect Help, Research, Showcase and project/voice/Activity flows with concise truthful guidance; no mass role/permission reconstruction.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-member-workflows-design.md](../specs/2026-10-01-mlai-member-workflows-design.md); read it and the parent design before execution.

**Dependencies:** Activity and voice acceptance records determine truthful invitations/help; learning and Work features add surfaces only after their own qualification.

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

- Administrator bot succeeds where a non-admin member cannot; acceptance must catch it — pinned in Task 3.
- Same display name with different snowflake cannot gain resolution authority — pinned in Task 1.
- Changing resolution preserves unrelated tags and does not archive/delete — pinned in Task 1.
- Help expansion exceeds 2000 characters or leaks owner diagnostics — pinned in Task 2.
- Public workflow answer must not include inaccessible thread/DM evidence — pinned in Task 2.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Provide explicit permission-checked forum resolution

**Files:**
- Create: `src/forum_resolution.rs`
- Modify: `src/commands_forum.rs`
- Modify: `src/main.rs`
- Modify: `src/command_catalog.rs`
- Modify: `src/command_catalog/tests.rs`

**Interfaces:** Produces resolution_tags(ResolutionFacts,ResolutionState); consumes current forum tag IDs and actor/thread author/manage facts.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `same_name_without_authority_denied` pins:

```text
assert result == Denied
```

  Test `resolve_preserves_unrelated_tags` pins:

```text
assert unrelatedTagsAfter == before; assert archiveCalls == 0
```

  Test `missing_or_duplicate_status_tags_refuse` pins:

```text
assert result == InvalidTags
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Keep policy transport-free, fetch current actor/thread facts, defer before REST and preserve unrelated applied tags. Add classic explicit resolution controls; no automatic suggestion path calls this mutation.
- [ ] **Step 4: Run focused verification:** `cargo test --locked forum_resolution; cargo test --locked command_catalog (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Connect honest member-facing help and work flows

**Files:**
- Modify: `src/command_catalog.rs`
- Modify: `src/command_catalog/tests.rs`
- Modify: `src/commands_help.rs`
- Modify: `src/commands_work.rs`
- Modify: `src/forum.rs`

**Interfaces:** Consumes accepted capability state and existing access-bound Work/project APIs; produces clamped eligible guidance for member/manager/owner.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `every_role_readiness_page_fits` pins:

```text
assert rendered.chars <= 2000; assert footerPresent == true
```

  Test `member_cannot_see_owner_diagnostics` pins:

```text
assert ownerOnlyEntriesVisible == false
```

  Test `inaccessible_source_never_reaches_public_answer` pins:

```text
assert privateEvidenceIncluded == false
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Print/read all capability combinations. Link existing typed project/decision and forum flows without duplicating stores or claiming unaccepted Activity/voice features.
- [ ] **Step 4: Run focused verification:** `cargo test --locked command_catalog; cargo test --locked commands_help (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Witness ordinary-member end-to-end server paths

**Files:**
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`
- Modify: `README.md`

**Interfaces:** Consumes Tasks 1–2 and accepted Activity/voice capabilities; produces member/manager/DM case receipts with exact installed identity.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `non_admin_forum_create_reply_resolve` pins:

```text
assert observedAsNonAdmin == true; assert historyPreserved == true
```

  Test `cross_guild_and_dm_cases_are_isolated` pins:

```text
assert otherUserPrivateFactVisible == false
```

  Test `readiness_language_matches_actual_layer` pins:

```text
assert noUnverifiedCapabilityAdvertised == true
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Use read-only permission inventory first; coordinate a small willing-member witness pass. Keep unsuccessful/unsupported cases in the ledger and propose narrow fixes only from actual failures. Do not rebuild roles or overwrite channels to make the test pass.
- [ ] **Step 4: Run focused verification:** `cargo test --locked pipeline::tests; python3 scripts/check-pages-liquid.py; ./check.sh (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.
