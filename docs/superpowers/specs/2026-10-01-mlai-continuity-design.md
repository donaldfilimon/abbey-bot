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
