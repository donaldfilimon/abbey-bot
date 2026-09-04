# Abbey Bot Full Modernization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Modernize Abbey Bot's Rust architecture, Discord interaction UX, provider routing, managed-service lifecycle, persistence truthfulness, and privacy-safe observability without weakening its compatibility or consent boundaries.

**Architecture:** Keep the pure-core/thin-Discord-shell boundary. Introduce typed command, persistence, provider, service, and observability cores, then adapt existing Poise, launchd, provider, and documentation surfaces around them. Deliver each independently testable slice through a review branch and exact-head CI before merging it to `main`.

**Tech Stack:** Rust 2024 on exact Rust 1.98.0, Tokio, Serenity 0.12.5, Poise 0.6.2, Songbird 0.6.0, tracing, Python deployment tests, POSIX launchd tooling.

**Spec:** Approved conversation plan dated 2026-09-04 plus the three design documents created by Task 1.

## Global Constraints

- Preserve Rust edition 2024 and exact Rust 1.98.0 support.
- Preserve existing slash commands, `Abbey: profile`, `Ask Abbey`, and Discord's `PRIMARY_ENTRY_POINT` launch command.
- Preserve the exact seven-tool production order and byte-compatible five-tool Abbey corpus.
- Preserve WDBX v1, guild settings, provider-manifest compatibility, and voice-consent persistence formats.
- Preserve non-privileged intents by default; add no new privileged intent.
- Preserve generated-mention suppression, self-or-moderator memory boundaries, four unsolicited-speech gates, and consent-gated voice behavior.
- Every ordinary command acknowledges before network access; `/voice leave` alone may close the media gate before its concurrent acknowledgement.
- Every rendered model/core answer passes through `clamp_message`.
- Never add a `GuildChannel` command parameter.
- Components remain classic Action Rows; do not use Components V2 on the pinned Serenity/Poise pair.
- Source, CI, provider qualification, installation, Discord acceptance, connector acceptance, voice acceptance, and managed-service acceptance remain separate evidence layers.
- Do not touch the live launchd service, real logs, owner env, providers, Discord configuration, or voice session without a fresh explicit authorization at that acceptance layer.
- Preserve existing unrelated untracked files, stashes, and the linked Cursor worktree.
- Do not hide dead code or warnings with module-wide allowances.

---

### Task 1: Materialize Approved Design Specifications

**Files:**
- Create: `docs/superpowers/specs/2026-09-04-discord-command-center-design.md`
- Create: `docs/superpowers/specs/2026-09-04-provider-runtime-modernization-design.md`
- Create: `docs/superpowers/specs/2026-09-04-service-observability-design.md`
- Verify: this implementation plan

**Interfaces:** Produces the binding specifications consumed by every later task.

- [x] Record the approved Discord catalog/help, member memory, image menus, voice split, typed voice mode, and classic admin-dashboard behavior.
- [x] Record the explicit normalized provider scoring, circuit state machine, conversation-local pinning, effect-aware fallback, and legacy compatibility behavior.
- [x] Record `PersistReport`, supervisor, structured event, readiness file, bounded JSONL retention, and fake-launchd acceptance behavior.
- [x] Verify the three specs repeat every Global Constraint without contradicting README.md or AGENTS.md.
- [x] Commit only the four planning/specification files.

### Task 2: Truthful Persistence Outcomes

**Files:** Modify persistence/runtime/admin-flush/shutdown modules and their tests.

**Interfaces:**
- Produces `PersistErrorCategory`, `PersistComponentOutcome`, `PersistOverall`, `PersistReport`, and an injectable `PersistenceSink`.
- `AppState::persist_all() -> PersistReport` becomes the only process-level persistence result.

- [ ] Write failing tests for MemoryOnly, Complete, Partial, and Failed truth-table rows.
- [ ] Test that canonical failure skips WDBX and that old files survive temporary-write and rename failures.
- [ ] Implement categorized, content-free reports and atomic file sync/type checks.
- [ ] Make `/admin flush`, scheduled persistence, and shutdown render/log the report instead of inferring success from a configured path.
- [ ] Run `cargo test persist::`, `cargo test runtime::`, and the relevant admin/shutdown tests.
- [ ] Commit the reviewed persistence slice.

### Task 3: Command Acknowledgement and Compatibility Characterization

**Files:** Modify voice command orchestration, global command registration helper, roadmap text, and tests.

**Interfaces:** Produces a crate-private acknowledged-context helper and a pure Entry Point merge helper.

- [ ] Add failing ordering tests for `/voice join`, `/voice resume`, and `/voice leave`.
- [ ] Defer join/resume before every guard or network path.
- [ ] Keep leave ordered as authorize synchronously, close media gate, then acknowledge concurrently with transition work.
- [ ] Add registration fixtures proving Entry Point name, handler, integration types, and contexts survive global bulk registration.
- [ ] Add characterization tests for backend precedence, FM/vision gating, tool order/corpus, self-test CLI, and existing persistence formats.
- [ ] Reconcile roadmap text with the unconditional-defer rule.
- [ ] Run command, voice-session, provider-config, tool, and registration tests.
- [ ] Commit the reviewed acknowledgement/compatibility slice.

### Task 4: Typed Command Catalog and Private Help Center

**Files:** Create a pure command-catalog/help module; modify command registration, component routing, README catalog region, and tests.

**Interfaces:** Produces `CommandKey`, `CommandKind`, `InteractionContext`, `Access`, `Capability`, `HelpSection`, `CommandSpec`, `registered_commands()`, and the versioned help component protocol.

- [ ] Add the complete catalog for every existing and planned leaf command/context menu.
- [ ] Add recursive Poise/catalog parity and README generated-region parity tests.
- [ ] Implement ephemeral `/help [section]` with context, permission, and capability filtering.
- [ ] Implement central component dispatch for `abbey:help:v1:<owner>:<expiry>:<section>` with 15-minute expiry and strict 100-character IDs.
- [ ] Ensure component acknowledgement precedes permissions/network work and full IDs are never logged.
- [ ] Test DM/member/moderator/webhook-manager/server-manager/admin matrices, unavailable capabilities, hidden voice channels, Discord limits, clamping, and Entry Point retention.
- [ ] Commit the reviewed catalog/help slice.

### Task 5: Privacy-Aligned Memory and Image Context Menus

**Files:** Modify memory/reputation and vision command adapters, registration/catalog, README, and tests.

**Interfaces:** Produces one reusable self-or-moderator memory authorization function, one memory-card renderer, and one shared resolved-attachment selector.

- [ ] Make `/reputation` ephemeral, self-defaulting, DM-capable for self, and cross-member gated by Manage Messages, Manage Server, or Administrator.
- [ ] Add guild-only ephemeral USER menu `Abbey: memory`, sharing `/recall` authorization and rendering.
- [ ] Add ephemeral MESSAGE menus `Abbey: describe image` and `Abbey: read image text` for guild and bot-DM contexts.
- [ ] Select the first real supported attachment deterministically; never fetch embeds, stickers, message URLs, or arbitrary remote links.
- [ ] Reuse `/see` and `/ocr` bounded fetching/decoding/provider/rendering paths; do not commit context-menu work to transcript, memory, reward, or learning state.
- [ ] Test authorization, DM isolation, all four image formats, misleading MIME/extension, multiple attachments, size/decoder/provider failures, clamping, ephemerality, and mention suppression.
- [ ] Commit the reviewed privacy/menu slice.

### Task 6: Member Voice UX and Classic Admin Dashboard

**Files:** Create pure voice/admin view models; modify voice/admin commands, central component routing, catalog/README, and tests.

**Interfaces:** Produces `MemberVoiceInput/View`, `AdminVoiceInput/View`, `VoiceModeChoice`, `AdminPage`, `AdminAction`, `AdminEffect`, dashboard reducer/view, and versioned admin component protocol.

- [ ] Make `/voice status` member-accessible and allowlist only coarse state, processing category, caller agreement, visible channel, and next action.
- [ ] Add ephemeral Manage Server-only `/voice diagnostics` containing the current content-free operational detail.
- [ ] Replace Discord free-form voice-mode input with Off/Local/OpenAI choices while retaining environment aliases.
- [ ] Add ephemeral Manage Server-only `/admin dashboard` with Overview, Conversation, Learning, Operations, and Confirm Reset pages using classic components.
- [ ] Route `abbey:admin:v1:<owner>:<guild>:<expiry>:<action>` centrally; recheck current permission and reload authoritative settings before every mutation.
- [ ] Require a second interaction before channel-transcript reset; keep flush truthful and export ephemeral.
- [ ] Test member-field allowlist, permission changes, hidden channel, mode eligibility, connected-session refusal, row/ID limits, stale-page idempotence, and reset scope.
- [ ] Commit the reviewed voice/admin UX slice.

### Task 7: Correct the Pure Adaptive Provider Router

**Files:** Modify provider domain/routing/catalog modules and focused tests only; do not wire production yet.

**Interfaces:** Produces `NormalizedScore`, `ProviderScoreProfile`, `ProviderFailureKind`, `CircuitPhase`, `CircuitSnapshot`, `RouteDecision`, and `RouteUnavailableReason`.

- [ ] Add failing tests for explicit `[0,1]` scores, exact 40/30/25/5 weighting, `n/20` qualification/live blending, configured-order tie breaks, and invalid numbers.
- [ ] Remove implicit capability-density, provider-class, and raw-latency normalization from the router.
- [ ] Implement injected-time failure windows: third transient failure in five minutes opens 60 seconds; half-open failure escalates to five then 15 minutes, capped; exactly one probe may reserve half-open.
- [ ] Block auth/identity/schema/sandbox/configuration/protocol drift until requalification.
- [ ] Exclude cancellation, invalid request, and busy outcomes from circuit/EWMA changes.
- [ ] Reject Retry-After outside one second through 15 minutes.
- [ ] Exclude open/blocked candidates, including sole and previously pinned candidates.
- [ ] Remove router-global sticky state.
- [ ] Run all provider domain/routing/catalog tests and commit the reviewed pure-router slice.

### Task 8: Single Production Provider Runtime

**Files:** Add provider runtime/conversation ownership; migrate runtime/generation/vision/voice/inspect adapters and tests.

**Interfaces:** Produces `ProviderRuntime` and `ProviderConversation`; temporarily retains crate-private legacy accessors.

- [ ] Characterize legacy env precedence and labels before migration.
- [ ] Make ProviderRuntime own catalog, eligible adapters, router, qualification state, capacity, and safe inspection state.
- [ ] Scope provider pinning to one conversation and independently track visible output and tool dispatch.
- [ ] Permit one fallback only before both effects; never fallback after first visible stream edit, tool dispatch, or image submission.
- [ ] Initially activate only current Anthropic, OpenAI-compatible, and qualified FM routes; detected unsupported adapters remain ineligible.
- [ ] Keep voice read-only/tool-incapable and vision single-provider.
- [ ] Rename the FM-only router during migration, remove it after cutover, and remove provider-wide dead-code/import allowances.
- [ ] Test legacy equivalence, unqualified exclusion, concurrent conversations, tool continuation, fallback effects, vision/voice boundaries, Inspect privacy, tool corpus, and manifest/self-test compatibility.
- [ ] Commit the reviewed ProviderRuntime slice.

### Task 9: Supervised Service Lifecycle and Bounded Shutdown

**Files:** Add service supervisor/persistence worker; refactor scheduler, connector loops, and main shutdown ownership; update dependencies/tests.

**Interfaces:** Produces `ServiceSupervisor`, `TaskName`, `TaskExit`, `ShutdownReason`, `ShutdownReport`, `SchedulerIntervals`, and cancellation-aware connector functions.

- [ ] Add direct `tokio-util` dependency and use `CancellationToken`.
- [ ] Replace detached scheduler loops with one named actor using `MissedTickBehavior::Skip` and serialized persistence requests.
- [ ] Make Telegram and Slack configuration explicit, secret-redacted, cancellable around I/O/backoff, and timeout-bounded.
- [ ] Treat external connector outages as degraded retries; treat unexpected task exit/panic as readiness-fatal.
- [ ] Replace duplicate signal/client-return shutdown paths with one root `tokio::select!`.
- [ ] Enforce 20-second overall shutdown with five-second voice, shard, task, and final-persistence budgets and exactly one final persist.
- [ ] Test paused scheduler cadence, cancellation, task panic/return, connector I/O/backoff cancellation, simultaneous shutdown triggers, abort/reap, and post-final-snapshot quiescence.
- [ ] Commit the reviewed lifecycle slice.

### Task 10: Privacy-Safe Observability, Readiness, and Bounded Logs

**Files:** Add observability/readiness/log writer modules; migrate interaction ledger and launchd plist; extend privacy tests.

**Interfaces:** Produces `RunIdentity`, closed `EventCode`/component/outcome/error enums, privacy-safe `OperationalEvent`, `ReadinessPublisher`, and rotating managed JSONL writer.

- [ ] Replace durable raw command errors and Discord IDs with command, success, categorized error, millisecond duration, and timestamp; deserialize legacy rows, discard legacy private fields, and require canonical privacy rewrite before managed ready.
- [ ] Use monotonic millisecond timing for owned intervals and Discord timestamp milliseconds for total interaction latency.
- [ ] Generate a cryptographic per-run nonce and hash the running executable; never trust env-supplied identity.
- [ ] Publish owner-only atomic readiness at the fixed managed path with PID, nonce, SHA, phase, Discord/scheduler state, coarse connector states, and last persistence category only.
- [ ] Publish ready only after state rewrite, scheduler start, Discord ready, registration, and presence; publish draining and remove only the same PID/nonce file.
- [ ] Add managed JSONL rotation: 8 MiB active file, five archives, 16 KiB event cap, directory 0700, files 0600, pre-write rotation, no symlinks/unexpected types.
- [ ] Preserve foreground human-readable stderr and leave the legacy `abbey-bot.log` untouched.
- [ ] Expand privacy gates to reject IDs, raw errors, paths, URLs, endpoints, models, prompts, transcripts, payloads, and media in structured events.
- [ ] Test legacy migration, accurate short latency, canary exclusion, readiness ownership/type/freshness/identity, rotation/concurrency/modes, and initialization-before-credential access.
- [ ] Commit the reviewed observability/readiness slice.

### Task 11: Offline launchd Transaction Verification

**Files:** Add readiness checker and fake-home/fake-launchctl installer suite; modify installer/check scripts and documentation.

**Interfaces:** Produces `deploy/check-service-readiness.py` and `deploy/test-install-launchd.py`.

- [ ] Make install success require fresh matching PID, run nonce, installed SHA, ready phase, scheduler running, Discord ready, and five additional stable seconds within a 30-second readiness budget.
- [ ] Require the same contract after rollback; stable PID alone never proves recovery.
- [ ] Preserve existing uninstall behavior: stop exact service, remove plist/readiness, leave binary/data/env/rollback/logs.
- [ ] Build a temp-HOME harness with fake cargo/launchctl/plutil/sleep and descendant-path assertions; never touch the real launchd domain or service.
- [ ] Cover fresh install, update, no/stale/wrong readiness, PID changes, bootstrap/rollback failures, locks, unsafe types, invalid env/hash, interrupts, uninstall, modes, and secret-canary output.
- [ ] Wire POSIX behavior tests into `check.sh`; Windows syntax/privacy-checks helpers but explicitly skips launchd execution.
- [ ] Commit the reviewed installer-verification slice.

### Task 12: Behavior-Neutral Decomposition, Dependency Refresh, and Closeout

**Files:** Split oversized Rust modules by responsibility; update lockfile/gates/docs/ledgers.

**Interfaces:** Preserve crate-private re-exports and all deliberate interfaces above.

- [ ] Move provider, runtime, command, brain-command, voice-command, local/offline voice, and voice-session responsibilities into focused modules without behavior changes.
- [ ] Keep every production Rust file below 1,000 lines; flag files above 800 for review; add a size ratchet and forbid module-wide dead-code/import allowances.
- [ ] Preserve voice epoch, media-gate, cancellation, leave, and actor-reap ordering during movement.
- [ ] In a separate lockfile commit, apply compatible patch/minor updates that retain Rust 1.98 and current Serenity/Poise/Songbird/Reqwest/Symphonia compatibility groups.
- [ ] Retain the reviewed OpenMLS patch and four explicit RustSec records unless compatible upstream changes genuinely remove them; add no OpenSSL/native-TLS graph.
- [ ] Update README, roadmap, live protocol, goals/todo, readiness/logging runbook, and mirrored AGENTS.md/CLAUDE.md with claim-honest evidence.
- [ ] Run focused tests after each movement, then the strict full gate with required sibling WDBX conformance and a fresh external target directory.
- [ ] Dispatch final whole-branch architecture/security review; fix all Critical/Important findings and record rulings.
- [ ] Push each reviewed branch, require exact-head Ubuntu/macOS/Windows CI, merge sequentially into `main`, fast-forward canonical main, then remove worktrees and branches.
- [ ] Do not perform real launchd, provider, Discord, connector, or voice acceptance without fresh authorization; record them as separate pending layers.
