# Abbey chief of staff implementation

This execution plan records the user's four-stage implementation request from
2026-09-24. The user message is the binding product specification; this file
organizes the remaining work for subagent review and recovery.

## Global constraints

- Work in `/Users/donaldfilimon/dev/active/abbey-bot-wt-chief-of-staff` on
  `abbey/chief-of-staff`. Preserve the canonical checkout and all other
  worktrees. Never `git checkout` the shared repository.
- Follow `AGENTS.md`: pure decisions, Discord translation in `commands*` and
  `gateway`, supervisor-owned async work, append-only ledgers, no second gateway.
- Preserve persona, roleplay, consent, WDBX, episode, privacy, and Rust module
  size contracts. Keep production modules below 1,000 lines.
- A successful work mutation requires a committed canonical state file.
  Ambiguous external writes require review and cannot replay automatically.
- Team reads require explicit project membership and fresh Discord channel
  access. Shared GitHub visibility requires a manager-approved repository.
- Automation starts disabled; quiet hours 22:00–08:00; daily delivery ceiling
  four including briefing; Donald's initial timezone America/New_York; other
  users and team projects choose their timezone. Five attributable observations
  precede learned changes; silence does not count as negative feedback.
- Use classic Serenity 0.12.5 components. Channel changes are additive and
  previewed; no mass Member grants, role escalation, destructive restructuring,
  repository pushes/merges/workflow execution, or Developer Portal automation.
- Run focused tests and strict all-target Clippy for each changed source slice;
  run `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` on integrated stage
  candidates (Tasks 8, 10, 12, and final 13). Windows/Linux evidence only on available
  hosts. CI evidence must match exact SHA; zero-step billing-locked jobs do not
  count. Live acceptance needs selected GitHub App repositories, pilot channel,
  credentials, and fresh everyone-present voice consent.

## Approved replacement execution order — 2026-09-24 09:00 EDT

The user-approved “Abbey: Complete Chief-of-Staff Workflows and Verified Release” plan supersedes earlier milestone ordering. Preserve this program's task identities and append-only evidence; do not restart completed foundation/model tasks.

1. Resume the interrupted uncommitted recall coordinator from committed face721; finish exact-source tests, independent review and correction before consumers.
2. Complete authorized recall configuration/inspection/reindex/forget/deletion cleanup and real briefing consumption; measure a bounded 1,000-row query.
3. Complete Task7 retained scheduling/delivery and Task8 REST adapter, audience proof and reset/send barriers.
4. Complete Task9 native task/manager controls, classic interactions and actual natural-language preview/confirmation. The native full strict gate closes only after this production workflow is connected; moving the gate after Task9 prevents isolated models from being treated as delivered features.
5. Independently review the Task10 Discord design; implement bound requester-only approval, retained per-effect execution and all requested Discord catalog, close direct-handler/CLI bypasses, then strict gate.
6. Complete Task11 GitHub App scope grants/read polling and Task12 approved writes, then strict gate.
7. Freeze final schema; implement reviewed Task13 guarded decoding including early privacy rewrite, exact preservation contract and hash-bound sidecar/rollback refusal. Reconcile documentation/ledgers, review whole branch, strict-gate exact candidate and available platforms/CI, prepare recovery, deploy atomically and complete distinct personal/shared/provider/vision/consented-voice acceptance.

Reviewed contracts in this program's SDD workspace remain requirements: work-recall-design.md; task-9-interactions-design.md and final PASS; task-13-rollback-design-draft.md revision2 and PASS. task-10-discord-operations-design.md requires independent review. Scope defaults and exclusions remain as specified below and in the approved user plan. One implementation writer at a time; independent spec and quality review per slice; no test-only API counts as a delivered feature. Missing GitHub installation/credentials/repository grants, pilot channel/membership, platform hosts and human voice participation block only corresponding acceptance.

## Task 1: Consolidate reviewed foundation branches

Inventory seven worktrees and integrate completed provider, Darwin lock,
Gemma marker, docs, and formatting changes without disturbing active owners.
Review diffs and run the strict gate. Completed in commits up to `8650b24`.

## Task 2: Canonical scoped work records and commands

Implement typed personal/team projects, goals, tasks, decisions, policy,
preferences, receipts, authorized `/work` commands, durable acknowledgment,
legacy state loading, status/snooze controls, and regression tests. Completed
in `3953a47` and `84d7a07`; scheduled delivery and rich components remain in
later tasks.

## Task 3: Shared approval core

Implement typed exact external proposals with content digest, human approval,
expiry, fresh authority and target checks, in-flight persistence, duplicate
click protection, and interrupted-attempt review. Completed in `ea2f585`;
shell adapters remain in later tasks.

## Task 4: GitHub links and read cache

Finish the in-progress typed GitHub issue/PR links, manager repository
allowlist, personal/team isolation, stale snapshots, canonical migration,
briefing source links, tests, and conditional freshness policy. Task 11 owns
live pagination and rate-limit transport.
Keep GitHub source text inert. Own only
`src/work.rs`, `src/work/github.rs`, `src/work/registry.rs`, `src/work/tests.rs`,
GitHub-specific new modules/tests, and their direct docs. Run focused tests
and strict gate, self-review, then commit.

## Task 5: Durable scope policy and schedule model

Implement the pure policy foundation for notifications. Scope policy and quota
by `WorkScope`, shared across all projects in one personal workspace or team
channel. Team destination is its bound channel; personal destination is the
owner's selected DM. Automation is disabled by default. Enabling later must
record scope, destination, IANA timezone, quiet hours, and ceiling. Default
quiet hours are 22:00–08:00 local time and the ceiling is four proactive
deliveries per scope per local day, including one daily briefing. Donald's
initial personal timezone is `America/New_York`; other users and shared
projects must choose an IANA timezone. Add explicit optional `remind_at` to
tasks; `due_at` alone never schedules a send. Build a pure next-batch planner
with injected UTC time, DST-safe calendar conversion, quiet-hour delay, one
catch-up for missed runs, and durable scope/day/kind/item dedupe keys. Combine
pending work in one daily briefing where possible. Test spring/fall DST,
default ceiling, multiple projects in one scope, restart, and old-state loading.
Own `src/work.rs`, `src/work/policy.rs`, new pure scheduling modules/tests, and
timezone dependency/lock only. Do not send Discord messages.

## Task 6: Attributable learning and command controls

Build on Task 5. Implement deterministic scope-local preference evidence with
feedback actor and delivery attribution. At least five attributable observations
are required before a learned change; silence is not negative feedback.
Each learned dimension needs five relevant attributable observations; unrelated
feedback cannot unlock it, and corrections below that threshold clear it.
Repeated snoozes suggest optional timing, dismissals reduce optional
follow-ups, usefulness feedback adjusts briefing ranking. Explicit timing
overrides learned timing. Learning cannot change permissions, quiet hours,
ceilings, or deadlines. Expose private evidence inspection, correction, reset,
disable learning, opt-in automation configuration with a durable authorizing
principal for future access rechecks, and an explicit reminder
command that discloses the ceiling. Team feedback stays in team scope and never
imports personal evidence. Commands recheck current scope access, remain
ephemeral, and commit canonical state before success. Own pure learning module,
`src/commands_work.rs`, command catalog/tests, README and related tests.
Legacy unattributed evidence must not count toward learned changes. Reject
feedback for unknown/unfinished deliveries, forged actors, or revoked scope
access; duplicate feedback must not inflate evidence. Corrections/reset recompute
all derived settings, including presentation/ranking and optional follow-ups.
Manager controls govern shared policy and shared reset; personal controls never
change another user or team profile. Read cross-task-acceptance-notes.md for
identity/default and recall boundaries. Validate with focused work/command tests
and strict all-target Clippy; Task 8 runs the integrated stage gate.

## Task 7: Supervised durable delivery lifecycle

Build on Tasks 5–6. Add a fakeable outbound seam and a supervisor-owned work
scheduler operation. Include aggregated meaningful work changes alongside
briefings and explicit reminders, using durable revision/change coverage so
updates combine and restart cannot burst or repeat historical changes.
Optional-follow-up suppression affects optional change notifications, never
explicit reminders or the configured daily briefing. Seed legacy change
coverage conservatively; ordinary loading must not announce all old records. Before network send, recheck fresh access and membership,
current policy and dedupe key; reserve a durable Attempting receipt with
`AppState::commit_work`. Send only after that commit. Settle confirmed sends
durably as Sent; an ambiguous outcome or interrupted attempt becomes
ReviewRequired and is never replayed automatically. A crash after confirmed
send but before receipt settlement remains review-required. Missed runs
consolidate into one catch-up, not a burst. Shutdown closes admission, cancels
work, observes joins, then attempts final persistence. Test persistence failure
before send, duplicate ticks, concurrent policy edits, revoke, restart,
uncertain send, and shutdown during active delivery. Own runtime delivery,
scheduler/service classification, recovery and fake transport tests.
An admitted delivery must retain settlement capability after admission closes;
calling the public new-mutation admission path during draining cannot be the
only settlement path. Reuse the retained transaction implementation under the
already-owned operation, without holding a store/persistence mutex across
network awaits. Test actual supervisor cancellation and observed joins with
a held send/settlement, rather than only pure receipt-state assertions.

## Task 8: Discord delivery shell and integrated gate

Build on Task 7. Add a Serenity adapter at the `startup`/`commands*` shell
boundary that fetches current channel permissions and member facts over REST,
uses inert mentions, and sends only a supervisor-owned authorized delivery.
Wire it through the retained scheduler without a second gateway. Source tests
use fake transport and never send live messages. Render representative empty,
degraded, paused, denied, and successful text and read it for clarity. Run
focused tests and strict `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` on the
integrated candidate. Own the shell adapter and startup wiring.

Before Task8 completion, implement the reviewed work-recall-design.md in
bounded source-model, WDBX adapter, durable admission/coordinator, failure-matrix
and command-control slices, each independently reviewed. Consume the existing
Task7 no-proposal work-commit prerequisite; do not duplicate it. Actual briefing
construction must use authorized recall and feedback ranking, with canonical
records retaining authority.

Shared-channel sends additionally require the complete conservative structural
audience proof in shared-channel-audience-seam.md (raw current REST facts, owner
and administrator bypass, overwrite ordering, member intersection across all
contributing projects, exact bot transport principal). Fail closed when the
proof is unavailable; do not enable privileged intents or alter role permissions.
Private-team opt-in is a separate destination with original scope retained and
no public fallback. Both private and shared workflows require their own evidence;
private success cannot close shared-channel acceptance.

## Task 9: Classic work components and native proposals

Add classic buttons/selects/modals for Add task, Done, Snooze, Explain,
Pause, and Review action, with a defer before REST and current authorization
on every interaction. Complete native task lookup/assignment/update and
explicit project manager controls. Conversational native changes produce
structured previews bound to actor, scope, exact arguments, expiry and current
record revision; commit only after authorized confirmation. Commands continue
to commit authorized native changes directly. Model, voice and unsolicited
paths cannot confirm. Keep component identifiers opaque/bounded and reject
forged, expired or stale submissions. Test duplicate clicks, access revocation,
personal/team isolation and representative rendered states.

## Task 10: Approved Discord operation adapters

Connect shared external approval core and Review action UI to Abbey messages,
threads/forums, scheduled events, additive channel setup, and existing moderation.
Extend typed operations only as necessary for the catalog. Render exact preview,
confirm authorized human, fetch fresh target/permission/hierarchy facts immediately
before execution, persist execution state, call REST, verify and durably settle
success/partial/review-required. Persist invalidation on stale approval. Reuse
moderation clamps and server normalization/additive planning. Respect rate limits,
inert mentions, current bot authority and human authority. Test recording transports,
revocation, hierarchy, duplicate confirmations, partial and uncertain operations.
No live writes in source validation; voice and unsolicited tools stay read-only.

## Task 11: GitHub App authentication and read polling

Implement GitHub App authentication for explicitly selected installations and
repositories. Bind provider repository grants to explicitly configured work
scopes; possession of App credentials or project-manager status alone must not
expose every installed private repository. Manager publication allowlists can
narrow these grants, never broaden them. Read issues, pull requests and checks
with least permissions.
Expose authorized native link/allowlist controls and snapshot status. Poll active
links every five minutes with bounded concurrency, ETag requests, pagination,
token expiry handling and rate-limit backoff. Preserve stale snapshots on failures.
Use source links and last successful refresh time. Treat repository text as
untrusted and constrain shared publication to manager-authorized repositories.
Test with local fake transport, including malicious bodies and expired credentials.
Document configuration without printing secrets or broadening repository access.

## Task 12: Approved GitHub issue and comment writes

Connect exact action proposals to GitHub issue create/update and comment writes.
Freshly recheck installation/repository allowlist, requesting and confirming
human authority, target facts and proposal digest/expiry. Persist execution before
network writes and reconcile results without automatically replaying uncertain
requests. Snapshot refresh failures cannot turn a successful verified write into
a claimed failed write. Test stale approvals, permission removal, duplicate clicks,
rate limits, ambiguous network outcomes and approved-write reconciliation with
fake transport. Push, merge and workflow execution remain excluded.

## Task 13: Final integration, review, deployment, and acceptance

Re-review full branch, render representative replies, update README and
append-only ledgers, run strict gate, verify exact-SHA CI or record blocked
lanes, and check Windows/Linux where hosts exist. Audit the original user
specification including scoped WDBX recall/preference evidence, native workflow
completeness and all current cross-task-acceptance-notes.md obligations.
Prepare tested binary, configuration, backup and rollback without silently
discarding work records; release-rollback-inspection.md records the old-binary
schema trap that must be covered. Preserve gateway-before-bot order. Deploy
with atomic installer, compare installed binary SHA-256 to tested artifact,
check readiness and durable load. Pilot Donald's DM and one selected shared
project; exercise work, GitHub, Discord approvals, provider, vision, and
human-participation voice acceptance. Close ledger items only with evidence;
record missing inputs as capability-specific release blockers.
