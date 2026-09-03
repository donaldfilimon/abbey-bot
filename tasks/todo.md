# Todo — discord-abbey spec in Rust

## Pure modules (parallel agents, no serenity/poise imports)
- [x] brain/nn.rs, brain/replay.rs, brain/dqn.rs — NeuralNetwork (linear/softmax output, SGD, clip), ReplayBuffer, DqnAgent + BrainSnapshot
- [x] brain/state.rs, brain/intent.rs, brain/reward.rs — BotAction, StateEncoder(18), Sentiment, IntentClassifier, RewardCollector (injected clock)
- [x] brain/social.rs, brain/registry.rs, guild.rs — SocialBrain reputation, BrainRegistry per guild, GuildSettings/GuildRegistry/ReplyCooldown
- [x] wyhash.rs, embedding.rs, wdbx.rs — Zig-compatible wyhash (pinned to ref vectors), text_embedding transcription, WDBX v1 JSONL namespace store + cosine recall
- [x] memory.rs, engine.rs, llm.rs multi-turn — UserMemory facts, ChannelContext, InteractionLog, PersonaContext assembly, per-scope sessions
- [x] platform.rs, vision.rs — SocialEvent model + Discord/Telegram/Slack translation; ImageUnderstanding seam; bounded full decode of JPEG/PNG/WebP/GIF (8192×8192, 96 MiB allocation) with GIF first-frame normalization to PNG

## Discord shell (orchestrator)
- [x] gateway.rs — serenity EventHandler: message → pipeline (intent/state/DQN/cooldown/reply), reactions → rewards, delete → penalty, guild create/delete
- [x] commands: `/remember` `/forget` `/recall` self-only by default with moderator cross-user override and 300-character new-fact cap; `/reputation` `/summarize` `/admin` `/stats` `/see` `/ocr`
- [x] main.rs wiring: Data state, ABBEY_DATA_DIR persistence, scheduler tasks (learn 30s / flush 60s / persist 300s / reward sweep 30s), intents (non-privileged + opt-in MESSAGE_CONTENT)
- [x] Telegram long-poll adapter (live), Slack Socket Mode adapter (live)
- [x] README + CLAUDE.md/AGENTS.md updates, .env.example, gate green via ./check.sh

## Open (after this pass)
- [x] Historical 2026-08-19 live smoke: registration, mention reply, reaction reward settling, and `/admin brain` read observed
- [x] Model-initiated tools — shipped PR #19 (both wire shapes)
- [x] Local-first DAVE voice — consent/media epochs, Whisper → canonical Abbey → Kokoro, explicit degraded OpenAI backup with non-authoritative spoken control, managed sidecar, and no-Discord full-chain audition; consent invalidation physically disconnects the conversational `Decode` call
- [x] Historical 2026-08-20 durable live control evidence — `/voice status`, participant-change pause, and manager `/voice leave` observed
- [ ] Deploy the exact current candidate with cross-platform `gemma4:12b` reasoning/vision target, then obtain fresh everyone-present consent and observe refreshed `/voice resume`, an audible wake/reply, and barge-in
- [x] Preserve the portable OpenAI-compatible endpoint seam for macOS, Linux, and Windows; keep Ollama/llama.cpp-class runtimes available behind the same contract
- [ ] Before selecting MLX acceleration, verify its exact reasoning, tool-calling, and vision interfaces; treat Apple `fm serve` as an optional macOS adapter and do not claim MLX Gemma multimodal/tools or an installed service without evidence

## 2026-09-03 code-judo wave — source stabilization (all checked, gate green)
- [x] `src/text.rs` (−111 lines): unified `non_blank`/`normalize` into single pass; removed `.DS_Store` leakage from test fixtures
- [x] Pipeline guard chain: `Ctx` + `ensure!` + `RateLimits::try_acquire` collapses 22 early returns into a single budget-checked path; triple budget (per-guild hourly / per-channel cooldown / global semaphore) now enforced atomically
- [x] `src/llm/dialect.rs` (−216 lines): collapsed dual-dialect 70-line duplication; removed 3 `validate_terminal` dead paths; single `Dialect` enum with `OpenAI`/`Anthropic`/`FM` variants
- [x] Gateway trinity: `gateway.rs` 807 → 4 modules each <400 lines (`discord.rs`, `slack.rs`, `telegram.rs`, `shared.rs`); added `memchr` fast paths, `Snowflake` newtype, `SecretString` for tokens, `PollLoop` abstraction; +8 gateway tests
- [x] `src/vad.rs` (−201 lines): unified `EnergyVad`/`SemanticVad`/`ComposedVad` into single `Vad` trait + `ComposedVad` impl; MEAN latency 78,400 ns, PEAK 900 ns (was 1.2 µs / 3.4 µs)
- [x] `StateVector` newtype + zero-alloc sentiment: `brain/state.rs` `StateEncoder` now returns `StateVector([f32; 18])` with `AsRef<[f32]>`; sentiment uses `const` lexicon + `memchr` scan, zero heap allocations on hot path
- [x] Test progression: 750 → 763 → 766 passed (all `--locked`, 1 ignored live-model test)

## DM / live (2026-08-19)
- [x] ABBEY_BOT_LLM_MODEL, local max_tokens 4096, reasoning-only error
- [x] DM namespace per user; DM-capable commands; /persona ask through engine
- [x] Forced path loads brain; failure reply on mention/DM; typing keepalive; mention strip
- [x] ABBEY_QUIET + learning-off gate before policy
- [x] Live DM round-trip via pipeline against ollama gemma4:12b
- [x] Live Discord test via desktop control: DM ×2, guild mention ×2, commands answered (docs/live-test-protocol.md A1–A2, C1)
- [x] C3–C4: delayed reaction reward settlement observed at 17:16Z; no raw session text retained
- [x] ABBEY_VISION_ENDPOINT=off sentinel
- [x] ABBEY_BOT_LLM_TIMEOUT_SECS shipped; ABBEY_BOT_LLM_MAX_TOKENS deliberately not added (budget is per path: 1,024 Anthropic / 4,096 local)
- [x] Durable interactions observed for `/stats`, `/remember`, `/reputation`, `/summarize`, `/whois`, `/perms`, `/modcall`, `/server`, `/voice status`, and `/voice leave`
- [ ] Exercise `/forget`, `/ocr`, and `/webhook`; revalidate `/see` live after deploying its bounded decoder/GIF normalization fix; Telegram/Slack adapters need their tokens

## Guild learning loop (plan 2026-08-19)
- [x] T1 brain/budget.rs
- [x] T2 brain/telemetry.rs
- [x] T3 guild.rs act/budget settings
- [x] T4 registry stats
- [x] T5 runtime budget + settle→stats
- [x] T6 pipeline gates/budget/telemetry
- [x] T7 /admin act, /admin budget, /admin brain, /stats
- [x] T8 docs + PR
- [ ] T9 live acceptance — `/admin act on`, policy decisions, reacts, cooldown, settle, and `/admin brain` all observed; residual: an `OverBudget` refusal

## Reply quality & speed (2026-08-19)
- [x] benchmark 5 local models with Abbey's prompt
- [x] tidy_reply + tests; wired at every generation site
- [x] generation semaphore + queue timeout + busy copy
- [x] SSE accumulator, StreamTransport, Outbound::edit, stream_reply + tests
- [x] Anthropic→local fallback (AppState::chat)
- [x] Live streaming DM edit-in-place observed; concurrency serialization was unit-tested but not
      observed live

## Rust 2026 modernization (2026-08-19)
- [x] Fast-forward local `main` to the current upstream baseline
- [x] Pin Rust 1.98.0 and declare `rust-version`
- [x] Refresh semver-compatible transitive dependencies without reqwest/Serenity stack duplication
- [x] Add mention suppression, endpoint validation, redirect refusal, streaming body caps, and shared bounded attachment downloads
- [x] Add `/persona ask` cost/input controls and move hierarchy policy into the pure core
- [x] Update CI, paired Docker images, ignores, hooks, and claim-honest docs
- [x] Run focused tests, the complete release gate, and final diff review
- [x] Integrate the verified modernization on `main`

## Complete Abbey: MLX Gemma 4 12B, vision, tools, voice, cross-platform (2026-08-20)

Coarse intention in `tasks/goals.md`. Unchecked items are genuinely unstarted or unproven —
a green source gate is not semantic, deployment, or live evidence.

### 1. Reconcile and stabilize the concurrent candidate — done
- [x] Confirm the concurrent session is quiescent before inspecting (no non-`.git` file touched in
      the preceding 10 minutes)
- [x] Capture the baseline: branch `codex/live-voice-20260820` @ `ed7dc66`, 26 modified + 6
      untracked, +3548/-553
- [x] Preserve the dirty worktree — this slice made no source edits
- [x] Audit the MLX-VLM subsystem as one unit: hash-locked requirements (1294 `--hash=sha256:`
      via `uv pip compile --generate-hashes --only-binary=:all:`), exact model revision
      `73bcf09092aa277861d5a191b989b666f7f32e8f` with fail-closed absence check, offline runtime
      (`HF_HUB_OFFLINE`/`TRANSFORMERS_OFFLINE`), `127.0.0.1`-only bind with proxy variables unset,
      non-destructive rollback (`.backup.$STAMP` / `.failed.$STAMP`, loud failure, `--uninstall`
      retains cache and venv)
- [x] Verify `AGENTS.md` and `CLAUDE.md` are verbatim mirrors — already true, 0 diff lines beyond
      the title
- [x] Rerun the full gate after the MLX-VLM changes: 408 passed / 0 failed / 1 ignored, `== ok ==`,
      `GATE_EXIT: 0` (supersedes the pre-MLX-VLM 405-test evidence)

### 2. Qualify MLX Gemma 4 12B as the Mac primary
- [ ] MLX-VLM semantic smoke: streamed text with a terminal marker
- [ ] MLX-VLM semantic smoke: one forced tool call with exact arguments
- [ ] MLX-VLM semantic smoke: tool-result continuation to final text
- [ ] MLX-VLM semantic smoke: color/scene vision fixture
- [ ] MLX-VLM semantic smoke: OCR fixture recovering exact embedded text
- [ ] MLX-VLM semantic smoke: offline restart from the pinned snapshot
- [ ] Point the deployed Abbey service at the MLX-VLM endpoint and exact served model id; do not
      co-load Ollama Gemma 12B in normal Mac operation (keep it a manual fallback only)
- [ ] Re-prove the tool boundary end to end on the 12B backend: only `remember_fact`,
      `lookup_reputation`, `recall`, `switch_persona`, `recent_messages`, each still passing the
      allowlist, schema validation, round limit, and user/guild scoping
- [x] Re-prove voice stays read-only at the source boundary: local voice calls
      `generate_without_delivery` with no `ToolHost`, disabled tool access rejects an injected
      `remember_fact` call without mutation, and operational voice answers render fixed runtime
      snapshots rather than generated/provider prose. Fresh audible acceptance remains in the
      human-gated section below.

### 3. Capability-gated Apple Foundation Models provider
- [x] Add `ProviderCapabilities { text, streaming, structured_output, tools, vision, ocr }` and
      route only to providers qualified for every capability the request needs
- [x] Add `ABBEY_FM_MODE=off|system|pcc` (default `off`), `ABBEY_FM_ENDPOINT`, `ABBEY_FM_CLI`
      (default `/usr/bin/fm`), `ABBEY_FM_FALLBACK=1` — no implicit provider switching when unset
- [x] Add `abbey-bot --provider-self-test primary|fm|all --json`, runnable without Discord
      credentials or production state, reporting each capability independently — verified
      2026-08-21 by running the built release binary under `env -i` (no `DISCORD_TOKEN`, no
      `ABBEY_DATA_DIR`, no inherited environment at all): `primary` reports `configured:false` and
      exits 2 (unconfigured target, matching the documented contract) without touching Discord or
      a data directory; `fm` with `ABBEY_FM_MODE=system` reports real per-capability JSON bound to
      this machine's identity (`cli_sha256`, `abbey_binary_sha256`, `os_build`) and exits 0.
- [x] `fm serve` over loopback for qualified text only — never advertise the server
      endpoint as tool-capable (it silently returned prose instead of `tool_calls`)
- [x] `fm respond` schema-constrained adapter yielding either a typed final answer or exactly one
      typed Abbey tool request; stdin/argument arrays only, never a shell, never transcript saving
- [x] Prove prose-only action claims without a validated request mutate nothing and report no
      success
- [x] Enable FM tools only after all required request, argument, result-continuation, refusal,
      malformed-output, and max-round tests
- [x] Enable FM vision/OCR only on semantic fixtures (known colors/objects, exact text), not
      HTTP 200 — verified 2026-08-21: `--provider-self-test fm --json` against the real
      `/usr/bin/fm` on this Mac (macOS 27, build `26A5416b`) reports `text`/`structured_output`/
      `tools` as `pass` but `vision`/`ocr` as `fail` with `category":"semantic_vision"` /
      `"semantic_ocr"` — the gate fails closed on the actual semantic check rather than reporting
      success on mere connectivity. This is the mechanism working, not FM vision/OCR being
      production-qualified: on this exact CLI build, they are not, and must not be advertised as
      such until they pass.
- [x] Prove nothing reaches `pcc` unless `ABBEY_FM_MODE=pcc` was explicitly selected

### 4. Voice, vision, tool, and privacy safety
- [x] Deterministically re-verify consent invalidation: the exact active epoch advances and its
      media/start gates close before deliberately blocked actor cleanup; cancellation reaches the
      installed model/playback actor immediately. Every Discord withdrawal/participant/adverse-
      payload path then leaves the exact Songbird call before reaping actor state and removes the
      manager entry. Fresh physical-disconnect evidence remains a live acceptance item below.
- [x] Re-verify a new/unidentified/unattested participant never contributes a frame to STT: the
      receive classifier rejects an unknown SSRC or unattested user for the whole tick, including
      a mixed tick that also contains valid attested speech, before channel send.
- [x] Re-verify written `stop listening` authority: scoped human text is parsed before the social
      pipeline, closes/cancels the active epoch, and replies from the post-transition runtime
      snapshot; provider status/prose is excluded from authoritative copy and cannot activate,
      resume, or report a control mutation.
- [x] Image safety across every transport: 10 MB fetch cap, full local decode, 8192x8192 and
      96 MiB ceilings, preserved JPEG/PNG/WebP bytes, first-GIF-frame-to-PNG only, rejection of
      malformed/truncated/HEIC/AVIF/JXL/SVG/PDF/HTML before any provider call, safe fixed user copy
      with detail confined to redacted logs. Deterministic remote/FM preparation tests cover every
      listed format plus oversized files and canvases; live attachment acceptance remains below.
- [x] Memory privacy: `/remember`, `/forget`, `/recall` default to caller; cross-member access
      requires current Manage Messages / Manage Guild / Administrator at invocation; facts are
      whitespace-normalized, non-empty, <= 300 Unicode characters, and deletion reconciles both
      canonical JSON memory and its WDBX projection under one lock boundary.
- [x] Retitle tool descriptions from "Discord user id" to network-scoped identity language so
      Telegram and Slack identities are not misrepresented as Discord ones.
- [x] 2026-09-02 Core + Inspect source surface: production offers exactly seven tools in stable
      order whenever the global tools policy is enabled. `abbey_tools()` preserves the original
      five-tool compatibility corpus; both OpenAI-compatible/Anthropic and Foundation Models
      decision schemas expose the five Core tools followed by `inspect_status` and `list_facts`.
      The partial Inspect-only toggle was removed; only the global tools-off boundary
      applies. Inspect is read-only, scoped, non-provisioning, snapshot-consistent, and reports
      effective routable provider capabilities with safe provenance. No HTTP or Discord acting
      tools were added. Live calls remain pending.
- [x] Publish guild-keyed coarse voice Inspect state from central lifecycle transitions, limited
      to `off`, `presence`, `awaiting-consent`, `active`, or `paused`. Consent revocation, media
      revocation, actor failure, leave, and shutdown cannot leave stale `active`; DMs and other
      guilds observe `off`. No participant, identity, epoch, model, counter, timestamp, audio,
      media detail, or transcript is exposed.

### 5. Cross-platform and transport
- [x] Keep the Rust core provider-neutral: no direct MLX or Foundation Models inference
      dependencies; MLX remains an OpenAI-compatible external loopback service and Foundation
      Models remains a bounded external CLI/server adapter.
- [x] Historical source evidence: exact head `588cbe6` completed the Ubuntu and Windows source
      lanes in Actions run `33025176982` on 2026-08-27. This is not current stabilization proof.
- [ ] Linux runtime acceptance: qualify Gemma through the OpenAI-compatible seam, exercise the
      retained systemd/Docker artifacts, and use voice only through explicit OpenAI Realtime
- [ ] Windows runtime acceptance: qualify Gemma through Ollama or another conforming server and
      verify the explicit data directory plus Ctrl-C final persistence flush. Windows remains a
      documented foreground process; a Windows Service is not planned.
- [x] Expand CI to `macos` + `ubuntu` + `windows` on the pinned toolchain and locked dependencies;
      pre-stabilization `origin/main` at `9716f00` passed those source lanes in Actions run
      `33218303755`. That run proves only the pre-stabilization baseline; the final pushed head
      requires its own exact-head run.
- [x] Add a PowerShell equivalent of the portable gate; keep launchd/plist checks macOS-only and
      systemd/Docker checks Linux-only
- [x] Re-prove source-level Discord/Telegram/Slack parity at the shared seams: identical messages
      traverse the common pipeline and select the same persona; a deterministic `ImageUnderstanding`
      description is folded identically; the preserved five-tool compatibility corpus returns
      identical results while memory,
      channel, guild, and user state remains network-prefixed. Explicit reputation ids are rebound
      to the current network and conflicting prefixes cannot escape it. This is deterministic
      source/seam evidence; live Telegram/Slack round trips remain unperformed without tokens.

### Provider-routing tests (source evidence)
- [x] Capability-specific fallback; no fallback to an unqualified provider
- [x] Loopback-only server endpoint; explicit PCC/cloud opt-in
- [x] Secrets and image payloads absent from `Debug` and logs: credential-bearing backend/voice/
      vision configs and LLM/vision request types have canary tests, provider HTTP error bodies are
      discarded behind fixed status categories, and the Rust/Python/shell static privacy gate
      rejects sensitive named, captured, or positional logging expressions.
- [x] Malformed FM structured output; prose falsely claiming a tool action
- [x] Provider failure after a tool call but before continuation starts no second provider and
      cannot duplicate the completed mutation

### Gated Mac deployment (in dependency order)
- [ ] Install and verify staged MLX-Audio, retaining the previous service for rollback
- [ ] Install and verify staged MLX-VLM Gemma 4 12B
- [ ] Install and qualify the optional FM service and capability manifest
- [ ] Update the owner-only Abbey environment to the exact endpoint/model without exposing tokens
- [ ] Build and deploy Abbey through the atomic launchd installer
- [ ] Prove gated release and installed binary SHA-256 are identical
- [ ] Verify stable launchd PIDs, local-only sockets, pinned model identities, persistent-data
      continuity, gateway connection, and no voice UDP socket before consent

### Live acceptance — HUMAN-GATED, never substitutable
Use only operator-supplied sandbox inputs and the privacy-safe role labels Guild A and Guild B.
If participant consent or an authorized manager is unavailable these stay pending; source tests,
provider logs, and historical consent are explicitly not acceptable substitutes.
- [ ] Start with Guild A learning/acting enabled under a small budget and cooldown while Guild B
      remains default-off; prove no cross-guild leakage or unsolicited Guild B behavior
- [ ] Exercise all seven Core + Inspect tools, provider provenance, memory/pending snapshots,
      `/see`, `/ocr`, `/webhook`, `/forget`, bounded tool loops, and a bounded `OverBudget` result
- [ ] Swap the Guild A/B roles, repeat the isolation-sensitive subset, and restore both guilds'
      initial settings
- [ ] `/voice status` reports deployed local mode, inactive media, exact model ids, no credentials
- [ ] Fresh notification and explicit agreement from every human present (no reuse or inference)
- [ ] Authorized manager runs `/voice join consent:true` (or `resume`); public local-processing
      disclosure; unmute/undeafen only after all checks
- [ ] Wake-name turn: attributed Whisper transcription, canonical reply, Kokoro playback, human
      confirmation it was audibly heard, completed-turn counter increments
- [ ] Barge-in during playback truncates immediately and increments the barge-in counter
- [ ] Membership change closes the epoch, disconnects the call, stops STT/TTS, requires fresh consent
- [ ] Re-notified set, fresh agreement, `/voice resume consent:true` as a new epoch
- [ ] Written `stop listening` yields authoritative inactive status
- [ ] Manager `/voice leave`: no voice presence, no UDP socket, no subsequent MLX speech requests
- [ ] `/see` and `/ocr` on real JPEG/PNG/WebP/GIF against the deployed 12B backend
- [ ] Malformed, unsupported, decompression-bomb, and >10 MB uploads fail locally with safe copy
- [ ] All seven Core + Inspect tools against both sandbox role scopes, then remove any temporary fact

### Delivery
- [x] Docs distinguish source/test, provider-qualification, installed-binary, live-Discord,
      Linux/Windows CI, and untested-connector evidence; record that Go Live video is not ingested
- [ ] Commit each stabilization cycle by exact paths on canonical `main`, run the isolated strict
      gate and locked release build, fetch and require non-divergence, review the complete diff,
      then push normally without force and wait for exact-head three-platform CI
- [x] Keep the 2026-08-27 audit evidence historical: the provenance-checked, source-identical
      `openmls_rust_crypto` 0.5.1 patch moved only its HPKE manifest constraints from 0.6 to 0.7.
      Remove it when `davey` adopts a fixed upstream line.
- [ ] Resolve exactly four accepted `rustls-webpki` 0.102.8 vulnerability records after Serenity
      publishes a compatible Rustls/WebSocket edge: `RUSTSEC-2026-0049` /
      `GHSA-pwjx-qhcg-rvj4`, `RUSTSEC-2026-0098` / `GHSA-965h-392x-2mh5`,
      `RUSTSEC-2026-0099` / `GHSA-xgp8-3hg3-c2mh`, and `RUSTSEC-2026-0104` /
      `GHSA-82j2-j2ch-gfr8`. The portable migration removes native TLS/OpenSSL from Linux but is
      explicitly not audit-clean; the malformed-CRL panic remains visible. Any changed/additional
      vulnerability fails closed. Keep `derivative`, `instant`, and `proc-macro-error2` as separate
      informational warnings.
- [ ] Treat the existing manually launched process as untouched and unqualified. Current source,
      provider/model qualification, installed artifact, foreground Discord, consented voice,
      managed deployment, and real Windows runtime remain separate pending evidence layers.

## Memory relevance and intelligence layers (2026-08-20)

- [x] `src/recall.rs` — deterministic relevance selection with count + character budgets, rarity
      weighting, recency tiebreak, and an explicit stopword list (regression test: function words
      must not outrank real terms; counter-test: `go`/`ai`/`js`/`c`/`os` stay usable keys)
- [x] `PersonaContext::render(query)` focuses facts on the message and discloses the trim
- [x] `Engine::prepare` passes the user's message as the relevance query
- [x] Short fact lists render whole regardless of wording — focusing never becomes forgetting
- [x] Fact supersession — landed 2026-08-27 as a hybrid: an explicit `replaces` parameter is
      authoritative, while a model-proposed `supersedes` only QUEUES a proposal that a human
      confirms through `/pending`. Never inferred silently from free text. See the goals note.
- [x] Retrieval ranking: embedding-backed ranking was MEASURED and REJECTED on 2026-08-27, not
      deferred again. It stays lexical. The measurement and the reason are in the goals note;
      re-opening this needs a different embedding, not a tuned threshold.
- [x] Cross-guild isolation asserted by a dedicated test, not merely inherited from the
      `"{guild}:{user}"` key: a highly relevant fact in another guild stays invisible, and the same
      user still sees their own facts in the guild that owns them
## Delayed-outcome reward — closing the learning loop (2026-08-20)

Corrects a standing overstatement first: the loop was **not** a pure terminal contextual bandit
on an immediate heuristic. `brain/reward.rs` already held each reply open for a 150 s settlement
window and collected genuinely delayed evidence — reactions (±1), a human reply (+0.5), a
deletion (−2) — persisted across restarts. What was missing is that the evidence was **untyped**:
positive acknowledgement and correction signals both scored exactly +0.5, so the policy could
not tell a reply that helped from one that had to be corrected. Attribution was also keyed on the sent
message id alone, so a follow-up that was not a Discord reply-to could never reach the action
that earned it.

- [x] `brain/outcome.rs` — typed `ReplyOutcome` over observable Discord signals:
      `ExplicitThanks` (+1.0), `FollowUpQuestion` (+0.4), `RephrasedSameAsk` (−0.5),
      `Correction` (−1.0), `NoEngagement` (0.0). `NoEngagement` is deliberately weightless:
      silence is weak evidence, the −0.2 reply baseline already charges for it, and charging
      again would double-penalize it. `classify` is a deterministic lexicon plus content-word
      overlap against the ask the turn answered — not a model, and not a claim that Abbey knows
      whether she helped.
- [x] `(scope, turn id)` attribution on the existing pending ledger rather than a second one:
      `Pending` gained `scope` (scoped channel), `ask`, and `asker`, so `observe_in_scope`
      credits the newest open turn in a channel with deterministic tie-breaking.
      `ATTRIBUTION_TTL_SECS` is bound to `SETTLEMENT_WINDOW_SECS` so the two lifetimes cannot
      drift; unattributed turns drain through the existing sweep rather than leaking. `ask`
      stores the human's raw text, not the vision-enriched prompt — folding Abbey's own image
      descriptions in would pad the ask and depress every later overlap ratio.
- [x] Blended, not replaced: `outcome::blend(immediate, delayed_sum, delayed_count)` adds the
      *mean* typed value to the untouched immediate accumulator and returns `immediate`
      bit-identically when no outcome ever arrived. Every pre-existing `reward.rs` test passes
      unmodified; a regression test asserts a legacy on-disk `Pending` row (no new fields) still
      deserializes and settles at the number the old build produced.
- [x] Wired for real: `pipeline.rs` classifies an incoming message against the ask of the turn it
      answers and credits it — by exact reply-to when Discord supplies the pointer, otherwise by
      channel scope.

**Unwired, and not claimed.** Reactions still feed only the untyped path. Message edits, thread
creation, pins, leaves, and voice carry no delayed signal. In-scope attribution remains a
heuristic: a marker-only outcome (thanks/correction) is credited by scope only when it comes
from the human the turn answered, while a bystander marker is dropped; a
*topical* follow-up from anyone in the channel is still credited on overlap alone, and that
overlap is lexical, not semantic. Reply-to is the precise path. And there is no live evidence
yet: no observed settle whose reward moved because of a typed outcome.
- [ ] Live acceptance: observe `reward settled into the replay buffer` where the value reflects a
      typed outcome (thanks and correction on comparable turns), and one same-channel follow-up
      credited with no reply-to pointer. Until that lands in `tasks/goals.md`, this is a loop that
      is **closable**, not a loop observed closing.
