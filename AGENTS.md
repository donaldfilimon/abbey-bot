# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

`README.md` is written for a human running the bot — commands, env vars, and the
per-feature design notes. Read it first. This file holds what the README leaves
out: the shape of the codebase, the rules that keep it that shape, and the traps
this project has already hit.

`AGENTS.md` is a verbatim mirror of this file for non-Claude agents — only the
header line differs. Apply any edit to both, or they drift.

## Commands

```bash
./check.sh          # the gate: fmt --check, clippy --all-targets --locked -D warnings, test --locked
cargo test <name>   # single test, substring-matched against the full path
cargo run           # needs DISCORD_TOKEN; see README
```

`cargo test moderation::` runs one module's tests. There is no `-p`
and no `--workspace` — this is a single binary crate. Note that means test
targets are inside the `bin`, so `--lib` matches nothing.

The gate runs `--locked` on purpose: the Dockerfile builds `--locked`, and
before the gate did too, a Cargo.toml bump without a regenerated lock kept CI
green while every deploy build died. The gate proves the property the deploy
depends on — do not remove the flag to "fix" a lock error; regenerate the lock.

**CI runs the real gate — since PR #4.** `.github/workflows/rust.yml` executes
`./check.sh` itself (fmt --check, clippy --all-targets --locked `-D warnings`, tests --locked) on push and PR to
`main`, and the runner's rustup honours `rust-toolchain.toml`, so CI and local
runs share the pinned nightly. Two caveats keep the local habit load-bearing:
green check marks older than PR #4 vouch only for `cargo build && cargo test`
(the workflow's original, weaker shape), and **no workflow on this repository
has ever executed** (account-level Actions billing lock) — the CI–local
alignment is configured, not verified, until the first green run. So "run
`./check.sh` locally before trusting a merge" remains the rule, now for
availability rather than coverage.

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
| `llm.rs` | Which generation backend the env selects, how its request is built (single- and multi-turn) and its response extracted (pure, behind a `Transport` trait) |
| `brain/nn.rs`, `brain/replay.rs`, `brain/dqn.rs` | The learning machinery from `docs/spec/brain.md`: dense net with explicit output activation, circular replay buffer, ε-greedy DQN with snapshot export/import |
| `brain/intent.rs`, `brain/state.rs`, `brain/reward.rs` | Intent classification (the spec's rule order, `Unknown` unreachable by design), the 18-dim state encoder + lexicon sentiment, delayed reaction-based rewards with an injected clock |
| `brain/social.rs`, `brain/registry.rs` | Reputation EMA with write-through flush; one policy per guild with restore/persist/evict, generic over a `Brain` trait |
| `guild.rs` | Per-guild settings + cache, reply cooldown, `/admin` rendering, scoped-id helpers (`"{platform}:{id}"`) |
| `memory.rs`, `engine.rs` | Facts, channel context, interaction log, `PersonaContext`; per-scope multi-turn sessions that survive a persona switch |
| `wyhash.rs`, `embedding.rs`, `wdbx.rs` | Zig-compatible wyhash (pinned to 188 reference vectors), abi's n-gram text embedding (pinned to abi's own vectors), a WDBX v1 JSONL store + guild-namespaced semantic recall |
| `platform.rs`, `vision.rs` | The network-agnostic event model and Telegram/Slack wire translation; the image-understanding seam (request/extract pure, transport injected) |
| `persist.rs` | The one JSON document the registries' store traits read and write, atomically |
| `pipeline.rs` | The spec's `SocialRouter`: triage → intent → state → policy → cooldown → persona → reply/react, behind an `Outbound` trait so it runs in tests |

`llm.rs` is pure in the same sense — it imports neither serenity nor poise —
but it is the one module that touches a network other than Discord: the single
live `Transport` impl POSTs JSON with reqwest. Request construction
(`build_request`) and response extraction (`extract_text`) are pinned by tests
through a recording fake, and **no test in this crate constructs the live
transport**, so the suite passes with no network, no key, and no env vars. Keep
that property when touching the backend; it is what keeps the gate runnable
anywhere.

`runtime.rs` holds the shared `AppState` (each registry behind its own
`Mutex`, locked briefly and never across a network `.await`), the scheduler
(learn 30 s / flush 60 s / persist 300 s / settle 30 s), and the live vision
transport; it imports neither serenity nor poise either.

`commands.rs` and `commands_brain.rs` are the files that translate *Discord
data* into those plain structs, and that is their whole job: fetch over REST,
build the struct, hand it to a pure function, post the string back.
`gateway.rs` is the same thing for gateway events (serenity `FullEvent` →
`SocialEvent` → `pipeline::handle`) plus the Telegram long-poll and Slack Socket
Mode adapters, which feed the *same* pipeline through their own `Outbound`.
`main.rs` also imports serenity — for `GatewayIntents`, `GuildId`, and the
`Client` builder — but it reads no guild data; it is env parsing and framework
wiring only. Those four files are the entire Discord surface.

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
eventually. The corollary that actually bit: **never declare a `GuildChannel`
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

**Docker base images float between Debian releases.** `rust:slim` re-aliases
to each new stable (it moved to trixie/glibc 2.41 while the runtime stage was
bookworm/2.36 — a combination that builds green and dies at `docker run` with
"GLIBC_2.xx not found"). Build and runtime stages must name the same release
(`rust:slim-bookworm` + `debian:bookworm-slim`); never pair a floating builder
with a pinned runtime. Remember also that this host has neither Docker nor
systemd, so `Dockerfile` and `deploy/abbey-bot.service` can only ever be
source-reviewed here, never artifact-verified — the README's honesty note says
exactly what is and is not verified; keep it true when touching either file.

**Everything pure takes `now: u64` and a seed; nothing pure reads the clock or
`rand`.** `runtime::now()` is the single wall-clock read; `brain::nn::Rng` is a
splitmix64 seeded by the caller. This is what makes reward settlement, cooldown,
eviction, and the DQN all deterministic under test. A `SystemTime::now()` or a
`rand` import inside `brain/`, `guild.rs`, `memory.rs`, `engine.rs`, or `wdbx.rs`
is a regression, not a convenience.

**Persistence is one JSON document plus one WDBX segment, not a database.**
`persist::Stores` implements the three store traits the registries speak and
writes `abbey-state.json` atomically; `wdbx::Recall` writes `wdbx.seg.0.jsonl`
in the `# ABI-WDBX v1` format (header required; unknown record types preserved
so a file shared with abi round-trips). A corrupt state file is a startup error,
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
section above; three more non-obvious lines are past fixes — don't simplify
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

## What has and has not been verified

**Verified live on 2026-08-19 (commit after PR #10, this clone, token from
`.env`):** the gateway handshake, `Ready`, and global command registration —
16 commands listed by `GET /applications/{id}/commands` (our 15 plus the app's
Entry Point), bot present in 58 guilds, process stable for the observation
window. That is the *first* live connection this bot has made; the earlier
attempt died in the ready callback on exactly the Entry Point trap below.

**Not verified live:** any interaction answered, any pipeline reply or
reaction, any reward settling, Telegram, Slack, vision, persistence under real
traffic. The gate proves those paths behind recording transports, and the
binary is confirmed to fail fast on a missing `DISCORD_TOKEN`, a non-numeric
or zero `ABBEY_GUILD_ID`, and a corrupt state file, and to write + reload its
data dir. Do not describe the commands as answering or the loop as learning
until someone has watched it happen.

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

**The spec suite (`docs/spec/`) is implemented in Rust with these residuals,
all recorded as Proposed in `tasks/goals.md`:** the Swift companion app and
Apple on-device models (not a Rust concern), voice (no `voice.md` was
supplied), Postgres (replaced by the file store above), model-initiated tools
(`RememberFactTool` etc. — the backends are plain chat completions today), and
the Slack HTTP Events listener (Socket Mode is implemented instead, so the
request-signing code was removed rather than left dead).

## Related, and easy to confuse

**`docs/spec/` is the Swift-era design this crate implements**, copied verbatim
from the `discord-abbey` skill's reference files on 2026-08-19. The Swift types
(actors, Fluent models, DiscordBM calls) are the *what*; the Rust modules above
are the *how*. When the two disagree, the Rust code plus its tests are current
and the spec is the record of intent — update the spec file only when Donald
changes the design.

**This repository has two live clones**: this one and `~/dev/active/abbey-bot`,
both tracking `origin/main` (`donaldfilimon/abbey-bot`, private). Concurrent
sessions have worked them simultaneously — `git fetch` before assuming either
is current, and never reason about "the" working tree from memory of the other.

`~/dev/archive/swift-discord` is a home-grown **Swift** Discord library with its
own gateway and REST targets. It shares no code with this crate and is not a
dependency, a port source, or a reference implementation. The `discord-abbey`
skill's reference files describe a Swift/Vapor/DiscordBM bot at
`~/Desktop/AbbeyBot`, which does not exist on this machine; the skill's
non-code guidance (capability map, output format rules, persona registers) is
what this project actually implements, and those reference files are historical.
