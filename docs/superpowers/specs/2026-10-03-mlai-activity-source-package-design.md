# Activity T4 Source Package Design

Status: Authorized source slice of parent Activity Task 4, 2026-10-03; qualification pending. Root is the
sole repository writer. This document synthesizes the reviewed external
`design.md`, `HANDOFF.md`, `review.md` and `sidecar-handoff.md`; none of their draft
tests or historical host observations become current verification here.

Parent: repository `docs/superpowers/specs/2026-10-01-mlai-activity-launch-design.md`
R3/R6/R7 and `docs/superpowers/plans/2026-10-01-mlai-activity-launch.md` Task 4.
Proposed repository destination:
`docs/superpowers/specs/2026-10-03-mlai-activity-source-package-design.md`.

## Scope and ownership

Implement and qualify an immutable source package, exact local Node identity,
finite ingress proposal and dry-run rollback helper. Use Python standard library
and the existing Court implementation; add no production dependency. Preserve
the current dirty checkout, index and historical receipts. No branch/worktree,
stage, commit, push, PR, deployed configuration or service change is included.

Create exactly these four scripts:

- `scripts/prepare-court-release.py`: preparation, verification and rollback CLI.
- `scripts/test-prepare-court-release.py`: 16 package/rollback policy methods and
  one real packaged-Node loopback integration method.
- `scripts/test-court-release-dns.py`: four direct ingress validator methods.
- `scripts/test-court-release-node-probe.py`: six actual direct-child probe methods.

Modify `check.sh` and `check.ps1` to invoke all three test scripts explicitly.
Update `activity/README.md` and `docs/activities.md` with exact source-only commands
and limits; append dated corrections to `docs/MLAI-LIVE-ACCEPTANCE.md` and
`tasks/goals.md`, and update `tasks/todo.md` without closing public/human rows.
No command catalog/readiness change is needed.

## Package and identity contract

The only copied assets, in existing Court digest order, are `app.js`, `court.js`,
`index.html`, `server/court.mjs`. Each asset is a regular file bounded to 4 MiB.
Do not copy OAuth examples, environment files, secrets or unrelated source.
Read all assets again before publication or successful replay. Refuse symlink
paths, output inside source, missing/oversized/changed assets and unsafe output.

`prepare(source: Path, output: Path, node_path: Path, ingress: dict | None = None)
-> Path` publishes `release-<manifest-sha256>` below an explicit owner-only output.
Use an exclusive short `.prepare.lock`, preserve another owner's lock and bind
cleanup to the created lock identity. Sync owned staged bytes/directories before
rename; release files are readonly and release directories are readonly on POSIX.
Existing matching release is verify-only replay with unchanged bytes/mtime.
Refused preparation cleans only its own unpublished stage. The output boundary
is an owner-only, cooperative directory, not an adversarial same-UID capability.

`release-manifest.json` is canonical sorted compact JSON with a final newline,
maximum 16 KiB, closed keys: `schema=1`, `status="prepared-source-only"`,
`court_protocol=2`, `court_digest`, ordered `assets` path/bytes/SHA256 rows, `node`,
`ingress`, `preparer_sha256`, `activation="operator-required"`. Manifest SHA256
identifies the directory. Court's existing four-asset digest identifies the
advertised HTTP version; these identities are distinct. The preparer hash is
self-attestation, not cryptographic producer authentication.

`verify(release: Path, current_node: bool = True) -> dict` checks canonical bytes,
manifest/directory identity, exact fixed file inventory and only `.`/`server`
directories, readonly regular assets, asset digest and current exact Node bytes.
Reject duplicate/nonfinite JSON, unknown keys, extra files/directories, writable
artifacts, tamper and runtime replacement. Public CLI verification retains
`current_node=True`; tests cannot substitute skipped Node identity for acceptance.

## Node observation contract

`inspect_node(path: Path) -> dict` resolves a regular executable of at most
256 MiB, hashes it before/after, and runs only `[resolved_path, "--version"]`.
Use EOF stdin, suppressed stderr, explicit PATH-only environment, no shell or
inherited provider/proxy variables, at most 128 stdout bytes and a 3-second
deadline. Accept only ASCII `v<positive-major>.<minor>.<patch>` with successful exit
and unchanged executable bytes. Record `path`, `bytes`, `sha256`, `version`.

On refusal, kill a still-running direct child, observe wait, and close stdout.
Do not claim process-group/descendant cleanup. Unsupported pipe readiness is a
closed failure; POSIX-only tests/skips do not qualify Windows runtime behavior.
The chosen existing Node must also run the actual packaged Court integration;
a syntactically valid version alone does not prove application compatibility.

## Finite proposal contract

`default_ingress() -> dict` and `validate_ingress(value: dict) -> dict` describe
only `status="proposal-only"`, backend `127.0.0.1:8791`, body 1,024 bytes,
256 connections and 5,000 ms request/header/socket/absolute-body deadlines.
GET routes are `/`, `/index.html`, `/app.js`, `/court.js`, `/health`; POST routes
are `/court`, `/.proxy/court`. Access/query/body logging is false and the deployed
digest header passes through. Unknown/widened keys, routes, limits or log toggles
refuse. Preserve the generic Court default port 8790; no configured 8791 process
is contacted or changed by this proposal.

`tls.origin` is null or a canonical lowercase HTTPS DNS root, no credential,
explicit port, IP/local host, path, query or fragment; keep the existing maximum
origin length of 253 characters. Every dot-separated label is 1–63 ASCII letters,
digits or internal hyphens with alphanumeric ends. Require at least two labels,
and preserve numeric/internal-hyphen/63-character valid controls. The confirmed
draft gap is the 64-character label; hyphen-edge examples in the initial review
were already refused and are protective controls, not fabricated RED evidence.
No DNS lookup, certificate/proxy file, HTTPS host operation or publication occurs.

## Rollback and source acceptance

`rollback_plan(candidate: Path, previous: Path, receipt: Path) -> dict` verifies
both exact packages and writes canonical `court-rollback-dry-run` metadata binding
their paths, manifest hashes, Court digests and Node hashes. Identical packages
produce `no-change`; otherwise `operator-review-required`. `actions_executed=[]`
always. Use owned synced staging plus exclusive hard-link receipt publication;
identical receipt replay is read-only and different/unknown existing receipt
refuses. Describe existing label `com.donaldfilimon.abbey-court` and loopback
8791 health without executing launchctl or HTTP.

Before any later activation, separately retain the exact original plist and
installed four assets, authorize operator retarget/start, observe stop/start and
health identity, and account for in-memory ballots resetting under a new epoch.
A read-only prepared prior directory is source inventory, not proof of active
service identity or operational rollback readiness.

Resolve owned macOS temporary roots in both original fixture classes before
testing: `Path(self.temp.name).resolve()` and `Path(folder).resolve()`. This removes
`/var` alias noise without relaxing production symlink refusal. Direct-child
sidecars already resolve their owned root. Source acceptance requires actual
package/DNS/probe results, the real packaged host at ephemeral loopback with
matching health/static/synthetic POST digests and observed termination, existing
`npm --prefix activity test`, Liquid, independent complete-diff review and a
stable `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` against unchanged inputs.
The 27 draft methods are an inventory, not a pass count. Retain actual counts,
exit status, logs, skips and source/Node identities; a skipped real Node test on
this Mac leaves package compatibility unqualified.

TLS/public host selection and finite ingress installation/receipt, public API
reachability, Portal mapping, real iframe READY, two participants, separate
instances, disconnection/malformed/stale/restart witnesses, managed-service and
installed identity, Linux and Windows qualification remain open. Do not populate
`ABBEY_ACTIVITY_READINESS_FILE`, `iframe_receipt` or `shared_receipt` from package
metadata, local fixtures or HTTP health. Keep `/engage invite kind:activity`
disabled unless its existing independent public/human readiness contract passes.
