# MLAI Opt-in Conversation Continuity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Resume user-confirmed work across restart without persisting raw conversation or widening access.

**Architecture:** Store a small user-confirmed continuity card in canonical state, referencing existing native Work content. Render it only after fresh scope authorization. Generated suggestions stay transient until explicit confirmation; do not create a second project/decision database.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-continuity-design.md](../specs/2026-10-01-mlai-continuity-design.md); read it and the parent design before execution.

**Dependencies:** Learning erasure contract first; reuse existing Work projects/decisions and immutable WorkContentRef provenance.

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

- A generated proposal is never confirmed by a model/tool or by stale click — pinned in Task 1.
- Permission revoked between preview and confirmation/context prevents inclusion — pinned in Task 2.
- Restart restores only unexpired exact-scope confirmed data — pinned in Task 3.
- DM owner changes or guild context cannot read a different user card — pinned in Task 2.
- Clear/erasure survives restart and projection rebuild without resurrection — pinned in Task 3.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Implement confirmed bounded continuity domain

**Files:**
- Create: `src/work/continuity.rs`
- Create: `src/work/continuity/tests.rs`
- Modify: `src/work.rs`

**Interfaces:** Produces ContinuityCard/ContinuityStore and confirm/context APIs; consumes existing WorkScope, WorkAccess, WorkContentRef and WorkError.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `stale_or_model_confirmation_refused` pins:

```text
assert staleConfirm == Stale; assert modelCannotConfirm == true
```

  Test `card_limits_and_expiry` pins:

```text
assert bytes <= 1600; assert refs <= 8; assert cardCount <= 256; assert expiry == now + 604800
```

  Test `missing_source_excludes_context` pins:

```text
assert context == None
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Keep proposals transient; require exact expected revision, caller authority and current source revisions. Inject time and use validated UTF-8 byte limits; reject oversize instead of silent truncation.
- [ ] **Step 4: Run focused verification:** `cargo test --locked continuity`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Integrate fresh authorization and private command flows

**Files:**
- Create: `src/commands_work/continuity.rs`
- Modify: `src/commands_work.rs`
- Modify: `src/runtime/memory_service.rs`
- Modify: `src/pipeline.rs`
- Modify: `src/command_catalog.rs`
- Modify: `src/command_catalog/tests.rs`

**Interfaces:** Consumes Task 1 APIs; produces AuthorizedContinuity passed into prompt preparation after current access recheck.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `revoked_access_between_show_and_confirm_denies` pins:

```text
assert writes == 0; assert contextIncluded == false
```

  Test `two_dm_users_and_guild_never_share_card` pins:

```text
assert otherScopeCardIncluded == false
```

  Test `commands_defer_and_render_private` pins:

```text
assert deferredBeforeNetwork == true; assert publicReply == false
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Use existing REST access seam and Work management rules. Bind private show/propose/confirm/clear commands and include bounded card only in the exact prepared scope. Update catalog/help size coverage.
- [ ] **Step 4: Run focused verification:** `cargo test --locked continuity; cargo test --locked command_catalog (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Persist and erase continuity without transcript retention

**Files:**
- Modify: `src/persist.rs`
- Modify: `src/persist/tests.rs`
- Modify: `src/runtime/memory_service.rs`
- Modify: `src/memory_gate.rs`

**Interfaces:** Consumes confirmed cards only and learning erasure integration; produces version/default migration and no-resurrection cleanup.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `restart_restores_only_current_confirmed_card` pins:

```text
assert restored == confirmedUnexpiredOnly
```

  Test `clear_and_member_erase_survive_projection_rebuild` pins:

```text
assert clearedContext == None; assert unrelatedCard == before
```

  Test `refused_episode_write_is_not_retried` pins:

```text
assert persisted == false; assert retryCount == 0
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Add serde-default canonical field and validated load bounds. Preserve existing admitted checkpoints, remove cache alongside canonical card, and exclude human voice transcript content absolutely.
- [ ] **Step 4: Run focused verification:** `cargo test --locked continuity; cargo test --locked persist; cargo test --locked memory_gate (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.

## Reviewed exact confirmation contract

Task 1 owns `ContinuityProposal { id: ProposalId, actor: u64, scope: WorkScope, base_revision: u64, presented_text: String, source_refs: BTreeSet<WorkContentRef>, expires_at: u64 }` in `src/work/continuity.rs`. `ProposalId { boot_nonce: [u8;16], sequence: u64 }` combines an infrastructure-generated fresh OS-random128-bit boot nonce with a monotonically checked session counter. Control encoding includes both; lookup checks both, and counter overflow refuses admission. IDs cannot intentionally be reused across boots; pending proposals remain transient, capped at256, expire after300seconds, and disappear at restart. Confirmation looks up the immutable presented text by ID; the client never submits replacement text. Thus no digest or production dependency is needed. A private human interaction is the only confirmation entry; model tools cannot construct the shell's confirmation authority.

`ContinuityStore::confirm(&mut self, proposal: &ContinuityProposal, actor: u64, access: &WorkAccess, current_source_refs: &BTreeSet<WorkContentRef>, now: u64) -> Result<ContinuityCard, WorkError>` verifies actor, exact scope, expiry, current source revisions, and stored card revision against base_revision. Task 2 owns the transient registry in `src/commands_work/continuity.rs` and resolves the control's proposal ID before calling confirm. Consume the proposal once; a different proposal against the same base revision is stale after the first succeeds. `context(&self, scope: &WorkScope, access: &WorkAccess, current_source_refs: &BTreeSet<WorkContentRef>, now: u64) -> Option<AuthorizedContinuity>` returns a private-constructor `AuthorizedContinuity { text: String }` only after all checks.

Add `two_proposals_same_base_revision_bind_exact_text`, `other_actor_control_denied`, `proposal_expired_or_missing_after_restart_denied`, and `changed_source_between_preview_and_click_denied` to Task1/2 source tests. Use two different texts and proposal IDs against base_revision0; confirming A stores only A, and confirming B afterward yields Stale. An old control never resolves to a newer proposal. Restart test creates a new proposal with the same numeric sequence under a different boot nonce, then clicks the old control and asserts denial without changing the new proposal.

