# WDBX training foundation implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Build the real WDBX-backed record, snapshot and receipt foundation required for actual Abbey, Aviva, ABI and Quesar weight training.

**Architecture:** Deterministic contracts live in ABI's store-agnostic abi-ai crate. WDBX persistence and private artifact publication live in abi-sea, which owns infrastructure. This first independently testable unit precedes trainer and serving plans and does not claim trained models.

**Tech Stack:** Existing Rust workspace, serde/serde_json, SHA-256 and abi-wdbx DurableStore; no optimizer or model dependencies in this unit.

**Spec:** `/Users/donaldfilimon/dev/active/abbey-bot/docs/superpowers/specs/2026-09-29-wdbx-persona-training-design.md`

## Global Constraints

- Work only in canonical checkouts; preserve unrelated changes and inspect ownership before writes. No worktrees, source copies or branch switches.
- ABI commands use `./tools/cargo.sh`, dependency resolution uses `--locked`, and tests use `< /dev/null`.
- Never open the live `~/.abi/` store. All persistence tests use temporary private directories.
- abi-ai remains deterministic and store-agnostic. Never add ABI/WDBX crate dependencies to Abbey-bot.
- Preserve existing CLI commands, MCP tools, goldens, and unchanged-weight disclosure. This unit adds library contracts only.
- No runner, launchd, production environment or service changes; no model downloads, paid resources or actual training in this foundation unit.
- Records and artifacts remain private. No completed-training claim without actual weight artifacts and independent evaluation.
- Each task gets a fresh implementer and task review; finish with one broad review. Workers never dispatch their own agents.
- Keep source modules below 1,000 lines. Use focused external test modules when needed.
- Commit only exact owned files after frozen-diff checks and the applicable gate. A patch is the fallback if concurrent ownership prevents a safe commit. Do not push or merge.

## Review Focus

- Duplicate record identifiers with changed content must refuse overwrite; Task 2 tests this.
- Export ordering must not change identities across process runs; Task 1 pins canonical byte fixtures.
- Symlinked artifact destinations must fail before private data is written; Task 3 tests this.
- Revocation racing snapshot selection must refuse execution eligibility; Task 2 pins selection against one observed registry state.
- Partial publication must never expose a completed receipt; Task 3 tests interruptions before registration.

## Task 1: Deterministic record and snapshot contracts

**Files:** Create ABI `crates/abi-ai/src/training_contracts.rs`, `crates/abi-ai/src/training_contracts/tests.rs`; modify `crates/abi-ai/src/lib.rs` and Cargo manifest only for existing workspace SHA-256 dependency.

**Interfaces:** Define serde types TrainingProfile (Abbey, Aviva, Abi, Quesar), SourceRef (repository revision, document digest, requirement id), ApprovedRecord (id, profile, family_id, input, target, sources, permission, approval, optional teacher identity), SnapshotManifest (version=1, ordered record digests, split, specification digests, root digest), and ContractError. Provide `validate_record(&ApprovedRecord) -> Result<(), ContractError>`, `canonical_record_bytes(&ApprovedRecord) -> Result<Vec<u8>, ContractError>` and `build_snapshot(&[ApprovedRecord], &SnapshotPolicy) -> Result<SnapshotManifest, ContractError>`.

SnapshotPolicy maps each family_id to Train or Evaluation and contains the
permitted specification digests. Permission carries an explicit approved-use
decision id. Approval contains approver identity, decision id and either Human
or QualifiedEvaluator with its artifact and qualification digests. TeacherIdentity
contains base, adapter, tokenizer, generation configuration and qualification
digests. SHA-256 values must be 64 lowercase hexadecimal characters; reject
empty identity strings. Validation distinguishes all these errors with typed
variants, not boolean acceptance.

- [ ] Write tests rejecting unknown profile, absent permission/approval, unqualified teacher, absent teacher base/adapter/tokenizer/config digests, and teacher self-approval. Assert human approval or a separately qualified evaluator is required.
- [ ] Write deterministic fixtures: input permutation produces identical manifest bytes/digest; duplicate id with differing content refuses; the same family cannot occupy multiple splits; teacher-generated records cannot populate held-out evaluation. Use policy-assigned family splits rather than random split assignment.
- [ ] Run `./tools/cargo.sh test --locked -p abi-ai --lib training_contracts < /dev/null`; observe failure before implementation.
- [ ] Implement the declared signatures with sorted canonical fields and explicit schema version. Hash exact canonical UTF-8 bytes; prohibit floating-point configuration values in these identity records.
- [ ] Run the same test command and require a nonzero passing test count, then inspect diff and obtain task review.
- [ ] Preserve a focused reviewed commit or patch containing only this task's files.

## Task 2: WDBX registry and current revocation eligibility

**Files:** Create ABI `crates/abi-sea/src/training_registry.rs`, `crates/abi-sea/src/training_registry/tests.rs`; modify `crates/abi-sea/src/lib.rs`.

**Interfaces:** Consume Task 1 contracts. Define `TrainingRegistry` over an explicitly supplied `DurableStore`; methods `admit(record: ApprovedRecord)`, `register_snapshot(manifest: SnapshotManifest)`, `revoke(record_id: &str, decision: RevocationDecision)`, and `eligibility(snapshot_id: &str) -> Result<Eligibility, RegistryError>`. All mutators return Result. Eligibility records the current registry revision; callers cannot reuse an older eligibility receipt after mutation. Store namespaced versioned JSON under `training:v1:` keys.

RevocationDecision contains authorized operator identity, decision id and reason.
Eligibility contains snapshot id and monotonically increasing registry revision.
Add `validate_eligibility(&Eligibility) -> Result<(), RegistryError>` for resume
and publication checks. Mutators return `Result<(), RegistryError>`; registry
construction accepts ownership of the store, which already holds WDBX's
advisory writer lock for its lifetime. Reuse that ownership; do not acquire a
second registry lock or delete lock files. Opening a competing writable store
must produce WDBX's WriterBusy error. Use one registry session for artifact
publication and receipt registration.

- [ ] Write temporary-store tests: admission persists through checkpoint/reopen; changed-content duplicate refuses; missing records/specification mismatches reject snapshot registration; no reference is made to the private live store.
- [ ] Write revocation tests after export, before new run and before resume. An old eligibility receipt must fail after revocation. Unknown snapshot and unsupported record schema fail explicitly.
- [ ] Run `./tools/cargo.sh test --locked -p abi-sea --lib training_registry < /dev/null` and observe expected failure.
- [ ] Implement immutable record keys and one registry-state publication containing snapshot/revocation references. Test that DurableStore ownership excludes competing writers and that dropping the session permits reopen; document the existing advisory-lock boundary.
- [ ] Run focused tests, inspect crash/reopen behavior, then task review and focused commit/patch.

## Task 3: Private artifact publication and honest run receipts

**Files:** Create ABI `crates/abi-sea/src/training_artifacts.rs`, `crates/abi-sea/src/training_artifacts/tests.rs`; extend Task 1 contracts for RunReceipt and ModelRegistration; extend Task 2 registry methods for receipt/model registration.

**Interfaces:** `publish_artifact(registry: &mut TrainingRegistry, root: &Path, expected_sha256: &str, bytes: &[u8]) -> Result<ArtifactRef, ArtifactError>`; `verify_artifact(root: &Path, reference: &ArtifactRef) -> Result<(), ArtifactError>`; `validate_run_transition(previous: &RunReceipt, next: &RunReceipt) -> Result<(), ContractError>`. Run states: Prepared, Running, Interrupted, Failed, Completed. Completed requires base/tokenizer/snapshot/config identities, step count, actual tensor-change evidence, artifact references and independent evaluation receipt. Define `select_model(registration: &ModelRegistration, registry: &TrainingRegistry) -> Result<QualifiedModel, RegistryError>`.

ArtifactRef contains schema version, relative content-addressed filename,
SHA-256 and byte length. RunReceipt contains run id, profile, state, immutable
input identities, seed, optimizer/configuration digest, step count, optional
artifact references, tensor-change summary and evaluation receipt. ModelRegistration
binds profile, completed run id, base/adapter/tokenizer artifacts, snapshot id,
evaluation id and classification ProductionLlm or Fixture. QualifiedModel
returns those verified identities and current registry eligibility. Registration
and selection reject Fixture. Publication requires Task 2's owned registry
session and never deletes another owner's lock. Move the pure transition validator
to abi-ai alongside its types; filesystem and lock effects stay in abi-sea.

- [ ] Write tests for missing/corrupt artifacts, digest mismatch, path traversal, symlink root/destination, competing lock owner and interrupted staged publication. Completed registration must remain absent after each failure.
- [ ] Write run tests: changed input identities refuse resume; absent tensor-change evidence refuses Completed; failed/interrupted run cannot silently become Completed; demo/fixture classifications cannot qualify as production LLM artifacts.
- [ ] Write selection tests: already promoted model affected by revocation refuses selection; model without independent passing evaluation refuses; teacher identity and qualification receipt must match registration. Current-service cutover is recorded as operator-required, never executed.
- [ ] Run `./tools/cargo.sh test --locked -p abi-sea --lib training_artifacts < /dev/null` and the Task 1 contract tests; observe expected failures.
- [ ] Implement staging, hash verification, atomic rename and registration-after-publication. On Unix use owner-only directories/files; refuse unsafe paths. Inject publication failure points only through test-owned adapters. Large model byte streaming is deferred to the trainer plan; this API verifies bounded fixture/artifact publication semantics.
- [ ] Run affected suites, task review and focused commit/patch.

## Task 4: Integrated foundation qualification and handoff

**Files:** Create ABI `crates/abi-sea/tests/training_foundation.rs`; add a concise source-contract description outside public capability claims, in `crates/abi-sea/README.md` if present or module documentation otherwise.

**Interfaces:** Use only Tasks 1–3 APIs. No new CLI/MCP surface. Test a seed-corpus export, registry reopen, fake candidate receipt, registration refusal without real weight evidence, and revocation invalidation.

- [ ] Write the integrated test first, using temporary WDBX stores and fixture artifact bytes; require all four profile names to survive serialization. Clearly label fake candidates unqualified.
- [ ] Run `./tools/cargo.sh test --locked -p abi-sea --test training_foundation < /dev/null`, observe failure, implement only necessary integration corrections, then require pass with nonzero test count.
- [ ] Run ABI `./tools/check.sh` before handoff; classify pre-existing shared-tree failures by evidence and fix only owned failures. Run Abbey `python3 scripts/check-pages-liquid.py` for plan/spec artifacts. Do not replace either repository's gate with narrow tests.
- [ ] Generate exact owned-file diff package and obtain broad independent review. Resolve material findings with one reviewed fix wave; preserve other agents' changes.
- [ ] Export applicability-checked focused patches and honest verification evidence. No training-completion claim.

## Required subsequent plans and full-goal completion

This foundation is not the requested final state. Subsequent reviewed plans must
select and pin a licensed pretrained LLM/tokenizer and real trainer, assess local
24 GiB memory feasibility, set numerical held-out thresholds before observation,
produce and independently qualify Abbey/Aviva/ABI weights, train and qualify
Quesar, run an independently reviewed teacher-to-student round, and integrate
qualified serving. Concrete base/trainer choices require current primary-source
verification and resource measurements. They are not implicit approvals of paid
compute or production-data use.

Bot deployment remains a separate operational plan after source publication and
successful exact-head CI. Provider qualification, installed-artifact identity
and actual readiness need operator-authorized evidence. The standing service
restriction must be explicitly reconciled before those steps.

## Plan review boundary

Read-only preflight: ABI main was 80dfe079ebe7413a6815c2d564be1f0aaa901267
and WDBX main was e45410356ceb4e96f0dc26a83b02e1c18cf94860; both checkouts
were clean. Recheck these moving references and active checkout owners before
implementation. WDBX DurableStore::open acquires its writer lock and holds the
File in the store, so this plan reuses that existing boundary.

The user approved the written specification and this plan, selected subagent-driven
development, and explicitly requested implementation on 2026-09-29.
Execution then proceeds through all four tasks without between-task permission
prompts, subject to the explicit operational and shared-checkout boundaries.
