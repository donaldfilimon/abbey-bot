# abbey-bot Documentation Index

## Repository Root

- [`AGENTS.md`](../AGENTS.md) — Coding agent guidance (canonical, mirrors `CLAUDE.md`)
- [`CLAUDE.md`](../CLAUDE.md) — Verbatim mirror of `AGENTS.md`
- [`tasks/goals.md`](../tasks/goals.md) — Authoritative ledger (12 goals, 8 done / 4 in_progress)
- [`tasks/todo.md`](../tasks/todo.md) — Granular task checklist with live acceptance gates
- [`tasks/session-log.md`](../tasks/session-log.md) — Session work log
- [`docs/brand.md`](brand.md) — Intelligence Without Limits positioning (Abbey Bot only; never Quesar)
- [`docs/MLAI-LIVE-ACCEPTANCE.md`](MLAI-LIVE-ACCEPTANCE.md) — Operator evidence checklist for live voice + MLX (not proof a run happened)
- [`docs/activities.md`](activities.md) — Launch Abbey from the VC rocket; Entry Point `launch`; no bot Go Live; Portal URL mapping
- [`docs/live-test-protocol.md`](live-test-protocol.md) — Broader live acceptance protocol (Guild A/B, privacy boundary)

## Specifications

- [`docs/spec/`](spec/) — Reference specifications (8 files):
  - `appleintelligence.md` — ABIEngine, LanguageModel protocol, Dynamic Profiles, Tool conformances
  - `brain.md` — NeuralNetwork, DQNAgent, ReplayBuffer, StateEncoder, RewardCollector, SocialBrain
  - `adaptivelearning.md` — 18-dim state encoder, deterministic sentiment, delayed rewards, AbbeyScheduler
  - `botarchitecture.md` — Swift/Vapor/DiscordBM canonical architecture (AbbeyBot sibling product)
  - `companionapp.md` — AbbeyCompanion macOS/iPadOS SwiftUI app, SwiftData mirrors, ConfirmationGate
  - `discordbmapi.md` — DiscordBM v1.16.x gateway, REST, interactions, components, signature verification
  - `multiguild.md` — GuildConfig, GuildRegistry, per-guild personas, reply cooldown, /admin surface
  - `vision.md` — ImageUnderstanding seam, Apple Vision + remote VLM, /see and /ocr
  - `platforms.md` — SocialAdapter, SocialRouter, Discord/Telegram/Slack adapters, scoped-ID namespacing
  - `SKILL.md` — discord-abbey skill orchestration layer with reference map

## Superpowers (Internal Working Docs)

- [`docs/superpowers/README.md`](superpowers/README.md) — This directory's index (ledger, corpus, architecture, plans, specs)
- [`docs/superpowers/plans/`](superpowers/plans/) — 4 execution plans
- [`docs/superpowers/specs/`](superpowers/specs/) — 8 design specs

## Contracts

- [`contracts/abbey/`](../contracts/abbey/) — ABI Program 1 corpus (81 artifacts, 88,328 bytes, SHA-256 `72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e`)
  - `corpus/manifest.json` — Inventory with per-artifact digests
  - `corpus/compatibility.md` — Extension preservation rules
  - `corpus/v1/schemas/` — Bounded local JSON schemas (7 taxonomies)
  - `corpus/v1/fixtures/` — 52 fixtures across valid/invalid/boundary/degraded/privacy/cancellation
  - `abbey-contracts.lock.json` — Aggregate lock file
- Python gate: `scripts/check-abbey-contracts.py`
- Rust gate: `src/contracts/verify.rs` (compiled into test suite)