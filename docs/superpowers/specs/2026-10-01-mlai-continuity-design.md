# MLAI Opt-in Conversation Continuity Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Resume user-confirmed work across restart without persisting raw conversation or widening access.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Learning erasure contract first; reuse existing Work projects/decisions and immutable WorkContentRef provenance.

## Current evidence

Engine transcripts are process-local and bounded to 20 turns/6000 characters. WorkStore already owns tasks, decisions, access, provenance and automation. No durable continuity card is claimed.

## Architecture

Store a small user-confirmed continuity card in canonical state, referencing existing native Work content. Render it only after fresh scope authorization. Generated suggestions stay transient until explicit confirmation; do not create a second project/decision database.

## Requirements

CONTINUITY-R1: One card per exact WorkScope, at most 256 cards total. Each card has schema_version=1, owner/authorized scope, revision>=1, at most eight WorkContentRef references, at most 1600 UTF-8 bytes of user-confirmed card text, and seven-day expiry.
CONTINUITY-R2: Card content is current task, confirmed decisions and unresolved question; generated text is a proposal until the member explicitly confirms the exact immutable proposal ID/text against its base card revision. No inferred personal profile or raw transcript is automatically stored.
CONTINUITY-R3: Use existing WorkScope/WorkAccess, projects/decisions and immutable WorkContentRef. Do not duplicate task/decision state or activate unimplemented Work recall admission accidentally.
CONTINUITY-R4: Before every context inclusion recheck current actor/room/project access. Permission loss, removed membership, expiry, missing/revised source or user clear yields no card in the prompt.
CONTINUITY-R5: DM cards are individual-owner-only and never enter guild or other-user prompts. Team card confirmation requires existing Work manage authority; a viewer cannot overwrite another scope.
CONTINUITY-R6: Persist card changes atomically through canonical state and episode-gate write admission where covered. Refused queued writes are dropped, not retried; generated proposals are never persisted on shutdown.
CONTINUITY-R7: Card clear and member erasure remove canonical and cached projections; unrelated cards remain. Prompt contribution is capped at 1600 bytes and consists only of authorized confirmed card/source data.
CONTINUITY-R8: New /work continuity show, propose, confirm, clear surfaces are private, deferred and catalog-guarded. Show expiry/revision/source references and read actual rendered copy before release.

## Interfaces and data ownership

Proposed `ContinuityCard { schema_version: u8, scope: WorkScope, revision: u64, confirmed_text: String, source_refs: BTreeSet<WorkContentRef>, expires_at: u64 }`. `ContinuityStore::confirm` as pinned in the reviewed exact confirmation contract below and `context(scope, access, source_revisions, now) -> Option<AuthorizedContinuity>` are pure; `AuthorizedContinuity` has private construction and bounded prompt text. Runtime owns fresh access facts and atomic publication.

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

- A generated proposal is never confirmed by a model/tool or by stale click. Test owner: implementation Task 1.
- Permission revoked between preview and confirmation/context prevents inclusion. Test owner: implementation Task 2.
- Restart restores only unexpired exact-scope confirmed data. Test owner: implementation Task 3.
- DM owner changes or guild context cannot read a different user card. Test owner: implementation Task 2.
- Clear/erasure survives restart and projection rebuild without resurrection. Test owner: implementation Task 3.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-continuity.md](../plans/2026-10-01-mlai-continuity.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.

## Reviewed exact confirmation contract

Task 1 owns `ContinuityProposal { id: ProposalId, actor: u64, scope: WorkScope, base_revision: u64, presented_text: String, source_refs: BTreeSet<WorkContentRef>, expires_at: u64 }` in `src/work/continuity.rs`. `ProposalId { boot_nonce: [u8;16], sequence: u64 }` combines an infrastructure-generated fresh OS-random128-bit boot nonce with a monotonically checked session counter. Control encoding includes both; lookup checks both, and counter overflow refuses admission. IDs cannot intentionally be reused across boots; pending proposals remain transient, capped at256, expire after300seconds, and disappear at restart. Confirmation looks up the immutable presented text by ID; the client never submits replacement text. Thus no digest or production dependency is needed. A private human interaction is the only confirmation entry; model tools cannot construct the shell's confirmation authority.

`ContinuityStore::confirm(&mut self, proposal: &ContinuityProposal, actor: u64, access: &WorkAccess, current_source_refs: &BTreeSet<WorkContentRef>, now: u64) -> Result<ContinuityCard, WorkError>` verifies actor, exact scope, expiry, current source revisions, and stored card revision against base_revision. Task 2 owns the transient registry in `src/commands_work/continuity.rs` and resolves the control's proposal ID before calling confirm. Consume the proposal once; a different proposal against the same base revision is stale after the first succeeds. `context(&self, scope: &WorkScope, access: &WorkAccess, current_source_refs: &BTreeSet<WorkContentRef>, now: u64) -> Option<AuthorizedContinuity>` returns a private-constructor `AuthorizedContinuity { text: String }` only after all checks.

Add `two_proposals_same_base_revision_bind_exact_text`, `other_actor_control_denied`, `proposal_expired_or_missing_after_restart_denied`, and `changed_source_between_preview_and_click_denied` to Task1/2 source tests. Use two different texts and proposal IDs against base_revision0; confirming A stores only A, and confirming B afterward yields Stale. An old control never resolves to a newer proposal. Restart test creates a new proposal with the same numeric sequence under a different boot nonce, then clicks the old control and asserts denial without changing the new proposal.

