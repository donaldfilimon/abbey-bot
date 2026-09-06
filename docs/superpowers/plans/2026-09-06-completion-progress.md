# Abbey Bot completion progress — 2026-09-06

This record tracks the request to finish the server-plan implementation, command-center modernization, and broader completion ledger. It preserves the full approved modernization scope. No overall completion is claimed.

## Checkout and delivery

The canonical Rust checkout is `~/dev/active/abbey-bot`. Another session committed and continued editing the server-plan engine while this session inspected it. This work therefore uses `codex/completion-20260906` in the sibling `abbey-bot-wt-completion-20260906`, based on `dac71a7`. Canonical `95d907f` was merged into the isolated branch as `149f157`; topic-preservation semantics were reconciled in `97c999b` and are under focused review. Final changes must still be integrated into canonical main. This session's worktree and branch must be removed after integration and before push/closeout under the machine policy.

The older `codex/command-center-20260904` branch is already an ancestor of main. Its uncommitted earlier catalog/help implementation remains preserved in its separately owned worktree. Current main supersedes its product behavior; restoring those old files would regress later changes. There is no reason to merge its committed history again.

Hosted baseline `2b916aad4e7d3861818a757e7b5d6900073cb763` passed all three Rust gate jobs in GitHub Actions run `34014242106`. This is baseline evidence only; it does not certify subsequent commits.

## Current verification

At `f5537eb`, `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` completed with exit 0 using the canonical sibling WDBX checkout. It passed formatting, deployment/Python/plist checks, privacy, contracts, TLS/RustSec debt checks, WDBX parity, synthetic Swift audio tests and release build, Clippy, 1,002 Rust tests with zero failures and two ignored live tests, and the locked Rust release build. The intermediate gate reused an existing Cargo target directory; the final closeout still requires the plan's fresh external target directory.

The built release binary was invoked with an empty credential/configuration environment. Missing `--json`, an invalid provider target, a missing server plan, and a missing voice-test output all returned exit 2. `--provider-self-test primary --json` with no configured provider returned exit 2 and one valid JSON object. No provider request or Discord connection was made.

The ignored local records are `.superpowers/baseline-strict-gate.log` and `.superpowers/compiled-cli-baseline.json` in this session's worktree. The retained four accepted dependency vulnerabilities are not a clean security audit.

## Requirement reconciliation

| Area | Current evidence | Remaining work |
|---|---|---|
| Server-plan engine | Canonical bot-member/blueprint fixes merged at `149f157`; `97c999b` preserves omitted topics and creates forum tags atomically; 68 server tests and all-target Clippy pass | Focused review and final integrated delivery validation |
| Task 1 specifications | All three approved design documents present and read | Preserve as binding requirements |
| Task 2 persistence | Existing `d000b41` / `2c8fee7` implementation and full-gate evidence cover truth table, canonical-before-projection ordering, atomic failure handling and reporting | Preserve authority during lifecycle/observability work |
| Task 3 compatibility | Existing acknowledgement, Entry Point, backend, tool, manifest, state and CLI tests pass | Closed by `5b06446` / `b002bd6`: controlled actual adapter ordering, literal consent fixture and README correction; focused tests, all-targets Clippy and independent review pass |
| Task 4 catalog/help | Existing `e39e13c` delivery, current parity and dispatch tests pass | Preserve during new surface registration |
| Task 5 memory/image menus | `4045ab1` implements shared card, memory USER menu, two image MESSAGE menus, bounded decoded attachment selection and registered-action privacy/state tests; independent review approved | Final integrated gate and live registration/interaction acceptance remain separate |
| Task 6 voice/admin UX | `12de8cc` / `ccdd6f7` / `aa65ffb` deliver member voice privacy, diagnostics, typed modes and the full dashboard; actual dispatcher/reset/export/provider-isolation tests and all-target Clippy pass; review approved | Preserve these boundaries during runtime/lifecycle work; final integrated gate and live acceptance remain |
| Task 7 router | `0fafed3` implements pure normalized scoring, strict compatible manifests, circuit/Retry-After policy and conversation-owned fallback; 114 focused tests and all-target Clippy pass | Independent review approved; production integration remains Task 8 |
| Task 8 provider runtime | Legacy production paths still separate | One runtime authority and conversation effect tracking, preserving legacy compatibility |
| Task 9 lifecycle | Persistence result exists; scheduler/connector ownership remains incomplete | Supervision, cancellation, serialized persistence and one bounded shutdown |
| Task 10 observability | Managed readiness/events/logging contract not implemented | Strict identity/schema/privacy, bootstrap failure channel, bounded JSONL and managed startup ordering |
| Task 11 installer | Current installer uses stable PID checks | Shared readiness checker and complete fake-launchd transaction matrix |
| Task 12 closeout | Some prior module splits exist | Size ratchet, remaining decomposition, compatible dependency refresh, documentation, final review, fresh strict gate, exact-head CI and integration cleanup |

WDBX episode integration remains proposal-only and default-off. Approval/execution events belong to the constitutional host; neither admin controls nor a successful proposal imply ledger authorization. The broader live acceptance ledger remains separate from these source tasks.

## Decisions

- Canonical `233b2df` deliberately treats omitted topics as unmanaged. Preserve a moderator's topic and re-parent safely; do not clear it merely because the plan omitted it.
- Preserve current implementations where evidence meets the approved contract; unchecked historical boxes do not justify duplicating working code.
- Follow the newer machine integration policy where the older implementation plan's branch/push sequence conflicts with it. Preserve independently owned worktrees.
- Use the 2026-09-04 provider specification over superseded router scoring/stickiness details.
- Complete source and fake-environment validation before preparing concrete provider, installation, Discord, connector, or consented voice acceptance. No individual voice agreement is inferred from the broad work request.

## Test artifact ownership

A concurrent checkout wrote into the initially reused Cargo target directory. Task 3 therefore forced a rebuild and verified the expected worktree test binary and exact new test counts. Subsequent work uses this isolated checkout's own target directory. The final gate still requires a fresh external target; no shared cached binary may substitute for exact-source evidence.

Task 5 validation: the full locked Rust suite passed 1,015 tests with two ignored before the final two registered-action fixtures were added. Those two fixtures and final all-targets Clippy passed afterward. This is not a claim that the full integrated suite has run at the new merged head.

## Expanded user direction at 03:21 EDT

The user explicitly requested all Rust/service modernization, better menus/UI/UX/logging, parallel brainstorming and implementation, builds and integration into main. The selected design is `docs/superpowers/specs/2026-09-06-guided-ux-operator-status-design.md`. Two additional plans cover guided help/private complete fact browsing and scoped statistics/operator status/recovery. They finish before the original Task 12 final checks. The new plans were selected under the user's delegated instruction to brainstorm and implement ideas; no separate user document review is claimed.

Task 6 review is approved at `aa65ffb`. The source fixes include the epsilon dashboard action, completed private permission-failure replies, actual reset/export/authorization tests, protected cross-guild state canaries, and independently counted fake provider endpoints. Task 7 pure provider policy is committed at `0fafed3` and independently reviewed. Task 8 runtime integration is preparing while guided help completes focused validation; compilation and commits remain coordinated across workers.

## Shutdown feasibility clarification

The binding service spec now records a narrow 2026-09-06 ruling under the user's
delegated service modernization instruction. The 20-second/four-stage policy is
a cooperative cleanup budget including abort, reap, required I/O, and runtime
waiting. Pinned Tokio does not cancel a started blocking filesystem operation.
Final outcomes distinguish completed persistence, an attempt that could not
start, and an attempt whose completion is unknown. An outstanding writer stays
owned and prevents a competing final write; the exceptional path uses an explicit
process-lifetime boundary with failure status instead of an indefinite runtime
Drop wait. Process exit is not claimed as successful joining or a universal OS
deadline. Whole-framework admission, consent writes, child processes, nested
voice work, and observability I/O are included. No live latency or durability
measurement has been performed. Implementation and review remain pending.

Guided-help Task 1 is committed at `8d5a06f8` and independently reviewed: conditional task buttons, exact invocation/visibility guidance, current permissions, fixed expiry, private responses and no browsing effects. Focused tests and all-target Clippy passed. Complete private fact browsing remains Task 2 of the added plan.

## Shared service protocol foundation

The read-only Python parser/private-file reader and shared service schema/corpus
are implemented as a partial Tasks 10/11 foundation. Twenty offline tests pass,
covering 215 document vectors and 28 freshness vectors; independent review
approved the source after cleanup and symlink-test fixes. Removing no-follow in
a review-only mutation causes six meaningful test failures. Rust conformance,
continuous readiness, checker CLI, installer transactions, gate wiring and live
service acceptance remain outstanding. Bootstrap has exactly six keys in the
binding specification; an earlier ignored supplemental count was corrected.

The optional baseline reader now returns absence only after validating every
parent and observing ENOENT at the fixed leaf. Missing parents, unsafe types,
malformed content, I/O errors and cleanup failures cannot become an empty nonce
baseline. An independent reviewer reran all 20 tests successfully.

Remote main was freshly fetched at `cf22506`; the canonical checkout is clean
at that commit. It adds constitutional memory-proposal adapters, per-guild gate
coverage, checkpoint admission and gateway deployment source beyond our last
canonical merge. Integrate that committed work after the provider runtime's
clean commit. Its live-operation ledger belongs to the other session and is not
new acceptance evidence from this modernization run.

The service specification now explicitly supplies pre-transaction nonce and
absolute deadline evidence to the unchanged three-argument checker via bounded
private stdin. It preserves shell rollback ownership, includes PID acquisition
in the same readiness budget, and requires current launchctl sampling. The
checker and fake installer implementation remain pending.
