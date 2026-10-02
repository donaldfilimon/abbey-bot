# MLAI Appropriate Initiative and Follow-ups Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Deliver useful explicitly opted-in task follow-ups with inspectable suppression and no repeated or cross-scope outreach.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Learning attribution/erasure and text typed delivery certainty first; continuity is optional input, never implicit outreach authorization.

## Current evidence

WorkStore already provides automation, destinations, quota/dedupe, Attempting/Sent/ReviewRequired receipts and interrupted-delivery recovery. One owner-directed engagement pass occurred; it is not indefinite DM authorization. Only MLAI was observed opted into unsolicited guild engagement.

## Architecture

Extend existing Work scheduling/delivery authority instead of adding a second scheduler or receipt store. Pure eligibility explains why an optional follow-up is allowed/refused; runtime rechecks access and owns the observed delivery.

## Requirements

INITIATIVE-R1: Optional follow-ups require explicit user/scope opt-in, an exact destination, source task/revision and expiry. A guild unsolicited choice never authorizes proactive member DMs or cross-server invitations.
INITIATIVE-R2: Maximum one optional task follow-up per task revision; personalized daily/weekly limits come exclusively from approved EngagementStore MemberPolicy; guild delivery also respects existing cooldown/hourly budget and quiet mode, taking the lower applicable limit.
INITIATIVE-R3: Reuse WorkDestination validation and audience intersection; recheck current installation, destination access, project membership and recipient opt-out before reservation and immediately before send.
INITIATIVE-R4: Reuse approved EngagementStore CandidateState Reserved, Sent, ReviewRequired for optional proactive follow-ups; required Work reminders retain DeliveryState Attempting, Sent, ReviewRequired. Persist reservation before send; confirmed native message ID settles Sent. Interrupted or PossiblySent work becomes ReviewRequired and is never blindly replayed.
INITIATIVE-R5: Closed eligibility reasons: Allowed, Disabled, Quiet, OptedOut, StaleTask, Expired, AccessDenied, Budget, Cooldown, AlreadyAttempted, ActivityUnavailable. Render aggregate/private explanations without exposing another user.
INITIATIVE-R6: Activity invitations require the accepted new public iframe/version; an old presence label or local preview is not eligible. No mass mentions, repeated cold DMs or new guild installations.
INITIATIVE-R7: Follow-up generation is read-only and source-bound; it cannot execute tools or persist inferred facts. Completed/cancelled tasks and expired cards remove eligibility.
INITIATIVE-R8: Persist engagement preferences and erasure through EngagementStore under WorkStore canonical schema/defaults. Recipient block/403 becomes a suppressed destination, not provider failure. Success is useful response/stop-rate evidence, never merely more messages.

## Interfaces and data ownership

Proposed `FollowUpFacts` holds explicit policy/access/task/revision/expiry/budget facts; `evaluate_follow_up(&FollowUpFacts, now) -> FollowUpDecision` is pure with the listed reason codes. `FollowUpIntent { scope: WorkScope, task: WorkContentRef, destination: WorkDestination, expires_at: u64 }` feeds the approved EngagementStore reservation and CandidateState lifecycle; no independent delivery ledger.

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

- Crash after Discord accepted send but before receipt publication cannot duplicate it. Test owner: implementation Task 2.
- One guild conversation does not authorize another guild or proactive DM. Test owner: implementation Task 1.
- Opt-out or permission loss after reservation prevents send. Test owner: implementation Task 2.
- Completed task or unavailable Activity suppresses a stale invitation. Test owner: implementation Task 1.
- Two simultaneous scheduler ticks share one reservation and daily quota. Test owner: implementation Task 2.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-initiative.md](../plans/2026-10-01-mlai-initiative.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.

## Approved engagement integration (normative)

Depends on [member engagement design](../specs/2026-10-01-member-engagement-design.md) and [plan](../plans/2026-10-01-member-engagement.md), Tasks1–4, before Initiative Task1/2 execution. Those operator-approved member controls are the only authority for optional personalized outreach: explicit daily1–4, IANA timezone, optional weekly1–28 capped at daily*7, default22–08 quiet hours, global/scoped stop, exact origin/private preference. No second quota, scheduler, preference store or delivery ledger. Existing explicitly configured Work reminders retain their current behavior.

Task1 owns `evaluate_follow_up(facts: &FollowUpFacts, now: u64) -> FollowUpDecision` and `propose_follow_up(store: &mut EngagementStore, intent: FollowUpIntent, source: SourceRef, member: u64, now: u64) -> Result<Option<u64>, WorkError>`. FollowUpFacts contains authorized:bool, source_current:bool, completed:bool, expires_at:u64, activity_ready:bool, and current member-policy decision. Require a human-origin SourceRef and existing task WorkContentRef; refuse invented source IDs. Use a deterministic dedupe key derived from scope/task/revision and never create a fake Work task. Task2 consumes the approved `reserve`, `validate_reserved`, `settle`, `recover_reserved` signatures verbatim; runtime delegates optional delivery to AppState::deliver_engagement, not WorkBatch. Extend EngagementStore Candidate provenance with `work_ref: Option<WorkContentRef>` in Task1 and its validated serde-default migration; existing conversation candidates use None. Completed/changed/erased Work references cancel candidates before send. Task2 owns that integration in src/runtime/engagement_delivery.rs and src/engagement/lifecycle.rs, preserving the approved single-flight Tick::Work owner.

Task3's private inspection reads existing /engage receipts; no duplicate stop command. Concurrent task follow-up and conversation follow-up consume the same global member charge ledger. Add a cross-domain two-guild-plus-DM budget test, source revision cancellation test, stop-after-reservation test and restart Reserved→ReviewRequired test. Activity readiness comes only from the approved operator receipt; voice invitations never grant listening consent.

