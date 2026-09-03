# Provider Routing Implementation Plan

**Date:** 2026-09-02
**Status:** Approved
**Design:** `docs/superpowers/specs/2026-09-02-provider-routing-design.md`
**Branch policy:** canonical checkout on `main`; no branch or worktree
**Cycle limit:** exactly five provider implementation cycles

## Entry Gate

Begin provider implementation only after the stabilization head equals
`origin/main` and its exact-head Ubuntu, macOS, and Windows jobs are green.
Before every cycle:

- confirm the canonical checkout and `main` branch;
- fetch and require non-divergence from `origin/main`;
- freeze and preserve all unassigned dirty paths;
- re-resolve the manually launched bot by PID, parent, owner, start time, cwd,
  executable path, and executable hash;
- recheck `launch.sh` and `run_bot.sh` mode and hash without printing their
  contents;
- leave `bot.log` ignored and unread;
- use a fresh external `CARGO_TARGET_DIR` so source work cannot replace the
  running checkout binary;
- stage exact assigned paths or hunks only, inspect the complete staged diff,
  and never use broad staging, stash, reset, checkout restoration, or history
  rewriting.

The existing `.gitignore` `.playwright-mcp/` hunk is unrelated and remains
outside every provider commit unless separately assigned by the owner.

## Cycle 1 — Catalog, Discovery, and Manifest

Commit: **feat(providers): add catalog discovery and qualification manifests**

### Assigned implementation surface

- `src/provider.rs` facade conversion plus compatibility implementation and
  exports
- provider catalog/config/discovery/manifest modules under `src/provider/`
- `src/provider/qualification.rs` compatibility and v2 identity integration
- provider catalog/discovery/manifest tests under `src/provider/`
- `.env.example` provider configuration contract hunks only
- `Cargo.toml` and `Cargo.lock` only if this cycle requires a reviewed runtime
  dependency

### Work

- Add `ProviderId`, `ProviderClass`, `DetectionState`, `Eligibility`,
  `ProviderDescriptor`, `IsolationCapabilities`, and asynchronous object-safe
  `TurnAdapter`.
- Parse the approved `ABBEY_PROVIDER_*` configuration with empty cloud and CLI
  allowlists meaning allow none.
- Add bounded, exact-path discovery. Do not port-scan, inspect credentials or
  sessions, load models, or run inference.
- Register detected adapters automatically while keeping ambiguous,
  unresolved, unqualified, identity-mismatched, or policy-denied providers
  ineligible.
- Read legacy v1 qualification records and write only content-free v2 provider
  record arrays.
- Preserve `--provider-self-test primary|fm|all --json` and add the Rust
  manifest compatibility required to read real legacy-v1 objects and read or
  write v2 arrays. Deployment publication remains assigned to Cycle 4.
- Make the Rust v2 writer use same-directory atomic replacement into a
  mode-0700 directory with mode-0600 files.
- Keep all existing application-facing turn, tool, and generation contracts.

### Focused verification

- exact environment parsing, including empty-deny allowlists;
- absent, detected, ambiguous, invalid, and qualified eligibility states;
- exact-path/version/hash discovery bounds and no forbidden probing;
- v1 compatibility reads and v2-only writes;
- malformed, schema/fixture/identity-mismatched, symlinked, wrong-owner,
  wrong-mode, oversized, and duplicate manifests fail closed;
- atomic publication preserves the previous manifest on failure;
- serialized records contain no content, credentials, endpoint/path values, or
  provider-controlled errors.

## Cycle 2 — Isolated Local and CLI Adapters

Commit: **feat(providers): add isolated local and CLI adapters**

### Assigned implementation surface

- provider adapter/transport/supervisor modules under `src/provider/`
- existing OpenAI-compatible code in `src/llm.rs` and `src/llm/` only where
  required to expose the preserved transport contract
- Foundation Models and legacy router extraction/compatibility in
  `src/provider.rs`
- platform-specific supervisor tests under `src/provider/`
- `Cargo.toml`/`Cargo.lock` for reviewed process and Windows Job Object support

### Work

- Preserve the existing OpenAI-compatible path for the private MLX adapter.
- Add an Abbey-owned Ollama daemon/native `/api/generate` adapter with
  structured output, `OLLAMA_NO_CLOUD=1`, private model root, and loopback-only
  endpoint. Never reuse or probe port 11434.
- Keep Foundation Models as the sole OS-managed local exception, bound to
  `/usr/bin/fm` and the OS build.
- Add the strict one-turn Claude CLI adapter using the approved flags, stdin,
  explicit model, empty MCP configuration, no tools, no persistence,
  `dontAsk`, and stream-JSON. Permit only explicitly configured provider API
  credentials.
- Detect Grok, OpenCode, Codex, Gemini, and Cursor while keeping them
  ineligible unless allowlist, external sandbox, sandbox attestation/hash,
  exact qualification, cancellation, and privacy requirements all pass.
- Launch runtime-supervised provider turns and provider processes by absolute
  path without a shell after clearing the environment and adding only fixed
  safe values plus the selected provider's explicit credentials. Existing
  deployment wrappers that do not execute a provider turn remain outside this
  supervisor contract.
- Enforce 512 KiB prompt, 256 KiB schema, 4 MiB stdout, 4 KiB stderr, and a
  300-second default deadline.
- On Unix, isolate a process group and escalate INT / two seconds / TERM / two
  seconds / KILL. On Windows, establish kill-on-close Job Object membership
  before provider execution can create descendants. Ensure no descendants
  survive any terminal path.

### Focused verification

- request and response codecs for MLX-compatible, Ollama, Foundation Models,
  and strict Claude CLI adapters;
- exact no-shell argument arrays and stdin use;
- environment clearing and per-provider explicit credential allowlisting;
- loopback/private-root/no-cloud Ollama policy and no port-11434 probing;
- prompt/schema/stdout/stderr/deadline boundaries;
- malformed structured output and provider-controlled error redaction;
- Unix process-group escalation and descendant cleanup;
- Windows creation-time Job Object assignment, immediate-descendant escape,
  kill-on-close, arguments, and environment coverage;
- unsafe agent CLIs are detected but never spawned while unqualified.

## Cycle 3 — Adaptive Routing and Typed Failures

Commit: **feat(providers): add adaptive routing and typed failures**

### Assigned implementation surface

- provider router/runtime/metrics/failure modules under `src/provider/`
- `src/generation.rs` and its tests for conversation pinning and effect-aware
  fallback integration
- `src/generation/foundation_models.rs` compatibility integration
- `src/runtime.rs` for one `ProviderRuntime` in `AppState`
- `src/runtime/provider_setup.rs` for provider-runtime construction
- `src/main.rs` provider-runtime construction and compatibility accessors
- `src/pipeline.rs`, `src/commands_brain.rs`,
  `src/commands_voice/discord.rs`, `src/voice_local.rs`, and
  `src/voice_self_test.rs` integration hunks
- `src/inspect.rs` effective-provider compatibility integration
- `src/vision.rs`, `src/vision/provider.rs`, and
  `src/vision/foundation_models.rs` provider-runtime integration hunks
- `src/pipeline/tests.rs` plus existing generation/Foundation Models tests
- provider routing/circuit tests under `src/provider/`

### Work

- Hard-filter by qualification identity, requested capability, scope/network
  policy, sandbox attestation, cloud/CLI allowlists, operator disablement,
  monetary budget, and circuit state.
- Score eligible providers per request class at 40% quality, 30% reliability,
  25% latency, and 5% locality.
- Accept explicit content-free quality, reliability, latency, and locality
  scores only in `[0, 1]`, with larger values meaning better outcomes. Reject
  out-of-range inputs and do not infer a raw-latency or provider-class
  normalization rule inside the router.
- Maintain content-free EWMA values with `alpha = 0.2`; blend qualification and
  live observations by `n/20` before 20 turns and use live EWMA thereafter.
- Keep monetary cost as a hard constraint, never a compensating score.
- Break ties by configured order, then stable provider ID.
- Perform no exploration on real Discord traffic.
- Pin one provider for the full tool conversation.
- Rename or make the current Foundation Models-only `ProviderRouter` private
  during integration so the approved adaptive `ProviderRouter` has one
  unambiguous owner.
- Preserve existing application-facing generation signatures and safe static
  route labels through compatibility accessors while provider IDs remain
  runtime-owned.
- Allow fallback only before both visible-output and tool-dispatch effects.
- Open the circuit after three transient failures in five minutes for 60
  seconds, then five minutes, then 15 minutes maximum; allow one half-open
  probe.
- Block auth, configuration, identity, sandbox, schema, and protocol drift
  until requalification.
- Respect `Retry-After` from one second through 15 minutes.
- Exclude user cancellation, invalid requests, and local busy rejection from
  provider-failure accounting.

### Focused verification

- every hard filter and denial reason;
- exact 40/30/25/5 score and deterministic ties;
- bounded higher-is-better score inputs and out-of-range rejection;
- `alpha = 0.2`, cold-start `n/20` blend, and transition at turn 20;
- no cost compensation and no live exploration;
- conversation stickiness across all tool rounds;
- no fallback after visible output or a dispatched tool;
- transient windows, escalation, half-open single probe, and reset behavior;
- permanent requalification failures and bounded Retry-After;
- excluded failure categories do not move breaker or reliability metrics;
- no prompt, response, or private context enters metrics.

## Cycle 4 — Private Models and Deployment Qualification

Commit: **feat(providers): provision isolated models and qualify deployments**

### Assigned implementation surface

- provider qualification/model identity modules under `src/provider/`
- `src/provider_self_test.rs` synthetic qualification integration
- `deploy/publish-provider-qualification.py` and its offline tests for v2
  publication
- `deploy/configure-mlx-primary.py` and its offline tests for v2 consumption
- other assigned `deploy/` MLX/Ollama/provider qualification scripts and their
  offline tests
- `.env.example` operator configuration hunks
- qualification-focused documentation hunks only

### Work

- Stage a fresh Abbey-private MLX copy pinned to revision
  `73bcf09092aa277861d5a191b989b666f7f32e8f`.
- Import the operator-selected Ollama model by immutable digest into the
  private root; never treat a moving tag as evidence.
- Publish a qualified v2 record only after all applicable binary, model, OS,
  tool-schema, sandbox, structured-output, cancellation, size-limit,
  environment-clearing, descendant-cleanup, and privacy fixtures pass.
- Preserve the current `--provider-self-test primary|fm|all --json` grammar.
  The publisher validates that transient self-test report before emitting the
  content-free v2 provider-record array, and the configurator consumes v2
  records while the real legacy-v1 decoder remains available for compatibility.
- Keep cloud providers disabled unless explicitly listed in
  `ABBEY_PROVIDER_CLOUD_ALLOW`.
- Preserve explicit legacy OpenAI/Anthropic compatibility without selecting
  it from ambient credentials.
- If the required immutable digest, model artifact, explicit credential, or
  sandbox attestation is not operator-supplied, fail closed and report only
  the corresponding qualification/deployment layer as pending.

### Focused verification

- immutable MLX revision and Ollama digest identity;
- private model roots and owner-only state publication;
- every required fixture gates manifest publication;
- an existing manifest survives any failed qualification;
- empty cloud allowlist produces zero cloud eligibility/calls;
- explicit legacy configuration works without ambient selection;
- missing operator inputs produce a fixed pending/ineligible result without a
  design fallback;
- qualification output remains content-free and privacy-safe.

## Cycle 5 — Offline and Cross-Platform Closure

Commit: **test(providers): close privacy platform and routing coverage**

### Assigned implementation surface

- fake-binary/fake-server and integration tests under `src/provider/`
- generation/tool-codec compatibility tests under `src/generation/`
- Unix provider process integration tests
- Windows Job Object/argument/environment tests and CI contract
- provider-related privacy corrections in MLX installer/smoke scripts and
  their tests under `deploy/`
- README, `.env.example`, `docs/live-test-protocol.md`, `tasks/goals.md`,
  `tasks/todo.md`, `AGENTS.md`, and `CLAUDE.md` provider/operator/privacy hunks
  only

### Work

- Close fake-binary and fake-server coverage for discovery, v1/v2 manifests,
  environment clearing, size/deadline enforcement, escalation, descendant
  cleanup, cancellation, malformed output, schema drift, policy denial,
  circuits, deterministic ties, cold starts, stickiness, and effect-aware
  fallback.
- Exercise all seven tools through MLX-compatible, Ollama, Foundation Models,
  and strict CLI codecs without live provider secrets.
- Run Unix process integration coverage and Windows Job Object, argument, and
  environment coverage in CI.
- Reconcile operator and privacy documentation and correct any installer/smoke
  output that discloses provider-controlled errors or private model paths.
  Retain only normalized identifiers, hashes, and fixed result categories.
- Keep `AGENTS.md` and `CLAUDE.md` identical except for their heading.
- Do not claim provider qualification, installation, live Discord, voice,
  managed deployment, or real Windows runtime from source/CI evidence.

### Focused verification

- complete provider-focused offline test inventory;
- exact seven-tool parity across every decision codec;
- privacy static gate plus runtime canaries;
- platform-gated Unix and Windows supervisor contracts;
- documentation/mirror consistency and evidence-layer review;
- no unrelated dirty path or private helper in the index.

## Provider-Wave Closeout

Use a fresh external target directory:

```sh
CARGO_TARGET_DIR=<isolated-directory> \
  ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh
CARGO_TARGET_DIR=<isolated-directory> \
  cargo build --release --locked
```

Then:

1. Review exact `Cargo.lock` changes and the complete provider-wave diff.
2. Recheck the live process and private helper identities.
3. Fetch `origin/main` and require non-divergence.
4. Merge a clean remote advance without rewriting history and rerun gates;
   stop for owner direction on an overlapping conflict.
5. Push normally with `git push origin main`; never force-push.
6. Resolve the exact remote SHA.
7. Wait for exact-head Ubuntu, macOS, and Windows jobs.
8. Begin operational transition only when the final provider SHA equals
   `origin/main` and all exact-head jobs are green.

## Post-Push Acceptance Boundary

The later operational sequence is the approved safe-transition,
pre-Discord qualification, foreground two-guild acceptance, fresh unanimous
voice acceptance, and managed exact-hash acceptance in
`docs/live-test-protocol.md`.

If the old process does not stop after exact-identity SIGINT, do not force-kill
it automatically. Missing model digests, credentials, sandbox attestations,
sandbox guilds, consenting participants, or a real Windows host leave only the
corresponding evidence layer pending.
