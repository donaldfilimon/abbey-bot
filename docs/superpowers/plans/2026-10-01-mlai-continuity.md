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

- [x] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

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

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [x] **Step 3: Implement the pinned interface/behavior:** Keep proposals transient; require exact expected revision, caller authority and current source revisions. Inject time and use validated UTF-8 byte limits; reject oversize instead of silent truncation.
- [x] **Step 4: Run focused verification:** `cargo test --locked continuity`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Integrate fresh authorization and private command flows

**Files:**
- Create: `src/commands_work/continuity.rs`
- Modify: `src/commands_work.rs`
- Modify: `src/runtime/memory_service.rs`
- Modify: `src/pipeline.rs`
- Modify: `src/command_catalog.rs`
- Modify: `src/command_catalog/tests.rs`

**Interfaces:** Consumes Task 1 APIs; produces AuthorizedContinuity passed into prompt preparation after current access recheck.

- [x] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

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

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [x] **Step 3: Implement the pinned interface/behavior:** Use existing REST access seam and Work management rules. Bind private show/propose/confirm/clear commands and include bounded card only in the exact prepared scope. Update catalog/help size coverage.
- [x] **Step 4: Run focused verification:** `cargo test --locked continuity; cargo test --locked command_catalog (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Persist and erase continuity without transcript retention

**Files:**
- Modify: `src/persist.rs`
- Modify: `src/persist/tests.rs`
- Modify: `src/runtime/memory_service.rs`
- Modify: `src/memory_gate.rs`

**Interfaces:** Consumes confirmed cards only and learning erasure integration; produces version/default migration and no-resurrection cleanup.

- [x] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

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

- [x] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [x] **Step 3: Implement the pinned interface/behavior:** Add serde-default canonical field and validated load bounds. Preserve existing admitted checkpoints, remove cache alongside canonical card, and exclude human voice transcript content absolutely.
- [x] **Step 4: Run focused verification:** `cargo test --locked continuity; cargo test --locked persist; cargo test --locked memory_gate (separate commands)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [x] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [x] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [x] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.

## Reviewed exact confirmation contract

Task 1 owns `ContinuityProposal { id: ProposalId, actor: u64, scope: WorkScope, base_revision: u64, presented_text: String, source_refs: BTreeSet<WorkContentRef>, expires_at: u64 }` in `src/work/continuity.rs`. `ProposalId { boot_nonce: [u8;16], sequence: u64 }` combines an infrastructure-generated fresh OS-random128-bit boot nonce with a monotonically checked session counter. Control encoding includes both; lookup checks both, and counter overflow refuses admission. IDs cannot intentionally be reused across boots; pending proposals remain transient, capped at256, expire after300seconds, and disappear at restart. Confirmation looks up the immutable presented text by ID; the client never submits replacement text. Thus no digest or production dependency is needed. A private human interaction is the only confirmation entry; model tools cannot construct the shell's confirmation authority.

`ContinuityStore::confirm(&mut self, proposal: &ContinuityProposal, actor: u64, access: &WorkAccess, current_source_refs: &BTreeSet<WorkContentRef>, now: u64) -> Result<ContinuityCard, WorkError>` verifies actor, exact scope, expiry, current source revisions, and stored card revision against base_revision. Task 2 owns the transient registry in `src/commands_work/continuity.rs` and resolves the control's proposal ID before calling confirm. Consume the proposal once; a different proposal against the same base revision is stale after the first succeeds. `context(&self, scope: &WorkScope, access: &WorkAccess, current_source_refs: &BTreeSet<WorkContentRef>, now: u64) -> Option<AuthorizedContinuity>` returns a private-constructor `AuthorizedContinuity { text: String }` only after all checks.

Add `two_proposals_same_base_revision_bind_exact_text`, `other_actor_control_denied`, `proposal_expired_or_missing_after_restart_denied`, and `changed_source_between_preview_and_click_denied` to Task1/2 source tests. Use two different texts and proposal IDs against base_revision0; confirming A stores only A, and confirming B afterward yields Stale. An old control never resolves to a newer proposal. Restart test creates a new proposal with the same numeric sequence under a different boot nonce, then clicks the old control and asserts denial without changing the new proposal.



## 2026-10-03 execution clarification: canonical authority and erased ownership

The existing whole-program execution direction is recorded in tasks/goals.md;
the Proposed header above describes the planning snapshot. These reviewed
clarifications supersede the earlier raw confirm/context signatures before new
product code is introduced.

The new schema1card also requires `confirmed_by: u64`, copied from the validated
member that confirmed its exact immutable proposal. Reject zero and a personal
confirmer different from its owner. Replacement replaces all text/refs; no
proposal history is retained. Member erasure removes a card confirmed by that
member in the erased exact scope, or linked through a current native task owner,
assignee or decision author, while retaining unrelated cards and underlying Work
records. No continuity schema is previously Current, so unowned team text never
receives a migration default.

Pure confirm/context take a borrowed current canonical `&WorkStore` alongside
fresh `WorkAccess`; the earlier raw reference set is insufficient authority.
Use existing `WorkStore::scope_projects(scope, access, manager)`: all projects
in the exact scope must authorize, and an empty scope refuses. Confirm/clear
require existing Work manager membership; context/show require Work membership.
A delegated Work manager without MANAGE_GUILD is allowed; a guild administrator
absent Work membership is refused. Do not change WorkAccess.can_manage semantics.
Validate every selected Task/Decision against exact native kind, project, current
revision and scope; newly added unrelated records do not invalidate selected refs.
Missing, changed or unauthorized selected source excludes the entire context.

`ContinuityStore::confirm` consumes an opaque, nonserializable
`ResolvedConfirmation`, not a raw proposal or client-submitted text. The transient
registry resolves the exact nonce/sequence ID, actor, scope and expiry once; only
the private native-human command shell owns that registry. The pure confirmation
then repeats canonical authority, expiry, source and base-revision checks.
Task1 proves inert/unresolved proposals, exact-text CAS and consumed-grant refusal;
actual native-human-versus-model dispatch exclusion remains Task2 and cannot be
proven by an intent boolean. No model/tool confirmation route may be registered.

The NEW domain module may be registered under cfg(test) for Task1's focused and
full source gate while production consumers are absent. This is tested domain
coverage, not a live continuity feature. Task2 must remove that staging only when
real private command/runtime consumers exist; Task3 must add canonical/episode/
erasure integration. All three tasks and all R1–R8 requirements remain required.


## 2026-10-03 Task1 execution evidence and integration ruling

Task1 is complete as the reviewed **tested domain**, staged under cfg(test).
Twenty-four focused tests pass, after missing-behavior and actual guard REDs.
Strict required-WDBX gate session2967 exited0:2,114Rust passed/0failed/8ignored,
339Python cases,16publication scenarios,Swift12+16,Clippy and locked release.
Identical804-input manifests fingerprint
`f1a6e2ce96642ff1555592127805fce6315d0be05a51ef8328344c0c47e7a6a7`;
HEAD/index and external fixture were preserved. These plan/evidence corrections
follow that gate and do not inherit its whole-tree fingerprint.
See [domain receipt](../../verification/2026-10-03-continuity-domain.md).
Production continuity and R1–R8 completion remain Partial pendingTasks2/3.
New proposal input is ContinuityDraft, grouping scope/base revision/exact text/refs;
it changes no previously Current public API.

Task2's abbreviated owner list must also include the actual memory/engine prompt
and grounding path, generation guard and visible-output boundaries, private ask/
modal delivery, startup and a gateway fresh-access adapter. A helper without
these callers cannot satisfyR4. Missing fresh access provider and non-Discord
native events exclude continuity. No stores/registry lock spans REST or disk IO.

Use one process-local checked continuity generation to invalidate every pending
proposal and already-resolved grant across clear/member erase, before protective
live removal and again before admission/publication. Restart destroys transient
grants, so no new durable tombstone database is needed. Global invalidation can
cancel unrelated pending controls; unrelated confirmed cards remain. Exhaustion
refuses new admission instead of wrapping. Failed deletion retains protective
live fencing and reports failure; restart safety requires verified canonical
publication/readback.

Task2 private flow/prompt adapters are implemented and tested before final
registration; enable production commands only whenTask3's retained canonical,
covered episode-admission and erasure owner exists. The final enabled vertical
slice qualifiesTasks2/3 together; neither is complete until its full contracts
and tests pass. Generic commit_work's engagement-only fence is insufficient.
Task3's minimal persistence prerequisites may be implemented before registration;
record that dependency order in the ledger. Preserve existing Work authority,
read-only voice/unsolicited/summary policy and exact scope isolation.


## 2026-10-03 reviewed receipt recovery and retained access addendum

Receipt-bearing cards carry optional validated lower-case64hex episode_receipt
(defaultNone for legacy ungated cards). Summary/Durable admission commits the
schema, scope, confirmer, revision, exact text, sources and expiry, excluding
its returned receipt. A replacement supersedes the exact prior receipt.
Receipt-bearing expiry/source exclusion does not silently delete the canonical
receipt join: keep it inert until an admitted clear/erasure; legacy no-receipt
cards retain existing bounded load pruning. All retained card metadata counts
against the256-card canonical cap.

An appended forget followed by failed canonical publication cannot be rolled
back or safely re-proposed. Use the existing read-only ABI CLI contract:
`wdbx episode verify <guild-ref> <receipt> --json` with the configured endpoint,
token-file and optionalCA argv. No new dependency/database. Exact request argv
binds the digest; current CLI does not echo it. Strict typed string fields
found=true, exact guild_ref, event_kind=memory_candidate,
memory_forgotten=false/true and signature_status=valid/unsigned distinguish
Live/Forgotten; invalid, unknown_key, missing/malformed fields, duplicate
critical fields, trailing JSON or nonzero exit are Unknown. Unsigned preserves
the existingC0 policy; this adds no cryptographic maturity or payload-commitment
claim. Verify exposes no class/text commitment: creation and receipt ownership
remain the canonical runtime join.

Receipt-bearing context requires positiveLive verification on every fresh
admission/dispatch/output boundary, alongside native access and final canonical
card/epoch checks. Missing usable gate excludes; no downgrade to ungated
context. Remote liveness is a current observation, not an atomic remote lease.
Local canonical monitoring remains inexpensive and transport-free.

Explicit clear/member erasure reconciles each exact old receipt. Forgotten
permits local-only deletion; Live permits one explicitly requested forget;
Unknown preserves card/receipt, keeps this boot's protective fence and reports
incomplete. Reserve bounded process-local exact-target Appended memo capacity
before a proposal; retain positive digest/sequence through local write failure.
An exact same-boot retry uses local-only publication. Restart uses verification,
never blind replay or refusal-text inference. Key memo by full card fingerprint,
original receipt and gate identity; do not evict unresolved entries. Each admitted
card deletion is exactly read back before proceeding to another card. A later
failure is partial and does not roll back earlier external or local effects.

Reuse bounded retained ABI child execution: clear credential environment,
capstdout/stderr, use configured timeout/cancellation, actual kill+wait and
observed joins. Closed service admission starts no verify/proposal. Native
GET-only access separately has a5-second bound; retained provider cancellation
and the canonical monitor surround the fresh check after reservation. Recheck
local generation both before and after waiting for canonical state.

These are additive rules for the new, still-PartialTasks2/3. They revise no
previouslyCurrent public/persistence/CLI contract and authorize no live source
validation. Qualification remains pending the enabled private/canonical/
covered-gate/erasure vertical slice and its stable authoritative gate.


## 2026-10-03 reviewed mutation linearization

The retained continuity mutation owner first acquires the existing async
persistence_preparation serial. Waiting for that serial is not a destructive
transition. Then it advances the checked boot generation and fences exact target
scopes before gate or disk IO, retaining the serial through observed canonical
publication and exact readback. Every production clear/member-erasure epoch
mutation uses this serial. Confirmation checks its resolved grant generation
again after acquiring it. This prevents a clear from invalidating an immutable
FIFO write after submission and losing its Appended receipt. A failed deletion
keeps its scope fenced; unrelated failed fences cannot be cleared by success.


## 2026-10-03 reviewed incomplete publication recovery

Reserve bounded pending/orphan capacity and require configured canonical writer
before proposing. Appended candidates whose final fresh actor/manager/source/
proposal-expiry/generation checks fail remain bounded transient orphan targets,
never canonical cards. Explicit clear/member erase selects canonical and orphan
receipts; restart discards orphan plaintext and controls, leaving only the remote
content-free commitment. This is an incomplete operation, not a saved card.

An authorized exact-target Replace/Delete may stage a pending publication after
its required gate effect. Every canonical snapshot applies that target overlay
against the exact expected old card, preserving unrelated rows. A generic write
cannot clear its context fence. Explicit retained reconciliation under the same
serial submits a fresh complete snapshot, verifies exact bytes and validated load,
and installs only its owned target and lineage. Write-then-error may reconcile
observed canonical bytes but still reports the actual failed sink result. Pending
scope metadata blocks early lineage publication; mismatched readback keeps the
expected lineage so stale snapshots cannot bless unrelated disk data. Only the
relevant scope is reconciled; unrelated failed scopes remain fenced. A scope is
unfenced only when all canonical/pending/orphan targets in it are settled. No
refused or unknown candidate is automatically re-proposed.

Execution ruling (2026-10-03): native command Data owns one boot-nonce proposal registry. A proposal becomes confirmable only after its private preview send succeeds and its generation and expiry are rechecked. Failed/unobserved previews are discarded. All four leaves remain catalog-deferred, ephemeral, and fresh-authorized. Verification remains pending for the complete enabled flow.

2026-10-03 reviewed publication rule: pending overlays apply atomically to a cloned card store. CAS conflict refuses every canonical/projection write, never publishes a partial overlay. Generic tuple/final-snapshot contracts are preserved through snapshot-only, serde-skipped canonical_preparation_failed metadata checked by persist_canonical_owned before sink IO; direct personal candidates propagate the overlay error immediately before both canonical writes. This adds no persisted field, dependency, or external API. Member erase reconciles only exact-scope pending targets linked through expected or desired cards; unrelated targets retain their fences. Observed pending deletions contribute to the successful removal count.
