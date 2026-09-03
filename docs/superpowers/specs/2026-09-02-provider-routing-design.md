# Provider Routing Design

**Date:** 2026-09-02
**Status:** Approved
**Scope:** Runtime provider discovery, qualification, isolation, routing, and
privacy-preserving acceptance for the Rust `abbey-bot`
**Implementation plan:**
`docs/superpowers/plans/2026-09-02-provider-routing.md`

## Objective

Replace the current compact primary/Foundation Models routing layer with one
provider runtime that can discover configured adapters, qualify exact runtime
identities, select only policy-eligible providers, supervise isolated calls,
and preserve one provider for an entire tool conversation.

This is a runtime architecture change, not a change to Abbey's application
contracts. The provider runtime remains behind the existing generation and
tool boundaries. It adds no provider-selection Cargo feature flags and does
not make ambient credentials, installed CLIs, or detected services eligible by
themselves.

## Preserved Application Contracts

- `ChatTurn`, `ModelTurn`, and `ToolSpec` remain the application-facing turn,
  response, and tool-schema contracts.
- Existing generation entry points remain application-facing.
- The existing `--provider-self-test primary|fm|all --json` grammar and
  transient report remain compatibility inputs. In Cycle 4, the publisher
  validates that report and emits the content-free v2 provider-record array.
- The original five-Core-tool compatibility corpus remains byte-compatible.
- Production continues to expose exactly the seven Core-plus-Inspect tools in
  stable order through every decision codec.
- `AppState` receives one `ProviderRuntime`; it does not accumulate provider-
  specific branches.
- Provider selection is runtime configuration. No provider-selection Cargo
  features are added.
- abbey-bot remains independent of ABI and retains its golden compatibility
  transcriptions.

## Domain Model

The provider domain adds these public types:

- `ProviderId`: stable normalized provider identity.
- `ProviderClass`: local server, OS-managed local, cloud, or agent CLI class.
- `DetectionState`: not detected, detected, ambiguous, or invalid exact
  configuration.
- `Eligibility`: the fixed reason a provider is routable, temporarily
  unavailable, or blocked pending configuration or requalification.
- `ProviderDescriptor`: provider identity, class, configured discovery
  boundary, declared capabilities, and safe provenance.
- `IsolationCapabilities`: the isolation properties qualified for that exact
  provider identity.
- asynchronous object-safe `TurnAdapter`: one provider turn expressed in the
  existing `ChatTurn`, `ModelTurn`, and `ToolSpec` vocabulary.

`ProviderCatalog` owns configured discovery and registered descriptors.
`ProviderRouter` owns hard filtering, performance-first scoring, deterministic
ties, conversation pinning, circuit state, and effect-aware fallback.
`ProviderRuntime` owns the catalog, router, qualification records,
supervision, and shared transports presented to `AppState`.

## Configuration Contract

Provider discovery, selection, and routing policy use the approved runtime
variables:

- `ABBEY_PROVIDER_DISCOVERY`
- `ABBEY_PROVIDER_ORDER`
- `ABBEY_PROVIDER_DISABLED`
- `ABBEY_PROVIDER_MANIFEST`
- `ABBEY_PROVIDER_STATE_DIR`
- `ABBEY_PROVIDER_CLOUD_ALLOW`
- `ABBEY_PROVIDER_AGENT_CLI_ALLOW`
- `ABBEY_PROVIDER_SANDBOX_RUNNER`
- `ABBEY_PROVIDER_SANDBOX_PROFILE`
- provider-specific binary, endpoint, model, immutable model identity, and
  explicit credential settings under `ABBEY_PROVIDER_<ID>_...`

An empty cloud allowlist means allow no cloud provider. An empty agent-CLI
allowlist means allow no agent CLI. An allowlist entry is necessary but never
sufficient: qualification, exact identity, sandbox policy, request capability,
and circuit/budget policy must also pass.

Legacy explicitly configured OpenAI-compatible and Anthropic settings remain
available for compatibility. Ambient credentials never select or authorize a
provider.

## Bounded Discovery

Discovery is configuration-directed and content-free. It may:

- inspect an exact configured executable path;
- obtain bounded version output from that exact executable;
- hash the exact configured executable and approved sandbox identity;
- parse the explicitly selected manifest and runtime configuration.

Discovery may not:

- scan ports or probe a general local service;
- search credential stores, dotfiles, keychains, or provider sessions;
- inspect user conversations or running provider sessions;
- load a model or run inference;
- infer authorization from ambient credentials.

Detected adapters register automatically. A provider becomes routable only
when the exact binary, version, model, OS, tool schema, and sandbox identity
required for that provider match a current qualified record and all policy
filters pass. Multiple matching binaries or an unresolved candidate produce
an explicit ambiguous, ineligible state.

## Capability Manifest

The reader accepts legacy version-1 records for compatibility. The writer
publishes only version 2.

Version 2 is an array of content-free provider records. Each record contains:

- normalized provider identity and class;
- exact binary, model, OS, tool-schema, and sandbox hashes where applicable;
- declared capability categories;
- fixed qualification status.

`Eligibility` remains runtime-derived from current policy, circuit, budget,
and identity state. It is not persisted as manifest evidence.

It contains no prompts, outputs, credentials, endpoints, executable paths,
private model paths, provider errors, user identities, or session data.

The state directory is owner-only mode `0700`. Manifest files are owner-only
mode `0600` and are replaced atomically in the same directory. A manifest with
a schema, fixture, or identity hash mismatch, or one that is malformed,
symlinked, ambiguous, wrong-owner, or incorrectly permissioned, fails closed.
No unapproved time-to-live is inferred. Publication occurs only after all
required synthetic fixtures for the exact identity pass.

## Adapters and Isolation

### MLX-compatible server

The existing OpenAI-compatible HTTP contract is retained for the fresh,
Abbey-private MLX deployment. The qualified model copy is pinned to revision
`73bcf09092aa277861d5a191b989b666f7f32e8f`.

### Ollama

Ollama uses an Abbey-owned daemon and native `/api/generate` structured
output. It is configured with `OLLAMA_NO_CLOUD=1`, a private model root, and a
loopback-only Abbey endpoint. Abbey neither reuses nor probes a user's general
port-11434 service. Qualification binds an immutable imported model digest;
a moving tag is never qualification evidence.

### Foundation Models

Foundation Models remains the sole OS-managed local exception. Qualification
binds `/usr/bin/fm`, the OS build, Abbey binary, tool schema, selected mode,
and synthetic fixture results.

### Claude CLI

The Claude adapter is one strict turn launched by absolute path without a
shell. It uses stdin plus `--bare`, `--restricted`, disabled slash commands,
an empty MCP configuration, no tools, wildcard tool denial, no session
persistence, `dontAsk`, an explicit model, one turn, and stream-JSON output.
It may receive only explicitly supplied provider API credentials. Subscription
OAuth and ambient keychain/session state do not qualify it.

### Other detected agent CLIs

Grok, OpenCode, Codex, Gemini, and Cursor may be detected, but remain
ineligible until all of the following are present and bound into the
qualification identity:

- an explicit agent-CLI allowlist entry;
- an external OS sandbox and approved profile;
- sandbox attestation/hash;
- exact binary/model/tool-schema qualification;
- cancellation and descendant-cleanup proof;
- environment-clearing and privacy proof.

Published headless or tool-control modes are treated as agent-runtime
features, not isolation guarantees. An allowlist cannot bypass sandbox or
qualification requirements.

## Shared Subprocess Supervisor

Turn adapters and provider processes supervised by `ProviderRuntime` are
started by absolute executable path without a shell. The environment is
cleared and rebuilt from fixed safe variables plus only the selected
provider's explicitly configured credentials. This contract does not describe
existing deployment installers or launchd plist wrappers that do not execute a
provider turn.

The supervisor enforces:

- 512 KiB maximum prompt input;
- 256 KiB maximum tool schema;
- 4 MiB maximum stdout;
- 4 KiB maximum stderr;
- 300-second default deadline;
- owner-only runtime directory mode `0700` and files mode `0600`.

On Unix, every provider call has its own process group. Cancellation, timeout,
limit failure, or shutdown sends `INT`, waits two seconds, sends `TERM`, waits
two seconds, then sends `KILL`. On Windows, the provider is assigned to a
kill-on-close Job Object before provider execution can create descendants.
No descendant may survive cancellation, timeout, limit failure, or runtime
shutdown.

## Eligibility and Routing

Every candidate is hard-filtered before scoring by:

- exact qualified binary/model/OS/schema/sandbox identity;
- required capability;
- request scope and network policy;
- cloud and agent-CLI allowlists;
- operator disablement;
- monetary budget;
- circuit-breaker state.

Only eligible providers are scored per request class:

```text
40% quality + 30% reliability + 25% latency + 5% locality
```

The router accepts each component as an explicit content-free normalized
score in the inclusive range `[0, 1]`, with larger values always meaning a
better outcome. It rejects out-of-range inputs. Raw latency, provider class,
or other operational values are not normalized implicitly inside the router;
qualification and bounded outcome producers must supply their declared score
inputs without adding a hidden routing policy.

Metrics are content-free exponentially weighted moving averages with
`alpha = 0.2`. Before 20 comparable turns, the qualification benchmark and
observed EWMA are blended linearly by `n/20`; at 20 turns and beyond, the live
EWMA is used. Quality signals may include schema validity, successful tool
completion, and existing bounded outcome signals, never prompt or response
content.

Monetary cost is a hard allowlist/budget constraint and final operational
limit. It is not a score that can override the approved performance-first
policy. Deterministic ties use configured provider order, then stable
`ProviderId`.

There is no active exploration on real Discord traffic. Synthetic
qualification is the only exploration source.

## Conversation Pinning and Fallback

One provider is pinned for the complete tool conversation. Fallback is
permitted only while both effect flags are false:

- `visible_output_posted == false`
- `tool_dispatched == false`

After either effect occurs, a failure is final for that conversation. The
runtime never duplicates visible output or a host mutation by replaying a turn
through another provider.

## Typed Failures and Circuit Breakers

Three transient failures within five minutes open a provider breaker for 60
seconds. A repeat opening escalates to five minutes, then to a maximum of 15
minutes. Half-open state permits one probe.

Auth, configuration, binary/model identity, sandbox, tool-schema, and protocol
drift block the provider until requalification. A valid `Retry-After` from one
second through 15 minutes is respected. User cancellation, invalid requests,
and local busy rejection do not count as provider failures.

## Model Provisioning and Qualification

Qualification is deployment-specific and synthetic:

- MLX uses a fresh private copy at the approved immutable revision.
- Ollama imports the operator-selected model by immutable digest into the
  Abbey-private root.
- Foundation Models binds the exact OS-managed CLI and OS build.
- Cloud providers and credentialed agent-CLI adapters require explicit
  allowlist entries and explicit provider credentials in addition to their
  isolation/identity checks. The OS-managed Foundation Models exception does
  not require a provider credential.

A version-2 record is published only after the applicable binary, model, OS,
tool-schema, sandbox, structured-output, cancellation, size-limit,
environment-clearing, descendant-cleanup, and privacy fixtures pass.

If an immutable model digest, model artifact, explicit credential, or sandbox
attestation is not operator-supplied, the provider fails closed. That leaves
the deployment/qualification evidence layer pending; it does not change this
design or authorize a weaker route.

## Verification and Evidence Boundaries

Offline fake binaries and servers cover discovery, manifest compatibility,
environment clearing, argument construction, limits, timeout escalation,
descendant cleanup, cancellation, malformed output, schema drift, cloud and
sandbox denial, circuit transitions, deterministic ties, cold starts,
conversation stickiness, and effect-aware no-fallback.

All seven tools are exercised through MLX-compatible, Ollama, Foundation
Models, and strict CLI decision codecs without live provider secrets. Unix
integration coverage proves process-group cleanup. Windows CI proves Job
Object, argument, and environment contracts; it is not a real Windows runtime
acceptance.

Evidence remains separated into:

1. focused source tests;
2. isolated strict gate and locked release build;
3. pushed `main` SHA;
4. exact-head Ubuntu/macOS/Windows CI;
5. exact provider/model/OS/schema/sandbox qualification;
6. installed artifact identity;
7. foreground two-guild Discord acceptance;
8. fresh consented, human-witnessed voice acceptance;
9. managed exact-hash deployment acceptance;
10. real Windows runtime acceptance.

No earlier layer implies a later one. Evidence retains only approved hashes,
normalized provider IDs, fixed result categories, timestamps, PIDs, aggregate
counters, guild-role labels, and human pass/fail attestations. It never retains
secrets, environment values, prompts, replies, Discord IDs, participant
identities, raw logs, audio, transcripts, or packet captures.
