# MLAI Release and Completion Qualification Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Prove source, installed artifact and live capability acceptance across the entire completion program.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Task 1 begins immediately after planning approval; final acceptance depends on all seven other workstreams.

## Current evidence

Full source gate exited0 with1568 passed/0failed/5ignored, offline Swift16, Clippy and release. Built release18b1a179 differs from installedb965ed9b; current managed service ready. No commit/push/PR was authorized. Live voice/new iframe remain Partial.

## Architecture

Use the existing transactional installer/readiness rollback and dated acceptance ledger. Freeze source identity before gates and installation. Maintain one row per requirement with its proper proof layer; an unavailable witness/host stays Blocked or Partial, never silently removed from scope.

## Requirements

RELEASE-QUALIFICATION-R1: Before every install, inspect shared-tree ownership and exact source fingerprint, compare release/installed SHA-256, verify episode gateway readiness and operator direction. Equal hashes mean no reinstall.
RELEASE-QUALIFICATION-R2: Use ./deploy/install-launchd.sh for authorized deployment only. Preserve owner environment/data; never use installers as source tests. Observe terminal result and current service-status, not merely plist presence.
RELEASE-QUALIFICATION-R3: Each source slice has meaningful focused nonzero tests, independent review and full ./check.sh. Docs/Activity run Liquid and Court tests. Record actual counts/exits; accepted RustSec debt and explicit WDBX skips are not clean security/conformance claims.
RELEASE-QUALIFICATION-R4: For persistence/WDBX contract changes, run ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh with the authoritative sibling fixture. Missing fixture blocks that specific claim. No ABI/WDBX dependency or external encryption/COSE/member-key erasure claim.
RELEASE-QUALIFICATION-R5: Qualification covers installed text/tool/vision/voice route identity independently. Synthetic provider/voice modes remain token-free and no second Discord gateway is started.
RELEASE-QUALIFICATION-R6: Every advertised platform gets its own gate/runtime evidence. Current target is macOS managed Discord. Linux/Windows source portability is preserved; Windows/Linux runtime release claims require their own available host gates, never inferred from macOS. Telegram/Slack deployment is separately operator-controlled.
RELEASE-QUALIFICATION-R7: Final completion matrix covers all project spec requirements, public new iframe, human voice matrix, text benchmark, learning/erasure, continuity, initiative and non-admin server flows. Any missing proof means program implementation is not complete.
RELEASE-QUALIFICATION-R8: Planning package completion is distinct: all specs/plans saved, dependencies/interfaces/test owners reviewed, every requirement mapped to a task and no missing references/placeholders. Plan completion authorizes no deployment, production feature or commit by itself.

## Interfaces and data ownership

ReleaseReceipt contains source fingerprint, source gate exit/counts, release SHA-256, installed SHA-256, installer terminal outcome and dated read-only readiness. AcceptanceRow contains requirement ID, source evidence, installed evidence, live/provider/human evidence and Current/Partial/Blocked/OutOfScope status. These are operator artifacts in the existing private evidence directory and docs ledger, not managed JSONL payloads.

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

- Moving source after gate invalidates attribution before install. Test owner: implementation Task 1.
- Installer timeout with live handle is a wait, not a failed transaction to restart. Test owner: implementation Task 1.
- Green source tests must not promote human voice/new iframe to Current. Test owner: implementation Task 3.
- Missing external fixture or hardware must stay a named proof gap. Test owner: implementation Task 2.
- Cleanup/rollback may retain recovery state; preserve it and inspect before retry. Test owner: implementation Task 1.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-release-qualification.md](../plans/2026-10-01-mlai-release-qualification.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.
