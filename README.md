# Abbey Bot

Abbey's Discord operational layer, in Rust — [serenity](https://github.com/serenity-rs/serenity)
0.12 with [poise](https://github.com/serenity-rs/poise) 0.6 for slash commands.

This is the Rust implementation. Prior Abbey bots existed in Bun/TypeScript, Zig,
and Swift; none of those are on this machine. The one adjacent thing that is —
`~/dev/archive/swift-discord`, a home-grown Swift Discord library — is unrelated
to this crate and shares no code with it.

## Commands

| Command | What it does |
|---|---|
| `/persona route <request> [as]` | Shows which persona takes a request and why. `as` is a dropdown that forces one. |
| `/persona ask <question>` | Routes the question to a persona and answers it via the configured generation backend (see "Configured backends"). With none configured, it says so. |
| `/whois <user>` | Profile read: identity, standing, roles, join date. |
| `/perms <channel> <user>` | Walks a channel's permission overwrites in Discord's evaluation order. Threads are redirected to their parent, which owns the overwrites they inherit. |
| `/modcall <user> <severity> [warnings] [timeouts]` | Recommends a moderation action and says whether *you* can carry it out — both the permission bit and role hierarchy (owner-target, admin-timeout, top-role comparison). |
| `/server <kind>` | Emits a role hierarchy, channel structure, and numbered setup steps. |
| `/webhook <channel>` | Incoming-webhook setup guide: steps, curl, and a safe-by-default payload. Threads get a curl carrying their actual `?thread_id=`; forums (and media channels) get post semantics — `thread_name` or `?thread_id=`. |
| `/remember <fact> [user]` | Store a durable fact about a member (ephemeral). Goes into both the plain memory and the WDBX segment for semantic recall. |
| `/forget <fact>` | Remove one of your facts; the `fact` option autocompletes over what is on record (Manage Messages). |
| `/recall [user]` | What Abbey remembers about a member, plus their standing. |
| `/reputation [user]` | A member's reputation score (0–1) in this server. |
| `/summarize [count] [as]` | Summarize the recent messages Abbey has seen in the channel via the backend; stores the summary as the channel's context. |
| `/see <image> [question]` / `/ocr <image>` | Image understanding through the configured vision endpoint: describe, or transcribe text. |
| `/stats` | Command usage counts, messages seen, this server's brain (ε / steps / buffer), pending rewards, which backends are on. |
| `/admin show\|persona\|learning\|vision\|cooldown\|brain\|flush\|export\|reset` | Per-server config and the learning loop's controls (Manage Server): default persona, learning on/off, vision on/off, unsolicited-reply cooldown, ε override + brain inspection, persist now, export the brain snapshot as JSON, clear this channel's transcript. |

Beyond commands, Abbey listens. Every message and reaction she can see runs
through the adaptive pipeline (see "The learning loop" below): a per-server
policy decides whether to stay silent, react, or reply; mentions and DMs always
get a reply; reactions to her replies are the reward signal. The same pipeline
serves Telegram (long-poll) and Slack (Socket Mode) when their tokens are set.

## Running

```sh
export DISCORD_TOKEN=...        # from the Developer Portal; never commit it
export ABBEY_GUILD_ID=...       # optional, but see below
cargo run
```

`ABBEY_GUILD_ID` registers commands to a single guild, which takes effect
immediately. Leave it unset and commands register globally, where propagation can
take up to an hour — fine once the command set is stable, miserable while
iterating. See `.env.example`.

## Configured backends

`/persona ask` answers come from an external or local model, never from the
bot itself — this crate contains no inference engine. Which backend answers is
selected from the environment, first match wins:

| Env var | Backend |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic Messages API (external, per-token cost); model `claude-sonnet-5`. A secret with `DISCORD_TOKEN`'s exact handling: env only, never in a commit or an image layer. |
| `ABBEY_BOT_LLM_ENDPOINT` | An OpenAI-compatible server, usually loopback (llama-server / ollama / mlx). Base URL only, e.g. `http://127.0.0.1:8080` — the bot POSTs to `<endpoint>/v1/chat/completions`, and the server's own model choice is what answers. |

With neither set, `/persona ask` replies that no generation backend is
configured, and the pipeline never speaks unsolicited (a mention gets the same
honest reply). No test requires either variable, a network, or a key — the gate
runs fully offline.

| Env var | What it enables |
|---|---|
| `ABBEY_DATA_DIR` | Persistence: `abbey-state.json` (guild config, brain snapshots, reputation, memory) + `wdbx.seg.0.jsonl` (the WDBX v1 segment holding semantic memory). Unset = in-memory, lost on restart. |
| `ABBEY_MESSAGE_CONTENT=1` | Requests the privileged MESSAGE_CONTENT intent (must also be on in the Dev Portal). Without it, only mentions and DMs carry a body, and the pipeline learns from those alone. |
| `ABBEY_VISION_ENDPOINT` / `_MODEL` / `_KEY` | Any OpenAI-compatible vision endpoint for `/see`, `/ocr`, and attachment folding. Falls back to `ABBEY_BOT_LLM_ENDPOINT` + `/v1`. |
| `TELEGRAM_BOT_TOKEN` | Runs the Telegram long-poll adapter beside the Discord gateway. |
| `SLACK_BOT_TOKEN` + `SLACK_APP_TOKEN` | Runs Slack over Socket Mode (`xoxb-` + `xapp-`). |

## Design notes

**Intents are `non_privileged()` by default.** That set already includes guild
messages and reactions — the events the learning loop listens to — but not
message *content*, presence, or the member list, so the bot deploys without
requesting privileged intents in the Dev Portal. `/whois` and `/perms` fetch
member data over REST instead. `ABBEY_MESSAGE_CONTENT=1` is the one opt-in, and
it needs the Dev Portal toggle as well or the gateway silently sends nothing.

**The learning loop** (`docs/spec/brain.md`, `adaptivelearning.md`,
`multiguild.md`) is the spec's design in Rust, one policy per server: an
18-dimensional deterministic state (intent one-hot, reputation, length, mention,
question, image, hour, channel heat, lexicon sentiment) feeds a `[18, 64, 32, 3]`
DQN choosing *stay / reply / react*; rewards settle 150 s later from reactions
(+1 each, capped at 3), human replies (+0.5), deletions (−2, immediate), and
silence-after-reply (−0.2); `stay` is always 0, so the silent policy dominates
early. Unsolicited output is rate-limited per channel (`/admin cooldown`,
default 20 s), `/admin learning off` pins a server to mentions-and-commands
only, and `/admin brain` shows ε / steps / buffer so the loop is inspectable
rather than a black box. Learning runs every 30 s, reputation flushes every
60 s, everything persists every 5 min and on shutdown.

**Nothing Abbey says is a template.** Replies, welcomes, and summaries come from
the configured backend or not at all; with none configured she says so. Persona
descriptions are transcribed from abi-ai's contracts; the multi-turn transcript
is per channel and survives a persona switch.

**Semantic memory is WDBX-shaped.** Facts are embedded with the same
Zig-compatible wyhash n-gram embedding abi uses (pinned to abi's own vectors)
and written to a `# ABI-WDBX v1` JSONL segment that abi's tooling can read. The
store is namespace-scoped by server: a fact stored in one guild is never
recalled in another.

The visible consequence: **`/whois` does not report online/idle/DND status.** That
needs `GUILD_PRESENCES`. Rather than print a status it cannot actually observe,
the summary says presence is unavailable. If you want it, enable the intent in the
portal *and* add it to the intent list in `main.rs` — both, or the gateway
silently sends nothing.

**Every command defers before touching the network.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. The deferral is unconditional; a command that defers only
sometimes is one that races eventually.

**Decision logic is separated from Discord.** `persona.rs`, `profile.rs`,
`perms.rs`, `moderation.rs`, and `server.rs` are plain Rust over plain structs
and know nothing about serenity, which is why the interesting behaviour is unit-tested
with no gateway connection at all. `commands.rs` is the only
file that translates between Discord types and those structs — including the
`SeverityChoice`, `ArchetypeChoice`, and `PersonaChoice` mirrors that keep
poise's derive out of the pure modules.

**`/server` emits a plan and creates nothing.** Building a server is a run of
structural changes, and those stay with a human who can see what already exists.
The blueprints are property-tested rather than eyeballed, because the mistakes
here are quiet ones: every gated channel must name a role the same blueprint
creates (otherwise the steps are unfollowable), every permission string must be
one serenity actually defines (a typo yields a step nobody can perform), and
every text-channel name must already be in the form Discord will store it —
Discord lowercases text names and hyphenates whitespace, so a blueprint saying
`General Chat` describes a server you do not get. Voice channels are exempt from
that rule, which is exactly the asymmetry that trips people, so `render` applies
the normalization per channel kind. `@everyone`'s grants are per-archetype
blueprint *data*, not a fixed line: gated archetypes strip it to read-only,
while the flat friend-group archetype grants Send/Connect/Speak there — because
with no gates and no granting role, `@everyone` is the only place speech can
come from. Two usability properties are pinned: every blueprint gives a
roleless joiner at least one visible channel, and a flat blueprint's base role
can actually speak.

**`/modcall` recommends and never acts.** It does not time out, kick, or ban
anyone; the decision stays with the moderator. It is ephemeral for the same
reason — a recommendation posted to the channel would be a public accusation.
Two properties of the ladder are deliberate and load-bearing: severity outranks
history (a severe incident bans on the first offence, so a clean record cannot
buy tolerance for a threat), and more history never yields a *lighter* action —
both are pinned by property tests. Every timeout is constructed through a clamp
at Discord's 28-day ceiling, so a future rung cannot produce a request the API
rejects. The command also checks whether the *invoking* moderator can actually
carry the recommendation out — the permission bit via serenity's canonical
`member_permissions` (which sees grants on `@everyone`), and Discord's refusal
rules: the owner cannot be actioned, administrators cannot be timed out, and
the actor's top role must sit strictly above the target's.

**Persona routing refuses to guess.** The skill's rule is to switch persona when
the task type is *unambiguous*; a request that pulls toward two personas equally
therefore falls back to Abbey rather than picking a winner on list order.

## Deploying

Two paths, both configured entirely through the environment (`DISCORD_TOKEN`,
optional `ABBEY_GUILD_ID`, `ABBEY_DATA_DIR`, the backend variables, and
`RUST_LOG`). Mount a writable `ABBEY_DATA_DIR` or learning resets on every
restart:

- **systemd** — `deploy/abbey-bot.service`, a hardened unit (`DynamicUser`,
  `ProtectSystem=strict`, empty capability set) whose install steps are in the
  unit file's own header. The token lives in `/etc/abbey-bot/env`, not in the
  unit.
- **Docker** — the multi-stage `Dockerfile`. Pass secrets with
  `docker run --env-file`; never bake a token into an image layer.

Honesty note: **verified live so far: gateway connect and global command
registration (2026-08-19; 16 commands including the app's preserved Entry
Point, 58 guilds). Not yet verified live: any command answering, the pipeline
replying or learning, Telegram, Slack, vision.** The gate proves those paths
behind recording transports, persistence round-trips, and the
startup/fail-fast paths. This host has neither Docker nor systemd, so both
deploy artifacts are **unverified as artifacts** — what is verified is that `cargo build --release
--locked` produces the binary they both wrap. CI is *configured* to run the
same gate as a local checkout, but no workflow on this repository has ever
executed (account-level Actions lock), so that alignment is unverified until
the first green run.

## Gate

```sh
./check.sh
```

CI (`.github/workflows/rust.yml`) runs exactly this script, so a green check
means fmt, clippy `-D warnings`, and the test suite — not merely "it built".

`cargo fmt --all -- --check`, then `cargo clippy --all-targets -- -D warnings`, then
`cargo test`.
