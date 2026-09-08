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
- [x] Run focused workflow tests, then integrate with the updated catalog/help interface and review.

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
- [x] Replace duplicate catalog condition interpretation with typed canonical decisions and one capability readiness map; express vision policy in conditions, independent of help grouping.
- [x] Complete integrated Rust tests, warnings-denied Clippy and independent catalog re-review, then hand stable source to the release task.

Earlier final workflow results recovered after interruption: 61 help tests passed and all-target Clippy exited zero. These predate the strict corrections and do not replace their fresh integration checks.

- Final correction validation: full Rust suite 1,222 passed, zero failed, five ignored. All-target warnings-denied Clippy passed after two small catalog lint corrections; the affected catalog suite was rerun on those final corrections: 18 passed, zero failed. Independent strict re-review and the narrow final recheck approved with no findings. Formatting, whitespace and Pages Liquid checks passed.
- An external process committed/pushed the main correction slice as `543e3b671424194b57a30e3f1ce888a2cbb22123` during validation. This task preserves that commit and records only its final catalog lint correction and ledger in a follow-up. No push or installation was performed by this task during structural correction work.
- Stable source handoff transfers all remaining aggregate release gates, hosted CI, installation and live acceptance to **Initialize Abbey bot**. Full strict release-gate and installed-artifact results must be recorded there; these Rust checks alone are not deployment evidence.

## Task 3 source closeout — September 8, 2026

- Implementation: `b6358c7f5b8492bd56d97a7f5a99593a1fd07171` (`fix(help): harden interaction validation and delivery failures`). The correction validates application and source-message channel binding and adds one bounded modal delivery-recovery attempt without replaying generation or completed tool effects.
- Independent read-only review of the three-file correction and surrounding routing, authorization, catalog, shared operations and regression coverage found no actionable defects. The review confirms Task 3 source closeout; it is not live acceptance.
- Verification observed in this session on the final source: `cargo test --locked commands_help::` passed 67 tests, zero failures; `cargo test --locked command_catalog::` passed 18 tests, zero failures; `cargo clippy --locked --all-targets -- -D warnings` passed. Source remained unchanged during closeout, so these results were retained rather than rerun for this documentation-only update.
- All five task controls are connected through the existing component/modal routing. Visibility and read-only launchers use catalog `Discoverability`; conversation execution uses `Invocation` and typed availability guidance. Memory and administration reuse their existing private operations; images and voice/music provide truthful input/status guidance. No catalog API or routing hook remains unwired for Task 3, and the seven model tools and persistence interfaces are unchanged.
- Handoff: **Initialize Abbey bot** retains the strict aggregate gate, hosted CI, installation and installed-artifact verification, provider qualification, and human Discord/voice acceptance. This closeout performs no installation or production interaction and closes none of the release owner's checkboxes. The externally created `pre-redeploy-abbey-bot-105` stash remains untouched.

## Final release sequence — updated September 6, 10:51 EDT

This sequence incorporates the operator's requirement for dynamic multi-guild,
multi-user behavior. The supplied operator and home-guild identities remain
transient verification inputs; neither becomes source policy or an authority
bypass. Existing source checks below are historical until a new stable candidate
passes the complete gate. The current detailed receipts remain in
`.superpowers/completion-20260906/delivery.json`.

### 1. Finish integration and freeze ownership

- [x] Preserve and publish the original Windows LF fixture correction.
- [x] Independently review and commit the subsequent Windows test-cfg repair.
- [x] Independently review and commit always-global registration with an optional
  immediate home-guild copy, preserving the Activity command's supported metadata.
- [ ] Complete the bounded guild-keyed voice registry and destination-free backend
  template. Select each guild's channel from its authorized initiating manager.
- [ ] Preserve the existing home consent ledger; isolate additional guild ledgers
  in private directories without changing the consent serialization format.
- [ ] Finish all slash, button, help, gateway and shutdown registry integration.
  Close admission before draining all sessions under the existing shared budget.
- [ ] Protect first-join preflight with cancellable guild reservations, require
  completed leave before rebinding a channel, and prevent stale retirement from
  orphaning a newer session.
- [ ] Configure individual Songbird calls, and hold exclusive host-music ownership
  through actual child and capture cleanup. A competing request fails before
  changing the shared native player.
- [ ] Update command/configuration/recovery documentation and both task ledgers.

The existing five workflows, exact provider capability selection, authorization
checks, seven model tools, consent semantics and persistence formats remain the
release contract. No arbitrary natural-language administration or web dashboard
is introduced.

### 2. Review and validate the completed source

- [ ] Run focused tests for changed permissions, member/manager/owner authority,
  stale/malformed/expired controls, missing inputs, provider failure, and completed
  mutations whose response delivery fails.
- [ ] Prove guild/user isolation, concurrent session creation, consent separation,
  pending-join cancellation, channel rebinding, closed admission, bounded capacity,
  per-call configuration and competing host-music rejection.
- [ ] Independently review all final diffs; resolve findings and rerun affected
  checks. Read rendered workflow/recovery text as well as testing invariants.
- [ ] Commit only reviewed changes on canonical `main`, then freeze the clean SHA.
- [ ] Run `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` with the dedicated external
  `CARGO_TARGET_DIR`. Record exact SHA, tracked-source digest, WDBX fixture digest,
  exit result, test results and release-binary hash; verify source stayed stable.

Any code, fixture, dependency or documentation change after this gate requires a
new final candidate and appropriate renewed release validation. Previous test
counts do not certify later edits.

### 3. Publish and require all hosted platforms

- [ ] Push canonical `main`, including both Windows repairs and integrated changes.
- [ ] Require Ubuntu, macOS and Windows Rust CI to succeed for exactly the final
  remote SHA. Fix any failure, review the fix, and repeat candidate validation.
- [ ] Verify local HEAD, `origin/main`, hosted head SHA and gate SHA agree.
  A running job or an older green SHA is not sufficient.

### 4. Transactionally install the verified candidate

- [ ] Revalidate the clean source and dedicated release artifact immediately before
  invoking `deploy/install-launchd.sh` with the dedicated build target.
- [ ] Let the installer select Cargo's reported executable and perform its existing
  stop/stage/start/readiness transaction. Retain rollback evidence on failure.
- [ ] Verify installed SHA-256 equals the candidate artifact, process identity is
  current, and managed readiness observes Discord, scheduler and persistence.
- [ ] Export the exact command payload from the final source; compare it with both
  actual global and optional home-guild registration. Verify Activity preservation
  and application-owner discovery without retaining actual Discord identities.

Do not run the audio-sidecar installer, permission prompts or production capture
endpoint as part of source validation.

### 5. Qualify providers against the installed binary

- [ ] Run the guarded synthetic primary provider probe with the installed hash and
  intended configuration; record each supported capability separately.
- [ ] Run the token-free synthetic local voice round trip against the same installed
  binary. Remove scratch audio after checking its bounded result.
- [ ] Record unsupported or unconfigured routes explicitly. Current v1 reports do
  not attest immutable model bytes or satisfy the broader v2 publication and
  complete model/manifest transaction requirements in the live protocol.

### 6. Exercise actual Discord workflows and voice

- [ ] Resolve two explicitly designated sandbox guilds and an ordinary-member tester
  alongside a manager. A home-guild identity alone does not establish either.
- [ ] In both guilds, exercise Conversation, Memory, Images, Voice & Music, and
  Administration as member and manager. Verify expected authorization, private
  ownership, current-state guidance, stale controls, and cross-guild/user isolation.
- [ ] Use synthetic data and restore each changed test setting after its check.
- [ ] Obtain current agreement from every voice participant before activation.
  Exercise independent guild session state and immediate authorized stop behavior.
- [ ] Obtain human confirmation of audible output, interruption, pause/resume and
  leave. Synthetic audio success or a ready service does not supply this evidence.

Follow `docs/live-test-protocol.md`; use only neutral Guild A/Guild B and role
labels in evidence. Missing human observations remain NOT OBSERVED. Do not invent
accounts, consent, audible results or permissions to close the checklist.

### 7. Reconcile delivery and close out

- [ ] Reconcile the canonical delivery record, task ledgers and live acceptance
  notes with observed source, CI, installation, registration and probe results.
- [ ] Keep engineering review logs separate from privacy-limited live evidence;
  protect evidence directories/files with 0700/0600 permissions.
- [ ] Report exact delivered SHA and CI links, installed artifact match, provider
  results, completed live checks and any remaining human acceptance requirements.
- [ ] Claim complete release acceptance only when the required observations exist;
  describe a deployed candidate with pending human checks accurately.
