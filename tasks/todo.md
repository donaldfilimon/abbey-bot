# Todo — discord-abbey spec in Rust

## Pure modules (parallel agents, no serenity/poise imports)
- [x] brain/nn.rs, brain/replay.rs, brain/dqn.rs — NeuralNetwork (linear/softmax output, SGD, clip), ReplayBuffer, DqnAgent + BrainSnapshot
- [x] brain/state.rs, brain/intent.rs, brain/reward.rs — BotAction, StateEncoder(18), Sentiment, IntentClassifier, RewardCollector (injected clock)
- [x] brain/social.rs, brain/registry.rs, guild.rs — SocialBrain reputation, BrainRegistry per guild, GuildSettings/GuildRegistry/ReplyCooldown
- [x] wyhash.rs, embedding.rs, wdbx.rs — Zig-compatible wyhash (pinned to ref vectors), text_embedding transcription, WDBX v1 JSONL namespace store + cosine recall
- [x] memory.rs, engine.rs, llm.rs multi-turn — UserMemory facts, ChannelContext, InteractionLog, PersonaContext assembly, per-scope sessions
- [x] platform.rs, vision.rs — SocialEvent model + Discord/Telegram/Slack translation; ImageUnderstanding seam (remote VLM request/extract, mime sniff)

## Discord shell (orchestrator)
- [x] gateway.rs — serenity EventHandler: message → pipeline (intent/state/DQN/cooldown/reply), reactions → rewards, delete → penalty, guild create/delete
- [x] commands: /remember /forget(autocomplete) /recall /reputation /summarize /admin /stats /see /ocr
- [x] main.rs wiring: Data state, ABBEY_DATA_DIR persistence, scheduler tasks (learn 30s / flush 60s / persist 300s / reward sweep 30s), intents (non-privileged + opt-in MESSAGE_CONTENT)
- [x] Telegram long-poll adapter (live), Slack Socket Mode adapter (live)
- [x] README + CLAUDE.md/AGENTS.md updates, .env.example, gate green via ./check.sh

## Open (after this pass)
- [x] Live smoke test: registration, mention reply, reaction reward settling all observed 2026-08-19 (`/admin brain` read still pending)
- [x] Model-initiated tools — shipped PR #19 (both wire shapes)
- [ ] voice.md was not supplied — voice remains out of scope

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
- [ ] Exercise `/reputation`, `/stats`, `/remember`, `/see` individually once the DM slash picker shows them; Telegram/Slack adapters need their tokens

## Guild learning loop (plan 2026-08-19)
- [x] T1 brain/budget.rs
- [x] T2 brain/telemetry.rs
- [x] T3 guild.rs act/budget settings
- [x] T4 registry stats
- [x] T5 runtime budget + settle→stats
- [x] T6 pipeline gates/budget/telemetry
- [x] T7 /admin act, /admin budget, /admin brain, /stats
- [x] T8 docs + PR
- [ ] T9 live acceptance — `/admin act on`, policy decisions, reacts, cooldown, settle all observed; residual: an `OverBudget` refusal and a `/admin brain` read

## Reply quality & speed (2026-08-19)
- [x] benchmark 5 local models with Abbey's prompt
- [x] tidy_reply + tests; wired at every generation site
- [x] generation semaphore + queue timeout + busy copy
- [x] SSE accumulator, StreamTransport, Outbound::edit, stream_reply + tests
- [x] Anthropic→local fallback (AppState::chat)
- [x] live: streaming DM observed (posted :39, final :44, "(edited)"); concurrency serialisation not observed (unit-tested)
