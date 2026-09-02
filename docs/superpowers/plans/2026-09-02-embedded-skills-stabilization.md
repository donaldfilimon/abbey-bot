# Embedded Skills Stabilization Implementation Plan

**Date:** 2026-09-02
**Status:** Approved
**Design:** docs/superpowers/specs/2026-09-02-embedded-skills-design.md
**Branch policy:** canonical checkout on main; no branch or worktree
**Cycle limit:** five implementation cycles

## One-Time Pre-Mutation Handoff and Inventory

Before changing source:

- Re-read the repository AGENTS.md and its CLAUDE.md mirror, plus the
  machine-level guidance they reference.
- Confirm the canonical checkout, current main branch, remotes, and complete
  worktree list.
- Fetch origin/main and record the exact ahead/behind count before mutation.
- Freeze the full tracked-modified and untracked manifest with each path's
  mode, size, and SHA-256 digest; separately prove that the index is empty.
- Record the mode and SHA-256 digest of launch.sh and run_bot.sh without
  printing their contents. Add both only to the checkout-local exclude file.
- Obtain explicit source handoff for all four dirty slices: Rust/toolchain/TLS,
  local launch handling, embedded Core/Inspect skills, and Discord startup
  diagnostics.
- Re-resolve the live bot by PID, parent PID, owner, start time, cwd,
  executable path, executable mode, and executable hash. Do not read its log or
  signal it.

The source handoff does not authorize an operational handoff of the running
bot. That process remains untouched until the separately approved post-provider
safe-transition protocol.

## Fixed Safety Baseline

Before every cycle:

- Confirm the checkout is the canonical
  /Users/donaldfilimon/dev/active/abbey-bot checkout on main.
- Confirm origin/main has not diverged.
- Preserve every dirty path not assigned to the current cycle.
- Re-resolve the manually launched bot by PID, owner, start time, cwd,
  executable path, and executable hash.
- Do not signal the bot, inspect or rotate its log, or replace its executable.
- Build and test with an isolated CARGO_TARGET_DIR outside the repository.
- Recheck launch.sh and run_bot.sh mode and hash; keep them untracked,
  checkout-locally ignored, and out of every stage.
- Never use broad staging. Stage only the explicit cycle paths or selected
  hunks, inspect the staged diff, and commit only after focused verification.

## Cycle 1 — Toolchain and TLS

Commit: **chore(toolchain): pin Rust 1.98 and enforce portable TLS**

### Assigned paths

- Cargo.toml
- Cargo.lock
- rust-toolchain.toml
- Dockerfile
- check.sh
- check.ps1
- .github/workflows/rust.yml
- scripts/check-linux-tls-tree.py
- scripts/test-check-linux-tls-tree.py
- security/rustsec-accepted-debt.json
- scripts/check-rustsec-debt.py
- scripts/test-check-rustsec-debt.py
- src/abbey_contracts.rs — digest-format compatibility hunks only
- src/commands.rs — WebSocket text compatibility hunks only
- src/gateway.rs — WebSocket text compatibility hunk only
- src/offline_voice.rs — fixed-array chunk compatibility hunks only
- src/provider/qualification.rs — digest-format compatibility hunk only
- src/voice_openai.rs — WebSocket configuration/text compatibility hunks only
- src/voice_openai/protocol.rs — WebSocket text and fixed-array chunk hunks only
- src/wdbx/tests.rs — Rust-version documentation hunk only

Several of these Rust files are shared with later cycles. Cycle 1 stages only
the named compatibility hunks; later Inspect/voice/provider hunks remain in the
working tree.

Machine-state synchronization, outside the repository commit:

- /Users/donaldfilimon/dev/active/AGENTS.md
- /Users/donaldfilimon/CLAUDE.md

### Work

- Make Rust 1.98 the exact Cargo, rustup, Docker, POSIX, PowerShell, and CI pin.
- Preserve the portable rustls dependency stack.
- Enforce the Linux TLS-tree contract.
- Record only the four exact accepted rustls-webpki 0.102.8 vulnerability
  records and compare their complete material fingerprints.
- Use cargo-audit 0.22.2 as deterministic report-format tooling. Its version
  pin is parser reproducibility, not an additional accepted dependency finding.
- Report informational cargo-audit warnings separately; do not silently add
  them to the four-vulnerability acceptance record.
- Make the expected output say that four vulnerabilities remain and the audit
  is not clean.
- Fail on any missing, added, or changed accepted vulnerability, package
  version, source, checksum, alias, patched/unaffected range, category, CVSS,
  informational state, or withdrawal state. A cargo-audit version mismatch is
  a tooling-contract failure, not a security-debt acceptance decision.
- Remove tracked launch.sh and run_bot.sh syntax checks.

### Focused verification

~~~sh
python3 scripts/test-check-linux-tls-tree.py
python3 scripts/check-linux-tls-tree.py
python3 scripts/test-check-rustsec-debt.py
python3 scripts/check-rustsec-debt.py
sh -n check.sh
~~~

Run the equivalent PowerShell and workflow-contract tests where available.
Review the exact Cargo.lock TLS route and advisory report.

## Cycle 2 — Scoped Inspect Data and Runtime

Commit: **feat(tools): harden scoped Inspect data and runtime**

### Assigned paths

- src/guild.rs
- src/runtime/memory_service.rs
- src/runtime.rs
- src/inspect.rs
- src/tools.rs
- src/pipeline.rs
- src/commands.rs
- src/generation/tests.rs
- src/generation/foundation_models/tests.rs
- src/pipeline/tests.rs

Stage only cycle-2 hunks in files that also contain later provider, voice, or
startup changes.

### Work

- Add a non-provisioning guild-settings lookup and use it for Inspect.
- Take facts and pending supersessions from one store-lock snapshot.
- Add ToolScope.now and replace all tool-local wall-clock reads.
- Render facts and pending replacements within the existing bound while
  reporting separate omitted counts honestly.
- Keep recall as semantic retrieval and list_facts as the canonical bounded
  subject view.

### Focused verification

- Unknown-guild inspection performs no cache or durable-store write.
- Existing cached and durable settings remain readable.
- Concurrent snapshots never pair facts from one state with pending records
  from another.
- Cross-guild and cross-user snapshots remain isolated.
- Remember, supersession, and budget behavior use an explicit timestamp.
- Long fact and pending lists report both remainder categories.
- Oversized pending pairs are omitted whole and reported.
- Every result remains within the existing character bound.

Run the focused Rust module tests for guild, memory service, Inspect, tools,
pipeline, and generation.

## Cycle 3 — Provider, Voice, and Schema Parity

Commit: **feat(tools): complete Inspect provider voice and schema parity**

### Assigned paths

- .env.example
- src/tools.rs
- src/inspect.rs
- src/runtime.rs
- src/provider.rs
- src/provider/qualification.rs
- src/generation.rs
- src/generation/foundation_models.rs
- src/generation/foundation_models/tests.rs
- src/main.rs voice-publication hunks only
- src/commands_voice.rs
- src/voice_session.rs
- src/voice_local.rs
- src/voice_openai.rs
- src/voice_openai/protocol.rs
- src/provider/tests.rs
- src/generation/tests.rs
- src/voice_session/tests.rs
- src/voice_session/control/tests.rs

Tests that live inline in src/inspect.rs, src/runtime.rs, src/voice_local.rs,
and src/voice_openai.rs are owned by the corresponding listed source path.
Shared files retain only Cycle 3 provider/voice/schema hunks in this stage.

### Work

- Make production tool packs exactly [Core, Inspect]; remove the partial
  Inspect toggle while retaining global tools-off behavior.
- Preserve abbey_tools() as the five-tool compatibility fixture.
- Add exact seven-tool OpenAI-compatible and Foundation Models schemas.
- Add strict Foundation Models adapters for inspect_status and list_facts;
  malformed or unsupported tools return protocol errors.
- Report effective provider capabilities and safe configured-versus-qualified
  provenance, never configured-but-unavailable capabilities.
- Replace the process-global rich voice view with a guild-keyed coarse state.
- Publish voice state at all command, actor, failure, consent, revocation, and
  stop transitions.
- Render a DM or another guild as off.
- Keep HTTP tools deferred.

### Focused verification

- Original five-tool compatibility corpus is unchanged.
- Both decision codecs expose exactly the seven approved tools in stable order.
- Malformed Inspect arguments fail closed.
- Provider views exclude endpoints, paths, models, hashes, keys, and raw
  provider errors.
- Disabled or unavailable providers are not reported as routable.
- Voice output is one of exactly five words and contains none of the prohibited
  identity, consent, model, counter, timestamp, media, or transcript fields.
- Cross-guild and DM voice tests prove isolation.
- Actor failure and media revocation cannot leave Inspect reporting active.

## Cycle 4 — Discord Credential Diagnostics

Commit: **fix(startup): report Discord credential selection safely**

### Assigned paths

- src/main.rs — credential selection, startup ordering, and its inline
  injected-environment/no-network tests only

### Work

- Represent the selected secret separately from its source category.
- Use nonblank DISCORD_TOKEN when present.
- Treat present-but-blank primary configuration as an error without fallback.
- Consult DISCORD_BOT_TOKEN only when the primary variable is absent.
- Distinguish absent, blank, non-Unicode, rejected, and accepted-source states.
- Keep token-free startup self-tests before credential selection.
- Authenticate Discord before constructing or starting schedulers, connectors,
  voice, or other live network work.
- Never print, debug, or retain token values.

### Focused verification

Use injected values and fake status responses for:

- both variables absent;
- primary only;
- both valid, proving primary precedence;
- blank primary with absent fallback;
- blank primary with valid fallback, proving no fallthrough;
- primary absent with valid fallback;
- primary absent with blank fallback;
- non-Unicode primary;
- accepted source labels;
- rejected diagnostics for both sources;
- non-authentication errors;
- formatting/debug redaction.

No focused test performs live Discord or connector work.

## Cycle 5 — Truthful Ledgers and Local Hygiene

Commit: **docs: reconcile Inspect toolchain and deployment evidence**

### Assigned paths

- README.md
- docs/live-test-protocol.md
- tasks/goals.md
- tasks/todo.md
- AGENTS.md
- CLAUDE.md
- only the bot.log ignore hunk in .gitignore

Do not stage unrelated .gitignore changes. Do not stage private local scripts.
Keep AGENTS.md and CLAUDE.md identical except for their intentional top
heading.

### Work

- State Rust 1.98 and the exact non-clean TLS debt.
- State the local-only launch-script boundary.
- State exact seven-tool parity and the five-tool compatibility boundary.
- State that provider qualification is distinct from source tests.
- State the actual deployment and live-evidence status.
- Remove premature claims that a gate proves installation, managed service,
  live Discord, provider qualification, or voice behavior.
- Replace broad process-kill guidance with exact PID, owner, start-time,
  executable, and hash verification.

### Focused verification

- Documentation consistency and mirror checks.
- No secret-like values, raw local model paths, Discord identifiers, prompts,
  responses, audio, or transcripts.
- No claim collapses local, pushed, CI, qualification, installed, foreground,
  voice, or managed evidence.
- The staged diff contains only the assigned documentation/hygiene slice.

## Stabilization Closeout

Use a fresh isolated target directory. Do not use or replace the live
target/release/abbey-bot.

~~~sh
CARGO_TARGET_DIR=<isolated-directory> \
  ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh
CARGO_TARGET_DIR=<isolated-directory> \
  cargo build --release --locked
~~~

Then:

1. Review the exact Cargo.lock diff.
2. Review the complete stabilization base-to-head diff and claims.
3. Recheck the live bot identity and private-script hashes.
4. Fetch origin/main.
5. Require non-divergence.
6. If remote advanced without overlap, merge without rewriting history and
   rerun the gates; stop on an overlapping conflict.
7. Push normally with git push origin main.
8. Resolve the exact remote SHA.
9. Wait for Ubuntu, macOS, and Windows checks for that exact SHA.
10. Do not begin the provider implementation wave until stabilization exact-
    head CI is green.

The running bot remains untouched through this source closeout. Its safe
transition occurs only after the final provider wave is pushed and green.
