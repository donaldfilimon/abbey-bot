# Community O7 Shadow Case Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans task-by-task with root as the sole repository writer and Abbey Reviewer independently reviewing the captured complete diff. Steps use checkbox syntax. The current user authorization permits implementation; do not introduce another approval, commit or worktree flow.

**Goal:** Implement and source-qualify the bounded human-assessed shadow case, independent review and subject appeal slice without enforcement.

**Architecture:** Pure moderation policy owns exact source-version cases and closed human decisions. Owner-only independent operational persistence uses retained CommunityFilesystem, current policy/proof checks, atomic publication and exact readback. Native commands prove current REST identities/roles/overwrites and privately expose only authorized cases or staff-only observed counts.

**Tech Stack:** Existing Rust 1.98.0/edition 2024, locked Serenity 0.12.5/Poise 0.6.2, serde/SHA-256, OwnedFile and service registry. No new production dependency.

**Spec:** [companion source design](../specs/2026-10-03-community-shadow-source.md); parent repository `docs/superpowers/specs/2026-10-01-community-autonomous-operations-design.md` and O7 in `docs/superpowers/plans/2026-10-01-community-autonomous-operations.md`; the retained recovery decision in that companion source design.

## Global Constraints

- Root is the sole source writer in the shared canonical checkout; preserve every existing edit, index and historical receipt. No checkout/reset/staging/commit/push/PR/worktree, deployment, restart, live provider or Discord mutation.
- Preserve version-1 absent/default policy serialization with serde default plus skip_serializing_if. Resolve existing ABBEY_COMMUNITY_POLICY once per AppState; None in memory; a config path is never authority.
- New captures are default-off and exact guild/owner/source scoped. Protected source capture is denied; protected private staff review is allowed. Stop/default-off denies new captures and path creation, while fresh authorized existing reviews/appeals and exact existing publication recovery remain available.
- Pure modules receive time from infrastructure. No native await under state/filesystem locks. Native proofs bind current exact IDs, complete roles and overwrites; proof age is at most 60 seconds and starts before GET work.
- Maximum 1,000 retained cases and 8 MiB; one independent review, one subject appeal and one independent resolution per exact source version. Fail closed at capacity; no invented retention/eviction, automatic retry or deletion.
- Persist only identities/version/digest, human assessment/severity/provenance and closed decisions. No raw source/transcripts/private facts/allegations/arbitrary feedback or transport errors in records/logs. Learning-memory erasure does not silently delete operational evidence.
- HumanModerator origin is explicit. Proposals have timeout at most 600 seconds; ambiguous/quotation content goes to HumanReview. No delete/timeout/kick/ban, notices, unsolicited DMs, classifier/provider work or enforcement activation. O2 global default remains unchanged.
- Production modules stay below 1,000 lines; over 800 requires independent review. External test-only modules stay below 1,000; split native retry tests into a child.
- Source-qualified means stable complete input manifest, authoritative strict gate exit 0 and no unresolved blocking findings. Human/classifier/live moderation/pilot/appeal utility/installed identity and Linux/Windows runtime qualification remain Partial/Unverified.

## Review Focus

1. Returned member/source IDs and incomplete roles: deny rather than allow a foreign identity or channel-only staff target.
2. Stop/default-off and post-publication uncertainty: existing exact recovery may confirm an old record, but cannot create a new path/case or overwrite original provenance.
3. Permission/source changes and queued work: source equality, current owner/digest and proof TTL must be rechecked before private results and publication.
4. Waiter timeout/cancellation and filesystem failure: retained work is observed joined, and visible bytes alone cannot produce a false success receipt.
5. Existing private review and removed/denied subjects: current exact staff/subject authorization prevents disclosure; unavailable practical appeal access remains a live activation gap.

---

### Task 1: Existing-API regression and repair

**Files:** Modify `src/moderation/contextual.rs`, `src/commands_help.rs` as attributable findings require, and the contextual native adapter; tests in `src/commands_help/dispatch_tests/` and inline contextual tests. Preserve root's already-applied global permission repair and evidence.

**Interfaces:** Existing contextual `qualify(Input) -> Result<Proposal, &'static str>`, native registered /modcall and current_permissions. Public source_message remains Option<String> with positive decimal u64 input; no parser/registration migration.

- [x] Add the supplied zero-author/target pure regression and current actual native contextual RED file. Keep the valid native control strict; collect source-ID, wrong-member, channel-only staff and missing-role mismatches together; keep no-mentions as an independent test.
- [x] Run `.claude/skills/run-abbey-bot/smoke.sh test actual_contextual_` and the exact pure regression filter, retaining nonzero counts and attributable failures before repair. For already-fixed global permission cases, retain historical RED and current GREEN evidence rather than undoing the repair.
- [x] Apply the smallest confirmed identity/role/overwrite/positive-ID/private-reply repair, keeping fixed errors content-free and genuine REST facts authoritative. No provider or moderation action is introduced.
- [x] Run the focused contextual and global native permission suites; require nonzero matched tests and exit 0. Read rendered private replies and review the repaired behavior before continuing.

### Task 2: Pure case domain and retained operational store

**Files:** Create `src/moderation/shadow.rs`, its `model.rs` and `tests.rs`; create `src/persist/moderation_shadow.rs` and tests; extend `src/community_ops.rs`, `src/moderation.rs`, `src/persist.rs` and `src/runtime/community_filesystem.rs` with child retained tests. Use external core draft mapping from HANDOFF.md.

**Interfaces:** `NewCase::human_assessed(SourceVersion, Input, FreshAuthority) -> Result<NewCase, &'static str>`; `CaseStore::apply(Mutation, &ShadowPolicy, digest, now)` and inspect/measured_counts; `transact(data, policy_path, digest, mutation, clock) -> Result<PublicationReceipt, &'static str>`; `AppState::publish_moderation_shadow(PathBuf, String, Mutation)` async retained owner. Add pure existing-candidate authority/match checks and read-only load_existing as specified in producer-review.md; runtime probe returns only an exact-candidate bool.

- [x] Add the domain/persistence/retained tests before production bodies. Pin legacy serialized policy bytes, explicit default-off, exact identity/source-version, severity <=600 seconds, ambiguity referrals, unique denominator, conflicting capture, expected revisions, independent actors, subject scope, malformed/capacity refusal and disjoint human counts.
- [x] Run the owning focused filters to observe missing API/behavior failures, then implement the bounded domain without transport/clock reads. Derive case ID only from immutable source version; errors preserve the original store.
- [x] Implement OwnedFile publication with mode-lock before shadow-lock, proof/policy checks after admission and before rename, directory-sync and exact bytes plus typed readback. Existing retry republishes AlreadyObserved without adding events. Add read-only load_existing with no create/chmod/lock and a retained exact-owned-candidate probe checking policy/digest/proof age before private load and before returning bool.
- [x] Observe write/sync/post-rename/late-proof failures, exact retry, malformed/nonprivate/symlink/oversized input, stopped existing recovery and no-existing zero metadata mutation. Block the real retained transaction, cancel/drop its waiter and request service shutdown/abort, then observe actual CommunityFilesystem join before freeze. Never substitute a dropped waiter for cleanup.
- [x] Run smoke filters `moderation::shadow`, `persist::moderation_shadow`, and `runtime::community_filesystem` with nonzero counts and exit 0; independently review the changed pure modules. Keep new APIs integrated in Task 3 before final warnings-denied qualification.

### Task 3: Native producer, four private leaves and real dispatch

**Files:** Root contextual producer and its `evidence.rs`; new `src/commands/modcase.rs` with evidence/rendering children; narrow registration edits in `src/commands.rs`, `src/main.rs`, `src/command_catalog.rs` and data; resolved config field in `src/runtime.rs`; native tests under `src/commands_help/dispatch_tests/modcase_native_tests.rs` and `retry_tests.rs`.

**Interfaces:** Actual private /modcall produces SourceVersion + fresh NewCase. Disabled/stopped/out-of-new-source scope invokes only the retained read-only existing probe; true alone may enter ordinary transact. /modcase show and appeal use A0 catalog admission plus exact native subject/staff proof; review and resolve_appeal use A2 plus native Delete Messages and independent actors. All use exact case ID and mutation revision.

- [x] Add actual recursive registered-dispatch tests before integration. Seed only through real /modcall with Guild/app/cached bot identity, String source snowflake and actual GET member/guild/channel/source responses. Observe missing command/production path failures; do not seed a ledger with boolean authority helpers.
- [x] Wire the current native producer: acknowledge privately before I/O; prove complete exact native IDs/roles/current overwrites/hierarchy and source identity; reload current owner policy; refresh all native facts/source version; construct NewCase. Preserve honest unsaved behavior when admission is off and the read-only exact candidate is absent. Unknown fails closed; exact existing current-owned match may reconcile through final transact without any new case.
- [x] Retain a bounded prospective case ID only after NewCase succeeds and before publication. Successful Changed and AlreadyObserved copy distinguishes new versus original existing provenance. Publication error/timeout gives the candidate ID without asserting record existence or failure; the retained owner may still finish. Do not print raw errors or source content.
- [x] Wire four private registered/catalogued leaves with exact current native facts before private case reads, final origin proof after source hydration, exact source for confident staff decisions, NeedsContext for unavailable/changed proof, expected revision, independent reviewer/resolver and subject-only original-origin appeal. Show returns only the exact authorized case; independently authorized staff counts never reach subjects. Protected private review remains allowed after Stop.
- [x] Run the native module filter with nonzero counts and exit 0. Verify actual capture→private delivery failure→Stop→exact retry preserves bytes/events; stopped/disabled missing ledger creates no operational/canonical paths; wrong identities/roles/overwrites/revision/origin/envelope, failed/held ack, source edits and independence refuse safely. Preserve canonical state/projection and no source plaintext; prove all non-GET requests are private callbacks/webhooks only.
- [x] Read printed human copy, check catalog/help registration and module-size limits, then review repaired native/pure cross-subsystem behavior. Keep native delivery uncertainty distinct from persistence post-rename fault evidence; do not claim they are one end-to-end native fault injection.

### Task 4: Independent complete-diff review and stable strict gate

**Files:** Preserve captured review/gate evidence outside source. Update the companion checklist and append dated corrections/receipt to `tasks/goals.md`; preserve existing historical receipts and parent O7 human/live gap labels.

**Interfaces:** Current HEAD + unchanged index + complete diff against HEAD + content manifest covering tracked and relevant untracked qualification inputs. Authoritative gate: `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`.

- [x] Inventory current writers and obtain handoff/freeze; capture HEAD/index/diff and complete qualification-input manifest outside shared source. Have Abbey Reviewer inspect the final complete diff, repairs, retained owner/privacy/policy interactions and every finding. Resolve confirmed blockers with attributable regression failures and focused suites.
- [x] Run `python3 scripts/check-pages-liquid.py`; run `npm --prefix activity test` only if Activity belongs to the final diff. Complete repo-required targeted module/privacy/catalog checks and applicable focused suites, retaining actual counts and ignored/skipped checks.
- [x] Run the strict authoritative gate with full log and actual exit status preserved. Set ABBEY_WDBX_REPO only if the required sibling is elsewhere. Compare complete before/after manifests; any changed input requires reconciliation, review and rerun against a stable source snapshot.
- [x] Write the exact source qualification receipt: HEAD, dirty-source fingerprint, manifest stability, gate exit, actual suite counts/skips, review findings/repairs/residuals. Append ledger evidence corrections and update checklist without rewriting history. Mark only the bounded source slice qualified; keep independent classifier agreement, measured live shadow pilot, human appeal usability after removal/access loss, installed identity, notices/activation and Linux/Windows runtime acceptance unverified/out of scope. Preserve proposal-only O7 and O2 defaults.


Source tasks complete through strict gate2 and independent review; these plan metadata changes follow that frozen snapshot. Final documentation-inclusive attribution is external. Parent human/live/operator tasks remain Partial as disclosed in the qualification receipt.
