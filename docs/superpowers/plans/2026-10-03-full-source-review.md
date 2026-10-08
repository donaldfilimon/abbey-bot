# Abbey Bot Full Source Review, Repair, and Qualification

Status: execution approved by Donald on 2026-10-03. This plan authorizes source review, attributable repairs, offline checks and evidence updates only.

Goal: review the complete shared diff, fix confirmed defects under coordinated ownership, and qualify an unchanged final source snapshot.

Architecture: preserve the canonical checkout and existing public contracts. Run six sequential independent Abbey Reviewer passes followed by cross-subsystem review, regression-first repairs and the authoritative strict gate.

Authority: the approved in-chat plan; existing repository AGENTS.md and the completion workstream specs define the contracts being reviewed. This is qualification work, not approval to implement future Proposed subsystems.

## Constraints

- Preserve tracked, staged and relevant untracked incoming changes. No checkout, reset, staging, commit, push, PR or worktree.
- One coordinated source writer; reviewers are read-only. Obtain handoff before overlapping edits.
- No dependencies, deployments, service changes, live provider calls, Discord mutations or human voice acceptance.
- Preserve pure policy seams, module limits, scoped memory, observed cancellation ownership and content-free observability.
- Preserve evidence outside shared source; distinguish source qualification from installation and live acceptance.

## Tasks

- [x] Capture HEAD/index, complete diff, relevant untracked evidence and complete input manifest; inventory active owners.
- [x] Sequentially review generation/provider lifecycle, learning/persistence/erasure, engagement/scheduling, voice, commands/forum authorization, and gates/documentation. Record concrete triggers, expected behavior, severity, location and proposed regression.
- [x] Review interactions across those seams; reconcile review coverage with the complete inventory.
- [x] For each confirmed behavioral defect, write and observe an attributable failing regression, apply the smallest repair and pass focused checks. Re-review repairs and the final full diff; defer cosmetic/unrelated suggestions.
- [x] Run applicable focused checks and docs Liquid validation. Run Activity tests if Activity changes belong to the reviewed diff.
- [x] Run `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`, preserving its actual exit status and log. Compare complete before/after manifests and index hashes; changed inputs invalidate attribution and require review and a new stable gate.
- [x] Produce a receipt with source identity, suite counts, exclusions, findings/repairs and proof gaps; append ledger corrections and update the companion checklist without rewriting history.

## Acceptance

A stable manifest, green authoritative gate and no unresolved blocking review findings establish source qualification only. Existing receipts remain historical. Linux/Windows runtime, installed identity, provider, Discord and human acceptance remain open unless separately exercised and recorded.

Execution evidence: repaired strict gate session14918 exited0 against an unchanged
800-input snapshot; both confirmed P2 defects have regression-first repairs and
independent final rereview. The receipt/checklist/ledger updates are subsequent
documentation inputs, covered by a separate final complete-tree confirmation
retained outside source. See [the source receipt](../../verification/2026-10-03-full-source-review.md).
