# WDBX-backed Abbey, Aviva, ABI and Quesar weight training

Status: written specification approved on 2026-09-29; foundation implementation authorized.
Date: 2026-09-29.

## Intent and acceptance boundary

Produce actual learned weight artifacts faithful to the canonical Abbey, Aviva
and ABI specifications, and train Quesar for its teacher/evaluator role. WDBX
owns training records and evidence. A separate optimizer trains the weights.
Dataset inspection, vector insertion, prompt prefixes and metadata acceptance
do not constitute completed model training.

This specification covers the training subsystem. Bot deployment hardening
remains a separate review unit. Live deployment, production configuration,
runner administration and launchd operations remain prohibited under the
current operator instruction. Approval of this document does not change that
restriction.

## Source contracts and current evidence

The canonical behavior sources are the sibling ABI repository's
`docs/spec/abbey-core-identity.mdx`, `docs/spec/multi-persona-technical.mdx`,
`docs/superpowers/specs/2026-08-22-abbey-system-constitution.md`, and executable
identity definitions in `crates/abi-ai/src/identity.rs`. Each dataset snapshot
records the exact repository revision and content digest of its requirements.
Normative requirements and descriptions of current implementation remain
distinct. Proposed WDBX capabilities are never labeled implemented solely
because the training specification mentions them.

ABI's current `crates/abi-ai/src/training.rs` accepts training metadata and
profile vectors while explicitly reporting unchanged model weights. Its
`abi-nn` optimizer produces a demo character language model. Neither establishes
production persona LLM training.

WDBX provides durable and versioned storage primitives. Its inspected REST
router offers health, stats, verification, insert and query operations, not a
dataset export or model registry. New training contracts must be implemented
and verified rather than assumed present.

Quesar's role comes from `quesar.cloud/src/lib/console.server.ts`: a model that
trains and improves the assistants. The site explicitly does not host that
model or run training. Website desk prompts and response-length caps are
surface constraints, not substitutes for the canonical identity specifications.

## Architecture and ownership

The subsystem consists of five units with independently testable interfaces:

1. A specification catalog maps versioned requirements to profiles and tests.
2. A WDBX-backed corpus registry admits approved records and exports immutable
   training and evaluation snapshots.
3. A trainer consumes only a verified snapshot and pinned base artifacts,
   then publishes weight artifacts and a run receipt.
4. An evaluator measures candidate behavior against a frozen independent suite.
5. A model registry admits qualified artifacts for explicit serving selection.

Training orchestration belongs in ABI; durable storage integration belongs in
WDBX. Abbey-bot consumes a qualified serving interface without new ABI/WDBX
path or Git crate dependencies. Quesar's website remains a separate consumer.
This document authorizes no edits to those sibling repositories before their
local instructions and ownership are checked during implementation planning.

Start with separately identifiable adapters on a shared pretrained base for
Abbey, Aviva and ABI. Quesar has its own trained teacher artifact and evaluations;
its exact base may differ when teacher quality warrants it. An adapter is a
real trained weight artifact, with the base required to reproduce inference.
Full-weight training is a later option, not a prerequisite for honest completion.
The implementation plan must select a compatible licensed base, tokenizer,
trainer and numerical configuration after verifying current primary sources,
local resources and a memory estimate. No base-model download, paid compute or
training run is authorized merely by naming an unspecified dependency here.

## Corpus and snapshot contract

Each admitted record carries a stable identifier, profile, task category,
versioned specification references, input/target content, provenance, usage
permission, review status and content hash. Records fail admission for missing
permission, unknown profile, invalid schema or unapproved targets. Synthetic
records identify their generator and reviewer; generated claims are not source
evidence. Teacher-generated records additionally bind the teacher base, adapter,
tokenizer and generation-configuration digests, qualification receipt, and
approval identity and decision. Admission rejects unqualified teachers and
missing approval evidence. Live Discord conversations and the private WDBX
store are excluded
unless specifically approved for training use.

Snapshot export records the selected record identities and hashes, specification
digests, canonical serialization version, split assignment and root digest.
Splits operate by scenario/source family before augmentation, preventing related
examples and near duplicates from leaking into held-out evaluation. Evaluation
records are not available to the training or teacher-generation input pipeline.
Exact export bytes can be reconstructed or verified from an immutable artifact.

WDBX stores registry and provenance records. Large dataset exports and weight
files use a private artifact directory with content digests and a WDBX identity
reference. Artifact publication stages complete files, verifies them, then
registers them; interrupted publication never registers partial files.
Readers reject missing artifacts, changed bytes and unsupported schema versions.

## Training, interruption and receipts

The trainer accepts a profile, verified snapshot, pinned base/tokenizer,
configuration and output location. It fails before optimization if identities
or permissions disagree. It records seed, optimizer, schedule, precision,
adapter configuration, library versions, device, step counts and measured losses.
Run states are prepared, running, interrupted, failed and completed; completion
requires published weight bytes and a valid receipt, not decreasing loss alone.

Checkpoints bind optimizer state, progress, RNG state and all input identities.
Resume refuses a different base, dataset or configuration. Competing writers
cannot publish the same run. Failures and interruptions retain honest receipts;
recovery cannot silently upgrade them to completed. A completed receipt binds
actual changed tensor values, artifact hashes and a load/inference check. A
different file hash alone is insufficient proof that training changed weights.

## Profile and teacher evaluation

Abbey is evaluated for technical correctness, warmth, useful disagreement,
uncertainty and honest capability claims. Aviva is evaluated for direct expert
reasoning, precise constraints and evidence-supported conclusions. ABI is
evaluated for evidence classification, permission reasoning, orchestration and
correct refusal/escalation. No learned model replaces deterministic runtime
authorization, consent, persistence or tool boundaries.

Quesar proposes training examples and diagnoses assistant weaknesses. Its
teacher evaluation tests correctness, requirement coverage, contradiction
detection and quality of critiques. Quesar-generated examples require an
independent approval step by an authorized human or separately qualified
evaluator whose decision does not rely solely on the generating artifact's
judgment. Quesar qualification must complete before its examples enter a
student snapshot. Initial persona training uses independently approved seed
data. Neither Quesar nor a candidate evaluates its own
outputs as sole qualification evidence. Teacher quality and student quality
receive separate receipts.

Before training, freeze a test suite and baseline outputs. Critical invariants
require zero violations in that suite: fabricated access or execution evidence,
unauthorized tool admission, disclosure across scopes and promotion of proposed
capabilities to current claims. Zero measured violations is bounded evidence,
not a universal guarantee. Behavioral and task-quality thresholds must be
specified numerically in the reviewed implementation plan before results are
observed. Report sample counts, rubric, baseline comparisons and uncertainty;
training loss does not substitute for these evaluations.

Qualification requires actual weights for all four named roles, independently
recorded evaluations and load/inference identity checks. Serving acceptance
additionally requires the selected base/adapter/tokenizer identities to match
the registry and observed response path. Bot source gates, model qualification
and installed-service acceptance remain separate evidence.

## Privacy, revocation and failure behavior

Keep corpora, artifacts and receipts private by default; never log credentials.
Approved-record access is scoped and auditable. Revoking a record prevents new
exports and identifies every affected snapshot and model. Removing a stored
record does not erase its influence from already trained weights. Affected
snapshots become invalid for new runs and resume. Affected models become
ineligible for new promotion and serving selection, including previously
promoted artifacts, until an operator-approved remedy such as retraining from
a clean snapshot is independently qualified. A currently active serving model
triggers operator notification and an explicit decision to select a qualified
unaffected fallback or refuse affected requests. This specification does not
authorize service operations to perform that cutover.

Offline tests cover permission refusal, split leakage, changed specification,
artifact corruption, missing base/tokenizer, interrupted publication, concurrent
ownership, invalid resume, training failure and qualification refusal. Include
revocation after export but before training, before resume, and selection of a
previously promoted affected model. Real
weight-training acceptance requires a recorded optimizer run with tensor-change
evidence. Fixture tests alone cannot prove it.

## Delivery sequence and review boundary

First implement corpus/snapshot and receipt contracts with temporary fixtures.
Then establish a real local training path and qualify the three persona artifacts.
Train and independently qualify Quesar's teacher path, followed by a reviewed
teacher-to-student round. Finally integrate qualified serving and separately
perform operator-authorized deployment acceptance. The plan must preserve all
four artifacts as required deliverables; a small fixture model only verifies
the pipeline and cannot be presented as completion of the requested LLMs.

Written-spec approval permits implementation planning. Training implementation,
dependency selection, resource commitments and operational acceptance need the
subsequent reviewed plan and its explicit execution boundaries.
