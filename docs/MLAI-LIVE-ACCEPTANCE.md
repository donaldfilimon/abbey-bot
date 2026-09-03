> **Update 2026-09-03 ~17:50 ET:** Live Abbey reinstalled after #53–#55 (`main` `98a498a`). Binary SHA `69f20f8fb0aa86f2…`, launchd PID **7970**, `connected user=Abbey`. Personality warm/sharp friend in binary. CLAUDE.md/AGENTS.md synced to live topology (#55). Sidecars unchanged: MLX-Audio `:8181` 200; Ollama `:11434` 200 host-only LLM; MLX-VLM `:8282` unpublished. `quesar.cloud` still Hostinger DNS parking. Dependabot rustls-webpki awaits serenity+poise crates.io bump (serenity `next` incompatible with poise 0.6.2).
>
> **Update 2026-09-03 ~17:15 ET:** Staged mlx-vlm 0.6.15 + Gemma 4 12B 4-bit (`73bcf090…`) **does** force `probe_status` and streamed `MLX_READY`. After a tool *result*, the 4-bit checkpoint loops `<|channel>thought\n<channel|>` into `content` until `finish_reason=length`. JSON tool-body→mapping + chat_template mapping-before-sequence are **necessary** (string bodies render as `value:"{…}"`; dicts otherwise hit Jinja `sequence` and 500) but **do not** stop the loop. `--enable-thinking` / thinking-budget / generation-prompt experiments also failed. `:8282` stays unpublished. Ollama `http://127.0.0.1:11434` remains the reasoner. Installer now patches encoding at install time so a later checkpoint can reuse the path; smoke still fail-closes on `TOOL_CONTINUATION_READY`.
>
# MLAI live acceptance — remaining parity gaps

> **Update 2026-09-03 ~16:30 ET:** Voice/sidecar operator path **#47** is on `main` (`dd9e6b4`) and live. Binary at `~/.local/libexec/abbey-bot/abbey-bot` matches release build; launchd `com.donaldfilimon.abbey-bot` PID 92752; log shows `operator env key presence (values withheld)` with all 10 expected keys present and `connected user=Abbey`. `/voice status` now reports sidecar 2s probe + loopback LLM line; join fails closed before the 10-minute sidecar prepare when the loopback LLM is missing. Plan docs **#48** also on `main` (`e329340`).
>
> MLX-Audio still live (`com.donaldfilimon.abbey-mlx-audio`); operator readiness remains `GET /` or `GET /v1/models`. Live process may also answer `GET /health` 200 — do not treat `/health` as the only probe.
>
> LLM host-only `http://127.0.0.1:11434` unchanged. Still human-gated: Office Hours live `/voice` 8/8; Discord Member mass-grant + Admin Administrator policy; quesar.cloud NS (Hostinger parking until LB IP + Cloudflare).

> **Update 2026-09-03 ~16:08 ET:** MLX-Audio is **LIVE** via launchd `com.donaldfilimon.abbey-mlx-audio` on `127.0.0.1:8181`. Readiness is `GET /` or `GET /v1/models` (prefer those; live process may also expose `/health`). Whisper + Kokoro loaded; offline smoke passed. Installer patches `webrtcvad` with `importlib.metadata` for setuptools 83 (`pkg_resources` removed). Do **not** probe `/health` for operator readiness.
>
> LLM: `ABBEY_BOT_LLM_ENDPOINT` must be host-only `http://127.0.0.1:11434` because `src/llm/dialect.rs` appends `/v1/chat/completions`. Vision keeps `/v1` (`http://127.0.0.1:11434/v1`). Generation backend configured; guild-scoped commands on MLAI `1275617641620443146`.
>
> Still human-gated: live `/voice` consent in Office Hours (Donald must be in that VC for 8/8). MLX-VLM still not loaded.

**Date:** 2026-09-03 (America/New_York)  
**Checkout:** `/Users/donaldfilimon/dev/active/abbey-bot`  
**Git:** `main` — this commit keeps webrtcvad `importlib.metadata` patch + `/v1/models` readiness; records live MLX-Audio + host-only LLM evidence  
**Scope:** two remaining gaps only — (1) live voice acceptance, (2) MLX acceleration qualification. Fail closed where evidence is missing. Historical 2026-08-20/22 voice notes do **not** qualify this process, this binary, or this config.

This file is an operator evidence checklist, not proof that the run happened. Do not treat source tests, provider logs, leftover venvs, Homebrew `mlx-lm`, Ollama `gemma4:12b-mlx`, or prior consent as substitutes.

---

## Current config state

Presence / booleans / ID match only. **No token or secret values.**

### Two env files (they are not the same)

| Key | Checkout `.env` (mode 0600, 8 keys) | Launchd `~/.config/abbey-bot/env` (mode 0600, 10 keys; this is what the running agent loads) |
|---|---|---|
| `DISCORD_TOKEN` | present (secret withheld) | present (secret withheld) |
| `ABBEY_GUILD_ID` | present, numeric, **MATCH** MLAI `1275617641620443146` | present, numeric, **MATCH** MLAI `1275617641620443146` (guild-scoped commands) |
| `ABBEY_BOT_LLM_ENDPOINT` | present, host-only `http://127.0.0.1:11434` | present, host-only `http://127.0.0.1:11434` — **must stay host-only**; `src/llm/dialect.rs` appends `/v1/chat/completions` |
| `ABBEY_BOT_LLM_MODEL` | present = `gemma4:12b` | present = `gemma4:12b` |
| `ABBEY_VISION_ENDPOINT` | present, loopback HTTP **:11434/v1** | present, loopback HTTP **:11434/v1** (vision **keeps** `/v1`) |
| `ABBEY_VISION_MODEL` | present = `gemma4:12b` | present = `gemma4:12b` |
| `ABBEY_VOICE_GUILD_ID` | **MISSING** | present, numeric, **MATCH** MLAI `1275617641620443146` |
| `ABBEY_VOICE_CHANNEL_ID` | **MISSING** | present, 19-digit numeric (value not copied here) |
| `ABBEY_VOICE_MODE` | **MISSING** | present = `local` |
| `ABBEY_VOICE_AUTOJOIN` | missing (default 0) | missing (default 0) |
| `ABBEY_VOICE_LOCAL_ENDPOINT` | missing (code default `http://127.0.0.1:8181`) | present = `http://127.0.0.1:8181` |
| `ABBEY_VOICE_LOCAL_STT_MODEL` / `_TTS_MODEL` / `_TTS_VOICE` / `_LANGUAGE` | missing | missing |
| `ABBEY_VOICE_WAKE_WORD_REQUIRED` | missing (example default `1`) | missing |
| `ABBEY_FM_MODE` / `_ENDPOINT` / `_CLI` / `_FALLBACK` / `_CAPABILITY_MANIFEST` | missing (FM stays off) | missing |
| `ABBEY_PROVIDER_MLX_*` / `ABBEY_PROVIDER_MANIFEST` | missing | missing |
| `OPENAI_API_KEY` / `ABBEY_VOICE_MODE=openai` | missing / not selected | missing / not selected |
| Telegram / Slack tokens | missing | missing |

Checkout extras (names only): `DISCORD_BOT_TOKEN` (secret withheld), `RUST_LOG`.

### Running process vs sockets (2026-09-03 ~15:56 ET, MLX-Audio rechecked)

| Item | Observed |
|---|---|
| `com.donaldfilimon.abbey-bot` | **loaded, running**, PID 14101, last exit 0, plist `~/Library/LaunchAgents/com.donaldfilimon.abbey-bot.plist` |
| `com.donaldfilimon.abbey-mlx-audio` | **loaded, running**, PID 21413 (runs=2), plist `~/Library/LaunchAgents/com.donaldfilimon.abbey-mlx-audio.plist` |
| `com.donaldfilimon.abbey-mlx-vlm` | **not loaded** — same; no LaunchAgents plist |
| TCP `127.0.0.1:8181` (Abbey MLX-Audio) | **listening** — readiness is `GET /` or `GET /v1/models` (**NOT** `/health` for stock mlx-audio 0.5.0). Whisper + Kokoro loaded; offline smoke passed. |
| TCP `127.0.0.1:8282` (Abbey MLX-VLM) | **not listening** |
| TCP `127.0.0.1:11434` | Ollama listening (checkout LLM/vision target) |
| TCP `127.0.0.1:8080` | Homebrew `homebrew.mxcl.mlx-lm` PID 1069 listening; `GET /v1/models` returned HTTP 200 with **empty body** — **not** Abbey's pinned VLM sidecar |
| Managed log | `generation backend configured` (`configured OpenAI-compatible endpoint`) |
| Command registration | guild-scoped (instant) because launchd env has `ABBEY_GUILD_ID` |
| Gateway | connected as `Abbey`; at least one MLAI guild message handled (`guild=Some(1275617641620443146)`) |

### MLX-Audio live API (2026-09-03 ~16:03 ET)

Live `GET http://127.0.0.1:8181/openapi.json` paths (operator readiness = `/` or `/v1/models`):

| Path | Methods | Live check |
|---|---|---|
| `/` | GET | HTTP 200 welcome JSON (`Welcome to the MLX Audio API server!`) |
| `/v1/models` | GET, POST, DELETE | GET 200; ids `mlx-community/whisper-large-v3-turbo-asr-fp16`, `mlx-community/Kokoro-82M-bf16` |
| `/v1/audio/speech` | POST | TTS (Kokoro); used by installer offline smoke |
| `/v1/audio/transcriptions` | POST | STT (Whisper); used by installer offline smoke |
| `/v1/audio/voices` | GET | 200 with `?model=mlx-community/Kokoro-82M-bf16` (`af_heart` present); bare GET is 400 |
| `/v1/audio/separations` | POST | Present in OpenAPI; not part of Abbey STT/TTS acceptance |
| `/health` | — | **Do not use for readiness.** Stock mlx-audio 0.5.0 returns HTTP 404. Operator readiness is `GET /` or `GET /v1/models`. |

Installer `wait_for_health` curls `GET /v1/models` (not `/health`). STT/TTS load still POSTs `/v1/models`.

Voice destination vs MLAI: **guild ID matches**. Channel is the single 19-digit ID in the launchd env. Historical 2026-08-20 notes called a prior presence target **Engineering**; this checklist does not re-publish the current snowflake. Confirm the locked channel with `/voice status` inside MLAI Community before joining.

---

## Gap 1 — live voice acceptance

Required lifecycle (todo + `docs/live-test-protocol.md` §4):  
**join `consent:true` → wake → barge-in → membership-close → resume → leave**  
plus owner/admin `/voice verify start` / `report` with `observed: 8/8`, written `stop listening`, and a human audible witness. Source tests and historical consent are **not** substitutes.

### Preconditions (MLX-Audio sidecar is up; human 8/8 and in-VC consent still failing)

Do **not** start the human run until these are true. Otherwise join will fail closed (local-speech health up to 600s) or produce no spoken reply.

1. Abbey MLX-Audio is installed, launchd-loaded, and serving on `127.0.0.1:8181` (Whisper + Kokoro + `af_heart`). **True today.** Operator readiness: `GET /` or `GET /v1/models` (**NOT** `/health` — mlx-audio 0.5.0 404). Human 8/8 in Office Hours is still required.
2. A loopback reasoning backend is configured **in the env the running process actually loads**. Launchd `ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434` (host-only — `dialect.rs` appends `/v1/chat/completions`); vision keeps `http://127.0.0.1:11434/v1`. **True today.**
3. Donald (Manage Server for join/resume/status; owner/Administrator for verify) is **physically in** the launchd-locked MLAI voice channel. Remote activation is refused.
4. Fresh unanimous consent from **everyone currently present**. Silence, history, and one person speaking for another do not count.
5. Abbey has View Channel, Send Messages, Connect, Speak in that VC and is not server-muted/deafened/suppressed.

### Exact slash commands (MLAI Community, locked VC)

Run these **in MLAI Community** (`1275617641620443146`) while Donald is **in the configured voice channel**. Join/resume/status require **Manage Server**. Verify requires **server owner or Administrator**. Leave is available to a manager **or** someone present in that channel.

| Step | Who | Command / action | Where |
|---|---|---|---|
| 0 | Owner/admin | `/voice status` | MLAI Community (any guild text channel is fine; command is guild-only and locked to this guild). Confirm mode `local`, destination, inactive media, no credentials. |
| 1 | Owner/admin | `/voice verify start` | Same guild, **before** join. Arms content-free counters; **disables conversation commits** while armed. Does not start capture. |
| 2 | Humans in VC | Publish/read the local-processing notice; every person currently present explicitly agrees | The locked voice channel (and its text chat as needed) |
| 3 | Manager, **in VC** | `/voice join consent:true` | Must be issued while Donald is in the locked VC. `consent:false` keeps voice off. |
| 4 | Human in VC | Wake turn: say a token-bounded wake name — **Abbey / Abby / Aviva / Abi** — then a short request | Locked VC |
| 5 | Human in VC | Barge-in: speak during Kokoro playback so playback truncates | Locked VC |
| 6 | Human | Membership-close: a person joins or leaves the VC (new/unattested participant) | Locked VC |
| 7 | Humans in VC | Fresh notice + unanimous consent for the **new** set | Locked VC |
| 8 | Manager, **in VC** | `/voice resume consent:true` | New consent epoch; do not reuse the old one |
| 9 | Human in VC chat | Written `stop listening` (authoritative in local mode) | Locked VC text |
| 10 | Manager or present member | `/voice leave` | Same guild |
| 11 | Owner/admin | `/voice verify report` | Same guild. Pass only if `observed: 8/8` **and** a human attests audible wake/reply + current unanimous consent |

Boolean parameter name is exactly `consent`. Discord UI: set it to **True**.

### Pass / fail criteria

Record only human pass/fail, coarse Inspect states, and verify counters. Do **not** copy identities, transcripts, audio, prompts, or raw logs into evidence.

| Check | PASS | FAIL / NOT OBSERVED |
|---|---|---|
| `/voice status` before join | mode `local`; media inactive; destination is the locked MLAI channel; no credentials | Wrong guild, voice unconfigured, or `openai`/`disabled` |
| Public notice | Posted before capture opens | Join succeeded without notice |
| Join | `/voice join consent:true` from an in-channel manager; Abbey unmutes/undeafens only after checks; Inspect `off` → `awaiting-consent`/`active` | Join from outside the VC; `consent` false; speech sidecar down; missing Connect/Speak |
| Wake | Human hears Kokoro reply; completed-turn counter increments; Whisper attributed the attested speaker | No audio; wake name ignored; unverified STT; “no generation backend” |
| Barge-in | Playback **audibly** stops mid-utterance; barge-in counter increments | Playback finishes; counter unchanged; error reported as barge |
| Membership-close | New/unknown/unattested participant immediately closes capture, playback, STT; conversational `Decode` disconnects; Inspect `paused`; no frame from the new person is processed | Session continues; new speaker transcribed |
| Resume | New notice + fresh consent + `/voice resume consent:true` starts a **new** epoch | Resume without re-consent; old epoch reused |
| Written stop | `stop listening` yields authoritative inactive status (local mode) | Spoken backup prose treated as authority |
| Leave | `/voice leave`: no voice presence, no UDP, no later MLX-Audio speech requests | Ghost presence / socket remains |
| Verify | `/voice verify report` = `observed: 8/8` **plus** human audible + consent attestation | Counters only, or verifier used as proof of consent |
| Other guild / DM Inspect | Voice state `off` | Leak of `active`/`paused` |
| Fail closed | If consent or audible witness is unavailable: leave immediately and mark voice **externally pending** | Substituting unit tests or 2026-08-20 history |

Inspect legal values only: `off`, `presence`, `awaiting-consent`, `active`, `paused`.

### What Abbey can automate vs what requires humans in VC

**Abbey can (sidecars + LLM env are live; human consent is not):**

- Reject join/resume unless `consent:true`, Manage Server, caller present in the locked channel, bot permissions, and local-speech health all pass.
- Post the public local-processing disclosure, then open the software media gate.
- Whisper STT → canonical read-only Abbey reply → Kokoro TTS on loopback.
- Truncate playback on barge-in and bump the aggregate counter.
- Close the epoch on membership / unattested SSRC, disconnect `Decode`, stop STT/TTS, require resume.
- Treat written `stop listening` / first-person consent withdrawal as authority (local mode).
- Tear down on `/voice leave`.
- Keep content-free `/voice verify` counters in process memory (cleared on restart). **While armed, conversation commits are disabled** — the verifier is not a spoken-quality test.

**Humans in the locked VC must still:**

- Be physically present (Donald for join/resume; every participant for consent).
- Give **fresh** unanimous consent each epoch.
- Run the slash commands above (Abbey will not self-join conversationally; `ABBEY_VOICE_AUTOJOIN` is presence-only and currently unset).
- Speak the wake name and the barge-in utterance.
- Cause the membership change (join/leave).
- Witness that playback was actually heard and that barge-in actually cut it.
- Write `stop listening`.
- Confirm `/voice verify report` against what they heard. The 8/8 counter is not proof of consent or audibility.

**Do not automate:** consent, audible witness, membership-change as a fake event, or using MLX access logs as live-voice evidence.

---

## Gap 2 — MLX acceleration qualification

Contract (`tasks/todo.md` / `tasks/goals.md` / README): before selecting MLX as the Mac primary, verify **exact** reasoning, tool-calling, and vision interfaces. Treat Apple `fm serve` as optional. **Do not claim MLX Gemma multimodal/tools or an installed service without evidence.**

Required semantic smokes (`deploy/smoke-mlx-vlm.py`, run by `deploy/install-mlx-vlm-launchd.sh` **before** publishing `127.0.0.1:8282`):

| Probe | Exact pass marker | Evidence today |
|---|---|---|
| Streamed text with terminal marker | reply exactly `MLX_READY` and stream `[DONE]` | **PASS on staged ephemeral** (after #50 null `tool_calls` skip). **NO** published `:8282`. |
| Forced tool call + exact arguments | one streamed `probe_status` with `{"marker":"ready"}`, finish `tool_calls` | **PASS on staged ephemeral**. **NO** published `:8282`. |
| Tool-result continuation | final text exactly `TOOL_CONTINUATION_READY` | **FAIL** — 4-bit Gemma loops `<|channel>thought` into content until `finish_reason=length`. Encoding patches + `--enable-thinking` do not clear it. |
| Color/scene vision fixture | exactly `red square, blue circle` | **NO** |
| OCR fixture | exact embedded `OCR_TEXT` | **NO** |
| Offline restart from pinned snapshot `73bcf09092aa277861d5a191b989b666f7f32e8f` | installer offline bind + health after restart | **NO** published service |
| Point Abbey at MLX-VLM endpoint + **snapshot path** as model id (not `gemma4:12b`) | `ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:8282` and matching vision vars | **NO** — launchd + checkout still use Ollama host-only `:11434` / `gemma4:12b` (not the Abbey MLX-VLM sidecar) |
| End-to-end Abbey tools on 12B | allowlisted `remember_fact`, `lookup_reputation`, `recall`, `switch_persona`, `recent_messages` (+ Inspect live still pending) | **NO** live MLX evidence |

### Installed vs missing

| Artifact | State |
|---|---|
| Pinned Gemma 4 12B 4-bit snapshot files under `~/.local/share/abbey-bot/mlx-vlm/huggingface/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/73bcf09092aa277861d5a191b989b666f7f32e8f` | **present** (weights + tokenizer + configs). Presence of files is **not** interface qualification. |
| `run-mlx-vlm` / `mlx-vlm-venv` / `current-venv` | **missing** |
| Launchd `com.donaldfilimon.abbey-mlx-vlm` | **not loaded** |
| `:8282` | **not listening** |
| Leftover staging dirs `~/.local/libexec/abbey-bot/.mlx-vlm-venv.new.*` (four) | leftover install attempts; not a live service |
| `~/Library/Logs/abbey-bot/mlx-vlm-preflight.log` | 2361 bytes, 06:38 ET: model loaded on temp port, `/health` 200, then streamed chat **failed** `stream_closed_before_completion` and the process shut down |
| MLX-Audio install (`whisper-large-v3-turbo-asr-fp16`, `Kokoro-82M-bf16`, `af_heart`, `run-mlx-audio`, launchd) | **installed and running** — `com.donaldfilimon.abbey-mlx-audio` PID 21413; `:8181` listening. Live `GET /health` 200; `GET /v1/models` lists both models; `GET /v1/audio/voices?model=mlx-community/Kokoro-82M-bf16` includes `af_heart`. Installer TTS/STT smoke already passed; live human 8/8 is still pending. |
| Provider capability manifest | **none** under `~/.config/abbey-bot` or `~/.local/share/abbey-bot` |
| Homebrew `mlx-lm` on `:8080` | running, **unqualified** for Abbey tools/vision; empty `/v1/models` body |
| Ollama `gemma4:12b` and `gemma4:12b-mlx` | tags **present** on `:11434`. This is the portable OpenAI-compatible seam / Ollama runtime, **not** the Abbey MLX-VLM sidecar and **not** MLX tool/vision evidence |
| FM self-test (historical 2026-08-21) | `text`/`structured_output`/`tools` pass; `vision`/`ocr` **fail closed**. Not this report’s MLX claim; do not advertise FM vision/OCR |

**Fail closed:** MLX is **not** selected as the Mac primary. Tool *calls* on this 4-bit snapshot are not enough — tool-*result* continuation still loops thought-channel tokens, so `install-mlx-vlm-launchd.sh` must not publish `:8282`. Do not point `ABBEY_BOT_LLM_ENDPOINT` at MLX-VLM. Ollama `:11434` remains the reasoner until a later checkpoint passes `TOOL_CONTINUATION_READY` on that exact snapshot together with the other semantic probes.

---

## Operator path when the sidecar is down or recovering

Sidecar is live as of ~15:56 ET. If it dies or is still loading, do **not** start the 8/8 human run:

1. `/voice status` in MLAI Community. Expect mode `local`, sidecar listening or a 2s down/timeout line, and loopback LLM named as configured or missing.
2. If LLM is missing: it is missing from `~/.config/abbey-bot/env` (not checkout `.env`). `deploy/check-launchd-env.sh` / `deploy/install-launchd.sh` refuse a voice destination without `ABBEY_BOT_LLM_ENDPOINT`.
3. If `:8181` is down: `deploy/install-mlx-audio-launchd.sh` (setuptools 83; webrtcvad patched via `importlib.metadata`). Operator readiness: `GET /` or `GET /v1/models` (**not** `/health` for stock mlx-audio 0.5.0). Log: `~/Library/Logs/abbey-bot/mlx-audio.log`.
4. `/voice join consent:true` fails closed immediately on connection-refused (no 10-minute hang for a missing LLM or a down TCP port). A sidecar that is up but still loading Whisper/Kokoro may still take up to 10 minutes; `/voice status` is the probe, not a second join.
5. If the sidecar dies mid-session, capture stops (failed-safe). Resume only after `/voice status` shows the sidecar listening **and** fresh consent.

## Operator runbook after blockers are cleared

Only after MLX-Audio is serving on :8181 (`GET /` or `GET /v1/models`; do not require `/health`), MLX-VLM smokes pass including exact `TOOL_CONTINUATION_READY` before any `:8282` publish (if that is the chosen reasoner), and the **launchd** env contains both voice IDs and a loopback LLM endpoint:

1. Restart only via the atomic installer / launchd path; do not mix checkout `.env` with `~/.config/abbey-bot/env` by hand in a way that drops voice or LLM.
2. `ABBEY_GUILD_ID=1275617641620443146` is already in the launchd env; `/voice` is guild-scoped. Keep `ABBEY_BOT_LLM_ENDPOINT` host-only (`http://127.0.0.1:11434`); do not add `/v1`.
3. Execute Gap 1 steps 0–11 in the locked VC with consenting humans.
4. Keep Guild A / Guild B isolation, `/see` `/ocr` live, and seven-tool live in their own protocol layers (`docs/live-test-protocol.md`). They are adjacent, not this gap.

---

## Explicit non-claims

- This document does **not** start installs, rewrites, or a live voice session.
- Homebrew `mlx-lm` ≠ Abbey `com.donaldfilimon.abbey-mlx-vlm`.
- Ollama `gemma4:12b-mlx` ≠ qualified MLX-VLM tools/vision.
- Snapshot weights on disk ≠ a passing smoke.
- 2026-08-20 `/voice status` / leave observations ≠ current 8/8.
- Managed Abbey being “connected” ≠ consented capture. Generation backend is configured; live `/voice` 8/8 is still human-gated on Donald in Office Hours VC.
