# Learning Task 5: source evaluation receipt

Status: Partial. The frozen synthetic corpus, production offline evaluator and
atomic nonfinite DQN import guard are implemented. Independent review and the
fresh combined strict gate are pending; no installed or live acceptance is claimed.

## Scope and evidence

- Schema 1 corpus: 100 distinct cases, 20 each supported, unsupported,
  contradictory-source, correction and empty-retrieval. Exact SHA256:
  `20237fe9320d68dd1ff60af329c308ec7f67322817e9cf86087fabd308b0fb0f`.
- Labels are agent-authored synthetic inputs, pending human adjudication.
  They are not a model judge or provider answer-quality measurement.
- `--learning-quality CORPUS.json --json` runs before runtime, credentials or
  configured state initialization, caps regular-file input at 1 MiB and emits
  only counts, declared provenance and corpus/evaluator-source hashes.
- Corpus result: TP 30, FP 0, TN 70, FN 0. Supported 20 accepts; unsupported,
  contradictory and empty-retrieval classes each 20 abstentions; correction
  class 10 accepts and 10 abstentions. Correction admission: 6 Repair, 14 Ignore,
  zero mismatches. Seed 1369907238, topology [18,64,32,3], untrained synthetic
  action distribution [stay,reply,react] = [92,5,3].
- Regression-first evidence: direct snapshot import accepted NaN epsilon;
  `cargo test --locked brain::quality_evaluation::nonfinite_snapshot_import_is_atomic
  -- --exact` exited 101 with 0 passed, 1 failed. The fix rejects nonfinite
  epsilon, layer weights/biases and replay reward/state values before mutation.
- Focused final checks with Rust 1.98.0 and CARGO_BUILD_JOBS=2: `cargo test
  --locked brain` exited 0 (239 passed); `cargo test --locked grounding` exited 0
  (42 passed); `cargo test --locked learning_quality_cli` exited 0 (2 passed).
  Brain coverage includes deterministic seeds, atomic topology/nonfinite import,
  scoped snapshot rollback/reset preserving canonical facts and the other scope.
- Python syntax, privacy, Rust module size, Liquid and diff checks exited 0.
  Main's additive CLI dispatch is 870 lines and requires independent review.

## Ownership and limits

New: `src/brain/quality_evaluation.rs`, `src/learning_quality_cli.rs`,
`tests/fixtures/learning-quality-v1.json`,
`scripts/test-learning-quality-startup.py`, and this receipt.
Modified: finite validation in `src/brain/dqn.rs`; additive module/startup
registration in `src/brain/mod.rs`, `src/main.rs`, `src/startup.rs`; named Python
startup gate in `check.sh` and `check.ps1`; corpus LF rule in `.gitattributes`;
README's offline-evaluation section. Existing dirty changes were preserved.

No policy weights/rewards/actions were tuned. No erasure production logic,
canonical fact authority, deployment, service, credentials, Discord or provider
operations were changed. No commits, pushes or dependencies were added.

Source identity: HEAD `e742375ff9b3d20b97df09c7fad10c1225ae957c` plus dirty files.
The report's evaluator-source hash covers its listed compiled pure dependencies
and CLI source; it is not a whole-checkout or installed-artifact identity.
The strict gate must be accompanied by a stable complete before/after manifest.

Operator-reviewed fixed-provider answers and manually adjudicated support on
this same corpus remain required before tuning. Independent human corpus review,
installed identity and Discord/runtime acceptance remain open.


## 2026-10-03 fix3 source qualification

Current source qualification supersedes the pending source-review/gate status in
the initial receipt above; the initial evidence remains historical. Task 5 is
**Partial overall** because human corpus adjudication and fixed-provider/manual
answer support remain open. No tuning is admitted by the synthetic measurements.

The bounded production evaluator and actual offline CLI passed fresh independent
spec and standards review, with zero open source findings. The fix3 review closed
LQ5-1 (invalid replay action), LQ5-2 (reused-agent replay append), and LQ5-3 (public
corpus preflight). Earlier review fixes bound dotted-version tokens before
grounding expansion and validate the opened regular-file handle; Unix opens are
nonblocking, so a substituted FIFO cannot wait for a writer. Public evaluation
requires 100 cases and all five classes at 20 each before case evaluation.

Snapshot import validates topology, layer shape, finite values and every action
before mutation, including width-mismatched replay rows. Successful import replaces
replay, preserving destination capacity and newest compatible entries; empty or
legacy-absent replay clears prior transitions. Legacy width mismatches are skipped
only after finite/action checks. Snapshots retain their existing semantics: target
synchronizes to online and destination RNG remains; no full training-process
snapshot is claimed. Full-state refusal tests include a trained lagging target,
wrapped replay exceeding the exported 1,000-row tail, cursor/capacity and RNG.
Task 4 erasure production code was not changed by this Task 5 repair.

Evidence is preserved under
`.superpowers/sdd/2026-10-01-mlai-learning-quality/task5-quality-evidence-20261003/`:
`fix3/` contains contemporaneous scoped pre-images, RED/GREEN logs and review diff;
`fix3-root-run/` contains complete gate manifests, terminal result and strict log.
The fresh [independent fix3 review](../../../wdbx/docs/reviews/2026-10-03-bot-quality-fix3.md)
passes both axes. Four import regressions and one public-corpus regression failed
with exit 101 before repair; final focused brain tests passed 242 and CLI unit
tests passed 5, with zero failures/ignores. Formatting, warnings-denied Clippy and
diff check exited 0. The original named corpus/held-out/rollback tests pass, but
individual historical RED runs for those three tests were not retained and are
not retrospectively claimed.

The parent-owned strict command
`CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited **0**
(session 32980). Rust 1.98.0 and Python 3.14.8 were selected; system compiler tools
remained ahead of alternate shims. Exact results:

- Rust: **2,087 passed, 0 failed, 8 ignored**, 106.35 seconds. Ignored live/special
  tests are not qualified by this run.
- Python unittest summaries: **339 cases** = 333 before Swift + 1 actual text
  benchmark startup + 5 actual learning-quality startup tests. Separately, the
  publication checker reported 16 scenarios; standalone named checks are not
  silently added to the unittest count.
- Swift: **12 + 16 tests passed**, followed by the offline audio-tap release build.
- Required pinned WDBX conformance, warnings-denied Clippy, formatting, privacy,
  instruction, module-size, Liquid, shell syntax (11 scripts plus smoke driver)
  and plist lint (6) passed. Locked bot release build finished in **2m 25s**.
- Accepted RustSec debt remains **5 vulnerabilities and 4 informational advisories**
  (3 unmaintained, 1 yanked); the audit is not clean. The log retains nonfatal
  macOS unwind-size and rust-objcopy/libLLVM debug-stripping warnings.

The actual release-process learning CLI tests passed all five cases, including
closed malformed/oversize refusal, FIFO refusal, dotted-token refusal and
false-positive failure exit. The emitted synthetic report remains TP 30 / FP 0 /
TN 70 / FN 0, corrections 6 Repair / 14 Ignore / 0 mismatches, seeded untrained
actions [92,5,3]. Its evaluator-source SHA256 is
`366cb3e7575c0f37ffb3322c3088674d6be455bf596a02af0ff775542783af5d`;
the frozen corpus SHA remains the value recorded above. These are lexical policy
and action counts, not provider quality or factual-confidence probabilities.

Complete gate identity: **799 files**, unchanged before/after, no changed files;
HEAD `e742375ff9b3d20b97df09c7fad10c1225ae957c`; aggregate source SHA256
`fc1cc1bb6e5c81457638dede4e712dd5c5a627127d4f51a4364189843677a457`.
The index hash was unchanged. The original pre-worker preservation baseline was
reconstructed, as previously disclosed; this stable gate does not retrospectively
prove every incoming dirty byte was preserved. Ignored evidence is outside the
799-file manifest.

Read-only post-gate hashing confirms candidate `target/release/abbey-bot` SHA256
`f4d76cd9e141b5ad04cba7fc9965d4a05b8b7359933d77a66d65655c1805582a`
differs from installed binary SHA256
`b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`.
This candidate is not proven installed or running. Parent-reported service-ready
observations concern the existing installation, not fix3 qualification. No deploy,
restart, live provider/Discord action, commit or push occurred in this slice.

This appended receipt, MLAI entry, progress entry and source-qualified plan checks
are **documentation deltas after the frozen gate**, verified separately by Liquid
and diff checks. They do not re-label the gated manifest as a hash of the later
documentation state. Human review of all 100 synthetic labels, operator-reviewed
fixed-provider answers/manual support before tuning, and installed/live acceptance
remain OPEN.
