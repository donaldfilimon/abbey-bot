# Activity T4 Source Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Qualify immutable Court source packages and inspectable dry-run ingress/rollback metadata while preserving the open public Activity acceptance work.

**Architecture:** One standard-library Python helper packages only the existing four Court assets and pins the exact observed Node executable. Three test scripts exercise package policy, actual ingress validation and actual direct-child lifecycle; existing Node Court tests and the strict bot gate qualify the unchanged source slice.

**Tech Stack:** Python standard library, existing dependency-free Node/Court, existing POSIX/PowerShell gates; no new dependency.

**Spec:** [source design](../specs/2026-10-03-mlai-activity-source-package-design.md); parent repository Activity spec/plan Task 4. Proposed saved plan: `docs/superpowers/plans/2026-10-03-mlai-activity-source-package.md`.

## Global Constraints

- Root is the sole repository writer; all other agents are read-only or external-only. Preserve every shared edit/index; no checkout, worktree, staging, commit, push, PR or service action.
- Copy only `app.js`, `court.js`, `index.html`, `server/court.mjs` in that digest order; asset bound 4 MiB and manifest bound 16 KiB.
- Node probe: exact `--version`, 128-byte stdout, 3-second deadline, PATH-only environment, EOF stdin, observed direct-child kill/wait; no shell, live provider or configured service.
- Ingress is proposal-only: `127.0.0.1:8791`, body 1,024 bytes, 256 connections, all four deadlines 5,000 ms, exact closed routes, no access/query/body logs.
- Package status is `prepared-source-only`, activation is `operator-required`, rollback is dry-run with `actions_executed=[]`; no readiness or human receipt is inferred.
- No production dependency, TLS/certificate/proxy installation, DNS/public host, Portal change, launchctl, deployment or Discord mutation is included.
- Keep the existing Activity invitation readiness gate and all public/human/installed/platform acceptance rows open; preserve historical ledger entries and append corrections.

## Review Focus

- Per-label DNS bounds can admit an unusable selected origin despite a short whole URL — actual 64-character RED in Task 1.
- macOS temporary aliases can cause unrelated symlink refusals — resolve only owned fixture roots in Task 1.
- Refusal can leave a directly spawned version producer alive — actual OS child observations in Task 2, before fallback cleanup.
- Existing lock/release/receipt replay can overwrite somebody else's evidence — exact-byte/mtime and held-owner checks in Task 3.
- Local packaged health can be mislabeled as public iframe readiness — source-only docs and unchanged readiness boundary in Task 4.

---

### Task 1: Import immutable packaging and repair actual DNS-label refusal

**Files:** Create `scripts/prepare-court-release.py`, `scripts/test-prepare-court-release.py`, `scripts/test-court-release-dns.py` from the reviewed external drafts. Save the reviewed source spec and this plan at their proposed repository paths.

**Interfaces:** Produce `prepare(source, output, node_path, ingress=None) -> Path`, `verify(release, current_node=True) -> dict`, `default_ingress() -> dict`, `validate_ingress(value) -> dict` and `PackageError` exactly as the spec/drafts; preserve manifest and Court digest separation.

- [x] Import the implementation and two owning test files under coordinated ownership. Resolve `PackageTests.setUp` root with `Path(self.temp.name).resolve()` and real-host `root` with `Path(folder).resolve()`; retain production symlink checks.
- [x] Run `python3 scripts/test-court-release-dns.py OriginDnsLabelTests.test_64_character_dns_labels_are_refused -v` before the validator repair. Retain the attributable assertion failure for `"a" * 64 + ".example"`, rather than an import/path error.
- [x] Repair `validate_ingress` with dot-separated label validation: length 1–63, alphanumeric ends, only internal hyphens. Preserve origin length 253, HTTPS-only root, IP/local/credential/port exclusions and every other exact proposal field.
- [x] Run `python3 scripts/test-court-release-dns.py -v`; expect four nonzero methods, no failures, including valid 63-character and internal-hyphen controls. Leading/trailing/empty labels are preservation controls; do not claim they were the earlier defect.
- [x] Run `python3 scripts/test-prepare-court-release.py PackageTests -v`; retain actual results for the 16-method package policy inventory: fixed allowlist, drift, canonical identity, immutable inventory, symlink/unsafe output, runtime replacement and replay ownership. Repair only attributable defects.

### Task 2: Qualify actual Node probe ownership and packaged Court execution

**Files:** Create `scripts/test-court-release-node-probe.py`; modify the imported preparer/original test only if actual regressions require a repair. Consume existing `activity/server/court.mjs` and its current tests without changing protocol.

**Interfaces:** Consume Task 1 `prepare`/`verify`; exercise actual `inspect_node(path) -> {path, bytes, sha256, version}` and actual `createCourtServer()` from the packaged module.

- [x] Import the six-method probe sidecar unchanged. Its owned Python shebang executables are real OS children, use resolved platform temp paths, and record subject wait/closed stdout/status before fixture fallback cleanup.
- [x] Run `python3 scripts/test-court-release-node-probe.py -v`; retain actual timeout, 129-byte output, status 17, malformed/non-ASCII/trailing output, self-byte drift and EOF/no-inherited-environment results. Correct real defects with attributable failing evidence; do not manufacture failures for already-correct behavior.
- [x] Run `python3 scripts/test-prepare-court-release.py RealNodePackageTest -v` with the existing selected Node (optional explicit `ABBEY_COURT_TEST_NODE` path). Require this Mac test to execute: packaged module only, ephemeral loopback port, health protocol 2 and digest/header, exact static bytes, synthetic POST totals/header, observed child termination. No configured 8791 or installed process is touched.
- [x] Run `npm --prefix activity test` and record the actual current Node identity, suite counts and terminal status. A missing/unsupported Node or skipped real package test leaves compatibility open and cannot qualify this slice.
- [x] Independently review bounded capture/deadline, direct-child ownership, exact runtime byte binding and distinction between local artifact proof and public readiness.

### Task 3: Verify dry-run rollback, wire all tests and document commands

**Files:** Modify `check.sh`, `check.ps1`, `activity/README.md`, `docs/activities.md`; consume all four new scripts.

**Interfaces:** Consume `rollback_plan(candidate, previous, receipt) -> dict`; CLI subcommands remain `prepare --source --output --node [--ingress]`, `verify RELEASE`, `rollback-plan --candidate --previous --receipt`.

- [x] Run the original named methods `PackageTests.test_rollback_binds_exact_verified_artifacts_and_receipt_replay_is_read_only` and `PackageTests.test_same_artifact_rollback_is_no_change_and_missing_or_tampered_prior_refuses`. Verify exact package/Node/path binding, `actions_executed=[]`, no-change, idempotent receipt bytes/mtime, tamper/missing-prior refusal and preservation of a conflicting receipt.
- [x] Add explicit POSIX gate calls for `scripts/test-prepare-court-release.py`, `scripts/test-court-release-dns.py`, `scripts/test-court-release-node-probe.py` near the existing Activity URL-map tests. Add each matching `Invoke-Checked -Executable "python" -Arguments @("scripts/<same-name>.py")` to `check.ps1`. Python syntax discovery alone is insufficient execution wiring.
- [x] Document separate source-only preparation/verify/rollback commands with explicit operator-owned absolute output paths and observed `release-<manifest-sha256>` results. Distinguish this directory identity from the served Court digest and a prepared prior source copy from active installed identity. Do not run service/TLS/Portal operations.
- [x] Print/read the resulting prose; require source-only/proposal-only/dry-run labels and original-plist/installed-assets preservation preconditions. Keep invitation readiness and anonymous ballot limitations unchanged.
- [x] Run all three Python test scripts separately, retaining each status/count/skip, then `python3 scripts/check-pages-liquid.py`. The reviewed inventory is 17 + 4 + 6 = 27 methods before platform skips; replace that inventory with observed results in the receipt.

### Task 4: Independent final review and stable strict source qualification

**Files:** Append `docs/MLAI-LIVE-ACCEPTANCE.md` and `tasks/goals.md`; update `tasks/todo.md`. Preserve external review evidence and full logs outside shared source.

**Interfaces:** Consume all prior results; produce an exact source qualification receipt, never a publication/readiness record.

- [x] Obtain independent review of the complete current shared diff and all relevant untracked inputs, including new scripts, test/gate wiring and docs. Reconcile confirmed defects and obtain focused regression/rereview evidence before freezing inputs.
- [x] After final source/docs changes, run current `npm --prefix activity test` and `python3 scripts/check-pages-liquid.py` separately. Record actual status/counts and real Node execution/termination; older Court or Community gates do not qualify these additions.
- [x] Append scoped evidence corrections/checklist residuals before the gate, then capture HEAD, unchanged index and complete tracked/relevant-untracked input manifest outside the source. Run `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` with full log and actual exit status. Set `ABBEY_WDBX_REPO` only if the required sibling is elsewhere; never hide the gate status behind a pipe.
- [x] Compare complete pre/post manifests. A changed input requires reconciliation, review and rerun. Require green terminal gate and no unresolved blocking source findings; record actual Rust/Python/Node counts, accepted debt, ignored/skipped checks, repaired DNS finding, Node identity and proof gaps.
- [x] Publish the external qualification receipt bound to that frozen fingerprint. If repository receipt/checklist text must change afterward, record it as a later correction and requalify the changed complete inputs. Report **source-qualified only**; keep HTTPS/public ingress, host/operator/service/installed, Portal/iframe/two-client/recovery/restart/two-instance and Linux/Windows acceptance open.

## Self-review and handoff

All package, probe, ingress, rollback, integration, gate and evidence contracts in
the companion spec map to the four tasks above. Each task has a bounded owning
check; source-only internal work is already within the requested execution scope.
Root applies and tests sequentially, with independent reviewers reading the
complete result. This external plan does not authorize or claim any later public,
service, Portal or human action, and does not invoke skill worktree/commit defaults.


Source tasks complete through strict gate2 and independent review; these plan metadata changes follow that frozen snapshot. Final documentation-inclusive attribution is external. Parent human/live/operator tasks remain Partial as disclosed in the qualification receipt.
