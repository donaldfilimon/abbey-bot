# CLAUDE.md

This file provides guidance to coding agents working in this repository.
`README.md` is written for a human running the bot — commands, env vars, and
per-feature design notes. Read it first. This file holds what the README leaves
out: the shape of the codebase, the rules that keep it that shape, and the traps
this project has already hit.

`AGENTS.md` and `CLAUDE.md` are verbatim mirrors except for their header line.
Apply every body edit to both, or they drift.

## Commands

```bash
./check.sh              # gate: fmt, deploy/privacy/contracts/rustsec/wdbx python checks, clippy -D warnings, tests, release build
ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh   # strict form used for release evidence (needs ../wdbx)
./check.ps1             # Windows gate; skips the POSIX/plist checks and three deploy python tests
cargo test <name>       # single test, substring-matched against full path;
                        # no -p or --workspace (single binary crate, tests in bin)
cargo run               # run the bot; needs DISCORD_TOKEN (or DISCORD_BOT_TOKEN fallback), optional ABBEY_GUILD_ID
./target/release/abbey-bot --provider-self-test primary --json   # --json is mandatory; exit 0 pass, 1 probe failure, 2 bad args/configuration
./target/release/abbey-bot --provider-self-test fm --json
./target/release/abbey-bot --provider-self-test all --json
./target/release/abbey-bot --voice-self-test out.wav             # token-free local TTS → STT loop; refuses to overwrite
python3 scripts/check-abbey-contracts.py                         # every scripts/*.py gate check runs standalone; --root defaults to contracts/abbey
deploy/install-launchd.sh [--uninstall]                          # Discord bot service; MLX installers listed below
```

`./check.sh` shells out to `cargo audit` and `scripts/check-rustsec-debt.py`
requires exactly version `0.22.2` (CI installs it); the Linux dependency tree is
checked to be Rustls/WebPKI-only, so a dep that pulls `native-tls`/`openssl`
fails the gate. Accepted RUSTSEC debt is enumerated in
`security/rustsec-accepted-debt.json` and the checker fails closed on any
missing, extra, or changed record. `.claude/settings.json` has a `PostToolUse`
hook that runs the full `./check.sh` in the background after every `.rs`
Write/Edit, and both it and `.codex/hooks.json` append to the gitignored
`tasks/session-log.md`; `tasks/goals.md` and `tasks/todo.md` are the ledgers.

`cargo test moderation::` runs one module's tests. Gate runs `--locked` on
purpose: a Cargo.toml bump without a regenerated lock keeps CI green while every
deploy build dies. The gate proves the property the deploy depends on — do not
remove the flag to "fix" a lock error; regenerate the lock.

The gate runs `scripts/check-wdbx-conformance.py`. With the canonical sibling
`../wdbx` checkout present, it compares frozen WDBX-v1 fixtures byte for byte.
Standalone CI reports that external layer as explicitly skipped; set
`ABBEY_REQUIRE_WDBX_CONFORMANCE=1` (and `ABBEY_WDBX_REPO`) for an
integration/release run where absence must fail.

## Architecture: pure core, thin Discord shell

The pure modules hold every decision the bot makes, and **none of them import
serenity or poise**. The entire decision suite runs with no gateway connection.

The Discord/network surface (imports serenity/poise / adapters):

| File | Role |
|---|---|
| `commands.rs`, `commands_brain.rs`, `commands_voice.rs` (+ `commands_voice/`) | Translate Discord data into plain structs and lifecycle calls |
| `gateway/` (`discord.rs`, `shared.rs`, `slack.rs`, `telegram.rs`) | `discord.rs` matches eight `FullEvent` variants (Ready, Message, ReactionAdd/Remove, MessageDelete, GuildCreate/Delete, GuildMemberAddition — reactions and deletes feed rewards, not replies) into `SocialEvent` → `pipeline::handle`; `shared.rs` holds the `Outbound` impl and `clamp_message`; Slack/Telegram are serenity-free adapters |
| `voice_session.rs`, `voice_local.rs`, `voice_openai.rs`, `voice_self_test.rs` (+ `voice_session/`, `voice_local/`) | Songbird-facing voice actors (see the voice paragraph below) |
| `main.rs` | Env parsing and framework wiring only; reads no guild data |

Pure subsystems the table above does not name but that you will touch:
`pipeline.rs` (decides *whether* to speak; the shell contract is its `Outbound`
trait, which is what lets the decision path run behind a recording fake),
`generation.rs` (decides *how*: progressive posting after 60 chars or 4 s, edits
at most every 2 s, `StreamEnd::{Text,Calls}` is the tool-loop seam),
`brain/` (DQN `[18, 64, 32, 3]`, replay 10k, reward settlement), `persist.rs`
+ `wdbx.rs` (no database: one `abbey-state.json` plus one `wdbx.seg.0.jsonl`
segment under `ABBEY_DATA_DIR`, temp-file + rename writes; WDBX keys are
`mem:{scoped_guild_id}:{vector_id}` with a scoped-user filter on read, which is
the privacy boundary), `provider/` + `provider.rs` + `provider_self_test.rs`
(`--provider-self-test` runs the legacy primary/FM probes only; the generic
route catalog and discovery in `provider/` and the `ABBEY_PROVIDER_*` env
family they parse are exercised by their own unit tests and are not yet wired
into the CLI or the running bot, so setting those variables changes nothing
at runtime), `contracts/` (`#[cfg(test)]`-only guard over the
pinned 81-artifact ABI corpus), `routing_signals.rs`, `grounding.rs`,
`recall.rs`, `vad.rs`, `offline_voice.rs`.

**Transcribe, never depend.** `wdbx.rs`, `embedding.rs`, `wyhash.rs`, and
`persona.rs` carry local transcriptions of ABI formats or algorithms (golden
vectors pin `wyhash`; the `wyhash` crate returns different values) rather than
a dependency on the `abi` workspace. `ask.rs` transcribes Aviva and Abi's
contract descriptions and suffixes; Abbey intentionally follows the separate
Grok Bot Abbey voice, and response prefixes are not transcribed. `persona.rs` is frozen
(29 keywords, 0.40/0.30/0.30 prior) and is not edited; new routing intelligence
composes on top in `routing_signals.rs`.

**If you find yourself writing an `if` inside a `#[poise::command]` function that isn't about fetching data, it belongs in a pure module instead.**

**Abbey default voice (code):** warm, sharp friend — result-first, clear, honest about uncertainty (`src/persona.rs` / `src/ask.rs`). Do not rewrite prompts toward help-desk filler.

## Live Mac backends (2026-09-03 — fail closed)

- **Reasoner:** Ollama host-only `ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434`. `src/llm/dialect.rs` appends `/v1/chat/completions`. Do **not** put `/v1` on the LLM base URL.
- **Vision:** keeps `/v1` (live: `http://127.0.0.1:11434/v1`).
- **MLX-Audio:** launchd `127.0.0.1:8181` via `deploy/install-mlx-audio-launchd.sh`. Operator readiness: `GET /` or `GET /v1/models` — **not** `/health` as the probe.
- **MLX-VLM:** `:8282` **unpublished**. Staged 4-bit Gemma tool-result continuation loops `<|channel>thought` into content until length; installer still fail-closes on `TOOL_CONTINUATION_READY`. Ollama remains the reasoner. Do not point `ABBEY_BOT_LLM_ENDPOINT` at `:8282` until that probe passes on the exact snapshot.
- **Discord ops:** Bot API + launchd (`deploy/install-launchd.sh`), not an Electron UI. Gap-fill only; **no mass Member grant** without Donald.
- **Gate / installers:** `./check.sh`; `deploy/install-mlx-audio-launchd.sh`; `deploy/install-mlx-vlm-launchd.sh` (publish only after smokes pass).

## Rules that are not preferences

**Defer before touching the network, unconditionally.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. Every command calls `ctx.defer()` or `ctx.defer_ephemeral()` first.
A command that defers only when it looks slow is a command that races eventually.
The one exception: `/voice leave` closes the voice media gate before its first
await, so its guard paths answer the interaction directly inside the 3-second
window and the defer runs concurrently with the transition lock.

**Never declare a `GuildChannel` parameter.** Poise resolves it with a REST fetch
*during argument parsing*, before the body and its defer ever run. Take `ChannelId`
and fetch after deferring, the way `/perms` does.

**Every rendered answer passes through `clamp_message`.** Discord rejects messages
over 2,000 codepoints after the defer has already succeeded, surfacing as "Message
too large." Every call that posts the output of a pure module is wrapped through
`clamp_message`. The exception is fixed guard strings of known length (e.g.
"This one only works inside a server.", thread-redirect line that interpolates a
channel id).

**Intents stay `non_privileged()` by default.** That set carries guild message and
reaction events; it does *not* carry message content, presence, or the member list.
`/whois`, `/perms`, and `/modcall` fetch over REST instead. The opt-in
`ABBEY_MESSAGE_CONTENT=1` must be enabled in the Dev Portal *and* set here — both,
or the gateway silently sends nothing.

**An ordinary unsolicited message is gated five times before the policy is
consulted** (`pipeline::guards`, in order): own traffic is ignored first, then the content
guard (passes if there is text, an attachment, or the message is forced), then
`ABBEY_QUIET=1` (operator, wins over everything) → the guild's `/admin act on`
(opt-in, default off) → `/admin learning off`. After the policy picks reply/react,
`RateLimits::try_acquire` checks the per-guild hourly budget (`brain/budget.rs`,
default 6/h, ceiling 60) and then the per-channel cooldown (default 20 s, ceiling
600) atomically under one lock pair; the Reply branch acquires only after the
backend check so a missing backend costs no budget. `OverBudget` and `CooledDown`
record **no** experience; a `Stay` decision does record a silence experience.
Replies stay open for a 150 s settlement window with a −0.2 baseline
(`brain/reward.rs`), so engagement has to earn the reward back. Generated
welcomes (`RouteDecision::Welcome`) are the exception: that branch runs only
`check_unsolicited` (quiet and act-off) and returns without `guards`, the
learning gate, the policy, or the rate limits.

**DMs are one-person guilds.** `SocialEvent::scoped_guild_id` returns
`"{network}:dm:{user}"` when there is no guild. A shared `"discord:dm"` would let
semantic recall surface one person's facts to another — the pipeline test
`two_dm_users_never_share_recall_or_facts` pins this.

**The forced path (mention/DM) loads the guild's brain before replying.**
`BrainRegistry::remember` drops experiences for unloaded guilds; without the touch,
every mention reply's reward settles into nothing. A forced reply that fails at the
backend posts `ask::render_failure` instead of dead air, and the typing indicator is
re-broadcast every 8 s while a local model thinks.

**Tool capability is explicit at the generation boundary.** Mentions, DMs, and
`/persona ask` enter `generation::generate_with_tools*` with a live `ToolScope`;
unsolicited policy replies, voice, and `/summarize` enter a read-only function
with an explicit persona and never construct the vocabulary or host. Production
exposes exactly seven tools in this stable order:
`remember_fact`, `lookup_reputation`, `recall`, `switch_persona`,
`recent_messages`, `inspect_status`, `list_facts`. `ABBEY_BOT_LLM_TOOLS=off`
suppresses the complete vocabulary; there is no partial Inspect toggle.

**`/persona ask` answers come from a backend or says so — never from the bot.**
`llm::Backend::from_values` picks `ANTHROPIC_API_KEY` first, else
`ABBEY_BOT_LLM_ENDPOINT` (an OpenAI-compatible base URL; the bot appends
`/v1/chat/completions`), else none — and a blank value counts as unset because
`.env.example` ships blank assignments. With no backend the command posts
`ask::degraded_reply`, which names the routed persona and states plainly that
nothing can answer; a template echo dressed up as AI is what the
2026-08-10 proposal forbids.

**Fetch over REST, not from the cache.** The cache is only as complete as the
intents held, so a cache read produces a silently thinner answer instead of an
error.

**Whether a moderator may act goes through `hierarchy_blocker`.** It encodes
Discord's refusal rules in one place — the owner cannot be actioned,
administrators cannot be timed out, and the actor's top role must sit *strictly*
above the target's — and returns the sentence explaining a refusal rather than a
bool, so the answer stays actionable. Any acting command added later must consult
it, not re-derive the rules at the call site.

## Traps this repository has already hit

**`Permissions` does not `Debug` into flag names.** It prints `Permissions(3072)` —
a raw bitfield. An early version derived permission names by scraping that and
would have rendered numbers into chat. Use `get_permission_names()`, which returns
client-facing strings (`"View Channel"`, `"Ban Members"`). Two tests pin the
strings this codebase hardcodes against that vocabulary, because a typo there
fails silently — `/modcall` would tell every moderator they cannot act.

**`Backend` and `LlmRequest` hand-write `Debug` — never `#[derive(Debug)]` on
anything that carries a credential.** Both hold the Anthropic key (in the enum
payload, and in the `x-api-key` header), and a derived `Debug` printed it in
full through paths nobody plans for — a `tracing` field, a panic message, a
failing `assert_eq!` in CI logs (PR #9 is the fix). If you add a type that holds
a secret, give it the same treatment and the same test. Related: the key travels
in a header, never in the URL, so no error message can include it.

**Match overwrites on snowflake id, never on name.** Discord permits two roles in
one guild to share a display name — divider roles routinely do — so a name-based
match pulls a stranger's overwrite into the chain and produces a confidently
wrong walkthrough. `perms::Scope` therefore carries the id alongside the name:
match on the id, render the name. The neighbouring shape rule is that
`perms::explain` takes a *pre-formatted* `channel_label` (`#general`,
`🔊 Lobby`), because only the caller knows the channel kind — a hardcoded `#`
here misrendered voice channels and categories.

**`GuildId::new` panics on zero.** It does not return an error, so a literal
`ABBEY_GUILD_ID=0` parses as a valid `u64` and then aborts the process — the
opposite of the fail-fast-with-a-sentence startup path the rest of `main.rs`
maintains. The explicit zero check before the call is that guard; do not fold it
away as redundant with the parse.

**A test can be structurally incapable of failing.** `MAX_TIMEOUT_MINUTES` is
enforced by a clamp that *every* `Action::Timeout` is constructed through, which
means the ladder sweep asserting no rung exceeds the ceiling can never fail no
matter what a future rung asks for — the clamp has already capped it. Only
`timeout_clamps_beyond_discords_ceiling`, which calls the constructor directly
with an over-long value, actually exercises the constant. When a constant becomes
load-bearing to silence a dead-code lint, check whether the test that justified it
still tests anything.

**Dead-code lints: this is a binary crate, so `pub` exempts nothing.** Clippy
runs with `-D warnings`, and a `pub` constant used only by tests is an error.
This has come up twice, and the resolution both times was one of two honest
moves, never `#[allow]`:

- *Make it load-bearing* if it deserves to be. `MAX_TIMEOUT_MINUTES` became a
  clamp every `Action::Timeout` is built through; `normalize_text_name` now runs
  inside `server::render`. Both are better code than before the lint fired.
- *Mark it `#[cfg(test)]`* if it is genuinely a specification constant the
  property tests enforce — `Archetype::ALL`, `NEVER_FOR_EVERYONE`,
  `MAX_CHANNELS_PER_CATEGORY`, `Action::severity_rank`.

**Discord rewrites text channel names and leaves voice names alone.** Text and
forum names are lowercased with whitespace hyphenated, so a blueprint saying
`General Chat` describes a server you do not get; `Squad 1` is a perfectly legal
voice channel. `server::render` normalizes per channel kind, and a test asserts
the voice exemption is actually exercised so the asymmetry cannot rot into an
untested claim.

**Voice is five actors with one cancellation rule.** `voice.rs` is the
provider-neutral policy; `voice_session.rs` serializes the lifecycle with
epoch-based cancellation (rejoin/pause/leave/move/shutdown advance the epoch
first, one provider task and one playback handle at a time); `voice_local.rs`
is the default local STT → cognition → TTS actor whose audio callbacks only
enqueue frames; `voice_openai.rs` is the explicit `ABBEY_VOICE_MODE=openai`
backup; `offline_voice.rs` is the MLX-Audio loopback client and knows nothing
about Discord; `vad.rs` is the single VAD trait (`EnergyVad`, `SemanticVad`,
`ComposedVad`) so the two paths cannot drift. Raw audio and transcripts are
never persisted, and while an operator verification run (`/voice verify`) is
armed, conversational commits are disabled.

**Everything pure takes `now: u64` and a seed; nothing pure reads the clock or
`rand`.** `runtime::now()` is the single wall-clock read; `brain::nn::Rng` is a
splitmix64 seeded by the caller. This is what makes reward settlement, cooldown,
eviction, and the DQN all deterministic under test. A `SystemTime::now()` or a
`rand` import inside `brain/`, `guild.rs`, `memory.rs`, `engine.rs`, or `wdbx.rs`
is a regression, not a convenience.

**`Experience` keys by guild, and reputation keys by `(guild, user)` — never by
a joined `"guild:user"` string.** Scoped ids already contain a colon
(`discord:123`), so the spec's split-on-first-colon would mis-split.
`persist.rs` joins with U+001F for the same reason.

**A passing property test is not evidence that output reads well.** The `/server`
blueprints passed every invariant while rendering `🗂help` with the glyph jammed
against the name. Print the rendered string and read it before shipping anything
a user sees.

## Deploy artifacts (reference only)

Configuration is env-only: `DISCORD_TOKEN` (or the `DISCORD_BOT_TOKEN`
fallback — primary wins, and a present-but-blank primary is an error that does
**not** fall through), optional `ABBEY_GUILD_ID`, `RUST_LOG`, backend vars
(`ANTHROPIC_API_KEY`, `ABBEY_BOT_LLM_ENDPOINT`, `ABBEY_BOT_LLM_MODEL`,
`ABBEY_BOT_LLM_CONCURRENCY`/`_QUEUE_SECS`/`_TIMEOUT_SECS`),
`SLACK_*`/`TELEGRAM_BOT_TOKEN`, and the
`ABBEY_VOICE_*` set (`ABBEY_VOICE_MODE=local` is the default and is never
inferred from key presence). Blank handling is variable-specific: backend
and voice parsers normalize optional blank values, but a present blank Discord
token fails, and `ABBEY_GUILD_ID` must be absent or a nonzero numeric snowflake.
Launchd uses its fixed private
data path and ignores an `ABBEY_DATA_DIR` line in the env file. The token lives in
`/etc/abbey-bot/env` (systemd) or `~/.config/abbey-bot/env` (launchd), never
baked into image layers.

`./check.sh` is the gate: fmt, then deploy shell/python/plist checks (including
mlx installers, privacy, contracts, rustsec, wdbx), then
`cargo clippy --all-targets --locked -- -D warnings`, then `cargo test --locked`,
then `cargo build --release --locked`.

## What has and has not been verified

Authoritative live Mac acceptance notes live in `docs/MLAI-LIVE-ACCEPTANCE.md`
(refresh there, not here; it already carries 2026-09-04 entries newer than this
file, and README's nine-layer evidence ladder is the rule for what each layer
proves — passing one never implies the next). As of 2026-09-03 ET: Ollama `:11434` is the reasoner;
MLX-Audio `:8181` is up; MLX-VLM `:8282` is not published (tool-continuation
blocked). Do not treat README MLX primary cutover snippets as the current
launchd primary.

## Related, and easy to confuse

**The authoritative live checkout is `~/dev/active/abbey-bot`.** The former
`~/sources/repos/abbey-bot` path is absent; a discarded redundant clone under
Trash is not an authority. Concurrent sessions can still share the active
working tree, so inspect its current status before editing and fetch before
making claims about `origin/main`.

`~/dev/archive/swift-discord` is a home-grown Swift Discord library with its
own gateway and REST targets. It shares no code with this crate and is not a
dependency, a port source, or a reference implementation. The separate active
Swift/Vapor/DiscordBM product is `~/dev/active/AbbeyBot`; it shares no code
with this Rust crate. Treat its architecture as an adjacent implementation, not
a dependency or source of runtime truth for this project.

## Host music output

`commands_voice/play.rs` controls native Spotify/Music and the independent music
track; `audio_tap.rs` validates loopback-only sidecar HTTP and converts bounded
s16le frames to the f32 PCM required by RawAdapter. `player_control.rs` and
`music.rs` hold script selection, input validation, access and ducking decisions.
Music arguments are argv data, never interpolated AppleScript source.

Music never creates or extends listening consent. `/voice resume consent:true`
keeps its existing meaning; `/voice resume-music` is output-only. Consent teardown
must destroy Decode before publishing `music_consent_teardown_complete` for the
exact resulting epoch. Only then may music reconnect with DecodeMode::Pass and
self-deafen. `/voice leave` synchronously cancels the music token as well as the
listening gate. Providers use `play_input` and replace only their own TTS handle;
`play_only_input` would silently destroy the independently owned music track.

The Swift and Rust offline tests share the header and idle-health fixtures in
`tools/abbey-audio-tap/Tests/AudioTapCoreTests/Fixtures`. They start no real capture.
The sidecar already excludes identified Discord, browser and terminal sources;
source evidence remains separate from permission, installation and audible
Discord/feedback acceptance. Do not run the installer, permission command or
production stream endpoint as part of source validation.
