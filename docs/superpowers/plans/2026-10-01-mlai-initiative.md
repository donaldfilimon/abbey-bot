# MLAI Appropriate Initiative and Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Execution authorized by Donald. Tasks1/2 and Task3 source qualified on 2026-10-03; Task3 human pilot remains Partial. See [source receipt](../../verification/2026-10-03-initiative-source.md).

**Goal:** Deliver useful explicitly opted-in task follow-ups with inspectable suppression and no repeated or cross-scope outreach.

**Architecture:** Extend existing Work scheduling/delivery authority instead of adding a second scheduler or receipt store. Pure eligibility explains why an optional follow-up is allowed/refused; runtime rechecks access and owns the observed delivery.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-initiative-design.md](../specs/2026-10-01-mlai-initiative-design.md); read it and the parent design before execution.

**Dependencies:** Learning attribution/erasure and text typed delivery certainty first; continuity is optional input, never implicit outreach authorization.

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

- Crash after Discord accepted send but before receipt publication cannot duplicate it — pinned in Task 2.
- One guild conversation does not authorize another guild or proactive DM — pinned in Task 1.
- Opt-out or permission loss after reservation prevents send — pinned in Task 2.
- Completed task or unavailable Activity suppresses a stale invitation — pinned in Task 1.
- Two simultaneous scheduler ticks share one reservation and daily quota — pinned in Task 2.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Define scoped optional-follow-up policy and preferences

**Files:**
- Create: `src/work/follow_up.rs`
- Create: `src/work/follow_up/tests.rs`
- Modify: `src/work.rs`
- Modify: `src/work/policy.rs`
- Modify: `src/commands_work/controls.rs`
- Modify: `src/engagement.rs` (created by approved Engagement Task1)
- Modify: `src/engagement/schedule.rs` (created by approved Engagement Task3)

**Interfaces:** Produces FollowUpFacts/Decision/Intent; consumes existing WorkScope/WorkDestination/preferences and exact source revision.

- [x] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `guild_opt_in_is_not_dm_or_cross_guild_opt_in` pins:

```text
assert decision == OptedOut for unmatched destination
```

  Test `stale_task_and_unaccepted_activity_refuse` pins:

```text
assert decision in [StaleTask, ActivityUnavailable]
```

  Test `optional_limits_use_lower_budget` pins:

```text
assert sharedMemberCharges == 1; assert taskRevisionCount <= 1
```

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [x] **Step 3: Implement the pinned interface/behavior:** Consume existing approved /engage MemberPolicy opt-in, stop, timezone and destination controls; add no independent opt-in or recipient budget. Keep required configured Work reminders separate. Preserve existing reminder contracts and print/read opt-in, stop and suppression copy.
- [x] **Step 4: Run focused verification:** `cargo test --locked follow_up; cargo test --locked work::policy (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Reserve and settle through existing delivery ownership

**Files:**
- Modify: `src/engagement/lifecycle.rs` (created by approved Engagement Task3)
- Modify: `src/runtime/engagement_delivery.rs` (created by approved Engagement Task4)
- Modify: `src/runtime/engagement_delivery/tests.rs` (created by approved Engagement Task4)
- Modify: `src/gateway/engagement_delivery.rs` (created by approved Engagement Task4)
- Modify: `src/service/scheduler.rs`

**Interfaces:** Consumes FollowUpIntent, typed OutboundFailure and approved EngagementStore reservation/CandidateState; produces existing durable reservation/settlement records.

- [x] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `accepted_send_crash_is_review_required_not_replayed` pins:

```text
assert state == ReviewRequired; assert resendCount == 0
```

  Test `revocation_after_reservation_prevents_send` pins:

```text
assert sendCount == 0
```

  Test `parallel_ticks_reserve_once` pins:

```text
assert reservations == 1; assert quotaCharge == 1
```

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [x] **Step 3: Implement the pinned interface/behavior:** Delegate to EngagementStore::recover_reserved and AppState::deliver_engagement; reuse Work audience intersection only for validating the optional task reference, and perform late authorization through engagement transport. Do not change required Work reminder reservation paths. Keep root-retained joins and reservation-before-send ordering; classify blocked destinations without charging provider circuit.
- [x] **Step 4: Run focused verification:** `cargo test --locked engagement::schedule; cargo test --locked engagement_delivery (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Inspect and erase follow-up state; qualify usefulness

**Files:**
- Modify: `src/work/policy.rs`
- Modify: `src/commands_work/controls.rs`
- Modify: `src/engagement.rs` (created by approved Engagement Task1)
- Modify: `src/engagement/schedule.rs` (created by approved Engagement Task3)
- Modify: `src/runtime/engagement_delivery.rs` (created by approved Engagement Task4)
- Modify: `src/commands_engage.rs` (created by approved Engagement Task2)
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`

**Interfaces:** Consumes eligibility and delivery receipts; produces private reason/expiry/status views and learning erasure hook for member-linked preferences/receipts.

- [x] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `inspection_does_not_leak_other_scope` pins:

```text
assert otherRecipientVisible == false
```

  Test `erase_prevents_future_scheduled_send` pins:

```text
assert candidateAfterErase == None
```

  Test `usefulness_denominator_counts_stops_and_failures` pins:

```text
assert total == useful + stopped + failed + unanswered
```

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Show attempted/confirmed/review-required distinctions. Use a bounded operator-approved pilot of at most three willing recipients; no unsolicited expansion while measuring. Record replies only when observed and honor stop immediately.
- [x] **Step 4: Run focused verification:** `cargo test --locked follow_up; cargo test --locked work (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.

## Approved engagement integration (normative)

Depends on [member engagement design](../specs/2026-10-01-member-engagement-design.md) and [plan](../plans/2026-10-01-member-engagement.md), Tasks1–4, before Initiative Task1/2 execution. Those operator-approved member controls are the only authority for optional personalized outreach: explicit daily1–4, IANA timezone, optional weekly1–28 capped at daily*7, default22–08 quiet hours, global/scoped stop, exact origin/private preference. No second quota, scheduler, preference store or delivery ledger. Existing explicitly configured Work reminders retain their current behavior.

Task1 owns `evaluate_follow_up(facts: &FollowUpFacts, now: u64) -> FollowUpDecision` and `propose_follow_up(store: &mut EngagementStore, intent: FollowUpIntent, source: SourceRef, member: u64, now: u64) -> Result<Option<u64>, WorkError>`. FollowUpFacts contains authorized:bool, source_current:bool, completed:bool, expires_at:u64, activity_ready:bool, and current member-policy decision. Require a human-origin SourceRef and existing task WorkContentRef; refuse invented source IDs. Use a deterministic dedupe key derived from scope/task/revision and never create a fake Work task. Task2 consumes the approved `reserve`, `validate_reserved`, `settle`, `recover_reserved` signatures verbatim; runtime delegates optional delivery to AppState::deliver_engagement, not WorkBatch. Extend EngagementStore Candidate provenance with `work_ref: Option<WorkContentRef>` in Task1 and its validated serde-default migration; existing conversation candidates use None. Completed/changed/erased Work references cancel candidates before send. Task2 owns that integration in src/runtime/engagement_delivery.rs and src/engagement/lifecycle.rs, preserving the approved single-flight Tick::Work owner.

Task3's private inspection reads existing /engage receipts; no duplicate stop command. Concurrent task follow-up and conversation follow-up consume the same global member charge ledger. Add a cross-domain two-guild-plus-DM budget test, source revision cancellation test, stop-after-reservation test and restart Reserved→ReviewRequired test. Activity readiness comes only from the approved operator receipt; voice invitations never grant listening consent.

## 2026-10-03 source execution addendum

Native admission/due-time and additive migration are pinned in
[the reviewed addendum](../specs/2026-10-03-initiative-native-admission.md).
Root is the sole source writer; subagents draft and independently review outside
the shared source. Additional owned seams: commands_engage/follow_up and existing
catalog; runtime/engagement_follow_up and delivery child modules; gateway Work
proof adapter; Engagement loading/validation/erasure and child regressions.
No incompatible existing public contract or production dependency is introduced.
Task3 source inspection/erasure can qualify independently; its actual usefulness
pilot remains open until separately authorized willing participants are observed.
