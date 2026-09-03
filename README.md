# Abbey Bot

> **Intelligence Without Limits** — Abbey’s Discord operational layer (Rust).  
> Persona routing, memory/WDBX, reputation, and consent-gated voice tooling.  
> Adjacent to the Swift `AbbeyBot` product; shared contracts, not shared runtime.

**Intelligence Without Limits.**

Abbey is your Discord companion for routing, memory, and calm ops help — with personas that say what they know and what they don’t.

Persona routing · Durable memory · Ops with guardrails. Invite Abbey · `/persona ask` · `/help`.

Abbey's Discord operational layer, in Rust — [serenity](https://github.com/serenity-rs/serenity)
0.12 with [poise](https://github.com/serenity-rs/poise) 0.6 for slash commands.

This is the Rust implementation. A separate active Swift/Vapor/DiscordBM product
lives at `~/dev/active/AbbeyBot`; it is an adjacent implementation, not a port,
dependency, or shared runtime. The archived `~/dev/archive/swift-discord` tree is
a home-grown Swift Discord library, not either bot. The Rust and Swift products
share no application code; cross-language agreement is established only through
the independently vendored Abbey contract corpus and each consumer's own gate.

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
| `/voice join consent:true\|resume consent:true\|leave\|status`; `/voice verify start\|report` | Discord voice locked to one env-configured guild/channel. Join/resume require Manage Server, the caller to be present, an explicit everyone-present consent attestation, and a public disclosure before the software media gate opens. A new, unidentified, or unattested participant closes the media epoch and disconnects the conversational `Decode` call; renewed consent starts a fresh call. Leave is available to someone present or a manager. The owner/admin-only local verifier spans join, participant pause/resume, and final leave with content-free in-memory counters; while armed it disables voice conversation commits. `ABBEY_VOICE_AUTOJOIN=1` is restart-resilient muted/self-deafened no-audio presence regardless of the selected conversational backend. |

**The model can call Abbey's own systems.** Whenever the existing tools policy
is enabled, production offers exactly `[Core, Inspect]` in this stable order:
`remember_fact`, `lookup_reputation`, `recall`, `switch_persona`,
`recent_messages`, `inspect_status`, `list_facts`. Both the OpenAI-compatible
and Foundation Models decision schemas expose all seven, and the Anthropic
request representation consumes the same shared definitions. The original
`abbey_tools()` definition and wire fixtures remain the byte-compatible
five-Core-tool corpus; Inspect is additive to that preserved contract. There
is no partial Inspect toggle: `ABBEY_BOT_LLM_TOOLS=off` suppresses the complete
vocabulary.

Calls run against the same memory/WDBX/reputation the slash commands use,
scoped to the server and person in the conversation, for at most three rounds.
`list_facts` is a bounded canonical fact-and-pending-replacement snapshot, not
semantic WDBX recall; it reports omitted facts and pending replacements
separately and never clips a replacement into a misleading partial value.
`inspect_status` exposes only effective routable capability categories and
safe `configuration` versus `qualified-manifest` provenance. Its guild-scoped
voice value is exactly `off`, `presence`, `awaiting-consent`, `active`, or
`paused`; it exposes no identities, counts, consent epochs, model names,
counters, timestamps, media, audio, or transcripts. None of the seven tools
posts, moderates, or changes configuration. A backend that rejects tooled
requests is retried once without tools and remembered. A 2026-08-19 gpt-oss:20b
session exercised `remember_fact`; that dated observation is historical
evidence, not proof about the currently installed service.

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

Credential selection is source-aware and fail-closed. A present, nonblank
`DISCORD_TOKEN` wins. `DISCORD_BOT_TOKEN` is consulted only when the primary is
absent; a present-but-blank or non-Unicode primary is an error and never falls
through. A blank or non-Unicode fallback has its own configuration error. The
selected secret has no token-bearing debug/display representation. After the
explicit token-free self-test modes, Abbey authenticates it with Discord
before guild parsing, application state, voice, schedulers, Telegram/Slack, or
framework setup. Accepted and rejected diagnostics name only the selected
environment-variable source and never its value.

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
reasoning or vision backend. The source/configuration target for the next
cross-platform reasoning/vision deployment is `gemma4:12b` through the
OpenAI-compatible endpoint seam. This operator choice supersedes both the interim
`gemma4:e4b` choice and the earlier `gpt-oss:20b` benchmark recommendation;
dated results below remain historical evidence.

**Current live topology (2026-09-03 ~18:00 ET):** Ollama `:11434` is the reasoner; MLX-Audio `:8181` is live; MLX-VLM `:8282` is unpublished. Do not point `ABBEY_BOT_LLM_ENDPOINT` at `:8282` until the installer smoke passes on the exact snapshot.

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
preloaded snapshot. Apple Foundation Models remains an optional secondary. Its
server facade is text-only and tool-incapable; CLI capabilities are enabled
only when a current, owner-only qualification manifest matches the Abbey
binary, `fm` executable, OS build, mode, and fixture version. It is not the
Gemma default.

## Configured backends

`/persona ask` answers come from an external or local model, never from the
bot itself — this crate contains no inference engine. Which backend answers is
selected from the environment, first match wins:

| Env var | Backend |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic Messages API (external, per-token cost); model `claude-sonnet-5`. This secret is environment-only and is never placed in a commit or image layer. |
| `ABBEY_BOT_LLM_ENDPOINT` (+ `ABBEY_BOT_LLM_MODEL`) | An OpenAI-compatible server, usually loopback (for example Ollama or llama.cpp/llama-server). Base URL only, e.g. `http://127.0.0.1:11434` — the bot POSTs to `<endpoint>/v1/chat/completions`. Plain HTTP is accepted only for loopback; remote endpoints require HTTPS, and credentials/query strings in the base URL are rejected. The source default model name and next cross-platform deployment target is **`gemma4:12b`**; this is not a claim about the currently installed service. The operator choice supersedes both the interim `gemma4:e4b` choice and the 2026-08-19 benchmark's `gpt-oss:20b` recommendation. The dated benchmark remains historical timing evidence: gpt-oss answered in 7–25 s, e4b in 13–37 s, and 12b in 32–94 s on that host. Ollama uses the model field; a server bound to one model may ignore it. Local replies stream: the message appears within ~4 s and grows; one generation runs at a time (`ABBEY_BOT_LLM_CONCURRENCY`), extra turns wait up to `ABBEY_BOT_LLM_QUEUE_SECS` (90) then get an honest "busy" line. Reasoning models are handled: the local budget is 4,096 tokens, and a reply whose budget went entirely to `reasoning` is reported as exactly that. |

The backend contract is intentionally portable. Linux and Windows retain the
same OpenAI-compatible endpoint seam and may use Ollama, llama.cpp, or another
runtime that passes Abbey's exact interface tests. On macOS, the pinned
MLX-VLM profile above is the *optional* unified Gemma adapter (live reasoner remains Ollama `:11434`; `:8282` unpublished); its installer will not
publish the service unless the production text/tool/vision wire shapes pass
offline. Apple `fm serve` is an optional OpenAI-compatible adapter, not the
cross-platform default; its current facade is suitable only with Abbey tools
disabled.

Apple Foundation Models can also be configured as an explicit secondary, and
is never discovered or selected implicitly:

```sh
ABBEY_FM_MODE=system                 # off (default), system, or explicit pcc
ABBEY_FM_ENDPOINT=http://127.0.0.1:1976
ABBEY_FM_CLI=/usr/bin/fm
ABBEY_FM_FALLBACK=1
ABBEY_FM_CAPABILITY_MANIFEST=$HOME/.config/abbey-bot/fm-capabilities.json
```

The endpoint must be loopback. Read-only fallback preserves the existing
Anthropic → configured local endpoint order before trying Foundation Models.
With no primary configured, plain streamed text may use `fm serve`; after a
streamed provider failure Abbey fails closed instead of risking a failure edit
followed by a second provider post. A tool-capable primary failure also fails
closed because a tool may already have run; Abbey does not restart the request
on a second provider without typed no-side-effect evidence. The server route is
structurally tool-incapable: a current macOS 27 probe returned
prose rather than an OpenAI `tool_calls` record. Tool-capable turns instead use
`fm respond --no-stream --schema` through an argument array and stdin, with a
private temporary schema, bounded output and timeout, strict final-or-one-tool
parsing, and no transcript-saving flag. `pcc` is a cloud route and occurs only
when `ABBEY_FM_MODE=pcc`; Apple may reject it outside an attributed Terminal
session, in which case Abbey fails closed. These `fm` flags are qualified
against the installed macOS 27 CLI and may need requalification after OS beta
updates.

Qualification runs before Abbey reads `DISCORD_TOKEN` or `ABBEY_DATA_DIR` and
uses only synthetic prompts, a no-side-effect `probe_status` tool, generated
shape/OCR images, and ephemeral turns. For the on-device route, publish the
successful report atomically as a private manifest; never qualify PCC as a
substitute:

```sh
./deploy/install-launchd.sh
release_binary=$PWD/target/release/abbey-bot
installed_binary=$HOME/.local/libexec/abbey-bot/abbey-bot
release_sha=$(shasum -a 256 "$release_binary" | awk '{print $1}')
installed_sha=$(shasum -a 256 "$installed_binary" | awk '{print $1}')
test "$release_sha" = "$installed_sha"

model_dir=$HOME/.local/share/abbey-bot/mlx-vlm/huggingface/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/73bcf09092aa277861d5a191b989b666f7f32e8f
manifest_dir=$HOME/.config/abbey-bot
manifest=$manifest_dir/fm-capabilities.json
mkdir -p "$manifest_dir"
chmod 700 "$manifest_dir"
env \
  -u ANTHROPIC_API_KEY \
  -u ABBEY_VISION_KEY \
  -u ABBEY_FM_ENDPOINT \
  -u ABBEY_FM_CAPABILITY_MANIFEST \
  -u ABBEY_DATA_DIR \
  -u DISCORD_TOKEN \
  -u OPENAI_API_KEY \
  -u TELEGRAM_BOT_TOKEN \
  -u SLACK_BOT_TOKEN \
  -u SLACK_APP_TOKEN \
  ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:8282 \
  ABBEY_BOT_LLM_MODEL="$model_dir" \
  ABBEY_BOT_LLM_TIMEOUT_SECS=600 \
  ABBEY_BOT_LLM_TOOLS=on \
  ABBEY_VISION_PROVIDER=remote \
  ABBEY_VISION_ENDPOINT=http://127.0.0.1:8282/v1 \
  ABBEY_VISION_MODEL="$model_dir" \
  ABBEY_FM_MODE=system \
  ABBEY_FM_CLI=/usr/bin/fm \
  ABBEY_FM_FALLBACK=1 \
  RUST_LOG=off \
  python3 deploy/publish-provider-qualification.py \
    --binary "$installed_binary" \
    --output "$manifest" \
    --target all \
    --timeout 900
```

Re-run the qualification whenever the Abbey binary, `fm` executable, selected
mode, OS build, or fixture version changes. A stale, malformed, symlinked,
oversized, wrong-owner, or group/world-readable manifest fails startup. To
publish, always use the checked-in publisher above: it verifies the passing
report and bound binary hash before an owner-only same-directory replacement,
and preserves an existing manifest on probe/validation failure. Publication is
POSIX-only because it relies on effective-user ownership and mode bits. The
Windows gate parses and privacy-checks the publisher and records its runtime
tests as skipped; it does not claim that a Windows host published a manifest.
Install through `install-launchd.sh` once while the previous known-good
environment is still active, and require the installed/release hashes above to
match before qualification. Qualify that already-installed binary so a later
rebuild cannot silently stale the manifest. Do not rerun the installer or
otherwise replace the binary between manifest publication and the complete
acceptance sequence. The configurator rechecks its exact SHA-256.

To atomically switch the owner environment to the pinned MLX primary while
preserving secret values and a private rollback copy, validate first:

```sh
python3 deploy/configure-mlx-primary.py \
  --binary "$installed_binary" \
  --model-dir "$model_dir" \
  --manifest "$manifest" \
  --dry-run
```

Then repeat the same command with `--apply-and-restart` in place of
`--dry-run`. Plain apply is intentionally unsupported. The apply mode publishes
the owner environment, restarts the fixed Abbey launchd agent without replacing
its binary, and requires one new PID to remain stable for a bounded window. If
the candidate fails to start or remain stable, it atomically restores the
byte-exact pre-cutover environment and verifies a second restart under the old
configuration. The private backup is retained even after a successful rollback;
a failed rollback restart is reported fail-closed and requires operator repair.
Both configurator modes acquire the installer's existing owner-only
`~/.local/share/abbey-bot/install.lock` before reading the environment, binary,
or manifest and retain it through validation, publication, restart, and any
rollback. Install, uninstall, and another cutover therefore cannot race this
transaction. A preexisting lock is never stolen automatically; inspect its PID
record and the owning process before any manual recovery.

The generated environment explicitly blanks inherited Anthropic, vision-key,
and `fm serve` values, pins `/usr/bin/fm`, and forces tools on. Prior secret
assignments remain commented in the environment and byte-exact in the private
rollback copy; neither dry-run nor apply-and-restart prints their values. Both
modes reject fake, stale, non-`all`, wrong-binary, wrong-model, wrong-`fm`,
wrong-OS, or capability-incomplete manifests before touching the owner
environment.

With neither set, `/persona ask` replies that no generation backend is
configured, and the pipeline never speaks unsolicited (a mention gets the same
honest reply). No test requires either variable, a network, or a key — the gate
runs fully offline.

| Env var | What it enables |
|---|---|
| `ABBEY_DATA_DIR` | Persistence: `abbey-state.json` (guild config, brain snapshots, reputation, memory) + `wdbx.seg.0.jsonl` (the WDBX v1 segment holding semantic memory). Unset = in-memory, lost on restart. |
| `ABBEY_BOT_LLM_TOOLS` | `off` disables the complete seven-tool Core-plus-Inspect vocabulary; default on. There is no separate Inspect switch. A tool-contract rejection degrades only that provider's tool route. |
| `ABBEY_FM_MODE` / `_ENDPOINT` / `_CLI` / `_FALLBACK` / `_CAPABILITY_MANIFEST` | Explicit Apple Foundation Models secondary. Mode defaults to `off`; fallback must separately be `1`; endpoint is loopback-only; CLI defaults to `/usr/bin/fm`. Enabling fallback also requires a matching owner-only qualification manifest. `system` is on-device; `pcc` is an explicit cloud selection and is not qualified by this repository's system-mode evidence. |
| `ABBEY_QUIET=1` | Never speak unsolicited, anywhere — mentions, DMs, and commands still answer. The operator's guard while the policy is untrained. Wins over every server's `/admin act on`. |
| `ABBEY_MESSAGE_CONTENT=1` | Requests the privileged MESSAGE_CONTENT intent (must also be on in the Dev Portal). Without it, only mentions and DMs carry a body, and the pipeline learns from those alone. |
| `ABBEY_VISION_PROVIDER` / `_ENDPOINT` / `_MODEL` / `_KEY` | `remote` (default) selects one verified OpenAI-compatible endpoint, `fm` selects only a manifest-qualified FM CLI, and `off` disables vision. Abbey never retries an image through another provider. JPEG, PNG, WebP, and GIF are decoded under 8192×8192-pixel and 96 MiB allocation limits before transport; GIF's first frame is converted to PNG. A 2026-08-19 Ollama/e4b screenshot result is historical only; it does not qualify the current MLX-VLM target or an installed FM CLI. |
| `ABBEY_VOICE_GUILD_ID` + `ABBEY_VOICE_CHANNEL_ID` | Enables `/voice` for exactly one Discord voice channel. With no destination, voice remains off on every OS. `ABBEY_VOICE_MODE` is `local` by default on macOS, `disabled` for presence only, or `openai` as an explicit cloud backup; Linux/Windows reject `local` configuration and require `disabled` or explicitly configured OpenAI Realtime. Local mode uses the loopback-only `ABBEY_VOICE_LOCAL_ENDPOINT` (default `http://127.0.0.1:8181`) with Whisper STT, Kokoro TTS, `af_heart`, and the existing loopback Abbey text backend. OpenAI mode alone requires `OPENAI_API_KEY`; a key never selects it. It is a direct, whole-response-buffered degraded backup without local ABI routing or WDBX context, and spoken control is non-authoritative—use `/voice leave` or write `stop listening` in voice chat. `ABBEY_VOICE_AUTOJOIN=1` always uses muted/self-deafened `DecodeMode::Pass` with no receive/playback actor. Conversation still requires `/voice join consent:true`; consent invalidation disconnects the conversational call and renewed consent requires `/voice resume consent:true`. |
| `TELEGRAM_BOT_TOKEN` | Runs the Telegram long-poll adapter beside the Discord gateway. |
| `SLACK_BOT_TOKEN` + `SLACK_APP_TOKEN` | Runs Slack over Socket Mode (`xoxb-` + `xapp-`). |

## Verification and acceptance layers

Abbey keeps these evidence layers separate; passing one never implies the next:

1. Focused source tests prove only their named contracts.
2. An isolated `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` plus locked release
   build proves the checked-out source on that host. It does not prove delivery.
3. A reviewed local SHA equal to `origin/main` proves normal mainline delivery.
4. Successful Ubuntu, macOS, and Windows jobs for that exact remote SHA prove
   hosted source/build coverage. Windows CI is not a real Windows live-runtime
   acceptance.
5. Synthetic provider fixtures and an identity-bound manifest prove only the
   exact provider binary/model/OS/schema/sandbox qualification they name.
6. An atomic install record with matching artifact hashes, environment source,
   listener ownership, and rollback target proves only installed identity.
7. A controlled foreground two-guild text/tool/image run proves live Discord
   behavior for that exact foreground binary and selected qualified provider.
8. Fresh unanimous consent plus a human-witnessed 8/8 lifecycle proves only the
   consented voice run.
9. Repeating the complete protocol through the managed service proves the
   managed deployment. It cannot be inferred from the foreground run.

The complete privacy-preserving operator sequence is in
`docs/live-test-protocol.md`. Telegram and Slack share source and CI coverage,
but are not live-qualified unless their own connector round trips are
explicitly recorded.

For final local voice acceptance, an owner or administrator runs `/voice verify
start` before the consented join and `/voice verify report` after the final
`/voice leave`. The process-memory-only report contains fixed milestone counts,
consent epochs, and participant counts, never participant identities, audio,
transcripts, responses, or message content. While the run is armed, voice
conversation commits are disabled. It observes decoded voiced input, local STT
completion, natural synthesized playback completion, an actual playback handle
stopped by barge-in, participant-change pause plus fresh-epoch resume, and the
final Songbird leave. It cannot prove that each human actually consented or
heard the response; those remain explicit manual witness facts, separate from
source tests and the offline audition.

Provider self-test exit codes are `0` when every capability required by the
selected runtime configuration passes, `1` for a required probe failure, and
`2` for invalid arguments, invalid configuration, or an unconfigured selected
target. Version-1 reports expose separate `primary`, `fm_server`, and `fm_cli`
entries; each text, streaming, structured-output, tools, vision, and OCR result
is still recorded as `pass`, `fail`, `unsupported`, or `skipped`. With remote
or disabled vision, FM text/schema/tools may qualify while failed FM vision/OCR
remain unadvertised in the manifest-derived runtime capabilities. Selecting
`ABBEY_VISION_PROVIDER=fm` makes both image probes required and fatal. Reports
contain only fixed failure categories and safe identity metadata, never
credentials, prompts, transcripts, image bytes, provider response bodies, or
production state. Default-off builds work on every supported OS; explicitly
configuring Foundation Models on a non-macOS host fails with an
unsupported-platform error.

Measured 2026-08-19 with Abbey's real prompt (full table and method in
`docs/benchmarks/2026-08-19-local-models.md`):

| model | typical reply | hidden reasoning | verdict |
|---|---|---|---|
| gpt-oss:20b | 7–25 s | light | 2026-08-19 benchmark winner; historical tool-calling evidence |
| gemma4:e4b | 13–37 s | moderate | 2026-08-19 runner-up; later interim choice, now superseded |
| **gemma4:12b** | 32–94 s | heavy | **source/config deployment target**; installed cutover requires separate evidence |
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
is not persisted. Abbey must retain View Channel, Send Messages, Connect, Speak, Stream, and
Use Embedded Activities and must not be server-muted/deafened/suppressed; startup rechecks those
conditions around activation, and channel/role/member changes stop the media
epoch if the call could become receive-only. `/voice leave` tears down both
sides, while `/voice status`
reports mode, phase, models, consent epoch, and bounded-queue counters without
content or credentials. The local-only `/voice verify start|report` surface is
further limited to the owner or an administrator. It keeps one ephemeral,
redacted acceptance run across consent epochs and suppresses voice transcript
commits while armed; a process restart clears it. In explicit `openai` backup
mode, Realtime is a degraded direct provider path: spoken stop detection is not authoritative, so
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
  `ABBEY_VISION_PROVIDER=remote` plus
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
  installer keep the env, learned state, WDBX segment, and logs owner-only.
  Those dated observations do not describe the current installation after a
  subsequent cutover. Use a current acceptance record containing exact hashes,
  PIDs, listeners, provider report, and model identities; source, CI, or WAV
  evidence alone never establishes what is installed.
- **Docker** — the multi-stage `Dockerfile`. Pass secrets with
  `docker run --env-file`; never bake a token into an image layer.

Honesty note: **live evidence is cumulative and command-specific.** On
2026-08-19, the operator's Discord client verified gateway registration, generated
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

An operator-designated sandbox voice channel separately proved persistent muted/self-deafened
`DecodeMode::Pass` presence, an earlier consented `Decode` activation, an
automatic participant-change pause, and a manager `/voice leave` recorded as
successful. The current code closes the media epoch and physically disconnects
the `Decode` call whenever consent is invalidated. Its private owner-only
full-chain audition proves local Kokoro → Whisper → canonical Abbey → Kokoro →
Whisper plus Songbird-playable formatting without Discord, a microphone, or
cloud credentials. It does not prove deployment, a reply heard by a human, or
barge-in. A fresh everyone-present consent epoch, renewed `/voice resume`, an
audible wake/reply, and interruption acceptance require their own current
record and remain unclaimed by source evidence. OpenAI Realtime is
an explicit degraded backup, not an offline path, and its spoken control is not
authoritative. `tasks/goals.md` retains the dated dependency-audit history.
The Linux graph is now Rustls/WebPKI-only: the gate rejects `native-tls`,
`openssl`, and `openssl-sys`, so the package-free Docker runtime uses compiled
roots. Serenity 0.12.5's Rustls WebSocket edge still pins `rustls-webpki`
0.102.8. Exactly four vulnerability records are accepted temporarily:

- `RUSTSEC-2026-0049` / `GHSA-pwjx-qhcg-rvj4`
- `RUSTSEC-2026-0098` / `GHSA-965h-392x-2mh5`
- `RUSTSEC-2026-0099` / `GHSA-xgp8-3hg3-c2mh`
- `RUSTSEC-2026-0104` / `GHSA-82j2-j2ch-gfr8` — malformed-CRL reachable panic

The deterministic checker binds each record to the exact package, version,
source, checksum, aliases, patched/unaffected ranges, categories, CVSS and
informational state, withdrawal state, and dependency identity. Any missing,
additional, or changed vulnerability fails closed. Its successful result says
that four vulnerabilities remain and the audit is **not clean**. The
`cargo-audit` 0.22.2 pin stabilizes report parsing; it is tooling, not a fifth
accepted finding. Informational warnings are reported separately. The fixed
0.103 line is not compatible with Serenity's current `tokio-tungstenite` 0.21
edge, so re-review is required when that upstream route or support policy
changes. Abbey also carries the provenance-checked `openmls_rust_crypto` 0.5.1
compatibility patch.

The checkout-local `launch.sh` and `run_bot.sh` are owner-private mode-0700
helpers, untracked and locally excluded. Tracked gates do not read or validate
them, and they must never be staged or used as public evidence. `bot.log` is
ignored and remains unread while a manually launched process is active. A
manual process is not a managed-service installation. Before any transition,
re-resolve its exact PID, owner, parent, start time, working directory,
executable path, and executable hash; signal only that verified process, never
use a broad process-name kill, and never replace its executable during source
work.

This host has neither Docker nor systemd, so those deploy wrappers remain
unverified as running artifacts. A locked release build proves only the built
binary it produced, not an installation or service.

## Gate

```sh
./check.sh
# On Windows PowerShell:
./check.ps1
```

CI (`.github/workflows/rust.yml`) runs Ubuntu and macOS through `check.sh` and
Windows through `check.ps1`, all with the exact Rust 1.98.0 toolchain. Every
lane proves formatting, Python syntax and hash locks, the static privacy gate,
the vendored Abbey corpus guard plus its Rust verifier, the Linux Rustls/WebPKI
dependency-tree invariant, Clippy with warnings denied, the offline test suite,
and the locked release build. The WDBX parity script runs in every lane but reports an explicit external
skip in a standalone checkout; it becomes required when
`ABBEY_REQUIRE_WDBX_CONFORMANCE=1` and `ABBEY_WDBX_REPO` identifies the canonical
sibling. POSIX deployment-shell syntax runs on Ubuntu/macOS; plist lint also
runs where `plutil` exists.

The delivered pre-stabilization baseline is
`9716f00f4b9dfe4c8ddfa1e126e74ba2cf9fdde1`. GitHub Actions run
`33218303755` completed its Ubuntu, macOS, and Windows gate jobs successfully
on 2026-08-28. That is historical cross-platform source/build evidence for
that SHA only. This stabilization work still requires its isolated strict
gate, locked release build, normal push, and exact-head CI before those layers
can be claimed. Provider qualification, installed identity, foreground/live
Discord, consented voice, managed-service acceptance, and real Windows runtime
remain separate and unclaimed by that baseline run.

The shared Rust sequence is
`cargo fmt --all -- --check`, platform-appropriate deployment validation, then
`cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`,
and `cargo build --release --locked`.
