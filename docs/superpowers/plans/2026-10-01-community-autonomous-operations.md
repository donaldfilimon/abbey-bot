# MLAI Community Operations Implementation Plan

> **For agentic workers:** Execute the approved tasks with explicit ownership and independent review. Preserve concurrent work; do not commit or push.

**Goal:** Deliver the approved MLAI community experience and a policy-enforced operations system in the existing Rust bot.

**Architecture:** Pure policies decide authorization and lifecycle. Existing runtime/service ownership drives work, Discord adapters execute and re-read, durable infrastructure reserves receipts before I/O. Existing gateway remains the only gateway.

**Tech stack:** Existing Rust/Serenity/Poise, serde, Tokio, owner-managed local providers; no new dependencies.

**Spec:** ../specs/2026-10-01-community-autonomous-operations-design.md

## Global constraints

- Local-only execution, explicit consent, 600-second contextual sanction ceiling.
- Five structural operations/day, two creations/day, seven-day target cooldown.
- Preserve IDs/history and sensitive access; fail closed on incomplete observations.
- No commits/pushes or new dependencies; preserve other sessions' edits.

## Review focus

- Uncertain remote success must remain reserved until reconciliation.
- Unknown permissions or stale policy must refuse mutation.
- A disabled memory policy must not disable invited conversation.
- Provider configuration must not permit an off-host fallback under local-only.
- Stop during I/O must retain an honest unresolved receipt.

## Tasks

- [ ] Capture live guild surfaces and role/action/channel matrix; document every surface disposition and missing evidence in workspace outputs.
- [ ] Implement independent participation guard, member-only personal fact mutation, and 120-second settings mapping; update semantic regression tests without reverting concurrent tests.
- [ ] Add local-only policy at provider admission and text/image execution; test remote/PCC refusal and local availability failures.
- [ ] Implement pure operations policy, persistent reservation/lease/receipt lifecycle, permission-preserving maintenance, and retained scheduled assessment; test bypass, duplicate, drift, uncertain outcomes, limits, and stop.
- [ ] Integrate owner review/stop/reconcile controls and classic dashboard operational status. Preserve command compatibility.
- [ ] Implement reviewed community blueprint/resources and explicit membership rules; migrate in revalidated batches and read back IDs/access.
- [ ] Run moderation recommendations in shadow mode; only enable bounded enforcement after measured review and appeal validation.
- [ ] Run ./check.sh and applicable strict WDBX conformance. Review the complete owned diff. Qualify installed runtime and member journeys separately before enabling autonomous operations.

This ledger is intentionally unchecked until tool-confirmed evidence exists. Baseline restoration work remains a separate owner; no deployment may publish unqualified shared changes.
