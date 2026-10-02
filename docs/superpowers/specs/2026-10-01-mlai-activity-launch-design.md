# MLAI Activity Production Launch Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Launch the new shared Court inside Discord with honest recovery and bounded anonymous room state.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Begin after release-baseline inventory; hosting and Portal actions require their own operator decisions.

## Current evidence

The local Node room server and client recovery pass three tests. The managed local preview serves the corrected client. Public HTTPS, the new iframe and two-client Discord acceptance are unverified. Existing Pages constants are static hosting, not proof of a shared API.

## Architecture

Keep one authoritative Node Court service behind one HTTPS origin. Serve frontend and API together; Discord instance rooms and explicit browser preview rooms are separate. Anonymous browser ballots are not verified member votes. No OAuth or bot credential is introduced into this game.

## Requirements

ACTIVITY-LAUNCH-R1: Protocol v2 adds protocol=2, an opaque room-incarnation epoch of 16–64 ASCII letters/digits/hyphen/underscore, and nonnegative safe-integer revision. Snapshot retains round, yes, no and vote; total ballots must be at most 100.
ACTIVITY-LAUNCH-R2: Read bootstraps epoch. Vote/next require the current epoch and round; stale mutations return 409 without mutation. Every successful mutation increments revision. Next remains at least 3,000 ms apart.
ACTIVITY-LAUNCH-R3: Keep 256 rooms, 100 browser players per room, 1,024-byte JSON request limit, 30-minute inactive-room expiry and 5-second request timeout. Inject clock and newEpoch factory in tests; runtime owns fresh room-incarnation creation.
ACTIVITY-LAUNCH-R4: Client rejects malformed snapshots and older revisions in the same epoch. Capture a client generation for each request; adopting a changed backend epoch invalidates older in-flight responses. Epoch change shows room restarted and adopts server state; it never replays solo votes.
ACTIVITY-LAUNCH-R5: Poll every 2,500 ms while visible; avoid overlapping polls. Hidden tabs pause routine polls and refresh on visibility return. Explicit votes and next actions are disabled only while their own request is pending and restored in finally.
ACTIVITY-LAUNCH-R6: Use the already-tested Node implementation on a Node-capable HTTPS host. A static-only host cannot satisfy the API contract. Before publication record the chosen existing host, process owner, health URL and routing method in an operator-reviewed release receipt. Sites requires a separate Worker/state adaptation design if chosen instead.
ACTIVITY-LAUNCH-R7: No client secret, bot token, chat content, raw participant identifier or voice capture. Show anonymous browser ballots in copy. Public ingress must enforce finite body/connection/time limits and avoid logging secrets or room/player query values.
ACTIVITY-LAUNCH-R8: Acceptance requires the new Court iframe, Discord ready handshake, two clients sharing one instance, two isolated instances, disconnection, malformed response, stale write and backend restart. Presence and the old shell are insufficient.

## Interfaces and data ownership

`CourtSnapshotV2 = { protocol: 2, epoch: string, revision: number, round: number, yes: number, no: number, vote: null | "yes" | "no" }`. `createCourtServer({ now, newEpoch })` injects authority identity. `validateSnapshot(value): CourtSnapshotV2 | null` and `adoptSnapshot(snapshot, requestGeneration): boolean` are client-only validation/ordering seams. Requests carry epoch and round on mutations; read carries neither authority claim.

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

- Late response from an older backend epoch must not replace recovered state. Test owner: implementation Task 2.
- Ordinary clients in separate Discord instances using their own derived keys must have distinct ballot state. Test owner: implementation Task 3.
- Restart after a vote must show reset and must not silently resubmit it. Test owner: implementation Task 2.
- Request or player capacity exhaustion must yield an honest finite failure. Test owner: implementation Task 1.
- A static frontend loading successfully must not be labeled a working shared Activity. Test owner: implementation Task 4.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-activity-launch.md](../plans/2026-10-01-mlai-activity-launch.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.

## Reviewed authority and expiry contract

`epoch` identifies a room incarnation, including backend boot identity. Replace the process-only injection with `createCourtServer({ now, newEpoch })`, where infrastructure supplies `newEpoch(): string`; call it on each room creation, including recreation after 30-minute inactivity. It must not reuse an epoch within the process or across boot identities. Room revision starts at zero only with a fresh epoch. A read may adopt a fresh epoch and increment the client's request generation; older outstanding requests cannot adopt afterward. A stale mutation receives 409 and triggers a new read without resending its vote.

Task 1 owns `expired_room_gets_new_epoch`: create room, vote to revision1, advance injected clock beyond1800000ms, read; assert fresh epoch/revision0/empty totals and old-epoch mutation409 unchanged. Task 2 owns `late_pre_expiry_response_cannot_restore_room`: hold an old POST read response, recover expired room, release held response; assert generation rejects it and no automatic vote replay.

Room keys derived from Discord instance IDs provide routing namespacing only. This intentionally anonymous public API accepts client-chosen room and player keys; knowledgeable clients can target another room, impersonate a ballot key or create multiple players. It proves browser-ballot consistency, not authenticated membership or one vote per Discord member. Store no private content or verified Discord identities. Authentication would require a separately reviewed design. Two-instance witnesses assert separation only for ordinary clients using their own instance keys.

