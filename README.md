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
| `/persona ask <question>` | Routes the question to a persona and answers it via the configured generation backend (see "Configured backends"). With none configured, it says so. Questions are capped at 2,000 characters and each user gets one accepted invocation per 30 seconds. |
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
| `/admin show\|persona\|learning\|vision\|cooldown\|act\|budget\|brain\|flush\|export\|reset` | Per-server config and the learning loop's controls (Manage Server): default persona, learning on/off, vision on/off, unsolicited-reply cooldown, `act on` opts the server in to unsolicited replies (default off), `budget` caps them per hour (default 6), ε override + brain inspection (last decision's Q-values, action histogram, recent reward mean, budget left), persist now, export the brain snapshot as JSON, clear this channel's transcript. |
| `/voice join consent:true\|resume consent:true\|leave\|status` | Discord voice locked to one env-configured guild/channel. Join/resume require Manage Server, the caller to be present, an explicit everyone-present consent attestation, and a public disclosure before the software media gate opens. A new or unidentified participant pauses capture and playback. Leave is available to someone present or a manager. `ABBEY_VOICE_AUTOJOIN=1` is restart-resilient muted/self-deafened no-audio presence regardless of the selected conversational backend. |

**The model can call Abbey's own systems.** On mentions, DMs, and `/persona
ask` the backend is offered five tools — `remember_fact`, `lookup_reputation`,
`recall`, `switch_persona`, `recent_messages` — in the OpenAI or Anthropic
shape as appropriate; calls run against the same memory/WDBX/reputation the
slash commands use, scoped to the server and person in the conversation, for
at most three rounds before the answer. None of them post, moderate, or change
config. A backend that rejects tooled requests is retried once without and
remembered (`ABBEY_BOT_LLM_TOOLS=off` disables outright). Verified live
2026-08-19 with gpt-oss:20b (`remember_fact` stored "favorite editor is Zed").

**DMs work.** A DM to Abbey is always answered (through the backend), keeps a
per-conversation transcript, and is its own one-person namespace
(`discord:dm:<user>`) for facts, recall, and reputation — two people DMing her
never share memory. `/persona`, `/remember`, `/forget`, `/recall`,
`/reputation`, `/summarize`, `/see`, `/ocr`, and `/stats` all work in a DM;
`/admin` and the guild-data commands stay guild-only.

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

On Apple Silicon, install the pinned local speech sidecar once before enabling
local voice:

```sh
./deploy/install-mlx-audio-launchd.sh
```

The installer creates an owner-only uv environment, verifies the exact Whisper,
Kokoro, and `af_heart` voice-pack revisions, then launches MLX-Audio offline on
`127.0.0.1:8181` and requires a TTS-to-STT smoke to pass. It does not activate
Discord listening; that still requires everyone-present consent followed by
`/voice join consent:true`.

## Configured backends

`/persona ask` answers come from an external or local model, never from the
bot itself — this crate contains no inference engine. Which backend answers is
selected from the environment, first match wins:

| Env var | Backend |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic Messages API (external, per-token cost); model `claude-sonnet-5`. A secret with `DISCORD_TOKEN`'s exact handling: env only, never in a commit or an image layer. |
| `ABBEY_BOT_LLM_ENDPOINT` (+ `ABBEY_BOT_LLM_MODEL`) | An OpenAI-compatible server, usually loopback (llama-server / ollama / mlx). Base URL only, e.g. `http://127.0.0.1:11434` — the bot POSTs to `<endpoint>/v1/chat/completions`. Plain HTTP is accepted only for loopback; remote endpoints require HTTPS, and credentials/query strings in the base URL are rejected. `ABBEY_BOT_LLM_MODEL` names the model — measured 2026-08-19 (`docs/benchmarks/2026-08-19-local-models.md`): **`gpt-oss:20b`** is the recommended default (7–25 s, light reasoning, tool calling), `gemma4:e4b` the runner-up; llama-server/mlx ignore the field, ollama requires it. Local replies stream: the message appears within ~4 s and grows; one generation runs at a time (`ABBEY_BOT_LLM_CONCURRENCY`), extra turns wait up to `ABBEY_BOT_LLM_QUEUE_SECS` (90) then get an honest "busy" line. Reasoning models are handled: the local budget is 4,096 tokens, and a reply whose budget went entirely to `reasoning` is reported as exactly that. |

With neither set, `/persona ask` replies that no generation backend is
configured, and the pipeline never speaks unsolicited (a mention gets the same
honest reply). No test requires either variable, a network, or a key — the gate
runs fully offline.

| Env var | What it enables |
|---|---|
| `ABBEY_DATA_DIR` | Persistence: `abbey-state.json` (guild config, brain snapshots, reputation, memory) + `wdbx.seg.0.jsonl` (the WDBX v1 segment holding semantic memory). Unset = in-memory, lost on restart. |
| `ABBEY_BOT_LLM_TOOLS` | `off` disables model tool calls; default on (auto-degrades on a 4xx). |
| `ABBEY_QUIET=1` | Never speak unsolicited, anywhere — mentions, DMs, and commands still answer. The operator's guard while the policy is untrained. Wins over every server's `/admin act on`. |
| `ABBEY_MESSAGE_CONTENT=1` | Requests the privileged MESSAGE_CONTENT intent (must also be on in the Dev Portal). Without it, only mentions and DMs carry a body, and the pipeline learns from those alone. |
| `ABBEY_VISION_ENDPOINT` / `_MODEL` / `_KEY` | Any OpenAI-compatible vision endpoint for `/see`, `/ocr`, and attachment folding. Falls back to `ABBEY_BOT_LLM_ENDPOINT` + `/v1`; `off` stops that. Measured 2026-08-19: `http://127.0.0.1:11434/v1` + `gemma4:e4b` describes a screenshot correctly in ~4–15 s (budget raised to 1,024 tokens because the model reasons before it answers). |
| `ABBEY_VOICE_GUILD_ID` + `ABBEY_VOICE_CHANNEL_ID` | Enables `/voice` for exactly one Discord voice channel. `ABBEY_VOICE_MODE` is `local` by default, `disabled` for presence only, or `openai` as an explicit cloud backup. Local mode uses the loopback-only `ABBEY_VOICE_LOCAL_ENDPOINT` (default `http://127.0.0.1:8181`) with Whisper STT, Kokoro TTS, `af_heart`, and the existing loopback Abbey text backend; model/voice/language overrides are in `.env.example`. OpenAI mode alone requires `OPENAI_API_KEY` and its Realtime overrides. A key never selects cloud mode. `ABBEY_VOICE_AUTOJOIN=1` always uses Songbird `DecodeMode::Pass`, mute, and self-deafen with no receive/playback actor. Conversation still requires `/voice join consent:true`; membership changes require `/voice resume consent:true`. |
| `TELEGRAM_BOT_TOKEN` | Runs the Telegram long-poll adapter beside the Discord gateway. |
| `SLACK_BOT_TOKEN` + `SLACK_APP_TOKEN` | Runs Slack over Socket Mode (`xoxb-` + `xapp-`). |

Measured 2026-08-19 with Abbey's real prompt (full table and method in
`docs/benchmarks/2026-08-19-local-models.md`):

| model | typical reply | hidden reasoning | verdict |
|---|---|---|---|
| **gpt-oss:20b** | 7–25 s | light | **default** — fastest, tool calling |
| gemma4:e4b | 13–37 s | moderate | runner-up, best register |
| gemma4:12b | 32–94 s | heavy | too slow |
| qwen3.5 / ornith:9b | 47–182 s | runaway | unusable here |

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
early. Unsolicited output needs the server's opt-in (`/admin act on`, default off — on
a token that sits in 58 servers nothing speaks up until an admin asks), then is
bounded twice: per channel by the cooldown (`/admin cooldown`, default 20 s)
and per server by an hourly budget (`/admin budget`, default 6/h; over budget
the policy's choice is neither acted on nor learned). `/admin learning off`
pins a server to mentions and commands, and `/admin brain` shows ε / steps /
buffer, the last decision's Q-values, the action histogram, recent reward mean,
and budget left — so the loop is inspectable rather than a black box. Learning runs every 30 s, reputation flushes every
60 s, everything persists every 5 min and on shutdown.

**Nothing Abbey says is a template.** Replies, welcomes, and summaries come from
the configured backend or not at all; with none configured she says so. Persona
descriptions are transcribed from abi-ai's contracts; the multi-turn transcript
is per channel and survives a persona switch.

**Learning and context survive restarts and maintain themselves.** Each
guild's brain snapshot carries its last 1,000 experiences, and replies still
inside their 150 s reward window are persisted too — a restart resumes warm and
drops no reward. Channels where Abbey has been invited (`/admin act on`, or
DMs) get a rolling summary refreshed by the backend every 30 new messages (the
spec's "rolling 2k-token summary"); it feeds every reply's context.

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

**Live voice is explicit, consent-gated, and audio-only.** Songbird 0.6 handles
Discord DAVE/MLS transport. A conversational call is constructed in decode
mode from the outset because Songbird cannot promote a running
`DecodeMode::Pass` UDP receiver to `Decode`; Discord mute/self-deafen plus a
separate epoch-bound software media gate keep frames inaccessible until the
public disclosure and final participant check pass. `/voice join consent:true`
and `/voice resume consent:true` require Manage Server and an in-channel
caller. Any new, unknown, or unattested speaker revokes the media epoch before
that frame can enter the bounded 20 ms input queue, cancels work/playback, and
applies mute/self-deafen. Local mode runs Whisper STT, canonical Abbey
cognition, and Kokoro TTS on loopback; voice turns are read-only and raw audio
is not persisted. `/voice leave` tears down both sides, while `/voice status`
reports mode, phase, models, consent epoch, and bounded-queue counters without
content or credentials. Discord Go Live video is not ingested; stream vision
needs a separate consented screenshot source and retention policy.

**Every command defers before touching the network.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. The deferral is unconditional; a command that defers only
sometimes is one that races eventually.

**Decision logic is separated from Discord.** Persona, profile, permission,
moderation, server, learning, memory, generation-shaping, platform, and tool
decisions live in plain Rust modules with no serenity or poise dependency. The
Discord shell is limited to `commands.rs`, `commands_brain.rs`, `gateway.rs`,
and `main.rs`: they fetch or translate native data, call the core, and deliver
the result. This is why the decision suite runs with no gateway connection. The
complete module map and its load-bearing boundaries live in `AGENTS.md`.

**Generated text cannot ping anyone.** Command responses, gateway posts, reply
references, and streaming edits all send an empty Discord allowed-mentions
policy. Model output and guild-derived text remain visible as text but never
notify a user, role, `@everyone`, or the replied-to author.

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
`RUST_LOG`; add the voice variables above only when live voice is wanted).
Mount a writable `ABBEY_DATA_DIR` or learning resets on every
restart:

- **systemd** — `deploy/abbey-bot.service`, a hardened unit (`DynamicUser`,
  `ProtectSystem=strict`, empty capability set) whose install steps are in the
  unit file's own header. The token lives in `/etc/abbey-bot/env`, not in the
  unit.
- **launchd (this Mac)** — `deploy/install-launchd.sh` builds `--release
  --locked`, installs the binary under `~/.local/libexec/abbey-bot`, the data
  dir under `~/.local/share/abbey-bot/data`, logs under `~/Library/Logs/abbey-bot`,
  and loads `deploy/com.donaldfilimon.abbey-bot.plist` as a user agent (restart
  on crash with a 30 s throttle, not after a clean exit). Secrets come from
  `~/.config/abbey-bot/env` (chmod 600), never from the plist.
  `--uninstall` reverses it. The env file must carry everything the bot
  should know — at minimum `DISCORD_TOKEN`, and for a useful bot also
  `ABBEY_BOT_LLM_ENDPOINT`, `ABBEY_BOT_LLM_MODEL`, and (if you want images)
  `ABBEY_VISION_ENDPOINT`/`ABBEY_VISION_MODEL`; `ABBEY_GUILD_ID`,
  `ABBEY_MESSAGE_CONTENT`, `ABBEY_QUIET` as you ran it by hand. With only the
  token, every DM answers the honest "no backend" line. launchd always uses
  its fixed private data path; an `ABBEY_DATA_DIR` line in the env file is
  ignored rather than being allowed to silently disable persistence.
  Installed and verified live on 2026-08-20: launchd runs the locked release
  binary with persistent data, gpt-oss:20b generation, gemma4:e4b vision, and
  guild-scoped command registration in the sandbox guild. Updates stage and
  validate the replacement before a SIGTERM-driven graceful stop, then publish
  by same-directory renames with rollback. The service umask and installer keep
  the env, learned state, WDBX segment, and logs owner-only.
- **Docker** — the multi-stage `Dockerfile`. Pass secrets with
  `docker run --env-file`; never bake a token into an image layer.

Honesty note: **verified live on 2026-08-19 from Donald's Discord client:
gateway + registration (16 commands, 58 guilds); slash commands answering
(`/admin export`, `/recall`, `/admin act`, `/persona ask`); DM and guild-mention
replies generated by ollama (`gpt-oss:20b`), streamed and edited in place; the
per-guild policy deciding, reacting, and settling rewards in an opted-in server;
cooldown and act-off gates holding; a model-initiated `remember_fact` tool call;
vision on `gemma4:e4b`. Not yet observed: Anthropic path/fallback (no key),
Telegram/Slack (no tokens), an `OverBudget` refusal, a refreshed rolling
summary, `/whois` `/perms` `/modcall` `/server` `/webhook` `/remember` `/forget`
`/reputation` `/summarize` `/see` `/ocr` from a client. On 2026-08-20 the
launchd release service, persistent data path, gateway connection, local
generation backend, and real three-turn backend pipeline were verified;
`tasks/goals.md` is the ledger of record. The original `/voice` path was
offline-verified (including a loopback Realtime WebSocket) and its no-audio connection mode was
observed live in Space Engineering on 2026-08-20: Discord showed `Abbey,
Deafened`, while service logs confirmed the requested channel and disabled
receive/provider streaming. A local output-only greeting then entered Discord's
speaking state and completed its 16.2-second track normally while Abbey stayed
self-deafened; no paid provider was called. That temporary greeting surface was
subsequently removed: the currently deployed build explicitly mutes and
self-deafens and cannot emit audio. The local-first candidate additionally
passed an end-to-end loopback Kokoro `af_heart` TTS to Whisper transcription
smoke, but a consented live Discord turn remains deliberately unclaimed. Its
DAVE/OpenMLS dependency audit is also explicitly recorded in the voice design
rather than reported green.**
This host has neither Docker nor systemd, so both
deploy artifacts are **unverified as artifacts** — what is verified is that `cargo build --release
--locked` produces the binary they both wrap. The exact stable Rust + locked
release-build gate passed in GitHub Actions on PR #24 on 2026-08-20.

## Gate

```sh
./check.sh
```

CI (`.github/workflows/rust.yml`) runs exactly this script with the exact Rust
1.97.1 toolchain, so a green check means fmt, launchd shell syntax (plus plist
lint where `plutil` exists), clippy `-D warnings`, the test suite, and the
locked release build used by deployment.

`cargo fmt --all -- --check`, deploy syntax, then
`cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`,
and `cargo build --release --locked`.
