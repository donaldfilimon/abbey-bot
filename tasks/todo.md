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
- [x] Live smoke test: registration, mention reply, reaction reward settling, and `/admin brain` read observed
- [x] Model-initiated tools — shipped PR #19 (both wire shapes)
- [x] Local-first DAVE voice — consent/media epochs, Whisper → canonical Abbey → Kokoro, explicit degraded OpenAI backup with non-authoritative spoken control, managed sidecar, and no-Discord full-chain audition; consent invalidation physically disconnects the conversational `Decode` call
- [x] Durable live control evidence — `/voice status`, participant-change pause, and manager `/voice leave` observed
- [ ] Deploy the exact current candidate with cross-platform `gemma4:12b` reasoning/vision target, then obtain fresh everyone-present consent and observe refreshed `/voice resume`, an audible wake/reply, and barge-in
- [x] Preserve the portable OpenAI-compatible endpoint seam for macOS, Linux, and Windows; keep Ollama/llama.cpp-class runtimes available behind the same contract
- [ ] Before selecting MLX acceleration, verify its exact reasoning, tool-calling, and vision interfaces; treat Apple `fm serve` as an optional macOS adapter and do not claim MLX Gemma multimodal/tools or an installed service without evidence

## DM / live (2026-08-19)
- [x] ABBEY_BOT_LLM_MODEL, local max_tokens 4096, reasoning-only error
- [x] DM namespace per user; DM-capable commands; /persona ask through engine
- [x] Forced path loads brain; failure reply on mention/DM; typing keepalive; mention strip
- [x] ABBEY_QUIET + learning-off gate before policy
- [x] Live DM round-trip via pipeline against ollama gemma4:12b
- [x] Live Discord test via desktop control: DM ×2, guild mention ×2, commands answered (docs/live-test-protocol.md A1–A2, C1)
- [x] C3–C4: 👍 on an Abbey reply → `Rewarded` → `reward settled into the replay buffer … loaded=true` (17:16Z)
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
- [x] live: streaming DM observed (posted :39, final :44, "(edited)"); concurrency serialisation not observed (unit-tested)

## Rust 2026 modernization (2026-08-19)
- [x] Fast-forward local `main` to the current upstream baseline
- [x] Pin Rust 1.97.1 and declare `rust-version`
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
- [ ] Re-prove voice stays read-only: spoken turns cannot invoke tools, mutate memory, or claim a
      voice-control action occurred

### 3. Capability-gated Apple Foundation Models provider
- [ ] Add `ProviderCapabilities { text, streaming, structured_output, tools, vision, ocr }` and
      route only to providers qualified for every capability the request needs
- [ ] Add `ABBEY_FM_MODE=off|system|pcc` (default `off`), `ABBEY_FM_ENDPOINT`, `ABBEY_FM_CLI`
      (default `/usr/bin/fm`), `ABBEY_FM_FALLBACK=1` — no implicit provider switching when unset
- [ ] Add `abbey-bot --provider-self-test primary|fm|all --json`, runnable without Discord
      credentials or production state, reporting each capability independently
- [ ] `fm serve` over loopback for qualified text and image only — never advertise the server
      endpoint as tool-capable (it silently returned prose instead of `tool_calls`)
- [ ] `fm respond` schema-constrained adapter yielding either a typed final answer or exactly one
      typed Abbey tool request; stdin/argument arrays only, never a shell, never transcript saving
- [ ] Prove prose claiming a tool action ("I will remember that") without a validated request
      mutates nothing and reports no success
- [ ] Enable FM tools only after all five pass request, argument, result-continuation, refusal,
      malformed-output, and max-round tests
- [ ] Enable FM vision/OCR only on semantic fixtures (known colors/objects, exact text), not
      HTTP 200
- [ ] Prove nothing reaches `pcc` unless `ABBEY_FM_MODE=pcc` was explicitly selected

### 4. Voice, vision, tool, and privacy safety
- [ ] Re-verify consent invalidation synchronously closes the exact media epoch before slow
      cleanup, cancels pending model work and playback, and disconnects the Songbird `Decode` call
- [ ] Re-verify a new/unidentified/unattested participant never contributes a frame to STT
- [ ] Re-verify written `stop listening` deterministically revokes the active epoch, and that
      generated/Realtime prose can never claim listening state changed
- [ ] Image safety across every transport: 10 MB fetch cap, full local decode, 8192x8192 and
      96 MiB ceilings, preserved JPEG/PNG/WebP bytes, first-GIF-frame-to-PNG only, rejection of
      malformed/truncated/HEIC/AVIF/JXL/SVG/PDF/HTML before any provider call, safe fixed user copy
      with detail confined to redacted logs
- [ ] Memory privacy: `/remember`, `/forget`, `/recall` default to caller; cross-member access
      requires current Manage Messages / Manage Guild / Administrator at invocation; facts
      normalized, non-empty, <= 300 Unicode chars, removed from both plain memory and WDBX
- [ ] Retitle tool descriptions from "Discord user id" to network-scoped identity language so
      Telegram and Slack identities are not misrepresented as Discord ones

### 5. Cross-platform and transport
- [ ] Keep the Rust core provider-neutral: no direct MLX or Foundation Models dependencies; Mac
      services stay external loopback processes
- [ ] Linux: same binary builds and tests, Gemma via the OpenAI-compatible seam, systemd and
      Docker retained, voice only via explicit OpenAI Realtime
- [ ] Windows: same binary builds and tests, Gemma via Ollama or another qualified server,
      explicit data directory, Ctrl-C with a final persistence flush, PowerShell setup docs, no
      Windows Service
- [ ] Expand CI to `macos` + `ubuntu` + `windows` on the pinned toolchain and locked dependencies
- [ ] Add a PowerShell equivalent of the portable gate; keep launchd/plist checks macOS-only and
      systemd/Docker checks Linux-only
- [ ] Re-prove the shared Discord/Telegram/Slack pipeline reaches identical persona/tool/memory/
      vision behavior with no identity leakage between networks

### Provider-routing tests (source evidence)
- [ ] Capability-specific fallback; no fallback to an unqualified provider
- [ ] Loopback proxy bypass; explicit PCC/cloud opt-in
- [ ] Secrets and image payloads absent from `Debug` and logs
- [ ] Malformed FM structured output; prose falsely claiming a tool action
- [ ] Provider failure after a tool call but before continuation

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
Guild `1009583217948491928`, voice channel `1486123994611585135`. Per the specification, if
participant consent or an authorized manager is unavailable these stay pending; source tests, MLX
access logs, and historical consent are explicitly not acceptable substitutes.
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
- [ ] All five tools against an isolated test user/guild scope, then remove any temporary fact

### Delivery
- [ ] Docs distinguish source/test, provider-qualification, installed-binary, live-Discord,
      Linux/Windows CI, and untested-connector evidence; record that Go Live video is not ingested
- [ ] Commit in reviewable groups (provider/deployment, safety/core, cross-platform tests/docs) and
      push the branch **without merging**
- [ ] Keep `cargo audit` honest and non-green: rustls-webpki plus DAVE/OpenMLS/libcrux advisories
      stay documented, with no hand-maintained cryptographic fork
