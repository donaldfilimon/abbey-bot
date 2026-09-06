# Dynamic Command Workflows Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development and test-driven-development. Read the approved spec and project AGENTS.md. Work in the canonical checkout; stage no unrelated changes.

**Goal:** Make every accepted Abbey interaction produce a useful result and make supported functions discoverable through guided, state-aware controls.

**Architecture:** Extend the existing pure catalog and thin Discord adapters. Keep authorization, readiness and delivery outcomes distinct. Reuse existing operations from both slash and guided entry points.

**Tech Stack:** Rust 1.98.0, Serenity 0.12.5, Poise 0.6.2, classic Discord components.

**Spec:** `docs/superpowers/specs/2026-09-06-dynamic-command-workflows.md` (Donald approved in this session, 07:53 EDT).

## Global constraints

- Preserve existing slash commands, permissions, current-participant voice agreement, immediate stop behavior, the seven-tool production vocabulary, and persistence formats.
- Never capture live voice or send test messages to others as source validation.
- Preserve current owner/guild/expiry binding and managed service ownership.
- No provider health claim from configuration alone; exact request class matters.
- Never replay a completed mutation to retry delivery.
- All changes receive regression coverage and independent review before deployment.

## Task 1: Availability and discovery

Owner: availability implementer. Files: `command_catalog.rs`, `command_catalog/`, `commands_help.rs`, `commands_help/tests.rs`, `commands_help/dispatch_tests.rs`, provider readiness methods/tests.

- [x] Write failing tests for unconfigured voice status reachability, permitted unavailable help entries, exact tool/text and description/OCR readiness, and fixed reason-specific guard responses.
- [x] Add a pure availability result carrying access vs capability/input blockers. Keep `eligible` as the executable authority and make help discovery explicitly different from readiness.
- [x] Query provider readiness by the operation's actual request class and tools policy. Do not perform remote qualification while rendering help.
- [x] Route help and guard copy through the new reason projection. Expose safe status even when voice is unconfigured.
- [ ] Run targeted catalog/help/provider tests; report changed interfaces to the workflow owner. Keep workflow hook integration sequential after this task.

## Task 2: Outcomes, administration and voice guidance

Owner: outcomes implementer. Files: `startup.rs`, new bounded error helper if needed, `commands_brain.rs`, `commands_brain/dashboard.rs`, `admin_dashboard.rs`, `voice_views.rs` and respective tests. Do not edit `commands_help.rs`.

- [x] Write failing tests for bounded fixed error guidance, a successful mutation with failed response delivery, effective unsolicited/vision policy reporting, and active voice wake-name guidance.
- [x] Own remaining framework error rendering with clamped fixed responses, mention suppression and categorized observability. Keep raw errors out of user responses.
- [x] Remove discarded dashboard delivery failures and never retry mutations when rendering fails. Publish a reusable delivery helper if it reduces repeated handling; send its interface to Task 1/3.
- [x] Show requested settings and effective blockers together, including learning, host quiet and guild vision policy. Distinguish configured from active voice.
- [x] Lead active voice guidance with a spoken wake-name example and retain immediate stop instructions.
- [x] Run targeted tests and independent review.

## Task 3: Guided workflows

Owner: workflow implementer. Files: new `commands_help/workflows.rs` and child test modules, shared conversation operation extraction, `commands_memory_browser.rs` as needed, central gateway modal routing. Parent/help hooks integrate after Task 1 releases its file.

- [x] Write failing tests for owner/context/expiry validation, malformed controls, exact action selection, and response outcomes.
- [x] Make task-home actions for Conversation, Memory, Images, Voice/Music and Administration. Conversation opens a question modal and uses the existing generation operation; Memory directly opens the private browser; Images provides specific attachment/menu guidance; Voice/Music opens current state and supported controls/guidance; Administration opens the existing authorized dashboard.
- [x] Keep reference navigation separate from task execution. Recheck permissions on input submission, acknowledge before I/O, use no fake slash interactions, and retain managed ownership for modal work.
- [x] Reuse existing domain operations; where an action still requires a slash command, supply a concrete exact command and prerequisite instead of implying that the button executed it.
- [x] Add bounded capability explanation for conversational requests without changing the seven model tools. Use only observed current facts and scoped permissions.
- [ ] Run focused workflow tests, then integrate with the updated catalog/help interface and review.

## Task 4: Integration and delivery

Owner: coordinator, with independent reviewer.

- [x] Review all changes against the approved design and the seven findings in the command review. Resolve all actionable defects.
- [x] Update README command UX and relevant behavior documentation; preserve generated command-catalog parity.
- [ ] Run `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` to completion and inspect its exit status.
- [ ] Commit only this task's reviewed files in canonical main. Preserve the unrelated pre-existing commit and do not invent hosted-CI proof.
- [ ] Install via the existing transactional installer; verify current readiness, installed binary digest and registered command payload.
- [ ] Run synthetic provider/voice checks against the installed artifact. Report live human command/voice acceptance separately.

## Execution ledger

- Baseline: canonical main `7ee3063`, clean; one pre-existing commit ahead of origin. Strict gate: 1183 Rust tests pass, four live ignores; provider and synthetic voice pass. Existing source deployed successfully before redesign.
- Ruling: preserve canonical checkout per machine policy and assign disjoint file ownership. No task branch or worktree needed; cost of overlap is a coordinated integration, not lost independent work.
- Ruling: this approved scope extends the catalog and workflows, not arbitrary natural-language Discord mutation capabilities or a new web application.

- September 6 implementation review: all original findings addressed; independent reviewer approved the source, including corrected streaming read-only unsolicited readiness and stream-only adapter checks.
- Focused evidence: dashboard outcome tests 2 pass, stream-only reservation/readiness regression 1 pass, missing executable adapter regression 1 pass, framework errors 2 pass, admin dashboard 5 pass, voice guidance 4 pass; operational capability appendix standalone tests 3 pass. Workflow integration checks are still being collected.
- Other concurrent work produced and pushed checkpoint commits `c2d92f3` and `e449362` before this implementation's verification finished. These checkpoints are preserved and are not final release evidence.
- Release ownership: Donald's separate **Initialize Abbey bot** task (`01a07694-e195-73e2-a1ef-d4bbd5ee6d5c`) owns the final strict aggregate gate, hosted CI, installation, installed-artifact qualification and evidence reconciliation after this task hands over a stable correction commit. Its deploy files, command-registration export test and live-test protocol are outside this implementation commit.

## Strict maintainability corrections (September 6, 08:37 EDT review)

Release task requested implementation of three review findings before candidate handoff.

- [x] Share provider admission and router hard-gate assessment between readiness and reservation; provider suite 137 pass, zero fail, one ignored. Independent provider re-review approved.
- [x] Move shared delivery observation to `gateway::interaction_outcomes`; keep framework translation at startup. Independent source re-review approved; integrated test remains pending.
- [ ] Replace duplicate catalog condition interpretation with typed canonical decisions and one capability readiness map; express vision policy in conditions, independent of help grouping.
- [ ] Complete integrated Rust tests, warnings-denied Clippy and independent catalog re-review, then hand stable source to the release task.

Earlier final workflow results recovered after interruption: 61 help tests passed and all-target Clippy exited zero. These predate the strict corrections and do not replace their fresh integration checks.
