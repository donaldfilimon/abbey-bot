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
- [ ] Live smoke test: token + `ABBEY_GUILD_ID` + `ABBEY_MESSAGE_CONTENT=1` + a backend; confirm command registration, a mention reply, a reaction reward settling, `/admin brain` moving
- [ ] Model-initiated tools (spec `appleintelligence.md` `Tool` conformances) on the Anthropic path — design decision first
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
- [ ] ABBEY_BOT_LLM_MAX_TOKENS / TIMEOUT tunables (Proposed)
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
- [ ] T9 live acceptance (MESSAGE_CONTENT) — Donald enables intent
