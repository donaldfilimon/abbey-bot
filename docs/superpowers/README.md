# Superpowers — Plans, Specs, and Architecture

**Not in Mintlify nav** — this directory is internal working documentation.

## Ledger

The authoritative ledger is `tasks/goals.md` (8 goals):

| Goal | Status |
|------|--------|
| Program 1 stable-Rust contract conformance | `done` |
| Full-duplex Abbey voice in Discord Engineering | `in_progress` |
| Implement the discord-abbey spec suite in Rust (abbey-bot) | `done` |
| DMs work end-to-end and the smart features are exercised live on Discord | `done` |
| Guild learning loop acts in opted-in servers (sub-project 3 of "improve all") | `in_progress` |
| Reply quality & speed (sub-project 1 of "improve all") | `done` |
| Smarter agent — tools (sub-project 2 of "improve all") | `done` |
| Breadth & ops (sub-project 4 of "improve all") | `in_progress` |
| Self-learning hardening (continuation of "improve all") | `done` |
| Modernize the Rust codebase and harden network boundaries | `done` |
| Complete all unfinished .md files | `done` |
| Complete Abbey: MLX Gemma 4 12B, vision, tools, voice, and cross-platform support | `in_progress` |

## Corpus

The contract corpus is `contracts/abbey/` — ABI vendored at revision `72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e` (81 artifacts, 88,328 bytes). Dual gate: Python (`scripts/check-abbey-contracts.py`) and Rust (`src/contracts/verify.rs`) independently verify lock, inventory, byte, digest, and privacy-taxonomy drift using closed reason codes and corpus-relative paths.

## Architecture

The architecture reference is `docs/spec/botarchitecture.md` — Swift/Vapor/DiscordBM canonical architecture (this is the Swift sibling product `AbbeyBot`, not the Rust `abbey-bot` crate). The Rust crate's architecture lives in `AGENTS.md` and `CLAUDE.md` at the repository root.

## Plans

Located in `docs/superpowers/plans/` (5 plans):

- `2026-08-19-guild-learning-loop.md` — per-guild opt-in, hourly budget, BrainStats, ABBEY_QUIET precedence
- `2026-08-19-finishing.md` — completion checklist for the 2026-08-19 spec slice
- `2026-09-02-provider-routing.md` — capability-gated routing, fallback semantics, self-test contract
- `2026-09-02-embedded-skills-stabilization.md` — skill sync, CLI adapters, cross-CLI drift repair
- `2026-09-03-residual-ops.md` — residual operational follow-ups after the 2026-09-03 wave

## Specs

Located in `docs/superpowers/specs/` (7 specs):

- `2026-08-19-guild-learning-loop-design.md` — design for guild-scoped learning loop
- `2026-08-19-reply-quality-speed-design.md` — streaming, tidy_reply, semaphore, Anthropic fallback
- `2026-08-19-tools-design.md` — tool vocabulary, dispatch, ToolHost, generation loop
- `2026-08-20-live-voice-design.md` — consent epochs, Songbird media, cancellation, degraded backup
- `2026-08-21-test-module-extraction-design.md` — pure module extraction strategy
- `2026-09-02-provider-routing-design.md` — ProviderCapabilities, FM gating, self-test, loopback-only
- `2026-09-02-embedded-skills-design.md` — skill-loop MCP, cross-CLI sync, runtime-native adapters
- `2026-08-19-tools-design.md` (also listed above) — model-initiated tools, both wire shapes

Additional reference specs in `docs/spec/` (9 specs):

- `appleintelligence.md` — ABIEngine, LanguageModel protocol, Dynamic Profiles, Tool conformances
- `brain.md` — NeuralNetwork, DQNAgent, ReplayBuffer, StateEncoder, RewardCollector, SocialBrain
- `adaptivelearning.md` — 18-dim state, deterministic sentiment, delayed rewards, AbbeyScheduler
- `botarchitecture.md` — Swift/Vapor/DiscordBM full architecture (this is AbbeyBot, not abbey-bot)
- `companionapp.md` — AbbeyCompanion macOS/iPadOS SwiftUI app, SwiftData mirrors, ConfirmationGate
- `discordbmapi.md` — DiscordBM v1.16.x gateway, REST, interactions, components, signature verification
- `multiguild.md` — GuildConfig, GuildRegistry, per-guild personas, reply cooldown, /admin surface
- `vision.md` — ImageUnderstanding seam, Apple Vision + remote VLM, /see and /ocr
- `platforms.md` — SocialAdapter, SocialRouter, Discord/Telegram/Slack adapters, scoped-ID namespacing