# MLAI and Abbey Completion Program Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed planning package; no implementation/deployment/publication approval is implied. **Goal:** Complete the Activity, voice, text, learning, continuity, initiative, member-flow and release requirements without dropping external acceptance.

**Architecture:** Nine independently reviewable workstreams (eight new plans plus the separately approved engagement plan) build on current Rust/Node seams. One canonical state/provider authority/Work delivery owner; separate source and live evidence.

**Tech Stack:** Existing Rust1.98.0 stable/edition2024, Serenity0.12/Poise, Python deployment tooling, dependency-free Node/browser Court.

**Spec:** [2026-10-01-mlai-completion-program-design.md](../specs/2026-10-01-mlai-completion-program-design.md), plus each linked sub-project design.

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

- Moving source invalidates the gate/install attribution — Release Task1.
- Late/unknown send must not produce duplicated tools/messages — Text Task2/3 and Initiative Task2.
- Revoked access/consent must close processing before later asynchronous work — Voice Task2/3 and Continuity Task2.
- Cross-guild/DM/private evidence must never widen — Continuity Task2 and Member Task2/3.
- Missing human/public/provider/platform evidence cannot become Current — Release Task2/3.

## Execution sequence

- [ ] **0: Close current release baseline** using Release Task1 after execution direction; first verify source/installed hashes. Planning does not run the installer.
- [ ] **1: Activity launch** — authoritative protocol, ordered client, instance binding, then approved HTTPS/Portal/two-client witness.
- [ ] **2: Text reliability** — closed telemetry, typed failures, retained bounded producer, installed baseline/tuning. Source instrumentation may precede Activity public approval while no public action is permitted.
- [ ] **3: Voice completion** — existing upgrade gate, retained ownership, lifecycle matrix, human witness; synthetic cases can proceed while waiting for willing participants.
- [ ] **4: Learning quality** — dedup/attribution, data minimization/audit, correction repair, honest erasure, held-out evaluation.
- [ ] **5: Continuity** — user-confirmed Work references, fresh authorization, canonical migration/clear.
- [ ] **6a: Approved member engagement** — execute its ten-task plan with its owner; Tasks1–4 establish the sole consent, global member budget and reservation authority before optional task follow-ups. Tasks5–9 add scoped conversation/community features; Task10 requires real acceptance.
- [ ] **6b: Initiative** — consume that engagement authority and receipts for optional Work follow-ups; private inspection/pilot.
- [ ] **7: Member workflows** — explicit forum resolution, eligible help, ordinary-member/cross-scope witness.
- [ ] **8: Whole-program release qualification** — Release Tasks2/3, all acceptance rows, no missing required layer.

Read-only review and planning can overlap; source/runtime mutations are sequential. An external blocker does not block independent source work, but its requirement stays open. Do not deploy a different source snapshot from the one gated/reviewed.

## Designs and executable plans

| Workstream | Design | Plan | Tasks | Requirements |
|---|---|---|---:|---:|
| MLAI Activity Production Launch | [spec](../specs/2026-10-01-mlai-activity-launch-design.md) | [plan](2026-10-01-mlai-activity-launch.md) | 4 | 8 |
| MLAI Voice Completion | [spec](../specs/2026-10-01-mlai-voice-completion-design.md) | [plan](2026-10-01-mlai-voice-completion.md) | 4 | 8 |
| MLAI Text Latency and Reliability | [spec](../specs/2026-10-01-mlai-text-reliability-design.md) | [plan](2026-10-01-mlai-text-reliability.md) | 4 | 8 |
| MLAI Bounded Learning Quality | [spec](../specs/2026-10-01-mlai-learning-quality-design.md) | [plan](2026-10-01-mlai-learning-quality.md) | 5 | 8 |
| MLAI Opt-in Conversation Continuity | [spec](../specs/2026-10-01-mlai-continuity-design.md) | [plan](2026-10-01-mlai-continuity.md) | 3 | 8 |
| MLAI Appropriate Initiative and Follow-ups | [spec](../specs/2026-10-01-mlai-initiative-design.md) | [plan](2026-10-01-mlai-initiative.md) | 3 | 8 |
| MLAI Member Workflows and Server Acceptance | [spec](../specs/2026-10-01-mlai-member-workflows-design.md) | [plan](2026-10-01-mlai-member-workflows.md) | 3 | 8 |
| Approved Member Engagement | [spec](../specs/2026-10-01-member-engagement-design.md) | [plan](2026-10-01-member-engagement.md) | 10 | Section coverage below |
| MLAI Release and Completion Qualification | [spec](../specs/2026-10-01-mlai-release-qualification-design.md) | [plan](2026-10-01-mlai-release-qualification.md) | 3 | 8 |

## Requirement coverage and completion audit

The requirement IDs in each design are normative. The table below maps every requirement to its owning tasks. Acceptance includes the design's Review Focus cases, even when the ordinary happy path passes. Global constraints apply to every task.

| Project | Requirement-to-task map |
|---|---|
| activity-launch | R1→T1,T2; R2→T1; R3→T1; R4→T2; R5→T2; R6→T4; R7→T3,T4; R8→T4 |
| voice-completion | R1→T1; R2→T1,T2; R3→T2,T3; R4→T2; R5→T4; R6→T3; R7→T4; R8→T4 |
| text-reliability | R1→T1,T4; R2→T1; R3→T2; R4→T2; R5→T3; R6→T3; R7→T4; R8→T4 |
| learning-quality | R1→T1; R2→T1; R3→T1; R4→T2; R5→T2; R6→T3; R7→T4; R8→T5 |
| continuity | R1→T1; R2→T1,T2; R3→T1,T2; R4→T2; R5→T2; R6→T3; R7→T3; R8→T2 |
| initiative | R1→T1; R2→T1,T2; R3→T2; R4→T2; R5→T1,T3; R6→T1,T3; R7→T1,T2; R8→T3 |
| member-workflows | R1→T3; R2→T1,T2,T3; R3→T1,T3; R4→T1; R5→T2; R6→T2; R7→T3; R8→T3 |
| release-qualification | R1→T1; R2→T1; R3→T1,T2; R4→T2; R5→T2; R6→T2; R7→T3; R8→T3 |

## Gates and external actions

Preserve actual terminal exits. ./check.sh is source evidence; installed identity requires SHA256 equality/current status; public Activity requires real API/iframe witnesses; voice requires personal consent/human witness. No global percentage or source test count substitutes for those layers.

Publication commit/push, Node host/routing changes, Portal mapping, participant voice consent and any deployment/service change have explicit operator gates. No unanswered question permits them. Planning records exact next action and leaves acceptance open instead of narrowing the objective.

## Execution handoff

Recommended: subagent-driven sequential implementation/review per task with one runtime mutation owner and a final whole-program reviewer. This package is ready for user design/plan review; execution method and authorization remain separate. Existing source fixes are preserved; no new product code was written for this planning package.

## Integration, ownership and approved-section coverage

Total:39 tasks across nine workstreams. The eight new designs have64 numbered requirements; the separately approved engagement design retains its original section-based requirements and is not rewritten. Its coverage is: preferences/eligibility→T1,T2; global calendar limits/stops/cancellation→T3; retained delivery/access/uncertainty→T4; classification/follow-ups/weekly→T5; Activity/voice invitation safety→T6; five guild community features→T7; mutual introductions→T8; feedback/observability/style→T9; source/install/live acceptance→T10. All ten tasks remain required. Initiative's reviewed integration contract is normative for optional Work follow-ups and adds no parallel member policy.

The external engagement spec and plan are owned by another current chat; this package links them and does not modify them. The shared checkout has changed during planning. Earlier1568-test source evidence and the previously observed release/installed hashes describe only their recorded snapshot, not today's moving tip. Release Task1 must establish a fresh stable baseline before execution/install claims. Preserve other writers' edits and inspect current ownership before starting a mutation.

Source-task `text` assertion blocks are acceptance pseudocode, not copied runnable tests. Named regressions must be implemented in each task's listed src/inline test owner using its pinned APIs and event order; operator witness assertions require actual receipts. Reviewed contract sections amend earlier abbreviated Interfaces descriptions. Independent review findings and their resolutions are recorded in the package review receipt.


## Concurrent operations boundary

Another current owner maintains the [community operations design](../specs/2026-10-01-community-autonomous-operations-design.md) and [operations plan](2026-10-01-community-autonomous-operations.md). Those artifacts and their ongoing source mutations remain separately owned and outside this package’s39 tasks. Before executing Text, Learning, Engagement or Release tasks, reconcile the live participation/local-only/provider and durable-state seams with that owner’s changes. Do not restore older guards or remote fallback. This roadmap grants no autonomous server mutation or moderation authority; those require their own approved policy and acceptance.
