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
| `/remember <fact> [user]` | Store a durable fact (ephemeral) in plain memory and WDBX. Facts are whitespace-normalized, non-empty, and capped at 300 Unicode characters. The subject defaults to you; choosing another member requires Manage Messages, Manage Guild, or Administrator. |
| `/forget <fact> [user]` | Remove a stored fact from plain memory and WDBX. The subject defaults to you and autocomplete is scoped to your own facts; choosing another member requires Manage Messages, Manage Guild, or Administrator. |
| `/recall [user]` | What Abbey remembers about you, plus your standing. Looking up another member requires Manage Messages, Manage Guild, or Administrator. |
| `/reputation [user]` | A member's reputation score (0–1) in this server. |
| `/summarize [count] [as]` | Summarize the recent messages Abbey has seen in the channel via the backend; stores the summary as the channel's context. |
| `/see <image> [question]` / `/ocr <image>` | Image understanding through the configured vision endpoint. JPEG, PNG, WebP, and GIF are fully decoded locally under 8192×8192-pixel and 96 MiB allocation ceilings before transport; GIF's first rendered frame is normalized to PNG. |
| `/stats` | Command usage counts, messages seen, this server's brain (ε / steps / buffer), pending rewards, which backends are on. |
| `/admin show\|persona\|learning\|vision\|cooldown\|act\|budget\|brain\|flush\|export\|reset` | Per-server config and the learning loop's controls (Manage Server): default persona, learning on/off, vision on/off, unsolicited-reply cooldown, `act on` opts the server in to unsolicited replies (default off), `budget` caps them per hour (default 6), ε override + brain inspection (last decision's Q-values, action histogram, recent reward mean, budget left), persist now, export the brain snapshot as JSON, clear this channel's transcript. |
| `/voice join consent:true\|resume consent:true\|leave\|status` | Discord voice locked to one env-configured guild/channel. Join/resume require Manage Server, the caller to be present, an explicit everyone-present consent attestation, and a public disclosure before the software media gate opens. A new, unidentified, or unattested participant closes the media epoch and disconnects the conversational `Decode` call; renewed consent starts a fresh call. Leave is available to someone present or a manager. `ABBEY_VOICE_AUTOJOIN=1` is restart-resilient muted/self-deafened no-audio presence regardless of the selected conversational backend. |

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

The installer creates a fresh owner-only uv environment from the committed,
SHA-256-hashed dependency and isolated-build locks (`webrtcvad` is the one
explicit source-build exception), verifies the exact Whisper, Kokoro, and `af_heart` voice-pack
revisions, then launches MLX-Audio offline on `127.0.0.1:8181` and requires a
TTS-to-STT smoke to pass. Installation is serialized; replacement stays staged
until healthy, and rollback refuses to mutate files if launchd cannot unload
the candidate. It does not activate
Discord listening; that still requires everyone-present consent followed by
`/voice join consent:true`.

Audit the entire local speech and cognition chain without a Discord token,
microphone, or call. Every AI request made by the bot is restricted to a
configured loopback endpoint:

```sh
cargo build --locked --release
set -a
. "$HOME/.config/abbey-bot/env"
set +a
env \
  -u DISCORD_TOKEN \
  -u ANTHROPIC_API_KEY \
  -u OPENAI_API_KEY \
  ABBEY_DATA_DIR= \
  ABBEY_VISION_ENDPOINT=off \
  ./target/release/abbey-bot \
  --voice-self-test "$HOME/abbey-local-audition.wav"
```

The fixed synthetic wake phrase runs through Kokoro TTS, Whisper STT, the same
canonical Abbey/Abi/Aviva generation path used by local Discord voice, spoken
text shaping, and Kokoro TTS again. It refuses non-loopback reasoning, fails if
Whisper loses the wake name, never reads a microphone or joins Discord, and
does not load production state, WDBX, rewards, or conversation history. It
creates the output WAV owner-only without replacing an existing file.
"Loopback-only" describes the bot's transport boundary; an arbitrary local
OpenAI-compatible service could itself proxy upstream. For strictly offline
operation, use locally resident models. The managed speech configuration uses
the pinned MLX-Audio Whisper and Kokoro models; that does not imply an MLX
reasoning or vision backend. The current cross-platform reasoning/vision model
name and deployment intent is `gemma4:12b` through the OpenAI-compatible
endpoint seam. This latest operator choice supersedes both the interim
`gemma4:e4b` choice and the earlier `gpt-oss:20b` benchmark recommendation;
dated results below remain historical evidence.

On Apple Silicon, the optional unified Gemma 4 acceleration profile is a
separate, pinned MLX-VLM service (it does not share MLX-Audio's environment):

```sh
./deploy/install-mlx-vlm-launchd.sh
```

It stages `mlx-vlm==0.6.15`, downloads
`mlx-community/gemma-4-12B-it-4bit` at revision
`73bcf09092aa277861d5a191b989b666f7f32e8f`, and requires exact offline
streamed-text, streamed tool-loop, colored-shape, and OCR smokes before publishing
the loopback service on `127.0.0.1:8282`. Configure Abbey with the exact local
snapshot path printed by the installer:

```sh
ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:8282
ABBEY_BOT_LLM_MODEL=$HOME/.local/share/abbey-bot/mlx-vlm/huggingface/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/73bcf09092aa277861d5a191b989b666f7f32e8f
ABBEY_BOT_LLM_CONCURRENCY=1
ABBEY_VISION_ENDPOINT=http://127.0.0.1:8282/v1
ABBEY_VISION_MODEL=$HOME/.local/share/abbey-bot/mlx-vlm/huggingface/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/73bcf09092aa277861d5a191b989b666f7f32e8f
```

The exact path matters: MLX-VLM uses each request's `model` value as a
load/cache key and does not alias the portable Ollama name `gemma4:12b` to the
preloaded snapshot. Apple `fm serve` remains an optional smaller text/vision
fallback (`model=system`, Abbey tools off), not the Gemma default.

## Configured backends

`/persona ask` answers come from an external or local model, never from the
bot itself — this crate contains no inference engine. Which backend answers is
selected from the environment, first match wins:

| Env var | Backend |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic Messages API (external, per-token cost); model `claude-sonnet-5`. A secret with `DISCORD_TOKEN`'s exact handling: env only, never in a commit or an image layer. |
| `ABBEY_BOT_LLM_ENDPOINT` (+ `ABBEY_BOT_LLM_MODEL`) | An OpenAI-compatible server, usually loopback (for example Ollama or llama.cpp/llama-server). Base URL only, e.g. `http://127.0.0.1:11434` — the bot POSTs to `<endpoint>/v1/chat/completions`. Plain HTTP is accepted only for loopback; remote endpoints require HTTPS, and credentials/query strings in the base URL are rejected. The cross-platform default model name and deployment intent is **`gemma4:12b`**. This latest operator choice supersedes both the interim `gemma4:e4b` choice and the 2026-08-19 benchmark's `gpt-oss:20b` recommendation. The dated benchmark remains historical timing evidence: gpt-oss answered in 7–25 s, e4b in 13–37 s, and 12b in 32–94 s on that host. Ollama uses the model field; a server bound to one model may ignore it. Local replies stream: the message appears within ~4 s and grows; one generation runs at a time (`ABBEY_BOT_LLM_CONCURRENCY`), extra turns wait up to `ABBEY_BOT_LLM_QUEUE_SECS` (90) then get an honest "busy" line. Reasoning models are handled: the local budget is 4,096 tokens, and a reply whose budget went entirely to `reasoning` is reported as exactly that. |

The backend contract is intentionally portable. Linux and Windows retain the
same OpenAI-compatible endpoint seam and may use Ollama, llama.cpp, or another
runtime that passes Abbey's exact interface tests. On macOS, the pinned
MLX-VLM profile above is the unified Gemma adapter; its installer will not
publish the service unless the production text/tool/vision wire shapes pass
offline. Apple `fm serve` is an optional OpenAI-compatible adapter, not the
cross-platform default; its current facade is suitable only with Abbey tools
disabled.

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
| `ABBEY_VISION_ENDPOINT` / `_MODEL` / `_KEY` | Any verified OpenAI-compatible vision endpoint for `/see`, `/ocr`, and attachment folding. Falls back to `ABBEY_BOT_LLM_ENDPOINT` + `/v1`; `off` stops that. The current cross-platform model target is `gemma4:12b`, but the selected runtime must still prove its vision interface. JPEG, PNG, WebP, and GIF are decoded under 8192×8192-pixel and 96 MiB allocation limits before transport; GIF's first frame is converted to PNG. Historical evidence from 2026-08-19: Ollama at `http://127.0.0.1:11434/v1` with `gemma4:e4b` described a screenshot in ~4–15 s. The hardened attachment path and the new 12b target still need fresh live `/see` validation after deployment. |
| `ABBEY_VOICE_GUILD_ID` + `ABBEY_VOICE_CHANNEL_ID` | Enables `/voice` for exactly one Discord voice channel. `ABBEY_VOICE_MODE` is `local` by default, `disabled` for presence only, or `openai` as an explicit cloud backup. Local mode uses the loopback-only `ABBEY_VOICE_LOCAL_ENDPOINT` (default `http://127.0.0.1:8181`) with Whisper STT, Kokoro TTS, `af_heart`, and the existing loopback Abbey text backend; model/voice/language overrides are in `.env.example`. OpenAI mode alone requires `OPENAI_API_KEY` and its Realtime overrides; it is a direct, whole-response-buffered degraded backup without local ABI routing or WDBX context, and spoken control is not authoritative there—use `/voice leave` or write `stop listening` in the configured voice chat. A key never selects cloud mode. `ABBEY_VOICE_AUTOJOIN=1` always uses Songbird `DecodeMode::Pass`, mute, and self-deafen with no receive/playback actor. Conversation still requires `/voice join consent:true`; a consent invalidation disconnects the conversational call and renewed consent requires `/voice resume consent:true`. |
| `TELEGRAM_BOT_TOKEN` | Runs the Telegram long-poll adapter beside the Discord gateway. |
| `SLACK_BOT_TOKEN` + `SLACK_APP_TOKEN` | Runs Slack over Socket Mode (`xoxb-` + `xapp-`). |

Measured 2026-08-19 with Abbey's real prompt (full table and method in
`docs/benchmarks/2026-08-19-local-models.md`):

| model | typical reply | hidden reasoning | verdict |
|---|---|---|---|
| gpt-oss:20b | 7–25 s | light | 2026-08-19 benchmark winner; historical tool-calling evidence |
| gemma4:e4b | 13–37 s | moderate | 2026-08-19 runner-up; later interim choice, now superseded |
| **gemma4:12b** | 32–94 s | heavy | **current operational default**; dated latency retained |
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

**Semantic memory is projected into WDBX.** Canonical facts live in the atomic
JSON state document and are embedded into WDBX with the same
Zig-compatible wyhash n-gram embedding abi uses (pinned to abi's own vectors)
in a `# ABI-WDBX v1` JSONL segment that abi's tooling can read. Legacy
WDBX-only facts are recovered once; thereafter startup repairs the projection
from JSON, so interrupted persistence cannot resurrect a forgotten fact.
Production inference and tools scope recall by both server and Discord user: a
fact stored for one person is never supplied to another person or guild. The
slash-command surfaces are self-only by default; cross-member `/remember`,
`/forget`, and `/recall` require Manage Messages, Manage Guild, or
Administrator, and `/remember` rejects empty or greater-than-300-character
facts before either store is changed.

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
disconnects the conversational `Decode` call. Local mode runs Whisper STT, canonical Abbey
cognition, and Kokoro TTS on loopback; voice turns are read-only and raw audio
is not persisted. Abbey must retain View Channel, Send Messages, Connect, and
Speak and must not be server-muted/deafened/suppressed; startup rechecks those
conditions around activation, and channel/role/member changes stop the media
epoch if the call could become receive-only. `/voice leave` tears down both
sides, while `/voice status`
reports mode, phase, models, consent epoch, and bounded-queue counters without
content or credentials. In explicit `openai` backup mode, Realtime is a
degraded direct provider path: spoken stop detection is not authoritative, so
any participant must use `/voice leave` or write `stop listening` in the
configured voice chat for a deterministic stop. Discord Go Live video is not
ingested; stream vision needs a separate consented screenshot source and
retention policy.

**Every command defers before touching the network.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. The deferral is unconditional; a command that defers only
sometimes is one that races eventually.

**Decision logic is separated from Discord.** Persona, profile, permission,
moderation, server, learning, memory, generation-shaping, platform, and tool
decisions live in plain Rust modules with no serenity or poise dependency. The
Discord shell is limited to `commands.rs`, `commands_brain.rs`,
`commands_voice.rs` and its adapter modules, `gateway.rs`, and `main.rs`: they
fetch or translate native data, call the core, and deliver the result. This is
why the decision suite runs with no gateway connection. The
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

**Persona routing mirrors ABI.** An explicit leading Abbey, Aviva, or ABI name
wins; otherwise canonical keyword weights are added to the 0.40/0.30/0.30 ABI
prior and ties deterministically favor Abbey. The selected route and normalized
weights remain visible through `/profile`.

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
  Installed and verified live earlier on 2026-08-20: launchd ran the locked
  release binary with persistent data, gpt-oss:20b generation, gemma4:e4b
  vision, and guild-scoped command registration in the sandbox guild. Updates
  stage and validate the replacement before a SIGTERM-driven graceful stop,
  then publish by same-directory renames with rollback. The service umask and
  installer keep the env, learned state, WDBX segment, and logs owner-only. The current
  offline-first voice/vision hardening candidate is newer than that installed
  evidence and remains pending a gated launchd deployment with `gemma4:12b` as
  the cross-platform reasoning/vision model target. The adapter remains the
  verified OpenAI-compatible seam; this does not claim an MLX or `fm serve`
  service is installed. Local source or WAV evidence is not a deployment claim.
- **Docker** — the multi-stage `Dockerfile`. Pass secrets with
  `docker run --env-file`; never bake a token into an image layer.

Honesty note: **live evidence is cumulative and command-specific.** On
2026-08-19, Donald's Discord client verified gateway registration, generated
DM/guild replies, the adaptive policy and reward settlement, historical local
`gpt-oss:20b` generation, and a model-initiated `remember_fact` call.
The current `gemma4:12b` choice supersedes both the interim `gemma4:e4b` choice
and that older default without rewriting either observation. By 2026-08-20,
durable Discord interaction records also
covered `/stats`, `/remember`, `/reputation`, `/summarize`, `/whois`, `/perms`,
`/modcall`, `/server`, `/voice status`, and `/voice leave`. `/forget`, `/ocr`,
and `/webhook` remain unobserved. A live `/see` invocation reached the older
vision path but failed on its attachment MIME/decoding behavior; the current
source fully validates JPEG/PNG/WebP/GIF and normalizes GIF's first frame, but
that fix still needs a post-deployment attachment revalidation.

Space Engineering separately proved persistent muted/self-deafened
`DecodeMode::Pass` presence, an earlier consented `Decode` activation, an
automatic participant-change pause, and a manager `/voice leave` recorded as
successful. The current code closes the media epoch and physically disconnects
the `Decode` call whenever consent is invalidated. Its private owner-only
full-chain audition proves local Kokoro → Whisper → canonical Abbey → Kokoro →
Whisper plus Songbird-playable formatting without Discord, a microphone, or
cloud credentials. It does not prove deployment, a reply heard by a human, or
barge-in. The exact current candidate is not yet deployed; a fresh
everyone-present consent epoch, renewed `/voice resume`, an audible wake/reply,
and interruption acceptance remain deliberately unclaimed. OpenAI Realtime is
an explicit degraded backup, not an offline path, and its spoken control is not
authoritative. `tasks/goals.md` remains the detailed evidence ledger, including
the non-green DAVE/OpenMLS dependency audit.
This host has neither Docker nor systemd, so both
deploy artifacts are **unverified as artifacts** — what is verified is that `cargo build --release
--locked` produces the binary they both wrap. The exact stable Rust + locked
release-build gate passed in GitHub Actions on PR #24 on 2026-08-20.

## Gate

```sh
./check.sh
```

CI (`.github/workflows/rust.yml`) runs exactly this script with the exact Rust
1.97.1 toolchain, so a green check means fmt, launchd shell syntax, Python lock
hash validation (plus plist lint where `plutil` exists), clippy `-D warnings`, the test suite, and the
locked release build used by deployment.

`cargo fmt --all -- --check`, deploy syntax, then
`cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`,
and `cargo build --release --locked`.
