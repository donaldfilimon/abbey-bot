# CLAUDE.md

This file provides guidance to coding agents working in this repository.

`README.md` is written for a human running the bot — commands, env vars, and the
per-feature design notes. Read it first. This file holds what the README leaves
out: the shape of the codebase, the rules that keep it that shape, and the traps
this project has already hit.

`AGENTS.md` is a verbatim mirror of this file for non-Claude agents — only the
header line differs. Apply any edit to both, or they drift.

## Commands

```bash
./check.sh          # gate: fmt, deploy/lock validation, clippy -D warnings, tests, release build
./check.ps1         # the same source gate on Windows (without POSIX/plist-only checks)
cargo test <name>   # single test, substring-matched against the full path
cargo run           # needs DISCORD_TOKEN; see README

# Synthetic provider qualification; runs before Discord/state initialization:
./target/release/abbey-bot --provider-self-test primary --json
./target/release/abbey-bot --provider-self-test fm --json
./target/release/abbey-bot --provider-self-test all --json

# Live run the way it is actually operated here (persists state; stop with SIGINT):
#   set -a; . ./.env; set +a
#   ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434 ABBEY_BOT_LLM_MODEL=gemma4:12b \
#   ABBEY_VISION_ENDPOINT=http://127.0.0.1:11434/v1 ABBEY_VISION_MODEL=gemma4:12b \
#   ABBEY_DATA_DIR=<dir> RUST_LOG=info,abbey_bot=debug ./target/debug/abbey-bot
#   pkill -INT -f target/debug/abbey-bot      # SIGINT → persists learning/memory first
# End-to-end against a real local model, no Discord (ignored by the gate):
#   ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434 ABBEY_BOT_LLM_MODEL=gemma4:12b \
#   cargo test live_dm -- --ignored --nocapture
```

`cargo test moderation::` runs one module's tests. There is no `-p`
and no `--workspace` — this is a single binary crate. Note that means test
targets are inside the `bin`, so `--lib` matches nothing.

The gate runs `--locked` on purpose: the Dockerfile builds `--locked`, and
before the gate did too, a Cargo.toml bump without a regenerated lock kept CI
green while every deploy build died. The gate proves the property the deploy
depends on — do not remove the flag to "fix" a lock error; regenerate the lock.

**CI runs the platform's real gate.** `.github/workflows/rust.yml` runs a
non-fail-fast Ubuntu/macOS/Windows matrix on every push and PR to `main`.
Ubuntu and macOS execute `./check.sh`; Windows executes `./check.ps1`. Every
lane checks formatting, Python locks/syntax, the privacy logging rule,
all-target Clippy with `--locked -D warnings`, locked tests, and the locked
release build. POSIX shell syntax runs on Ubuntu/macOS and plist lint runs when
`plutil` is present. The runner's rustup honours `rust-toolchain.toml`, so CI
and local runs share exact stable Rust 1.97.1. Historical CI runs prove only
the commit and lanes they actually executed.

## Architecture: a pure core with a thin Discord shell

The pure modules hold every decision the bot makes, and **none of them import
serenity or poise** (no count here — it rots; the table is the list):

| Module | Decides |
|---|---|
| `persona.rs` | Which of Abbey/Aviva/Abi answers a request, and why |
| `profile.rs` | How a member's profile reads as one paragraph |
| `perms.rs` | How a channel's overwrites resolve, in Discord's evaluation order |
| `moderation.rs` | Which action an incident warrants, given severity and history |
| `server.rs` | Role hierarchy, channel structure, and setup steps per archetype |
| `webhook.rs` | The incoming-webhook guide: steps, curl, payloads, thread/forum semantics |
| `ask.rs` | The fixed system prompt per persona for `/persona ask`, and the reply shape for answer / failure / no-backend |
| `llm.rs`, `llm/protocol.rs`, `llm/stream.rs`, `llm/transport.rs` | Backend selection and typed failures; strict completed-response/tool parsing; byte-safe terminal SSE; recording and live HTTP transports |
| `brain/nn.rs`, `brain/replay.rs`, `brain/dqn.rs` | The learning machinery from `docs/spec/brain.md`: dense net with explicit output activation, circular replay buffer, ε-greedy DQN with snapshot export/import |
| `brain/intent.rs`, `brain/state.rs`, `brain/reward.rs` | Intent classification (the spec's rule order, `Unknown` unreachable by design), the 18-dim state encoder + lexicon sentiment, delayed reaction-based rewards with an injected clock |
| `brain/social.rs`, `brain/registry.rs` | Reputation EMA with write-through flush; one policy per guild with restore/persist/evict, generic over a `Brain` trait |
| `guild.rs` | Per-guild settings + cache, reply cooldown, `/admin` rendering, scoped-id helpers (`"{platform}:{id}"`) |
| `memory.rs`, `runtime/memory_service.rs`, `engine.rs` | Canonical JSON facts + coordinated WDBX projection, channel context, interaction log, `PersonaContext`; per-scope multi-turn sessions that survive a persona switch |
| `wyhash.rs`, `embedding.rs`, `wdbx.rs` | Zig-compatible wyhash (pinned to 188 reference vectors), abi's n-gram text embedding (pinned to abi's own vectors), a WDBX v1 JSONL store + guild-namespaced semantic recall |
| `platform.rs`, `vision.rs`, `vision/*` | The network-agnostic event model and Telegram/Slack wire translation; image validation/normalization off the async runtime, the OpenAI-compatible provider contract, and pure rendering |
| `provider.rs`, `provider_self_test.rs` | Explicit Foundation Models config, manifest-bound server/CLI capabilities, loopback-only routing, the bounded schema-constrained `fm respond` adapter, and synthetic provider qualification |
| `persist.rs` | The one JSON document the registries' store traits read and write, atomically |
| `tools.rs` | The model-callable tool vocabulary, both request wire shapes, and `dispatch` against a `ToolHost` (the runtime implements it over `AppState` as `ToolScope`) |
| `pipeline.rs`, `pipeline/tests.rs` | The spec's `SocialRouter`: triage → intent → state → policy → cooldown → persona → reply/react, behind an `Outbound` trait so it runs in tests |
| `generation.rs` | How a reply is produced once the pipeline decides to speak: explicit tooled and read-only entry points, the bounded tool loop, local-path streaming (`stream_reply`: post/edit pacing), `Delivery`/`Ask`/`Round` |
| `voice.rs`, `offline_voice.rs` | Explicit local/disabled/OpenAI voice policy; loopback MLX-Audio client, bounded PCM framing, VAD/segmentation, and spoken-text shaping |
| `voice_session.rs`, `voice_session/control.rs`, `voice_local.rs`, `voice_openai.rs`, `voice_openai/protocol.rs`, `voice_self_test.rs` | Epoch/session-bound consent and media lifecycle, typed text control, local cognition, bounded Realtime protocol state, and the no-Discord full local-chain audition |

The `llm` module is pure in the same sense — it imports neither serenity nor poise —
but it is the one module that touches a network other than Discord: the single
live `Transport` impl POSTs JSON with reqwest. Request construction
(`build_request`) and response extraction (`extract_text`) are pinned by tests
through a recording fake, and **no test in this crate performs network I/O**,
so the suite passes with no network, no key, and no env vars. Keep
that property when touching the backend; it is what keeps the gate runnable
anywhere.

`runtime.rs` holds the shared `AppState` (each registry behind its own
`Mutex`, locked briefly and never across a network `.await`), the scheduler
(learn 30 s / flush 60 s / persist 300 s / settle 30 s), and the live vision
transport. `runtime::MemoryService` is the only production mutation/snapshot
boundary for durable facts: JSON is authoritative and WDBX `mem:*` rows are a
reconciled semantic projection. The runtime imports neither serenity nor poise.
`http_body.rs` is the shared incremental response reader: it treats
`Content-Length` as an early hint and enforces the byte cap again on every
actual chunk before retaining it.

`commands.rs`, `commands_brain.rs`, and `commands_voice.rs` translate *Discord
data* into those plain structs and lifecycle calls. The ordinary command files
fetch over REST, build the struct, hand it to a pure function, and post the
string back; the voice shell additionally owns Songbird Call construction and
one `commands_voice::on_gateway_event` adapter for operational text, immutable
voice-state payloads, and permission events, while `voice_session` and the
actors own decisions.
`gateway.rs` is the same thing for gateway events (serenity `FullEvent` →
`SocialEvent` → `pipeline::handle`) plus the Telegram long-poll and Slack Socket
Mode adapters, which feed the *same* pipeline through their own `Outbound`.
`main.rs` also imports serenity — for `GatewayIntents`, `GuildId`, and the
`Client` builder — but it reads no guild data; it is env parsing and framework
wiring only. Those five files are the entire Discord surface.

**This split is the reason the entire decision suite runs with no gateway
connection** (a count is deliberately not written here — it rots), and it is
load-bearing rather than stylistic. Putting decision logic into a command
body makes it untestable in this repository, because there is no test harness for
a Discord interaction here and no token to build one with. If you find yourself
writing an `if` inside a `#[poise::command]` function that isn't about fetching
data, it belongs in a pure module.

### Adding a command

1. Put the logic in a pure module with tests.
2. If it takes a closed set of options, add a `…Choice` enum **in
   `commands.rs`** with `#[derive(poise::ChoiceParameter)]` plus a `From` impl
   to the pure type. See `SeverityChoice`, `ArchetypeChoice`, and
   `PersonaChoice`. The mirror exists so the pure module never gains a poise
   dependency — that dependency is what would make it un-unit-testable. Put the
   user-facing wording in `#[name = "..."]` on each variant: **doc comments on
   variants never reach Discord** (verified in the derive macro), so a bare
   variant renders as its ident.
3. Write the thin command; **defer first** (see below).
4. Register it in the `commands: vec![…]` list in `main.rs`. Forgetting this
   compiles fine and silently ships nothing.

## Rules that are not preferences

**Defer before touching the network, unconditionally.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. Every command here calls `ctx.defer()` or
`ctx.defer_ephemeral()` first — including `/persona`, which makes no network call
at all. A command that defers only when it looks slow is a command that races
eventually. The one deliberate exception is `/voice leave`: an authorized stop
must close the voice media gate before its first await, so its guard paths
answer the interaction directly (cache/interaction-payload reads only, well
inside the 3-second window) and the defer runs concurrently with the
transition lock — do not "fix" it back to a leading defer. The corollary that
actually bit: **never declare a `GuildChannel`
parameter** — poise resolves it with a REST fetch *during argument parsing*,
before the body and its defer ever run. Take `ChannelId` and fetch after
deferring, the way `/perms` does. Guild fetches go through
`fetch_member_and_guild`, which runs the two independent requests concurrently;
`PartialGuild` already carries `roles` and `owner_id`, so do not add a separate
`guild_id.roles()` call.

**Every rendered answer passes through `clamp_message`.** Discord rejects
messages over 2,000 codepoints after the defer has already succeeded, which
surfaces as a raw "Message too large." error instead of an answer. A legitimate
`/perms` walkthrough measures past the limit, so the clamp is not theoretical.
Every call that posts the output of a pure module is wrapped. The deliberate
exception is the handful of fixed guard strings ("This one only works inside a
server.", the thread-redirect line) — literals of known length, plus one that
interpolates a channel *id*, so all are bounded far below the limit by
construction. That exception is not a precedent: anything whose length depends on
guild data goes through the clamp.

**Intents stay `non_privileged()` by default.** That set carries the guild
message and reaction events the pipeline listens to; it does not carry message
*content*, presence, or the member list. `/whois`, `/perms`, and `/modcall`
fetch over REST instead, and `profile::summarize` *states* that presence is
unavailable rather than printing a status it cannot observe. The single opt-in
is `ABBEY_MESSAGE_CONTENT=1` → `MESSAGE_CONTENT`; it must be enabled in the Dev
Portal **and** set here — both, or the gateway silently sends nothing. Without
it, Discord delivers empty bodies for anything but mentions and DMs, and the
pipeline deliberately does not consult the policy on a blank (`Ignored("no
content available")`) — training on noise would bias `stay`.

**Unsolicited speech is gated four times before the policy is consulted:
the blank-content guard first (no content → nothing to learn from), then in
order `ABBEY_QUIET=1` (operator, wins over everything) → the guild's
`/admin act on` (opt-in, default off) → `/admin learning off`.** After the policy picks reply/react: per-channel cooldown,
then the per-guild hourly budget (`brain/budget.rs`, default 6/h); over budget
returns `Outcome::OverBudget` and records **no** experience — silence was not
the policy's choice, so it must not be taught as one. Mentions, DMs, and
commands bypass all of it and are counted as `forced_replies` in the guild's
`BrainStats` (`brain/telemetry.rs`), never as decisions. Every policy decision
logs one `policy decision` line with the Q-values.

**DMs are one-person guilds.** `SocialEvent::scoped_guild_id` returns
`"{network}:dm:{user}"` when there is no guild, and `commands_brain` scopes the
same way, so a DM's facts, WDBX recall, reputation, and brain never touch
another DM user's. A shared `"discord:dm"` would have let semantic recall
surface one person's facts to another — the pipeline test
`two_dm_users_never_share_recall_or_facts` pins this.

**The forced path (mention/DM) loads the guild's brain before replying.**
`BrainRegistry::remember` drops experiences for unloaded guilds; without the
touch, every mention reply's reward settled into nothing. Also: a forced reply
that fails at the backend posts `ask::render_failure` instead of dead air, and
the typing indicator is re-broadcast every 8 s while a local model thinks.

**Tool capability is explicit at the generation boundary.** Mentions, DMs, and
`/persona ask` enter `generation::generate_with_tools*` with a live `ToolScope`;
unsolicited policy replies, voice, and `/summarize` enter a read-only function
with an explicit persona and never construct the vocabulary or host. The loop
is bounded (`MAX_TOOL_ROUNDS = 3`), streams on the
local path (`StreamEnd::Calls` means "run the tools and stream again"), retries
once without tools on a 4xx and disables only that provider's tool route for
the process. Every tool result is a short string (`tools::truncate`); no tool posts,
moderates, or changes config; `switch_persona` changes only the conversation's
persona and keeps the transcript. Adding a tool means: a `ToolSpec` in
`abbey_tools`, a `ToolHost` method, a `dispatch` arm, and a test — nothing in
the shells.

**Generated replies are shaped, queued, and (locally) streamed.** Every
generated reply passes `ask::tidy_reply` (persona-echo/heading strip,
sentence-boundary cut at 1,900 chars) before the clamp; every generation takes
a slot from `AppState.generation` (1 for a local endpoint — ollama wedged under
concurrent requests — 4 for Anthropic) or gets the honest "busy" line after
`ABBEY_BOT_LLM_QUEUE_SECS`; the local path streams (`generation::stream_reply`:
post after 60 chars / 4 s, edit every 2 s, final edit with the tidied text, a
stream that dies after posting edits in the failure line). Model choice is
measured, not guessed — `docs/benchmarks/2026-08-19-local-models.md`. That
benchmark originally recommended `gpt-oss:20b`; Donald first selected
`gemma4:e4b` for its stronger register, then selected the larger
`gemma4:12b` as the source/config cross-platform deployment target on
2026-08-20. That selection is not installed-service evidence.
The Apple-silicon acceleration profile is a separate pinned `mlx_vlm.server`
on loopback port 8282 with `mlx-community/gemma-4-12B-it-4bit`; Abbey must send
the installer's exact local snapshot path as both request model values because
MLX-VLM does not alias the portable Ollama name. `fm serve` is an optional
text fallback with tools structurally disabled. FM CLI tools/vision/OCR are
usable only when the current owner-only qualification manifest binds passing
semantic fixtures to the running Abbey binary, CLI, OS, mode, and fixture
version. It is not the Gemma default.

**Private request material never enters diagnostics.** Do not trace, print, or
derive an unredacted debug representation for credentials, authorization
headers, prompts, transcripts, request/response bodies, structured private
context, data URLs, or image bytes. Log fixed categories and bounded metadata
such as status, media type, byte count, and executable/binary hash instead.
`scripts/check-privacy.py` statically rejects raw sensitive expressions in Rust
tracing/log/print macros, Python output/log calls, shell output, and any
`#[instrument]` that does not use `skip_all`.
Keep runtime canary tests for redacted configuration/request representations;
the static gate complements those tests rather than replacing them.

**Evidence layers never collapse into one claim.** Source gates, provider
self-test JSON, installed artifact/hash/PID/listener checks, observed live
connector turns, and current participant-consented voice are separate
acceptance layers. Telegram/Slack source parity is not a live connector claim;
an offline WAV is not Discord voice proof; joined muted presence is not capture;
and historical observations do not qualify a replaced binary, model, CLI, or
OS build. Provider self-test uses synthetic fixtures and ephemeral state before
Discord credentials and `ABBEY_DATA_DIR` are read.

Publish provider evidence only through
`deploy/publish-provider-qualification.py`; it validates the passing report and
bound binary hash before an owner-only atomic replacement. Its ownership/mode
publication tests are POSIX-only. Windows CI parses and privacy-checks that code
and records the runtime test as skipped, never as publication evidence.

**Local voice is a macOS adapter, not a portable default.** With no voice
destination, voice is off on every platform. On Linux and Windows an explicit
`ABBEY_VOICE_MODE=local` fails configuration; operators must select `disabled`
or explicitly configure OpenAI Realtime. A cloud key alone never changes mode.

**Generated Discord text never pings.** Poise responses and Serenity's HTTP
client both default to an empty `CreateAllowedMentions`, and gateway posts and
edits set it explicitly. Model output, guild-derived names, and reply references
remain visible text but cannot notify users, roles, everyone, or reply authors.
Generation and vision clients refuse redirects and cap bodies while streaming;
remote OpenAI-compatible endpoints require HTTPS, while HTTP is loopback-only.
Detailed backend errors go to tracing, while `ask::render_failure` emits stable
public categories rather than provider bodies.

**Abbey never speaks unsolicited without a backend, and never from a template.**
The policy may choose `reply`, but with no backend configured the pipeline
treats that as silence; a mention or DM gets `ask::degraded_reply`. Welcomes and
summaries follow the same rule. The template-echo-as-AI failure is the thing the
2026-08-10 proposal forbids; keep it forbidden.

**Commands that recommend do not act.** `/modcall` never times out, kicks, or
bans; `/server` creates no roles or channels. Both are ephemeral. Keep it that
way unless the user explicitly asks for an acting command — a moderation
recommendation posted publicly is an accusation, and structural changes need
someone who can see the existing server. The pipeline's unsolicited replies and
reactions are the one deliberate exception, and they are fenced: per-channel
cooldown (default 20 s), `/admin learning off` pins a server to mentions and
commands, and the `stay` action is the neutral baseline the policy starts from.

**`/persona ask` answers come from a backend or says so — never from the bot.**
`llm::Backend::from_values` picks `ANTHROPIC_API_KEY` first, else
`ABBEY_BOT_LLM_ENDPOINT` (an OpenAI-compatible base URL; the bot appends
`/v1/chat/completions`), else none — and a blank value counts as unset because
`.env.example` ships blank assignments. With no backend the command posts
`ask::degraded_reply`, which names the routed persona and states plainly that
nothing can answer; a template echo dressed up as AI is what the
2026-08-10 proposal (`docs/`) forbids. The persona descriptions in `ask.rs` are
a hand **transcription** of abi-ai's `ProfileContract` — not a path dependency
on `../abi`, because that would break this clone's build — so when the
contracts change over there, this table drifts until someone re-copies it.
The command caps questions at 2,000 characters and applies a 30-second
per-user cooldown before entering the shared generation queue.

**Fetch over REST, not from the cache.** The cache is only as complete as the
intents held, so a cache read produces a silently thinner answer instead of an
error.

**Permission math belongs to serenity, not to this crate.** Guild-level
permissions come from `PartialGuild::member_permissions(&member)` — it seeds
from `@everyone` (which `Member.roles` never contains), and returns
`Permissions::all()` for the owner and for Administrator. A hand-rolled fold
over `member.roles` is how this codebase once told moderators they could not
act when they could; the hand-rolled version is deleted, do not reintroduce
one.

**Whether a moderator may act goes through `hierarchy_blocker`.** It encodes
Discord's refusal rules in one place — the owner cannot be actioned,
administrators cannot be timed out, and the actor's top role must sit *strictly*
above the target's — and returns the sentence explaining a refusal rather than a
bool, so the answer stays actionable. Any acting command added later must consult
it, not re-derive the rules at the call site.

## Traps this repository has already hit

**`Permissions` does not `Debug` into flag names.** It prints `Permissions(3072)`
— a raw bitfield. An early version derived permission names by scraping that and
would have rendered numbers into chat. Use `get_permission_names()`, which
returns the client-facing strings (`"View Channel"`, `"Ban Members"`). Two tests
pin the strings this codebase hardcodes against that vocabulary, because a typo
there fails silently — `/modcall` would tell every moderator they cannot act.

**`Backend` and `LlmRequest` hand-write `Debug` — never `#[derive(Debug)]` on
anything that carries a credential.** Both hold the Anthropic key (in the enum
payload, and in the `x-api-key` header), and a derived `Debug` printed it in
full through paths nobody plans for — a `tracing` field, a panic message, a
failing `assert_eq!` in CI logs (PR #9 is the fix). `debug_never_prints_the_api_key`
pins the redaction; if you add a type that holds a secret, give it the same
treatment and the same test. Related: the key travels in a header, never in the
URL, so no error message can include it.

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

**Docker base images float between Debian releases.** A floating builder once
moved to trixie/glibc 2.41 while the runtime stayed on bookworm/2.36 — a
combination that builds green and dies at `docker run` with "GLIBC_2.xx not
found". Build and runtime stages must name the same release; the current pair is
`rust:1.97.1-slim-trixie` + `debian:trixie-slim`. Never update only one stage.
Remember also that this host has neither Docker nor
systemd, so `Dockerfile` and `deploy/abbey-bot.service` can only ever be
source-reviewed here, never artifact-verified — the README's honesty note says
exactly what is and is not verified; keep it true when touching either file.

**Everything pure takes `now: u64` and a seed; nothing pure reads the clock or
`rand`.** `runtime::now()` is the single wall-clock read; `brain::nn::Rng` is a
splitmix64 seeded by the caller. This is what makes reward settlement, cooldown,
eviction, and the DQN all deterministic under test. A `SystemTime::now()` or a
`rand` import inside `brain/`, `guild.rs`, `memory.rs`, `engine.rs`, or `wdbx.rs`
is a regression, not a convenience.

**What persists of the learning loop.** `BrainSnapshot` carries weights, ε,
step count, and the last `SNAPSHOT_EXPERIENCES` (1,000) experiences; pending
rewards are exported into the state document at every persist and restored at
start. Telemetry (`BrainStats`) and the budget buckets are deliberately
in-memory. The rolling summariser (`AppState::refresh_summaries`, every 10 min)
only touches channels whose guild has `act on` or that are DMs, and takes the
generation slot like any other turn.

**Persistence is one JSON document plus one WDBX segment, not a database.**
`persist::Stores` implements the three store traits the registries speak and
writes `abbey-state.json` atomically; `wdbx::Recall` writes `wdbx.seg.0.jsonl`
in the `# ABI-WDBX v1` format (header required; unknown record types preserved
so a file shared with abi round-trips). JSON memory is the canonical fact set;
the versioned startup migration recovers legacy WDBX-only facts once, then
rebuilds only the WDBX `mem:*` projection without touching unrelated records.
Persistence snapshots both under one lock order and publishes JSON first, so a
crash cannot resurrect a deleted projection row. A corrupt state file is a startup error,
never a silent fresh start — a fresh start would discard every guild's
learning. The spec's Postgres/Fluent layer is recorded as Proposed in the goal
ledger; do not add a database dependency to "match the spec" without Donald.

**`Experience` keys by guild, and reputation keys by `(guild, user)` — never by
a joined `"guild:user"` string.** Scoped ids already contain a colon
(`discord:123`), so the spec's split-on-first-colon would mis-split.
`persist.rs` joins with U+001F for the same reason.

**A passing property test is not evidence that output reads well.** The `/server`
blueprints passed every invariant while rendering `🗂help` with the glyph jammed
against the name. Print the rendered string and read it before shipping anything
a user sees.

## Deploy artifacts

`Dockerfile` and `deploy/abbey-bot.service` wrap the same
`cargo build --release --locked` binary; configuration is env-only
(`DISCORD_TOKEN`, optional `ABBEY_GUILD_ID`, `RUST_LOG`, and `/persona ask`'s
backend vars `ANTHROPIC_API_KEY` / `ABBEY_BOT_LLM_ENDPOINT` — the systemd token lives in `/etc/abbey-bot/env`, never in the
unit or an image layer). The Debian-release pairing trap is in the traps
section above; four more non-obvious lines are past fixes — don't simplify
them away:

- **The runtime stage installs no ca-certificates, deliberately.** TLS roots
  are compiled in (webpki-roots via hyper-rustls and tokio-tungstenite;
  nothing links native-tls or openssl), so the system cert store is never
  read. Adding the package back is a dead layer, not a fix.
- **`RestartSec=30`, not 5.** A bad token exits 1 forever, and a 5-second
  restart loop re-auths against Discord fast enough to draw IP-level rate
  limiting.
- **`AF_UNIX` stays in `RestrictAddressFamilies`.** On hosts where nsswitch
  delegates entirely to nss-resolve, blocking unix sockets hard-fails DNS.
- **launchd `Umask=63` stays.** Learned guild/member state, semantic memory,
  and logs are private data. The launch command owns one fixed data path after
  sourcing env, and the installer stops the old process before recursively
  removing group/other access. Replacement binary and plist are staged,
  validated, renamed in-directory, and restored from backups if bootstrap fails.
- **The MLX-Audio environment is hash locked.** Keep the runtime and isolated
  build inputs, their uv-compiled `--generate-hashes` locks, the
  `--require-hashes`/`--build-constraint` install, and
  `deploy/check-python-locks.py` together. `webrtcvad` is the sole explicit
  source-build exception; do not widen it.

## What has and has not been verified

**What is and is not verified lives in `tasks/goals.md` (Current vs Proposed
per goal, dated) — read it before claiming anything works.** As of 2026-08-20
the following have been seen live from Donald's Discord client: gateway +
registration (16 commands, 58 guilds), slash commands answering, DM and
guild-mention replies (streamed, edited in place), the per-guild policy
deciding/reacting in an opted-in server, cooldown and act-off gates holding,
rewards settling into replay buffers, a model-initiated `remember_fact` tool
call, vision on gemma4:e4b, and the launchd-managed release service with
persistent state plus local generation/vision configuration. Not seen live:
Anthropic path/fallback (no key),
Telegram/Slack (no tokens), `/see` `/ocr` from a client, an `OverBudget`
refusal, a refreshed rolling summary.

**Operational facts learned live:** `gemma4:26b` wedged its ollama runner
(HTTP 000 after 100 s); the benchmark's speed-first recommendation was
gpt-oss:20b, while `gemma4:e4b` had the best register and became Donald's
interim choice. Donald then selected the larger `gemma4:12b` as the
source/config cross-platform deployment target on 2026-08-20
(`docs/benchmarks/2026-08-19-local-models.md`). A "research …" DM
exceeded 120 s under concurrent generation → the default backend timeout is
300 s (`ABBEY_BOT_LLM_TIMEOUT_SECS`) and generation is serialised. Keystroke-
driven testing must check Discord is frontmost first — the operator may be
typing elsewhere.

**Discord's Entry Point command breaks poise's global registration.** Apps
with Activities enabled carry an auto-created `PrimaryEntryPoint` command
(`launch`), and `poise::builtins::register_globally` does a bulk overwrite
that omits it — Discord rejects the whole request ("You cannot remove this
app's Entry Point command in a bulk update operation") and the setup callback
errors out with a live gateway and no commands. `main.rs`'s
`register_globally_keeping_entry_point` reads the existing Entry Point back
and re-sends it alongside ours. Deleting it would be the easy fix and would
disable the app's Activity — not this bot's call. Guild-scoped registration
(`ABBEY_GUILD_ID`) never hits this, which is why the trap hides during dev.

**The spec suite (`docs/spec/`) is implemented in Rust; residuals are recorded
as Proposed in `tasks/goals.md`:** the Swift companion app and Apple on-device
models (not a Rust concern), consented live validation of the newer local voice
path, Postgres
(replaced by the file store above), and the Slack HTTP Events listener (Socket
Mode is implemented instead). Tools shipped (PR #19) and are not a residual.

## Related, and easy to confuse

**Design records:** `docs/superpowers/specs/*` (approved designs per
sub-project), `docs/superpowers/plans/*` (the one executed plan),
`docs/benchmarks/2026-08-19-local-models.md` (historical measurements behind
the later `gemma4:12b` operator choice),
`docs/live-test-protocol.md` (how the bot is exercised from a Discord client).

**`docs/spec/` is the Swift-era design this crate implements**, copied verbatim
from the `discord-abbey` skill's reference files on 2026-08-19. The Swift types
(actors, Fluent models, DiscordBM calls) are the *what*; the Rust modules above
are the *how*. When the two disagree, the Rust code plus its tests are current
and the spec is the record of intent — update the spec file only when Donald
changes the design.

**The authoritative live checkout is `~/dev/active/abbey-bot`.** The former
`~/sources/repos/abbey-bot` path is absent; a discarded redundant clone under
Trash is not an authority. Concurrent sessions can still share the active
working tree, so inspect its current status before editing and fetch before
making claims about `origin/main`.

`~/dev/archive/swift-discord` is a home-grown **Swift** Discord library with its
own gateway and REST targets. It shares no code with this crate and is not a
dependency, a port source, or a reference implementation. The `discord-abbey`
skill's reference files describe a Swift/Vapor/DiscordBM bot at
`~/Desktop/AbbeyBot`, which does not exist on this machine; the skill's
non-code guidance (capability map, output format rules, persona registers) is
what this project actually implements, and those reference files are historical.
