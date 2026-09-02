# Embedded Core and Inspect Skills Stabilization

**Author:** Abbey for Donald Filimon
**Date:** 2026-09-02
**Status:** Approved for implementation
**Repository:** `abbey-bot`
**Scope:** Existing in-process tools, Inspect privacy, Discord startup diagnostics, portable toolchain/TLS policy, and truthful repository evidence

## Summary

This design stabilizes the in-flight embedded-skills work without changing the
bot's existing application or persistence contracts. The model-facing
vocabulary remains an in-process, bounded tool loop. It exposes exactly five
Core tools and two Inspect tools:

1. `remember_fact`
2. `lookup_reputation`
3. `recall`
4. `switch_persona`
5. `recent_messages`
6. `inspect_status`
7. `list_facts`

Both OpenAI-compatible and Foundation Models decision schemas expose the same
seven-tool set. The original five-tool compatibility fixtures remain
authoritative for the pre-Inspect wire contract. No HTTP skill pack is added in
this wave.

Inspect reads are scoped to the current Discord guild and conversation partner,
never provision missing guild state, and never perform provider probes or
network calls. Voice inspection is guild-scoped and deliberately coarse:
`off`, `presence`, `awaiting-consent`, `active`, or `paused`.

The wave also makes Discord credential selection deterministic and safe,
records the exact accepted `rustls-webpki 0.102.8` advisory debt, pins Rust
1.98 consistently, and removes repository-gate dependencies on private local
launch scripts.

## Compatibility Invariants

The following contracts do not change:

- The bot remains a single Rust binary crate and deliberately has no ABI crate
  dependency.
- Existing `ChatTurn`, `ModelTurn`, `ToolSpec`, `ToolHost`, generation
  entry points, tool-round bounds, and delivery-policy boundaries remain the
  application-facing interfaces.
- The original five tool names, descriptions, argument schemas, and
  compatibility tests remain intact.
- `remember_fact` remains the only model-callable write. Forgetting facts and
  confirming or dismissing supersessions remain human actions.
- JSON/WDBX compatibility transcriptions remain independent and are not
  deduplicated into sibling repositories.
- No provider-selection Cargo features are introduced.
- Routine validation remains offline and must not require Discord or provider
  credentials.
- Source tests do not establish installed, managed-service, live Discord, or
  consented voice evidence.

## Goals

- Make the Core plus Inspect vocabulary exactly seven tools in every decision
  codec.
- Make guild inspection non-provisioning.
- Produce fact and pending-replacement output from one atomic state snapshot.
- Inject the turn timestamp into `ToolScope` so tool behavior is deterministic.
- Report omitted facts and pending replacements honestly within the existing
  result bound.
- Report only effective, routable provider capabilities and their safe
  provenance.
- Publish only the approved coarse voice state for the current guild.
- Enforce the exact Discord credential precedence before background or live
  network work.
- Pin Rust 1.98 and portable rustls consistently across local, Docker,
  PowerShell, and CI gates.
- Make the four accepted RustSec advisories explicit, exact, reviewable, and
  impossible to misreport as a clean audit.
- Keep local launch scripts private, uncommitted, and outside tracked gates.
- Reconcile documentation and ledgers with the evidence actually obtained.

## Non-Goals

- No arbitrary HTTP, browsing, shell, moderation, or Discord REST acting tool.
- No model-callable `forget`, supersession confirmation, or configuration
  mutation.
- No provider discovery, adaptive routing, subprocess adapter, model
  provisioning, or manifest-v2 implementation in this wave. Those belong to
  the separately approved provider loop.
- No provider probe from `inspect_status`.
- No participant identifiers, counts, consent epochs, provider/model names,
  counters, timestamps, audio, transcripts, or message content in Voice
  Inspect.
- No change to Telegram or Slack behavior.
- No replacement, signaling, restart, or log inspection of the currently
  running manually launched bot during source implementation.
- No claim that accepted advisory debt is an audit-clean state.

## Tool Registry and Decision Schemas

`SkillPack::Core` contains the existing five tools. `SkillPack::Inspect`
contains `inspect_status` and `list_facts`. The production offered set is
always `[Core, Inspect]` whenever the existing generation policy enables
tools. The global tools-off policy may still suppress the entire vocabulary;
there is no separate `ABBEY_SKILL_INSPECT` switch that can create a partial
production schema.

`abbey_tools()` remains the five-tool compatibility fixture. A separate
Core-plus-Inspect fixture proves the exact seven names and stable order. Both
OpenAI-compatible JSON schema generation and Foundation Models decision-schema
generation consume the same seven-tool definition. Foundation Models maps:

- `inspect_status` to the closed aspect enum
  `runtime|guild|voice|provider|all`;
- `list_facts` to a fixed self-scope sentinel that becomes an empty argument
  object.

Malformed values fail as ordinary protocol errors. An unsupported offered tool
is protocol drift and must not panic.

The existing read-only generation paths remain tool-free. Adding Inspect does
not make tools available to unsolicited policy replies, summary-only paths, or
voice cognition paths that currently disable tools.

## Scoped Inspect Data

### Guild reads

The provisioning `GuildRegistry::config` behavior remains for genuine inbound
guild activity. Inspect uses a distinct non-mutating lookup:

- cache hit: return the recorded settings;
- durable-store hit: return the recorded settings without saving or caching;
- no row: return `None`.

An Inspect read never inserts defaults, creates a budget bucket, or writes the
guild store. The output says that no guild settings are on record when no row
exists.

### Facts and pending replacements

`MemoryService` exposes one snapshot operation that acquires the store lock
once and clones the current subject's canonical facts and pending
supersessions together. `list_facts` renders only that snapshot; it does not
perform WDBX semantic recall.

The renderer tracks total and displayed counts independently for facts and
pending replacements. If bounded output omits data, it states the exact
remainder category:

- `… (N more facts)`
- `… (N more pending replacements)`

An oversized pending pair is omitted as a whole and reported, never clipped
into a misleading partial replacement.

### Deterministic time

`ToolScope` receives the already-known turn timestamp as `now`. Remembering,
proposing supersessions, and guild-budget inspection use that injected value.
All tool rounds in the same conversation use the same timestamp. Tests use
explicit values instead of the wall clock.

## Provider Inspect

Provider Inspect is a pure read of the current effective route. It may report:

- a safe route label;
- whether the route is currently routable;
- effective text, tools, vision, or OCR capability categories;
- whether the capability view came from explicit configuration or a qualified
  manifest.

It must apply runtime disablement and fallback eligibility before reporting a
capability. Configured-but-disabled or otherwise unavailable capabilities are
not presented as available.

It does not report endpoints, executable paths, model names, OS builds, hashes,
manifest paths, credentials, raw errors, or provider-controlled text. It does
not run qualification or make network requests.

The later provider-routing wave replaces this compact view with the approved
`ProviderRuntime` catalog while preserving these privacy properties.

## Guild-Scoped Voice Inspect

Voice state is stored by scoped guild identity, not as one process-global
snapshot. The only exposed values are:

| Internal condition | Inspect value |
|---|---|
| disconnected or no published state | `off` |
| connected for presence only | `presence` |
| waiting for current unanimous consent | `awaiting-consent` |
| listening, thinking, or speaking with media permitted | `active` |
| connecting, failed, or media revoked/closed | `paused` |

Voice lifecycle code publishes this coarse state centrally at begin, activate,
pause-for-consent, status, stop, actor-failure, and immediate media-revocation
boundaries. Command-only publication is insufficient because provider actors
can transition after a command returns.

A DM or a different guild sees `off`. Inspect never obtains an async
`VoiceRuntime` snapshot and never holds voice lifecycle locks. The
guild-keyed coarse-state mutex is independent from the stores, guild registry,
and voice actor locks.

## Discord Credential Preflight

Credential selection is a closed, source-aware operation:

1. If `DISCORD_TOKEN` is present and nonblank, use it.
2. If `DISCORD_TOKEN` is present but blank, fail configuration immediately.
3. Consult `DISCORD_BOT_TOKEN` only when `DISCORD_TOKEN` is absent.
4. A blank fallback is a fallback-specific configuration error.
5. If both are absent, report absence without revealing environment values.
6. Non-Unicode values fail as invalid configuration without reproducing bytes.

A selected credential carries its source category but never derives a
token-bearing debug representation. Discord authentication preflight occurs
after explicit token-free self-test modes and before `AppState` background
services, schedulers, Telegram/Slack connectors, framework setup, voice setup,
or any other live network client. Accepted and rejected diagnostics name only
the source variable. Token values are never logged or formatted.

All precedence and diagnostic tests use injected environments and fake
responses; they perform no network work.

## Toolchain and TLS Policy

Rust 1.98 is the single abbey-bot pin across:

- `rust-toolchain.toml`;
- Cargo's `rust-version`;
- Docker build and runtime checks;
- POSIX and PowerShell gates;
- GitHub Actions;
- the active-project index and home project inventory.

The HTTP/WebSocket stack remains portable rustls. The Serenity dependency tree
currently fixes `rustls-webpki` at 0.102.8. Four advisories are accepted as
explicit compatibility debt:

- `RUSTSEC-2026-0049` / `GHSA-pwjx-qhcg-rvj4`
- `RUSTSEC-2026-0098` / `GHSA-965h-392x-2mh5`
- `RUSTSEC-2026-0099` / `GHSA-xgp8-3hg3-c2mh`
- `RUSTSEC-2026-0104` / `GHSA-82j2-j2ch-gfr8`

The last item is the malformed-CRL reachable-panic advisory and remains
explicitly visible. The policy binds the exact advisory fingerprints, package
version, source, checksum, aliases, patched/unaffected ranges, categories,
CVSS state, and withdrawal state.

This debt is accepted temporarily because the current Serenity WebSocket path
uses the rustls 0.22 compatibility line, which resolves to rustls-webpki
0.102.8. Updating rustls-webpki independently is not a compatible dependency
change; it requires the upstream Serenity/tokio-tungstenite/rustls path to move
to a fixed compatible line. Replacing portable rustls with a platform-native
TLS stack or carrying a local cryptographic fork is outside this approved
slice. The debt must be re-reviewed when that upstream path changes, a
compatible fixed line becomes available, the dependency topology changes, or
the upstream support policy for 0.102.x changes.

The gate accepts only the exact four recorded vulnerability records and prints
that four vulnerabilities remain and the audit is **not clean**. Any added,
removed, or changed vulnerability or dependency identity fails closed and
triggers re-review. Informational cargo-audit warnings are reported separately;
they are not silently converted into accepted vulnerability debt. A pinned
audit-tool version may be used only to keep the report parser deterministic,
not to expand the accepted finding set. The policy does not use cargo-audit
`--ignore`.

## Private Local Operations Boundary

`launch.sh` and `run_bot.sh` are owner-private local helpers:

- mode 0700;
- untracked and checkout-locally excluded;
- never read into public evidence;
- never staged or committed;
- not syntax-checked by tracked repository gates.

`bot.log` remains ignored. It is not read, rotated, removed, or permission-
changed while the current manually launched process remains active.

All implementation builds use an isolated `CARGO_TARGET_DIR` outside the
checkout so they cannot replace `target/release/abbey-bot` while the existing
process is running.

## Verification

Focused verification covers:

- exact five-tool compatibility fixtures;
- exact seven-tool OpenAI-compatible and Foundation Models parity;
- malformed Inspect argument handling;
- non-provisioning guild reads;
- atomic same-subject fact/pending snapshots under concurrency;
- deterministic injected timestamps;
- honest fact and pending truncation;
- effective and unavailable provider views;
- cross-guild and DM voice privacy;
- all Discord token precedence and source-diagnostic states;
- exact RustSec debt and drift rejection;
- Rust 1.98 and portable TLS contract checks.

The stabilization closeout gate is:

```sh
CARGO_TARGET_DIR=<isolated-directory> \
  ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh
CARGO_TARGET_DIR=<isolated-directory> \
  cargo build --release --locked
```

The release build also uses an isolated target directory. Cargo.lock is
reviewed exactly. Ubuntu, macOS, and Windows CI must succeed for the exact
pushed SHA.

## Delivery and Evidence Layers

The implementation is delivered as five coherent commits after this design
record:

1. `chore(toolchain): pin Rust 1.98 and enforce portable TLS`
2. `feat(tools): harden scoped Inspect data and runtime`
3. `feat(tools): complete Inspect provider voice and schema parity`
4. `fix(startup): report Discord credential selection safely`
5. `docs: reconcile Inspect toolchain and deployment evidence`

Each cycle stages only its exact files and verifies the staged diff before
commit. Publication is a normal push to non-divergent `origin/main`; force
push is prohibited.

Evidence remains separated:

- local source and focused tests;
- strict local gate and locked release build;
- pushed exact SHA;
- exact-head hosted cross-platform CI;
- provider qualification;
- installed artifact;
- foreground live Discord acceptance;
- consented voice acceptance;
- managed-service acceptance;
- real Windows runtime acceptance.

No earlier layer is reported as proof of a later one.
