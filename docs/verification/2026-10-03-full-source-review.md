# Full shared-source review and qualification — 2026-10-03

**Current: repaired source qualified.** Independent review has no unresolved
blocking findings. The repaired strict gate exited 0 against an unchanged
800-input snapshot. This receipt and the companion ledger/checklist updates are
subsequent documentation inputs; their complete-tree confirmation is recorded in
the external [final qualification receipt](/Users/donaldfilimon/.codex/verification/abbey-bot-20261003-0726/qualification.md)
and [gate3 result](/Users/donaldfilimon/.codex/verification/abbey-bot-20261003-0726/gate3-result.json).
Those external records avoid a self-referential source fingerprint. Final
complete-tree qualification requires their green gate and unchanged manifests.

## Identity and preservation

- Canonical checkout: `/Users/donaldfilimon/dev/active/abbey-bot`, branch `main`.
- HEAD: `e742375ff9b3d20b97df09c7fad10c1225ae957c`.
- Original and repaired-gate index SHA256:
  `83c7456df6d4a61d3fd6e822b7a44f166e55cafc9093775c0a5f433efb79e28f`.
- Repaired gate: 800 tracked and nonignored untracked inputs; aggregate SHA256
  `6bb0d2d1dcfb1bff572d844b4749692c899b1cd96146f1f865bbcab3afcc0ac9`.
  Complete before/after manifests are identical, including modes and paths.
- Required external WDBX projection fixture SHA256:
  `a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90`.
  Cross-repository parity passed; no missing-fixture skip was used.
- Rebuilt candidate binary SHA256:
  `ef99768f7c0b8360e6b4bb0f853acd93c7c7fc4a1086b2fe27ad1261227dd62d`.
  This identifies the local candidate only; installed identity was not checked.

Before edits, the complete binary diff against HEAD, staged diff, exact index
hash, manifest and all 24 initial nonignored untracked files were preserved
outside the shared source. Active chat/process ownership was inventoried; no
overlapping source writer was observed. Root was the sole source writer, with
read-only independent reviewers. No checkout, reset, staging, commit, push, PR,
worktree, dependency addition or deployment occurred.

Evidence directory:
`/Users/donaldfilimon/.codex/verification/abbey-bot-20261003-0726`.
It contains `baseline.json`, `baseline.patch`, `index.patch`, `untracked.tar.gz`,
`repairs.patch`, complete gate logs/manifests/results, focused RED/GREEN logs,
review reports and the execution ledger. Incoming changes remain preserved.

## Review and repairs

Six sequential Abbey Reviewer passes covered generation/provider lifecycle;
learning/persistence/erasure; engagement/scheduling; voice; commands/forum
authorization; and gates/documentation. Cross-subsystem review reconciled all
100 tracked-diff paths plus 25 untracked paths, including the approved plan.
The [fresh final review](/Users/donaldfilimon/.codex/verification/abbey-bot-20261003-0726/final-review.md)
independently reconciled that inventory and reviewed every repair and its caller
paths. Production modules above 800 lines received explicit review. Final
documentation additions receive a separate review in `documentation-review.md`.

| Finding | Trigger and expected behavior | Repair and regression |
|---|---|---|
| G1, P2, closed; `src/generation/stream_delivery.rs` | Personal-memory authorization changes after an accepted streamed preview. Replace the identified preview with static context-change guidance and observe producer cleanup. | Route trusted ContextChanged progress failures through terminal replacement. Preserve other delivery errors and uncertainty without retry. Two retained-production-path regressions cover successful and uncertain replacement, joins, permits, telemetry and absence of transcript/reward commits. |
| L1, P2, closed; `src/brain/reward/recovery.rs` | Canonical pending action 3 or usize::MAX previously passed recovery validation and could reach invalid DQN indexing. Reject malformed state atomically. | Validate all pending actions through BotAction::from_index before restore/load/publication. One canonical regression covers tracked and legacy rows, unchanged state/bytes on rejection, and valid actions 0/1/2 through save/reopen/settlement. |

Root changed only those two production files and their existing test modules,
`src/generation/retained_tests.rs` and `src/persist/tests/reward_recovery.rs`,
beyond the incoming behavioral work. Public APIs, persistence format, transport
boundaries and production dependencies are unchanged. No actionable deferred
minor finding remains; cosmetic and unrelated refactoring is outside this task.

Attributable RED evidence: L1 exit 101 accepted invalid legacy action 3;
G1 exit 101 had zero replacement edits in both regressions. Final focused checks
all exited 0: withdrawal 2, action-bound 1, generation 61, reward recovery 9,
brain 242. These filters overlap and are not additive unique suite counts.
An initial telemetry type error, a missing expected rendered prefix and a
canonical-lineage fixture mistake were corrected; they are not additional
behavioral RED evidence. The obsolete owned compile was terminated before the
corrected run; no other project's process was stopped.

The printed and reviewed replacement text is:

> **Abbey** — The context for this answer changed while I was checking it. Please ask again.

## Repaired strict gate

Command: `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`.
Actual exit: **0**, terminal session 14918. Full log: `gate2.log`; terminal exit:
`gate2.exit`; identity/counts: `gate2-result.json`; complete manifests:
`gate2-before.json` and `gate2-after.json`, in the external evidence directory.

- Rust: **2,090 passed, 0 failed, 8 ignored**, 78.27 seconds.
- Python: **339 unittest cases**: 333 before Swift, 1 actual rebuilt text
  benchmark startup and 5 actual rebuilt learning CLI startup cases.
- Provider qualification publication: **16 scenarios**, counted separately.
- Swift: **12 AudioTap runtime + 16 core tests**, synthetic PCM only.
- Format, deployment/privacy/contracts/instructions/security/plist/shell/Python
  syntax, Liquid docs, required WDBX parity, offline macOS audio build,
  warnings-denied Clippy and locked release build passed. Release: 4m41s.
- Offline text startup emitted 48 successful synthetic probes from 60 localhost
  calls. Learning startup emitted the bounded 100-case corpus report with
  explicit agent-authored-label provenance. Neither used a live provider.
- RustSec is **accepted debt**, not clean: 5 vulnerabilities and 3 informational
  unmaintained advisories. Historical receipts listing 4 informational advisories
  remain historical; the current locked dependency inventory removed the yanked
  informational entry. No new production dependency was introduced here.
- A nonfatal macOS linker unwind warning is retained in the log. There were no
  gate-level missing-fixture skips. Activity source is absent from this reviewed
  diff, so `npm --prefix activity test` is not applicable; docs Liquid passed.

The eight Rust exclusions are the command-payload operator export, two episode
gateway/config acceptance tests, live DM, live FM CLI, and three personal-memory
subprocess child helpers. Parent subprocess tests exercise the child helpers;
their direct ignored status is not an omitted live acceptance claim.

## Evidence corrections and remaining acceptance

Learning Task4 erasure/reset source and Task5 evaluator/import/offline CLI source
are included in this complete review and new stable gate. Task4's earlier
2,064-test command exited 0 but its 793→795 input drift prevents stable-source
attribution. The later stable combined 799-file/2,087-test Task5 receipt included
the reviewed Task4 source. Neither historical fingerprint identifies this repair.
Ledger corrections are appended, preserving all historical receipts.

**Partial overall:** all 100 synthetic labels still require independent human
adjudication; fixed-provider answers and manually adjudicated support are required
before tuning. Installed artifact identity, managed-service readiness, provider
quality/latency, live Discord/Activity, platform runtime and human voice acceptance
remain unverified here. Linux and Windows were not exercised. Future Continuity,
native optional-task Initiative, their erasure hooks and the watcher decision
remain separate program obligations.

Review scope is complete change-inventory review plus repaired caller paths;
it does not claim a fresh line-by-line reread of every unchanged repository input
or proof against every possible race. Source qualification alone is the result.
