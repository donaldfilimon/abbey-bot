# MLAI Bounded Learning Quality Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Make personality adaptation and learning rewards reliable, inspectable and deletable within honest limits.

**Architecture:** Retain one per-guild brain and template-only style ledger. Improve attribution before expanding reward volume. Minimize reward inputs, add private aggregate explanations, and define member-linked erasure separately from aggregate trained influence.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-learning-quality-design.md](../specs/2026-10-01-mlai-learning-quality-design.md); read it and the parent design before execution.

**Dependencies:** Text failure categorization first; continuity and initiative consume this audit/erasure contract.

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

- Replayed add/remove reaction across restart contributes at most once — pinned in Task 1.
- Two competing conversations must not assign feedback to the newest turn arbitrarily — pinned in Task 1.
- A legacy raw-ask record must disappear after migrated publication — pinned in Task 2.
- A quoted correction must not trigger repair or durable fact mutation — pinned in Task 3.
- Erasing a member must not resurrect state from caches, projection or late settlement — pinned in Task 4.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Make attribution and reaction reward idempotent

**Files:**
- Modify: `src/brain/reward.rs`
- Modify: `src/brain/reward/tests.rs`
- Modify: `src/persist.rs`
- Modify: `src/persist/tests.rs`
- Modify: `src/gateway/interaction_outcomes.rs`
- Modify: `src/pipeline.rs`

**Interfaces:** Produces ReactionKey active ledger and FeedbackAttribution; consumes existing ReplyTurn/pending settlement.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `reaction_add_remove_restart_is_idempotent` pins:

```text
assert contribution after duplicateAdd == once; assert after doubleRemove == 0
```

  Test `competing_scope_turns_are_ambiguous` pins:

```text
assert attribution == Ambiguous; assert delayedRewardUnchanged == true
```

  Test `expired_turn_cannot_be_reopened_by_eviction` pins:

```text
assert reopened == false
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Carry reactor identity through the shell, hash only at the policy boundary, enforce 4096-entry bound/300-second retention, and preserve reward clamp/window. Register external tests in existing binary module seams.
- [ ] **Step 4: Run focused verification:** `cargo test --locked brain::reward; cargo test --locked pipeline::tests (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Minimize pending ask persistence and explain outcomes

**Files:**
- Create: `src/brain/ask_signature.rs`
- Modify: `src/brain/mod.rs` (this task registers ask_signature)
- Modify: `src/brain/outcome.rs`
- Modify: `src/brain/telemetry.rs`
- Modify: `src/persist.rs`
- Modify: `src/persist/tests.rs`
- Modify: `src/commands_brain/dashboard.rs`

**Interfaces:** Produces bounded AskSignature and LearningAudit; consumes Task 1 attribution and canonical legacy pending rows.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `legacy_ask_migrates_without_raw_republication` pins:

```text
assert signature.tokens.len() <= 32; assert encodedPending lacks rawAsk
```

  Test `audit_is_aggregate_and_labels_action_values` pins:

```text
assert text excludes memberIds/rawAsk; assert label == "action values"
```

  Test `no_feedback_preserves_existing_settlement` pins:

```text
assert settledReward == oldBaseline
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Compute bounded hashed overlap features using existing Wyhash contract, retain closed marker flags, and version/default migration. Print/read diagnostics and require <=2000 characters without clamp loss. Preserve episode-gate admitted checkpoints.
- [ ] **Step 4: Run focused verification:** `cargo test --locked brain::outcome; cargo test --locked persist; cargo test --locked commands_brain (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Recover explicitly attributed corrections

**Files:**
- Create: `src/brain/correction.rs`
- Modify: `src/brain/mod.rs` (this task registers correction)
- Modify: `src/pipeline.rs`
- Modify: `src/generation.rs`
- Modify: `src/grounding.rs`

**Interfaces:** Produces CorrectionDecision; consumes exact attribution and currently authorized grounding. Repair is read-only.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `quoted_wrong_is_not_correction` pins:

```text
assert decision == Ignore
```

  Test `repair_without_evidence_admits_uncertainty` pins:

```text
assert answerDoesNotInventFact == true
```

  Test `repair_never_emits_memory_edge` pins:

```text
assert mutatingToolCalls == 0; assert memoryEdges == 0
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Separate repair outcome from positive engagement. Use fixed recovery instructions and current authorized evidence; refuse automatic stored-fact replacement even when the model requests it.
- [ ] **Step 4: Run focused verification:** `cargo test --locked correction; cargo test --locked generation::tests (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 4: Define linkable erasure and scoped learning reset

**Files:**
- Create: `src/brain/erasure.rs`
- Modify: `src/brain/mod.rs` (this task registers erasure)
- Modify: `src/runtime/memory_service.rs`
- Modify: `src/brain/reward.rs`
- Modify: `src/brain/social.rs`
- Modify: `src/brain/addenda.rs`
- Modify: `src/persist.rs`
- Modify: `src/commands_brain.rs`

**Interfaces:** Produces LearningEraseReport and explicit manager-only aggregate reset path; consumes canonical member key and Tasks 1–2 indexes.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `erase_restart_cache_and_late_settlement_do_not_resurrect` pins:

```text
assert memberLinkedRows == 0 after restart; assert lateSettlementIgnored == true
```

  Test `erase_preserves_other_members_and_facts` pins:

```text
assert otherMemberState == before; assert canonicalFactsUnrelated == before
```

  Test `personal_erase_does_not_claim_weight_unlearning` pins:

```text
assert report.aggregateReset == false; assert honestLimitShown == true
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Inventory every linkable store/index first. Implement idempotent removals and late-settlement tombstone expiry sufficient for pending window. Reuse existing /forget fact behavior and add explicit learning category controls; require guild-manager confirmation for full scoped brain reset.
- [ ] **Step 4: Run focused verification:** `cargo test --locked erasure; cargo test --locked persist; ./check.sh (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 5: Evaluate learning usefulness before tuning policy

**Files:**
- Create: `tests/fixtures/learning-quality-v1.json`
- Create: `src/brain/quality_evaluation.rs`
- Modify: `src/brain/mod.rs` (register quality_evaluation)
- Modify: `src/brain/outcome.rs`
- Modify: `src/grounding.rs`
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`

**Interfaces:** Consumes Tasks 1–4; produces frozen 100-case corpus and measured quality/action distribution report, not answer confidence probabilities.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `corpus_has_five_disjoint_classes` pins:

```text
assert total == 100; assert eachClass == 20
```

  Test `heldout_claims_have_support_or_abstain` pins:

```text
assert unsupportedSpecificAccepted == 0
```

  Test `rollback_preserves_canonical_facts` pins:

```text
assert factsAfterBrainRollback == factsBefore
```

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [x] **Step 3: Implement the pinned interface/behavior:** Add deterministic seeds, topology/nonfinite import regressions and scoped snapshot comparison using existing DQN APIs. Do not tune reward weights/actions until operator review of held-out measurements.
- [x] **Step 4: Run focused verification:** `cargo test --locked brain; cargo test --locked grounding (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

Task5 evidence update (2026-10-03): Steps2–7 are checked for the bounded source
slice only, based on independent spec/standards PASS, strict32980 exit0 on stable
799-file SHA `fc1cc1bb6e5c81457638dede4e712dd5c5a627127d4f51a4364189843677a457`,
and the [source receipt](../../verification/2026-10-03-learning-quality-v1.md).
Step1 remains unchecked as a historical RED-evidence limitation: all three named
tests pass, but their individual pre-implementation failures were not retained.
The actual NaN/action/replay/full-corpus RED regressions are preserved and satisfy
the recorded Step2 failure check; no retrospective RED is fabricated. Overall
Task5 remains Partial: the 100 synthetic labels are agent-authored, independent
human adjudication and fixed-provider/manual support are OPEN before tuning,
and no deployed/live identity proof is supplied. These plan checks are a later
documentation delta, not part of the frozen source-gate manifest.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.

## Reviewed learning interfaces and evaluator

Task1 owns canonical persistence/default migration of the bounded reaction ledger and settled reward markers in `src/persist.rs`/`src/persist/tests.rs`. Persist these together with pending outcomes before exposing completion; restart tests reload the serialized state before replaying add/remove. Never promise idempotence from transient caches alone.

Task2 owns `LearningAudit { exact: u64, unique: u64, duplicate: u64, ambiguous: u64, expired: u64, unsupported: u64 }` and `LearningAudit::record(&mut self, attribution: FeedbackAttribution) -> ()`, `snapshot(&self) -> LearningAudit` in `src/brain/telemetry.rs`; values are counters only. `AskSignature::from_text(text: &str) -> AskSignature` uses the existing pinned hash seam and at most32 distinct normalized token hashes; tests pin deterministic normalization, bounds and absence of raw text after migrated publication.

Task3 owns `evaluate_correction(attribution: FeedbackAttribution, quoted: bool, source_turn: Option<u64>, authorized: bool) -> CorrectionDecision` in `src/brain/correction.rs`: only exact/unique authorized nonquoted attribution with a current source turn may Repair; everything else Ignore. It supplies no fact mutation authority.

Task4 owns `LearningEraseReport { pending: usize, reactions: usize, social: usize, style: usize, continuity: usize, engagement: usize, aggregate_reset: bool }`. `erase_member(state: &mut LearningEraseState<'_>, scope: &str, member: u64, now: u64) -> LearningEraseReport` receives mutable references to canonical pending/reaction/social/style stores, plus optional continuity and engagement stores as they land. `reset_scope(state: &mut LearningEraseState<'_>, scope: &str, confirmed_manager: bool, now: u64) -> Result<LearningEraseReport, WorkError>` refuses unconfirmed/nonmanager callers and preserves canonical facts. LearningEraseState is owned in erasure.rs; runtime alone builds it under documented lock order. Tombstones cover300seconds after erasure, reject late attribution, and persist with the removal transaction. Do not claim individual aggregate-weight unlearning.

Task5 creates `src/brain/quality_evaluation.rs` and registers it in `src/brain/mod.rs`. Fixture schema1 has100 distinct cases,20 each of supported, unsupported, contradictory-source, correction and empty-retrieval. Each case has id, class, supplied source records `{id, revision, current, text}`, proposed claim `{text, cited_source_ids}`, and human-curated expected `Accept|Abstain` plus rationale. Fixture text is synthetic. `evaluate_case(case: &QualityCase) -> QualityDecision` calls the explicitly bounded lexical and structural checks defined below on only supplied records; no provider/network. `evaluate_corpus(cases: &[QualityCase]) -> QualityReport` counts TP/FP/TN/FN against curated labels, separately by class, rejecting missing/duplicate IDs and unknown source references. Acceptance on the frozen synthetic corpus requires zero false-positive accepted claims against its curated labels; this is not a general semantic-support guarantee; false negatives are reported, not hidden. This proves deterministic grounding policy, not provider answer quality. A separate operator-reviewed fixed-provider run on the same corpus records model outputs and manually adjudicated support; required before tuning, with no automatic model judge or confidence claim. Add fixtures for an invented source, stale source and contradictory source and assert Abstain. Own tests live in quality_evaluation.rs, fixtures only in tests/. Record report alongside source identity; DQN seed/import tests are additional checks, not the evaluator.


Task5 evaluator's exact implementation: construct `Grounding::from_sources` from current cited source records only; call `grounding::check(&case.claim.text, &grounding)`. A claim is Accept only if citations exist, are current, have no fixture-declared contradiction, and Verdict::is_grounded is true; otherwise Abstain. Correction cases also pass through evaluate_correction and must match their curated expected repair/ignore label. This is a lexical/specificity policy check with explicitly curated contradiction labels, not semantic truth inference. QualityCase fields include `contradictory: bool`, optional correction facts and `expected_correction: Option<Repair|Ignore>`. All boolean labels are synthetic fixture inputs, never inferred model fact edits. Human adjudication remains required for provider answer quality.

