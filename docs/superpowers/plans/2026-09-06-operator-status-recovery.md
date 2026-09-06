# Operator Status and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make scoped bot statistics, persistence outcomes, and current local service evidence understandable without disclosing private logs or changing service state.

**Architecture:** Pure Rust presentation consumes existing scoped brain/budget and typed persistence/provider outcomes. A small local Python adapter consumes the exact managed readiness/bootstrap validator and fixed host evidence. It presents a current observation and performs no repair, restart, provider probe or data mutation.

**Tech Stack:** Rust 1.98, existing Serenity/Tokio adapters, Python 3 standard library, fake launchd/filesystem/clock primitives from the approved installer task.

**Spec:** `docs/superpowers/specs/2026-09-06-guided-ux-operator-status-design.md`, sections 3/4 and applicable acceptance requirements.

## Global Constraints

- Reuse the exact Task 10/11 readiness/bootstrap schemas, fixtures and validators; unknown/duplicate keys, bad types and unsafe files remain rejected.
- The existing three-argument transaction checker CLI is unchanged. The new observation helper accepts default invocation or --help only.
- Exit 0 means a currently validated ready observation; exit 1 means not ready/unavailable/unknown; exit 2 means invalid invocation or unsupported host.
- Never expose raw errors, IDs, PID, nonce, hash, paths, endpoints, model names, credentials, logs or content in observation/recovery output.
- Do not open owner env, production state, consent files or logs. Never write, repair modes, delete, restart, or probe a provider.
- Use the exact age <=30,000 ms and future skew <=2,000 ms predicate. An observation has no transaction-start floor or five-second installation-acceptance claim.
- All host tests use fake HOME, commands, process liveness and clocks. No real launchd or service operation runs during implementation.
- Task 1 follows modernization Tasks 8/10; Task 2 follows Task 11. Both finish before modernization Task 12 final checks and merge.

## File map

- `src/operator_guidance.rs`: pure persistence/provider category guidance; focused test module.
- `src/scoped_stats.rs`: pure scoped stats presentation if extraction avoids expanding command modules.
- `src/commands_brain.rs` and reviewed dashboard module: thin extraction/rendering only.
- `src/commands_context.rs` and vision command failure paths: consume typed recovery guidance.
- `deploy/service_readiness.py`: shared validator extraction if Task 11 has not already separated it.
- `deploy/service-status.py`: narrow local read-only entry point.
- `deploy/test-service-status.py`: fake host evidence, output canaries and no-write/probe acceptance.
- README/runbook and check.sh/check.ps1: documented command and appropriate platform test wiring.

### Task 1: Scoped Statistics and Fixed Recovery Guidance

**Interfaces:** Consume existing `PersistReport`, `PersistOverall`, `PersistComponentOutcome`, Task 8 `ProviderFailureKind`, and current scoped brain/budget snapshot. Produce:

```rust
pub fn persistence_guidance(report: &PersistReport) -> &'static str;
pub fn provider_guidance(kind: ProviderFailureKind, manager: bool) -> &'static str;
pub struct ScopedStatsInput<'a> {
    pub scope_label: &'a str,
    pub brain_summary: &'a str,
    pub budget_per_hour: u32,
    pub tokens_left: f32,
}
pub fn render_scoped_stats(input: &ScopedStatsInput<'_>) -> String;
```

Scope labels are fixed guild/your-DM labels selected by the shell, never raw IDs. The brain summary is generated from the current scoped brain authority. Do not pass the global interaction/memory/reward counters into this type.

- [ ] Add regression fixtures with activity in guild A, guild B and two DMs. Capture A's actual /stats output, change only unrelated global interaction/messages/rewards, and assert A's output stays identical. Keep A's own brain/budget changes observable. Do not merely assert an implementation-specific field list.
- [ ] Add persistence guidance tests for every overall/component outcome, including canonical committed with failed projection and canonical rename followed by directory-sync failure. Retain the precise existing component rendering.

```rust
let report = PersistReport::from_components(
    PersistComponentOutcome::Committed,
    PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory),
);
assert!(!persistence_guidance(&report).contains("all service checks passed"));
assert!(persistence_guidance(&report).contains("projection"));
```

- [ ] Run the new focused tests to observe failure. Extract the scoped renderer and remove global aggregates from /stats. Preserve its registration, guild/DM availability, private response, clamp and allowed-mentions policy. Do not add platform identifiers to Task 10 durable rows.
- [ ] Implement exhaustive fixed guidance from typed categories. Complete explains both saved components; MemoryOnly explains no durable state directory; Partial/Failed retain the component-specific truth and direct investigation to the host operator. Configuration/authentication/availability/provider failures use fixed role-appropriate retry/manager guidance, not raw strings or host-log instructions for members.
- [ ] Attach guidance to reviewed Operations results and image/provider error paths. Preserve authority, two-step reset, original expiry, permission refresh and all self-test/provider schema behavior. Add canary assertions that secrets/paths/IDs/raw error content never reach output.
- [ ] Run scoped stats, operator guidance, command/dashboard/context and privacy tests, formatting and all-targets Clippy. Update README/runbook prose; commit exact owned paths and obtain independent review.

### Task 2: Local Read-only Service Status

**Interfaces:** Consume Task 11's exact shared decoding, private-path, freshness and bootstrap validation. Extract shared functions without changing existing checker behavior. Produce a narrow injectable host observation function and entry point:

```python
from dataclasses import dataclass
from enum import Enum

class ObservationKind(Enum):
    READY = "ready"
    NOT_READY = "not_ready"
    UNAVAILABLE = "unavailable"
    BOOTSTRAP_FAILED = "bootstrap_failed"

@dataclass(frozen=True)
class ServiceObservation:
    kind: ObservationKind
    discord: str | None = None
    scheduler: str | None = None
    telegram: str | None = None
    slack: str | None = None
    persistence: str | None = None
    bootstrap_code: str | None = None

def observe(host) -> ServiceObservation: ...
def render(observation: ServiceObservation) -> str: ...
def main(argv: list[str]) -> int: ...
```

The optional fields come exclusively from validated closed readiness/bootstrap enums. Rendering uses explicit lookup maps and a fixed unknown fallback; no arbitrary supplied value is interpolated. Host injection exposes only read operations: bounded service capture, installed-binary digest, private readiness/bootstrap bytes, process liveness, current time and monotonic deadline. No production env switch enables fake host behavior.

- [ ] Build fake ready/starting/draining, optional degraded connector and partial persistence evidence using Task 10/11's shared fixtures. Add identity-changing, dead PID, stale/future, wrong SHA, missing/oversized/unsafe file and matching/nonmatching bootstrap cases. Assert that one observation never claims an installation transaction passed.
- [ ] Add read/probe/write spies. Any owner env/log/state/consent access, file write/removal, chmod, bootout/bootstrap/kickstart or provider/network probe fails the test. Fake launchctl may only receive the exact read-only service print request; bound captured output at 64 KiB and each subprocess at two seconds. Reuse safe binary hashing and private-file bounds from the installer validator.
- [ ] Run `python3 deploy/test-service-status.py` to observe failing behavior before implementation. Extract the shared validator as needed and rerun the existing checker/fake installer suite immediately to prove unchanged acceptance semantics.
- [ ] Implement the observation sequence: capture PID; check liveness; hash fixed installed binary; read/validate readiness and current freshness; reread service identity and readiness identity; accept only unchanged PID/nonce/SHA and current valid state. Missing evidence remains unknown/unavailable. Use matching bootstrap categories for fixed failure guidance. No synthetic transaction start or old-nonce exclusion belongs here.
- [ ] Implement exact argument handling and fixed output. Example healthy output starts with `Abbey service: ready (current observation)` followed by closed Discord/scheduler/connector/persistence labels. Invalid or unsafe evidence selects a fixed unsuccessful sentence with no raw diagnostic. Return the exit code specified in Global Constraints.
- [ ] Run the full fake status matrix, secret/content/path canaries, existing exact checker/installer tests, Python syntax checks and gate-specific privacy checks. Wire POSIX execution and Windows syntax/privacy-only behavior, document the observation/acceptance difference, commit exact owned paths and obtain independent review.

## Plan self-review

Design section 3 maps to Task 1; section 4 maps to Task 2. Existing lifecycle/log schemas and provider qualification remain owned by the approved modernization. The helper's Python object is an internal view, not a new wire schema. Task 12's final fresh strict gate, cross-platform CI and merge include both tasks.
