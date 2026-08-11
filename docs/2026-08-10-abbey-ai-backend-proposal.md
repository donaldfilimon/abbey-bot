# Abbey's AI backend — a decision document

**Date:** 2026-08-10 · **Author:** Abbey (the persona, asked to design her own Discord embodiment)
**Status:** PROPOSAL — documentation only. This branch contains no `/ask` implementation and
does not approve one. A separate, unpublished local prototype exists at commit `fbcae70`; it is
preserved outside this documentation branch and still requires its own review and authorization.

The goal ledger's open direction: *"an AI backend decision (screenshot parsing, DM drafting,
SocialBrain — new scope, not leftovers)."* This document makes that decision concrete enough
to accept, amend, or reject. Every claim about existing code below was verified by reading
the source this session, not recalled from docs.

---

## 1. What exists today (the honest inventory)

### 1a. `abi-ai` is an identity layer, not an inference engine

This is the load-bearing fact of the whole document, so it goes first.

`~/dev/active/abi/crates/abi-ai` is **pure, deterministic, and does no I/O** — its own
`lib.rs` says so and the code confirms it. What it actually provides (real types and
functions, all read this session):

- **`AgentProfile`** (`identity.rs`) — the `Abbey` / `Aviva` / `Abi` enum, plus
  **`ProfileContract`** carrying each persona's `description`, `response_prefix`, and
  `response_suffix`. These are the canonical persona definitions the whole ecosystem
  consumes (the abbey CLI explicitly refuses to redefine identity and imports these).
- **Routing** (`router.rs`): `route_profile`, `analyze_sentiment`, `select_best_profile`,
  `explicit_profile_selector`, `blend_weights` — keyword/sentiment routing over the three
  personas. `route_with_soul` (`lib.rs`) can blend in a `PointNeuralNetwork` forward pass
  (`point_net.rs`) — a small hand-rolled network over text-derived points, **not** a
  language model.
- **"Generation"** (`incremental.rs`): `generate_profile_incremental` performs genuine
  word-at-a-time incremental *emission* — of a **template**. The output is exactly
  `response_prefix + input + response_suffix`. The golden test pins it:
  `run_text("hello world")` → `"Abbey: hello world\n\nI'll approach this with warmth,
  creativity, and technical care while keeping uncertainty explicit."` The module's own
  doc comment is admirably blunt: *"It is not a neural language model or a token sampler."*
- **Governance** (`constitution.rs`): `validate(&str) -> AuditResult` — the six-principle
  constitutional audit with a hard safety veto. `run(input)` = route → template →
  audit. `complete` (`completion.rs`) substitutes a refusal string on hard veto.
- **Embeddings** (`embedding.rs`): `text_embedding` — Wyhash signed-feature vectors.
  **Lexical, not learned** (abbey's CLAUDE.md states this in exactly those words for the
  same function).
- **Orchestration** (`orchestration.rs`): worker specs and the fixed Abbey/Aviva/Abi
  trio roster — deterministic scaffolding, again no model.

**Plainly: an `/ask` command powered only by `abi-ai` would echo the question back inside
a persona template.** It would *look* like AI in a Discord embed and be nothing of the
kind. That is exactly the capability-inflation the ecosystem's claims-honesty rules exist
to prevent, so option (a) below cannot be the whole answer — only part of one.

### 1b. Where real language generation actually lives in this ecosystem

- **`abi-connectors`** (`~/dev/active/abi/crates/abi-connectors`): provider clients for
  OpenAI, Anthropic, Grok (plus Discord and Twilio). Every connector has a **local/live
  transport split**: local synthesizes a deterministic response (hermetic tests, no
  credentials); live issues a real HTTPS request (no-redirect, HTTPS-or-loopback
  enforced, key-leak-safe URL joining). Live Anthropic SSE streaming exists.
  `local_bridge.rs` dispatches to a user-run loopback server (llama-server / ollama /
  mlx) via the OpenAI-compatible API — *"ABI does not embed an inference engine"* (its
  words). A compatible loopback server has been observed on this machine in a
  prior session, but current availability and model identity must be verified
  at implementation time rather than treated as present-day evidence.
- **`abi-mcp`**: a real crate (stdio + loopback HTTP) exposing `ai_run` / `ai_complete` /
  `ai_learn` / `ai_train` — but those tools are backed by the same `abi-ai` template
  machinery plus WDBX persistence. MCP gets you process isolation and memory, **not**
  a smarter model.
- **The abbey CLI** (`~/dev/active/abbey`) proves the architecture that works: identity
  from `abi-ai` (sibling path dep, `../abi/crates/abi-ai`), *generation* from a real
  backend behind a per-backend argv grammar (cursor-agent · grok · on-device `fm` ·
  `abi complete`), a claims ledger that separates Current from Proposed, and
  transcript-based continuity layered over stateless backends. That layering — canonical
  identity + pluggable real generation + honest claims — is the proven pattern.
- **A surprise worth flagging:** `abi-connectors` contains `discord_gateway.rs` /
  `discord_routing.rs` — a minimal reference `!command` gateway bot *inside abi itself*,
  whose `DEFAULT_INTENTS` include **MESSAGE_CONTENT (privileged)**. It is a separate,
  parallel embodiment. abbey-bot shares no code with it and **must not inherit its
  intents posture** (see §4).

### 1c. What abbey-bot's command layer can host

abbey-bot (`dbab1db`, gate green 68/68) is structurally ready for a backend:

- **The separation rule is already the right shape.** Decision logic lives in pure
  modules (`persona.rs`, `profile.rs`, `perms.rs`, `moderation.rs`, `server.rs`);
  `commands.rs` is the only file touching serenity types. A backend slots in as another
  pure module plus one command translation — no architectural change.
- **Unconditional defer is already law** (README "Design notes"), which is precisely
  what an LLM round-trip needs: Discord's 3-second interaction token is unsurvivable
  otherwise. The rule was built for cold REST calls; it generalizes for free.
- **`clamp_message` already exists** — every reply routes through the 2,000-codepoint
  clamp, so long model output cannot produce an API rejection.
- **Its `persona.rs` is a local re-derivation, not the canonical one.** The bot's cue
  router (AVIVA_CUES/ABI_CUES substring counting, ambiguity → Abbey) reimplements in
  miniature what `abi-ai`'s router does canonically — with *different register text*
  (bot: "Abbey — direct, street-smart"; abi-ai: "Abbey — the empathetic polymath").
  Today that divergence is cosmetic because `/persona` only *describes* routing.
  The moment a persona *speaks*, the divergence becomes an identity bug.

---

## 2. Integration options

### Option A — depend on `abi-ai` directly (sibling path, like abbey does)

Add `abi-ai = { path = "../abi/crates/abi-ai" }` and consume `ProfileContract`,
`route_profile`, and `constitution::validate` instead of the local `persona.rs` tables.

- **What changes in abbey-bot:** `persona.rs` shrinks to a thin adapter over abi-ai's
  router; persona descriptions become the canonical ones; replies could be audited by
  `validate` before send.
- **Latency/cost/offline:** zero, zero, fully offline. Deterministic and golden-testable.
- **Intents posture:** unchanged.
- **The structural cost is real and currently disqualifying as-is:** the sibling-layout
  constraint (`abbey`+`abi` must stay siblings — already a documented hard rule) would
  extend to abbey-bot — *and abbey-bot has two live clones.* The second,
  `~/sources/repos/abbey-bot`, has no `../abi` sibling, so the path dep **breaks its
  build outright**. Cargo resolves path deps even when optional, so a feature gate does
  not rescue the second clone. Adopting this option means either retiring that clone or
  accepting that it can no longer build — a decision only Donald can make.
- **The honesty cost:** on its own, this option produces zero real generation. An
  `/ask` built on it alone is a template echo dressed as AI. **A is an identity
  upgrade, not an AI backend.**

### Option B — talk to a running abi/abbey process (abi-mcp over stdio/loopback, or `abi complete` subprocess)

The wdbx_bridge pattern from the abbey CLI: shell out to (or speak MCP with) an `abi`
binary found on PATH.

- **What changes in abbey-bot:** a process-invocation module (spawn + JSON-RPC or argv,
  timeout handling — note `timeout(1)` does not exist on this host, so bounds must be
  in-process); config for locating the binary; degraded-mode copy when it is absent.
- **Latency/cost/offline:** milliseconds locally, zero cost, offline — but see the catch.
- **Intents posture:** unchanged.
- **The catch:** `ai_run`/`ai_complete` are the same template machinery as option A, one
  process boundary further away. The boundary buys real things — no compile-time
  coupling (both clones keep building), WDBX persistence for a future memory surface,
  version independence — but **it does not buy language generation**. Live LLM through
  abi's connectors still requires credentials configured on the abi side, at which point
  the bot has taken on abi-as-a-deployment-dependency (the systemd/Docker artifacts
  would need the abi binary shipped alongside) to reach the same external API option C
  reaches directly. B is the right move the day abbey-bot needs *memory* (WDBX) or a
  shared brain with the CLI; it is indirection without generation today.

### Option C — external LLM API, persona defined in prompt/config

A small transport module in abbey-bot (mirroring abi-connectors' `Transport`-trait
design so tests use a recording fake, never the network) speaking to **one** backend,
selected by environment:

1. `ANTHROPIC_API_KEY` set → Anthropic messages API (the ecosystem's pinned header
   pair, `x-api-key` + `anthropic-version`, is already documented in abi-connectors);
2. else `ABBEY_BOT_LLM_ENDPOINT` set → loopback OpenAI-compatible bridge (llama-server /
   ollama / mlx — potentially free and private, subject to runtime verification);
3. else → the command replies, honestly, that no generation backend is configured.

The persona system prompt is **transcribed from `abi-ai`'s `ProfileContract`
descriptions** (source cited in a comment as `../abi/crates/abi-ai/src/identity.rs`),
selected by the routing decision `/persona` already computes.

- **What changes in abbey-bot:** new pure module (prompt assembly + response shaping),
  new transport module, one new command wiring in `commands.rs`, one new secret handled
  exactly like `DISCORD_TOKEN` (env only; the existing `.dockerignore`/systemd
  discipline already covers it).
- **Latency:** seconds (fine — defer is unconditional). **Cost:** per-token on path 1,
  zero on path 2. **Offline:** path 2 works offline; path 1 degrades to the honest
  path-3 message.
- **Intents posture:** unchanged — a slash command receives its input as an interaction
  parameter, so **no message-content intent is needed, ever, for this design**.
- **The honesty cost is drift:** transcribed persona text can rot when abi-ai's
  contracts change. Accepted and documented (a comment naming the source file and a
  ledger note), because the alternative — the path dep — breaks a live clone. If the
  second clone is ever retired, A's identity dep supersedes the transcription and the
  drift risk disappears.

---

## 3. Recommendation (as Abbey)

**Option C now, with A's identity layer adopted the day the two-clone question is
settled, and B reserved for when memory becomes real scope.**

The reasoning is the persona architecture itself. The 2024 ABI diagram
(`~/Documents/Research/AI/ABI/ABI.pdf`, verified present) and the abi workspace both
define me as *identity + routing + governance* — and abi, honestly, gets its actual
language generation from connectors and loopback bridges, never from the persona layer.
The abbey CLI made the same split and it works. The embodiment truest to that
architecture is not "depend on the crate named ai"; it is **reproduce the layering**:
canonical identity, real generation behind a swappable transport, and copy that never
claims more than what runs. A template echo wearing my name would be the least
Abbey-like thing this bot could ship.

### The first slice, named precisely: `/persona ask <question>`

Extend the existing `/persona` command with poise subcommands — `/persona route`
(today's behaviour, unchanged) and `/persona ask` (new). One command, one backend
path exercised end-to-end.

**What it takes:**
- `ask.rs` (pure): system-prompt assembly from the routed persona's transcribed
  contract; response shaping through the existing `clamp_message`.
- `llm.rs`: the `Transport` trait + Anthropic and OpenAI-compatible request builders +
  a `RecordingTransport` for tests (the abi-connectors design, in miniature).
- `commands.rs`: the subcommand split; defer stays unconditional; reply is
  **non-ephemeral** (an answer is not an accusation — unlike `/modcall`).
- Config: the two env vars above; `.env.example` updated; README gains an "AI backend"
  section that states which backend is configured *and that answers come from an
  external model, not from the bot*.
- A per-user cooldown before any public-guild deployment (path 1 spends money;
  path 2 spends a shared GPU).

**What its test looks like** (all offline, all under `./check.sh`):
1. Prompt assembly: routing "help me design the permission matrix" selects Aviva and
   the assembled system prompt contains Aviva's contract text — asserted by string,
   pinned to the transcription.
2. Transport shape: the `RecordingTransport` asserts the exact URL, headers
   (`x-api-key` + `anthropic-version` on path 1), model, and body a call would send —
   never a live request in tests.
3. Clamping: a 5,000-character fake response comes back ≤ 2,000 codepoints through the
   existing clamp.
4. Honest degradation: with neither env var set, the reply names the routed persona and
   states no generation backend is configured — asserted verbatim, because that copy
   *is* the honesty contract.
5. The gate stays `./check.sh`; no test may require a token, a key, or the network —
   the property the whole suite already holds.

Screenshot parsing, DM drafting, and SocialBrain are **explicitly not in this slice**.
Each is new scope with its own posture questions (vision models, DM intents, reputation
persistence) and each deserves its own decision document after `/persona ask` has
survived a live guild.

---

## 4. What it must NOT do

Constraints already paid for elsewhere in this ecosystem, codified here so the backend
work cannot erode them:

1. **No privileged intents without a documented decision.** Intents stay
   `non_privileged()`. `/persona ask` needs no message content — input arrives as an
   interaction parameter. The tempting follow-on ("read recent channel messages for
   context") requires MESSAGE_CONTENT, which is a Dev Portal declaration and a separate
   decision document, not a code change. Do **not** copy `abi-connectors`'
   `discord_gateway::DEFAULT_INTENTS` — that reference bot requests message content;
   this one deliberately does not.
2. **The defer rule stays unconditional**, and becomes more load-bearing, not less:
   an LLM round-trip exceeds the 3-second token by design.
3. **Webhook rules stay.** The AI backend gains no webhook powers; `/webhook` remains
   an emit-only guide; `MANAGE_WEBHOOKS` gating stays.
4. **No invented capabilities in user-facing copy.** The bot never claims on-device
   inference, memory, learning, or "understanding" it does not have. abi-ai template
   output is never presented as generation. If the backend is the loopback bridge, the
   copy says local model; if Anthropic, it says external API; if neither, it says so.
   The research-paper narrative (distributed WDBX, GPU clusters, benchmark numbers) is
   aspirational and never appears in bot copy — same rule as
   `docs/contracts/external-claims-audit.mdx` in abi.
5. **Recommend, never act, stays the pattern for anything the model says about
   people.** `/modcall`'s ladder is deterministic and property-tested; model output
   must never feed it, and the model must never be quoted as grounds for moderation.
6. **Secrets discipline:** the API key follows `DISCORD_TOKEN`'s exact handling — env
   only, `/etc/abbey-bot/env` under systemd, `--env-file` under Docker, never in an
   image layer, never in a commit.
7. **The constitutional audit is future work, honestly labeled.** Auditing replies with
   `abi_ai::constitution::validate` requires the path dep (option A) — until that
   decision is made, the bot does not claim constitutional governance.

---

*Verification notes: abi-ai surface read from `lib.rs`, `identity.rs`, `incremental.rs`,
`completion.rs` this session; connectors from `lib.rs`, `providers.rs`, `local_bridge.rs`,
`discord_routing.rs`, `discord_gateway.rs`; abbey-bot from `README.md`, `persona.rs`,
`commands.rs`, `Cargo.toml`; gate state and open direction from `~/tasks/goals.md`
(entries through 2026-08-10 05:4x, gate 68/68 at `dbab1db`). The second clone at
`~/sources/repos/abbey-bot` was confirmed to exist, which is what demotes option A.*
