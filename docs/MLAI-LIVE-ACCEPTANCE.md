# MLAI live acceptance — remaining parity gaps

> **Update 2026-09-03 ~15:34 ET (post-checklist):** Launchd Abbey now has `ABBEY_BOT_LLM_*` + `ABBEY_VISION_*` (Ollama `http://127.0.0.1:11434/v1` / `gemma4:12b`) — generation backend **configured**. `ABBEY_VOICE_LOCAL_ENDPOINT` set. MLX-Audio **install in progress** (HF snapshot fetch); `:8181` still down until that finishes and `com.donaldfilimon.abbey-mlx-audio` loads. MLX-VLM still not loaded. Human 8/8 voice acceptance still required in Office Hours.


**Date:** 2026-09-03 (America/New_York)  
**Checkout:** `/Users/donaldfilimon/dev/active/abbey-bot`  
**Git:** `main` @ `a0d5a669f085a3111f48148a398aa7974a0be8cc` (ahead of `origin/main` by 4; dirty `AGENTS.md` / `tasks/todo.md`)  
**Scope:** two remaining gaps only — (1) live voice acceptance, (2) MLX acceleration qualification. Fail closed where evidence is missing. Historical 2026-08-20/22 voice notes do **not** qualify this process, this binary, or this config.

This file is an operator evidence checklist, not proof that the run happened. Do not treat source tests, provider logs, leftover venvs, Homebrew `mlx-lm`, Ollama `gemma4:12b-mlx`, or prior consent as substitutes.

---

## Current config state

Presence / booleans / ID match only. **No token or secret values.**

### Two env files (they are not the same)

| Key | Checkout `.env` (mode 0600, 8 keys) | Launchd `~/.config/abbey-bot/env` (mode 0600, 4 keys; this is what PID 14101 loads) |
|---|---|---|
| `DISCORD_TOKEN` | present (secret withheld) | present (secret withheld) |
| `ABBEY_GUILD_ID` | present, numeric, **MATCH** MLAI `1275617641620443146` | **MISSING** (managed process registered **global** commands) |
| `ABBEY_BOT_LLM_ENDPOINT` | present, loopback HTTP **:11434** (Ollama) | **MISSING** |
| `ABBEY_BOT_LLM_MODEL` | present = `gemma4:12b` | **MISSING** |
| `ABBEY_VISION_ENDPOINT` | present, loopback HTTP **:11434/v1** | **MISSING** |
| `ABBEY_VISION_MODEL` | present = `gemma4:12b` | **MISSING** |
| `ABBEY_VOICE_GUILD_ID` | **MISSING** | present, numeric, **MATCH** MLAI `1275617641620443146` |
| `ABBEY_VOICE_CHANNEL_ID` | **MISSING** | present, 19-digit numeric (value not copied here) |
| `ABBEY_VOICE_MODE` | **MISSING** | present = `local` |
| `ABBEY_VOICE_AUTOJOIN` | missing (default 0) | missing (default 0) |
| `ABBEY_VOICE_LOCAL_ENDPOINT` | missing (code default `http://127.0.0.1:8181`) | missing (same default) |
| `ABBEY_VOICE_LOCAL_STT_MODEL` / `_TTS_MODEL` / `_TTS_VOICE` / `_LANGUAGE` | missing | missing |
| `ABBEY_VOICE_WAKE_WORD_REQUIRED` | missing (example default `1`) | missing |
| `ABBEY_FM_MODE` / `_ENDPOINT` / `_CLI` / `_FALLBACK` / `_CAPABILITY_MANIFEST` | missing (FM stays off) | missing |
| `ABBEY_PROVIDER_MLX_*` / `ABBEY_PROVIDER_MANIFEST` | missing | missing |
| `OPENAI_API_KEY` / `ABBEY_VOICE_MODE=openai` | missing / not selected | missing / not selected |
| Telegram / Slack tokens | missing | missing |

Checkout extras (names only): `DISCORD_BOT_TOKEN` (secret withheld), `RUST_LOG`.

### Running process vs sockets (2026-09-03 ~15:31 ET)

| Item | Observed |
|---|---|
| `com.donaldfilimon.abbey-bot` | **loaded, running**, PID 14101, last exit 0, plist `~/Library/LaunchAgents/com.donaldfilimon.abbey-bot.plist` |
| `com.donaldfilimon.abbey-mlx-audio` | **not loaded** — `launchctl print gui/501/…` = service not found; no LaunchAgents plist |
| `com.donaldfilimon.abbey-mlx-vlm` | **not loaded** — same; no LaunchAgents plist |
| TCP `127.0.0.1:8181` (Abbey MLX-Audio) | **not listening** |
| TCP `127.0.0.1:8282` (Abbey MLX-VLM) | **not listening** |
| TCP `127.0.0.1:11434` | Ollama listening (checkout LLM/vision target) |
| TCP `127.0.0.1:8080` | Homebrew `homebrew.mxcl.mlx-lm` PID 1069 listening; `GET /v1/models` returned HTTP 200 with **empty body** — **not** Abbey's pinned VLM sidecar |
| Managed log | `WARN no generation backend — Abbey answers honestly that she cannot` (twice: 15:28 ET and 15:31 ET) |
| Command registration | managed log: `registered global commands — propagation can take up to an hour` because launchd env lacks `ABBEY_GUILD_ID` |
| Gateway | connected as `Abbey`; at least one MLAI guild message handled (`guild=Some(1275617641620443146)`) |

Voice destination vs MLAI: **guild ID matches**. Channel is the single 19-digit ID in the launchd env. Historical 2026-08-20 notes called a prior presence target **Engineering**; this checklist does not re-publish the current snowflake. Confirm the locked channel with `/voice status` inside MLAI Community before joining.

---

## Gap 1 — live voice acceptance

Required lifecycle (todo + `docs/live-test-protocol.md` §4):  
**join `consent:true` → wake → barge-in → membership-close → resume → leave**  
plus owner/admin `/voice verify start` / `report` with `observed: 8/8`, written `stop listening`, and a human audible witness. Source tests and historical consent are **not** substitutes.

### Preconditions (currently failing)

Do **not** start the human run until these are true. Otherwise join will fail closed (local-speech health up to 600s) or produce no spoken reply.

1. Abbey MLX-Audio is installed, launchd-loaded, and healthy on `127.0.0.1:8181` (Whisper + Kokoro + `af_heart`). **Not true today.**
2. A loopback reasoning backend is configured **in the env the running process actually loads**. Launchd env has **no** `ABBEY_BOT_LLM_ENDPOINT`. **Not true today.**
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

**Abbey can (once sidecars + LLM env are actually live):**

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
| Streamed text with terminal marker | reply exactly `MLX_READY` and stream `[DONE]` | **NO** — no live :8282; 06:38 ET preflight on ephemeral `:53952` logged `stream_closed_before_completion` after 2 tokens |
| Forced tool call + exact arguments | one streamed `probe_status` with `{"marker":"ready"}`, finish `tool_calls` | **NO** |
| Tool-result continuation | final text exactly `TOOL_CONTINUATION_READY` | **NO** |
| Color/scene vision fixture | exactly `red square, blue circle` | **NO** |
| OCR fixture | exact embedded `OCR_TEXT` | **NO** |
| Offline restart from pinned snapshot `73bcf09092aa277861d5a191b989b666f7f32e8f` | installer offline bind + health after restart | **NO** published service |
| Point Abbey at MLX-VLM endpoint + **snapshot path** as model id (not `gemma4:12b`) | `ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:8282` and matching vision vars | **NO** — checkout uses Ollama `:11434` / `gemma4:12b`; launchd has no LLM at all |
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
| MLX-Audio install (`whisper-large-v3-turbo-asr-fp16`, `Kokoro-82M-bf16`, `af_heart`, `run-mlx-audio`, launchd) | **not installed** — share dir exists with empty `rollback/` only; no Hugging Face cache; `:8181` down |
| Provider capability manifest | **none** under `~/.config/abbey-bot` or `~/.local/share/abbey-bot` |
| Homebrew `mlx-lm` on `:8080` | running, **unqualified** for Abbey tools/vision; empty `/v1/models` body |
| Ollama `gemma4:12b` and `gemma4:12b-mlx` | tags **present** on `:11434`. This is the portable OpenAI-compatible seam / Ollama runtime, **not** the Abbey MLX-VLM sidecar and **not** MLX tool/vision evidence |
| FM self-test (historical 2026-08-21) | `text`/`structured_output`/`tools` pass; `vision`/`ocr` **fail closed**. Not this report’s MLX claim; do not advertise FM vision/OCR |

**Fail closed:** MLX is **not** selected as the Mac primary. Do not announce multimodal Gemma, tool-calling, or `/see`/`/ocr` via MLX until `install-mlx-vlm-launchd.sh` publishes a healthy :8282 **and** the six semantic probes above pass on that exact snapshot.

---

## Operator runbook after blockers are cleared

Only after MLX-Audio is healthy on :8181, MLX-VLM smokes pass on :8282 (if that is the chosen reasoner), and the **launchd** env contains both voice IDs and a loopback LLM endpoint:

1. Restart only via the atomic installer / launchd path; do not mix checkout `.env` with `~/.config/abbey-bot/env` by hand in a way that drops voice or LLM.
2. Prefer setting `ABBEY_GUILD_ID=1275617641620443146` in the launchd env so `/voice` re-registers instantly in MLAI instead of waiting on global propagation.
3. Execute Gap 1 steps 0–11 in the locked VC with consenting humans.
4. Keep Guild A / Guild B isolation, `/see` `/ocr` live, and seven-tool live in their own protocol layers (`docs/live-test-protocol.md`). They are adjacent, not this gap.

---

## Explicit non-claims

- This document does **not** start installs, rewrites, or a live voice session.
- Homebrew `mlx-lm` ≠ Abbey `com.donaldfilimon.abbey-mlx-vlm`.
- Ollama `gemma4:12b-mlx` ≠ qualified MLX-VLM tools/vision.
- Snapshot weights on disk ≠ a passing smoke.
- 2026-08-20 `/voice status` / leave observations ≠ current 8/8.
- Managed Abbey being “connected” ≠ consented capture (and today it has **no generation backend**).
