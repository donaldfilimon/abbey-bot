# MLAI Activity Production Launch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed; planning complete does not mean execution approved.

**Goal:** Launch the new shared Court inside Discord with honest recovery and bounded anonymous room state.

**Architecture:** Keep one authoritative Node Court service behind one HTTPS origin. Serve frontend and API together; Discord instance rooms and explicit browser preview rooms are separate. Anonymous browser ballots are not verified member votes. No OAuth or bot credential is introduced into this game.

**Tech Stack:** Existing Rust 1.98.0/edition2024, Serenity0.12/Poise, existing Python deployment gates; dependency-free Node/browser Court where relevant. No new production dependency approved.

**Spec:** [2026-10-01-mlai-activity-launch-design.md](../specs/2026-10-01-mlai-activity-launch-design.md); read it and the parent design before execution.

**Dependencies:** Begin after release-baseline inventory; hosting and Portal actions require their own operator decisions.

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

- Late response from an older backend epoch must not replace recovered state — pinned in Task 2.
- Ordinary clients in separate Discord instances using their own derived keys must have distinct ballot state — pinned in Task 3.
- Restart after a vote must show reset and must not silently resubmit it — pinned in Task 2.
- Request or player capacity exhaustion must yield an honest finite failure — pinned in Task 1.
- A static frontend loading successfully must not be labeled a working shared Activity — pinned in Task 4.

## File ownership and task boundaries

The task file lists are owned scopes, not permission to revert surrounding changes. Workers are not alone. Run tasks sequentially; reviewer can reject one deliverable independently. Existing-file names were checked during planning. Create paths are Proposed new modules, which require explicit parent mod registration. Unit tests live in src or inline; tests/ holds fixtures only.

---

### Task 1: Version and bound authoritative room state

**Files:**
- Modify: `activity/server/court.mjs`
- Modify: `activity/server/court.test.mjs`

**Interfaces:** Produces CourtSnapshotV2 and injected createCourtServer({now,newEpoch}); consumes existing anonymous room/player keys.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `v2_stale_epoch_and_round_do_not_mutate` pins:

```text
assert status == 409; assert snapshot == before
```

  Test `v2_capacity_and_body_limits` pins:

```text
assert rooms <= 256; assert players <= 100; assert 1025-byte request status == 413
```

  Test `v2_next_and_revision` pins:

```text
assert next at 2999ms fails; assert accepted mutation revision == prior + 1
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Add v2 fields and validate all mutations before touching votes/round. Keep TTL and capacity behavior explicit. Add request-timeout/oversized-body coverage to the real HTTP harness.
- [ ] **Step 4: Run focused verification:** `npm --prefix activity test`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 2: Make client recovery ordered and restart-aware

**Files:**
- Modify: `activity/court.js`
- Modify: `activity/server/court-client.test.mjs`
- Modify: `activity/README.md`

**Interfaces:** Consumes Task 1 snapshot. Produces validateSnapshot and adoptSnapshot, plus one in-flight poll and client-generation guard.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `late_old_epoch_cannot_replace_new_epoch` pins:

```text
assert current.epoch == newEpoch; assert soloUploadCount == 0
```

  Test `restart_resets_authoritatively_without_replay` pins:

```text
assert restartNotice == true; assert current.round == server.round
```

  Test `poll_visibility_and_finally` pins:

```text
assert overlappingPolls == 0; assert controlsEnabledAfterFailure == true
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Retain tested solo fallback. Fence all asynchronous replies by captured generation and revision. Pause routine hidden-tab polls, refresh on return, and print/read restart/unavailable copy.
- [ ] **Step 4: Run focused verification:** `npm --prefix activity test`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 3: Bind game rooms to embedded instances

**Files:**
- Modify: `activity/app.js`
- Modify: `activity/court.js`
- Modify: `activity/index.html`
- Modify: `activity/server/court-client.test.mjs`

**Interfaces:** Consumes validated Discord ready/context state; produces deriveCourtRoom({instanceId,previewRoom}): string | null. Embedded mode requires instanceId; explicit browser mode requires a valid previewRoom.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `two_instances_never_share_room` pins:

```text
assert roomA != roomB; assert votesInB == 0
```

  Test `missing_instance_does_not_invent_shared_context` pins:

```text
assert embeddedWithoutInstance.shared == false
```

  Test `anonymous_copy_does_not_claim_member_identity` pins:

```text
assert displayedLabel == "Anonymous browser ballots"
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Use instance-bound room derivation without guild/DM content. Validate Discord message origin/source through the existing ready bridge. Preserve plain-browser preview.
- [ ] **Step 4: Run focused verification:** `npm --prefix activity test`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

### Task 4: Prepare HTTPS publication and actual iframe acceptance

**Files:**
- Modify: `activity/README.md`
- Modify: `docs/activities.md`
- Modify: `docs/MLAI-LIVE-ACCEPTANCE.md`

**Interfaces:** Consumes Tasks 1–3. Produces operator-reviewed host/routing receipt and actual iframe acceptance records; no new host project is created by planning.

- [ ] **Step 1: Write failing regressions in the owning inline/src test module or real HTTP fixture.**

  Test `publication_receipt_distinguishes_static_and_shared` pins:

```text
assert publicFrontendReachable && publicPostApiReachable
```

  Test `discord_two_client_acceptance` pins:

```text
assert A.epoch == B.epoch && A.round == B.round && A.revision == B.revision
assert A.yes == B.yes && A.no == B.no; assert B.vote == null until B votes
assert separateInstanceVotes == 0 for clients using their own instance-derived room keys
```

- [ ] **Step 2: Run the focused command below and confirm the named new regression fails for the intended invariant, not compile/environment noise.** For operator-only tasks, use the receipt/witness assertions as acceptance checks; do not manufacture a failing unit test or perform a live action without its authorization.
- [ ] **Step 3: Implement the pinned interface/behavior:** Inventory connected/configured hosting without changing it. Prepare an exact Node-compatible release, finite ingress configuration and rollback procedure. Obtain publication authorization, then human Portal mapping/iframe witness. If no host is authorized, report that specific blocked release step without treating local proof as publication.
- [ ] **Step 4: Run focused verification:** `python3 scripts/check-pages-liquid.py; npm --prefix activity test; ./check.sh (run separately, preserving each exit status)`. Semicolon-separated entries in this documentation mean separate tool calls; preserve each exit status. Expected: all selected tests pass, nonzero count; operator rows require actual receipts, not command success alone.
- [ ] **Step 5: Review the complete owned diff, print/read changed user-visible text, run applicable docs/Activity checks, and obtain independent review.** Keep the interface/spec contract explicit in the review receipt.
- [ ] **Step 6: Run `./check.sh` once this independently reviewable source deliverable is stable; record terminal exit, counts and source identity.** Do not duplicate a still-running Cargo/gate handle. For docs-only/operator records use Liquid/diff checks and the existing applicable source evidence instead of re-running an unchanged build.
- [ ] **Step 7: Record Current/Partial/Blocked evidence and unresolved proof layers in `docs/MLAI-LIVE-ACCEPTANCE.md`.** No commit/push/PR in this step; those require separate user instruction.

## Self-review and execution handoff

Check requirement coverage, exact interfaces, pinned values, file ownership, failure/test mapping, nonzero test filters, source versus installed/live claims, and privacy. Fix ambiguity before execution. Parent roadmap records dependency order and coverage.

Recommended execution: one implementing subagent followed by a fresh reviewer for each task, then a whole-program acceptance audit; serialized runtime writers. The user reviews the package and selects/confirms execution method before implementation. No skill worktree/commit default overrides the canonical-checkout/no-commit instructions.

## Reviewed authority and expiry contract

`epoch` identifies a room incarnation, including backend boot identity. Replace the process-only injection with `createCourtServer({ now, newEpoch })`, where infrastructure supplies `newEpoch(): string`; call it on each room creation, including recreation after 30-minute inactivity. It must not reuse an epoch within the process or across boot identities. Room revision starts at zero only with a fresh epoch. A read may adopt a fresh epoch and increment the client's request generation; older outstanding requests cannot adopt afterward. A stale mutation receives 409 and triggers a new read without resending its vote.

Task 1 owns `expired_room_gets_new_epoch`: create room, vote to revision1, advance injected clock beyond1800000ms, read; assert fresh epoch/revision0/empty totals and old-epoch mutation409 unchanged. Task 2 owns `late_pre_expiry_response_cannot_restore_room`: hold an old POST read response, recover expired room, release held response; assert generation rejects it and no automatic vote replay.

Room keys derived from Discord instance IDs provide routing namespacing only. This intentionally anonymous public API accepts client-chosen room and player keys; knowledgeable clients can target another room, impersonate a ballot key or create multiple players. It proves browser-ballot consistency, not authenticated membership or one vote per Discord member. Store no private content or verified Discord identities. Authentication would require a separately reviewed design. Two-instance witnesses assert separation only for ordinary clients using their own instance keys.

