> **Update 2026-09-07 ~22:24 ET — `/admin show` classic page select live:** Merged [#99](https://github.com/donaldfilimon/abbey-bot/pull/99) (`0c69942`) + clippy fix [#101](https://github.com/donaldfilimon/abbey-bot/pull/101) (`dd102dc`). `/admin show` attaches a classic String Select (`page-select`) into Overview/Conversation/Learning/Operations; dashboard nav uses the same select; action rows remain classic buttons. Fail-closed owner/guild/expiry + Manage Server. Entry Point `launch` preserved; `/voice` `consent:true` slash gate unchanged; Components V2 still crate-blocked. `deploy/install-launchd.sh` redeployed; launchd PID **64772**, Discord ready; binary SHA-256 `67904c61d86f5cc76d4b22cca757d69d118dc2b78ee5582d01b2415c1db9a56a` (matches `target/release`). `#bot-ops` note `1546707652153843732`. **Still Donald:** Portal Activity URL map (P0); OAuth secret host (P2). No tokens echoed.

> **Update 2026-09-07 ~20:46 ET — P3 forum helpers live:** Merged [#97](https://github.com/donaldfilimon/abbey-bot/pull/97) (`20f350d`). `deploy/install-launchd.sh` redeployed; launchd PID **6711**, Discord ready, **24** guild commands (`/forum` with `draft`/`post`/`perms`). Binary SHA-256 `66b09473a17eb58b887a8e25eca5e93b68c01a05378e99c849f02c7298131492`. `#bot-ops` note `1546683311672524841`. Dependabot getrandom #96 already on main. **Still Donald:** Portal Activity URL map (P0); user-install Installation Contexts; OAuth secret host (P2). No tokens echoed.

> **Update 2026-09-07 ~19:45 ET — MLAI IA refresh + Abbey redeploy (evolve, not wipe):** REST snapshot taken before mutate. `abbey-bot --server-plan blueprints/mlai-community.toml --guild 1275617641620443146 --stage reveal --apply` applied seven changes: Owner hoist+#05070d; Team/Moderator/Personas brand colours; Console/Member clear colour; `#abi-runtime` moved under **THE STACK**. Additive and all category overwrite stages were already clean. Manual fix: `#help` Member overwrite dropped Manage Threads (kept View/Send/Create Public Threads/Send in Threads). Skipped deleting two `new role` stubs (holders>0). Private staff/product gates still deny `@everyone` View. Welcome/rules pins left (content current). `#bot-ops` status message `1546667804558889041`. Checkout `main` `3aafc7b` already matched `origin/main`; `deploy/install-launchd.sh` redeployed live binary SHA-256 **`792aed2c76b3651f2a41cfff779c329ab768cf6a950974766e30c59c4f3c2494`** (matches `target/release`); launchd PID **53978**, Discord ready, 23 guild commands. **Still Donald:** Developer Portal Activity URL map (P0) — see `docs/activities.md`. No channel wipe, no mass Member grants, no Portal clicks invented.

> **Source-only update 2026-09-06 — strict source gate complete, no live transition:**
> Tested source `ff5d594` passed the fresh WDBX-required strict gate using an
> external build target: **1,183 Rust tests passed, zero failed, four live tests
> intentionally ignored**, warnings-denied all-target Clippy, 26 offline installer
> tests and the locked Rust release build and offline Swift release build. Deployment/Python, privacy, 81-artifact
> contracts, required WDBX, TLS, module-size and Swift checks passed. The RustSec
> policy retains four accepted vulnerabilities and three unmaintained warnings;
> it is not a clean audit. Canonical integration, push and exact-head hosted CI
> remain pending, with final delivery proof reserved for canonical
> `.superpowers/completion-20260906/delivery.json`. This pass did not install or
> restart a service, qualify a provider, change Discord settings, activate voice
> or observe audible output. The dated live evidence below remains historical
> for its recorded artifact and configuration; it does not qualify the new
> managed contract. Follow [the live protocol](live-test-protocol.md) before
> promoting any layer.

> **Update 2026-09-04 ~19:07 ET — community redesign saved and blueprint repair installed:** Native Discord onboarding now uses eight defaults (six chattable), three participation tasks, a rewritten Abbey welcome, and public interest mappings. Help is a default; ABI/WDBX are optional. The Community Guidelines resource now contains the existing screening standards and useful channel links; Help posting guidance was improved. A full client reload and roleless newcomer preview verified saved content and navigation. Two archives remain outside onboarding; custom Guide banner requires Level 2 (current Level 1). No channel deletion, private-channel opening, or member-role grant occurred.
>
> Source **`71468e1`** repairs read-only blueprint posting/thread denies and misleading role inheritance wording. `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` passed **870 tests, 0 failed, 2 intentional ignores**, warnings-denied clippy, locked release, deployment checks, and WDBX parity. Managed launchd installation succeeded; installed and tested SHA-256 both **`da7fbe97a1f96ba740b550c9ce424b0b51b60c099f45426be33db368709b5417`**. PID **24593**, one run/no exits; gateway connected at **19:06:02 ET**. Native `/server community` at **19:07 ET** returned the corrected blueprint ephemerally. Rollback: `rollback/abbey/20260904T230557Z.FnLdiU`. Existing automatic voice presence returned muted/deafened; logs confirm decoding/transmission disabled. No conversational voice activation or audible test occurred. Source remains local/unpushed; this is not hosted CI evidence. README adds a reviewed Apollo Rust coding-skill reference; runtime remains the seven compiled tools, with no SKILL.md loader added.

> **Update 2026-09-04 ~08:52 ET — queue failure repair installed:** Source **`106c97d`** preserves earlier in-flight and queued recognition when a sequence gap damages the current utterance. Every STT failure, including empty or malformed responses, and unexpected task failure now closes media through one failure transition; completed speech cannot be silently dropped while capture remains open. Shared activity reporting keeps pending recognition visible after speech, playback completion, and recoverable generation failures. Four regressions reproduced the defects before repair. The final `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` passed **813 tests, 0 failed, 2 intentional ignores**, warnings-denied clippy, WDBX parity, deployment checks, and locked release build. Independent bounded review found no remaining production issue; its two test synchronization findings were fixed before this gate.
>
> The managed installer exited successfully. Installed SHA-256 **`3e2fe27bcc4a9c41a22a38445dfda96a1cf3c1159cca6050ed8417dccb345b7c`** matches the tested release; launchd PID **34682** is stable (one run, no exits), with gateway readiness at **08:51:33 ET**. The runtime environment is byte-for-byte unchanged. Rollback is retained under `rollback/abbey/20260904T125128Z.gMS4XD`. The 08:52 ET REST snapshot shows Abbey in Office Hours, self-muted and self-deafened, with server mute/deaf/suppression false. No activation or human-audible test was performed after this restart. Hosted checks must be read on the final PR head, and current participant verification/application status remains separate from these transport flags.

> **Update 2026-09-04 ~08:35 ET — final scoped repair installed:** The managed installer completed successfully for source **`3fdd70f`**, including ready-output queue independence, actor-owned capture-time follow-up handling, the bounded recognition failure path, and the earlier review corrections. Installed SHA-256 **`9388c49bb5049becd61ab8da78a942b1f8128b8e728745d844b1ff692b371744`** matches the tested release. Launchd PID **22029** is running (one run, no exits); the gateway connected at **08:33:55 ET**. The service environment is byte-for-byte unchanged, and rollback is retained under `rollback/abbey/20260904T123350Z.i6HdMO`. The final local WDBX-required gate passed **809 tests, 0 failed, 2 intentional ignores**, clippy, and locked release build.
>
> No activation command, participant attestation, native screen capture, or live audio test was performed after this restart. The 08:35 ET REST snapshot places Abbey in Office Hours with all mute/deaf/self-mute/self-deaf/suppression flags false; these transport flags do not establish the application media-gate phase. The Codex voice session has ended, and its end says nothing about current Discord membership. Human-audible acceptance of this installed artifact remains a later step through the existing participant-verification and consent policy. No automatic consent or activation loop was added.

> **Update 2026-09-04 ~08:22 ET — live reply reached output; ready-reply delay reproduced:** On installed `b161f5b`/PID **99822**, turn 136 was accepted at **08:11:06.296 ET**. Generation queue wait was effectively zero, generation took **16.939 seconds**, and synthesis took **2.511 seconds**. Older ordinary utterances continued finishing transcription until **08:11:39.047**, then playback started **1.5 ms** later: **13.301 seconds** after synthesis and **32.752 seconds** after acceptance. Speech stopped the track after **0.932 seconds**. No new wake prompt or consent request was needed. Existing logs cannot identify whether that interruption came from the requester, another participant, or uncertain/overlapping input.
>
> The native **current-stream** snapshot at about **08:15:30 ET** showed Abbey SSRC **3143**, **66 successful decryptions / 1 failure**, **77 received packets**, and **97 normally decoded frames**. These cumulative counters establish current receive/decrypt/decode activity, but cannot map individual packets to turn 136 or prove human hearing. The earlier SSRC 3131 snapshot is separate historical evidence. No transport, encryption, client route, or consent-policy change was made.
>
> The source correction now lets ready output start during actual silence while older recognition continues. Explicit queued replacement and authorized withdrawal still stop that output; current speech and consent guards remain active. Capture-time follow-up bounds keep speech from before playback from being reclassified as an implicit follow-up afterward. Content-free logs add recognition duration, ready-output wait, and interruption speaker relationship/overlap metadata. Wake-state transitions are actor-owned, preventing concurrent recognition from handing an old playback window to a pending new question. Recognition has a ten-second deadline from capture, including queue wait; expiry immediately revokes the media epoch and then stops the call/playback, so a later withdrawal cannot remain behind the general speech client timeout. A delayed 13.3-second synthetic response exercises the observed backlog timing class: the expected outcome at the ten-second bound is a visible Failed state with an explicit recovery command, no automatic reactivation, and no late-response reopening. **23 focused voice tests passed** and the final `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` passed **809 tests, 0 failed, 2 intentional ignores**, warnings-denied clippy, and locked release build; the original ready-output stall and stale-follow-up failures were reproduced before their fixes. This source correction is not yet installed; human-audible acceptance remains unresolved.

> **Update 2026-09-04 ~08:03 ET — current consent confirmed; application active:** Donald explicitly attested that all three currently present humans consent, including the newly observed participant, and confirmed that he is listening through AirPods. A fresh native `/voice status` check showed **Listening**, media gate **OPEN**, pending start **NO**, consent epoch **1**, **3 participants attested**, and session epoch **4**. The helper submitted only the read-only status command; no join/resume/reset was needed. There is no remaining consent or activation blocker for this attested participant set. The service remains the tested `b161f5b` build/PID **99822**; subsequent source review corrections are not installed in this active call. The source follow-up keeps ephemeral context-menu preparation from mutating shared persona/session state and keeps queued transcription visible as Thinking. Its `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` passed **802 tests, 0 failed, 2 intentional ignores**, warnings-denied clippy, and the locked release build; the focused regressions first reproduced both defects.
>
> The deployed log records an addressed operational reply accepted at **08:02:36.085 ET**, playback started at **08:02:37.363**, and confirmed barge-in stop at **08:02:37.904** (turn 32, about **0.54 seconds** of playback). This fixed operational response has no model generation-stage timing and did not complete naturally. Human hearing and a live model-response latency result remain unconfirmed. Preserve the existing consent policy; current explicit attestation does not grant consent to future participants merely because they belong to the server.

> **Update 2026-09-04 ~08:02 ET — deployed voice repair; live activation withheld:** Commit `b161f5b` is installed through `deploy/install-launchd.sh`, including pending-reply preservation, first-frame VAD protection, configured wake-name hints, and the local spoken-response latency fix. Installed SHA-256 **`a88e68c14a18f63f72438e7097c807625126758a62a4918b30c5a1a4c6b696b6`** matches the tested release. Launchd PID **99822** is running (one run, no exits), gateway connected as Abbey at **07:56:30 ET**, the service environment is byte-for-byte unchanged, and the installer retained rollback. The full `./check.sh` passed **798 tests, 0 failed, 2 intentional ignores**, including warnings-denied clippy, deploy/privacy/contracts/WDBX checks, and locked release build.
>
> **Latency evidence is generation-only:** On the same local Ollama `gemma4:12b` model and synthetic prompt, disabling optional thinking produced the reply in **1.475 seconds**, versus **14.614 seconds** with the default (first content **0.763 vs 13.695 seconds**). The request change is limited to read-only spoken replies on the measured loopback HTTP port 11434 / host-only base URL / exact model; persona preparation and grounding remain intact. Text/tool requests and other backends retain their existing request bodies. New content-free timing logs separate generation-queue wait, generation, and synthesis. No post-restart addressed-turn or stage-completion events were observed by the 08:00 ET check; this is not yet a live end-to-end latency result.
>
> **Earlier client evidence narrows the silence investigation:** Before this restart, native Discord's Abbey inbound stream reported **129 successful decryptions, zero decrypt failures, and 189 normally decoded frames**. This rules out a blanket DAVE send/decrypt failure for that observed stream, but does not establish human hearing. The inspected client had Abbey volume 100%, no local/server mute, Discord output Default (AirPods Pro) at 100%, and connected/selected AirPods Pro with system volume 69% and mute off; no route mismatch was demonstrated. Donald's earlier report that he heard no reply remains unresolved. No encryption change or client output change was made.
>
> **Consent/activation boundary:** The native helper observed a new participant who was outside the previously attested set and withheld `/voice join` and `/voice resume`. Parent task is obtaining that participant's consent. Runtime application phase has not been inspected through `/voice status`. Immediately after deployment, REST showed muted/self-deafened autojoin presence; the 08:00 ET read-only REST snapshot instead showed Abbey still in Office Hours with all mute/deaf/self-mute/self-deaf/suppression flags false. These transport flags do not prove application consent activation. Do not activate from prior membership consent; human-audible acceptance is paused for the newly observed participant decision.

> **Update 2026-09-04 ~07:42 ET — voice silence investigation:** The cancellation fix in `0c16a7e` is deployed through the managed installer; launchd PID **57997**, installed SHA-256 `94a571aee732baa31fa74586eff5f58cb65b53398e0e777063e10d9a152efea3` matches the tested release. The service environment is unchanged and rollback is retained. Its full local gate passed **795 tests, 0 failed, 2 intentional ignores**; its synthetic local audition had **100% word recall**. Ordinary speech now preserves pending recognition and replies. A reviewed follow-up covers first-frame VAD candidates and configured wake-name hints; that follow-up is not installed in this active call.
>
> **Human-audible acceptance is still failing:** Donald explicitly heard no audio during the operational reply logged at **07:35:19–07:35:23 ET** (epoch 6, turn 29). A later model reply **naturally completed at 07:40:17 ET** (epoch 8, turn 19); process UDP output increased from **242 to 41,200 bytes** across it, but human hearing of that reply is not yet confirmed. Discord REST reports Abbey in Office Hours with mute, deaf, self-mute, self-deaf, and suppression all false. Playback lifecycle and outgoing packets do not prove that Discord clients rendered audible audio.
>
> Participants explicitly consented to the active session; preserve it during diagnosis. No additional recording, provider switch, or encryption downgrade was introduced. [Songbird issue #310](https://github.com/serenity-rs/songbird/issues/310) reports per-join DAVE send failures with the same Songbird 0.6.0 / davey 0.1.4 versions; this is an investigation lead, not a confirmed cause here. Client-local mute/volume/output and encrypted delivery remain under investigation. The selected Gemma model also remains slow: the latest synthetic full chain took **117.7 seconds**.

> **Update 2026-09-03 ~21:50 ET:** Live Abbey still up — launchd PID **26416**, binary SHA `931f01860c6b48c7…` matches `target/release` and `main` `15c0f15` (#62 Activity shell). Activity Pages live at `https://donaldfilimon.github.io/abbey-bot/activity/` (HTTP 200). **Discord Portal URL map still Donald** (login unfinished). Sidecars: MLX-Audio `:8181` 200; Ollama `:11434` 200; MLX-VLM `:8282` unpublished. `quesar.cloud` NS still Hostinger parking (`byte`/`pixel.dns-parking.com`). Dependabot config added with ignores for serenity major / rustls-webpki (poise 0.6.2 break); alerts #1–#4 remain open until coordinated crates.io bump.

> **Update 2026-09-03 ~18:55 ET:** Minimal Activity client lives at `activity/` (GitHub Pages `/activity/` after merge). Portal URL map `/` → `donaldfilimon.github.io/abbey-bot/activity` is still a Donald click. Voice overwrites already had Stream + Use Activities; no PUT. Bots still cannot Go Live. Consent/wake-word unchanged.
>
> **Update 2026-09-03 ~18:48 ET:** Office Hours overwrites grant Stream + Use Embedded Activities for Member and Abbey roles. Voice join also requires those bits (`required_voice_permissions`). See `docs/activities.md` — Activity/rocket launch is the path; bots cannot classic Go Live.
>
> **Update 2026-09-03 ~18:30 ET:** Conversational voice waits for the shared generation slot (up to 180s) instead of failing busy against concurrent text; `ABBEY_VOICE_AUTOJOIN` is muted/self-deafened presence only (conversation still needs `/voice join consent:true`).
>
> **Update 2026-09-03 ~18:03 ET:** `cargo build --release --locked` on `main` `c5f95d1`, installed to `~/.local/libexec/abbey-bot/abbey-bot`. Binary SHA `a61c6a53c56930cb…`, launchd PID **11541** (runs=15, last exit=0), `connected user=Abbey`. Sidecars unchanged: MLX-Audio `:8181` 200; Ollama `:11434` 200 host-only LLM; MLX-VLM `:8282` unpublished.
>
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

Activities / Entry Point (rocket, `/launch`) and Stream/Use Embedded Activities fail-closed join bits: see `docs/activities.md`.

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
plus owner/admin `/voice verify start` / `report` with `observed: 8/8`, an Abbey mention with written `stop listening`, and a human audible witness. Source tests and historical consent are **not** substitutes.

### Preconditions (MLX-Audio sidecar is up; human 8/8 and in-VC consent still failing)

Do **not** start the human run until these are true. Otherwise join will fail closed (local-speech health up to 600s) or produce no spoken reply.

1. Abbey MLX-Audio is installed, launchd-loaded, and serving on `127.0.0.1:8181` (Whisper + Kokoro + `af_heart`). **True today.** Operator readiness: `GET /` or `GET /v1/models` (**NOT** `/health` — mlx-audio 0.5.0 404). Human 8/8 in Office Hours is still required.
2. A loopback reasoning backend is configured **in the env the running process actually loads**. Launchd `ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434` (host-only — `dialect.rs` appends `/v1/chat/completions`); vision keeps `http://127.0.0.1:11434/v1`. **True today.**
3. Donald (Manage Server for join/resume/status; owner/Administrator for verify) is **physically in** the launchd-locked MLAI voice channel. Remote activation is refused.
4. Fresh unanimous consent from **everyone currently present**. Silence, history, and one person speaking for another do not count.
5. Abbey has View Channel, Send Messages, Connect, Speak, Stream, and Use Embedded Activities in that VC and is not server-muted/deafened/suppressed.

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
| 9 | Human in VC chat | Mention Abbey and write `stop listening` (authoritative in local mode) | Locked VC text |
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
| Written stop | Mentioning Abbey and writing `stop listening` yields authoritative inactive status (local mode) | Spoken backup prose treated as authority |
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
- Treat an Abbey mention with written `stop listening` / first-person consent withdrawal as authority (local mode).
- Tear down on `/voice leave`.
- Keep content-free `/voice verify` counters in process memory (cleared on restart). **While armed, conversation commits are disabled** — the verifier is not a spoken-quality test.

**Humans in the locked VC must still:**

- Be physically present (Donald for join/resume; every participant for consent).
- Give **fresh** unanimous consent each epoch.
- Run the slash commands above (Abbey will not self-join conversationally; `ABBEY_VOICE_AUTOJOIN` is presence-only and currently unset).
- Speak the wake name and the barge-in utterance.
- Cause the membership change (join/leave).
- Witness that playback was actually heard and that barge-in actually cut it.
- Mention Abbey and write `stop listening`.
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

## 2026-09-29 read-only continuation checkpoint

The current source candidate is canonical `main` at
`fe1987c76947e015c3a54e1ca4a762663829ffb6` with uncommitted CI hardening and
provider qualification work. The strict local gate passed with 1,551 Rust tests,
five ignored operator/live tests, 16 offline Swift tests, required WDBX
conformance, warnings-denied Clippy, and locked release builds. This is source
acceptance for that working tree, not installed-artifact or live acceptance.

A fresh `python3 -I deploy/service-status.py` observation reports Abbey ready,
Discord ready, scheduler running, and completed persistence. Telegram and Slack
are disabled. The built binary SHA-256 is
`170694e4ade7db6f5e871776d611e8242b3c6f9c5480d705eaf9079782260d63`;
the installed binary SHA-256 is
`776c5b1ee91e9c192460804746b1b3f3159508b586b59f2814f62119fa5b87aa`.
They differ, so this candidate has not been shown to be the installed artifact.
No service, runner, production environment, or Discord state was changed.

The current FM plan remains incomplete: operator tooling is being implemented;
auto-deployment and the live FM switch are unverified. PCC refusal must remain
visible while a separately qualified system mode may serve. Human Discord and
voice acceptance, fresh participant consent, and platform/provider qualification
remain distinct from the source gate. Older dated observations above are
historical and do not override this checkpoint.

The exact-head GitHub `Gate (macOS)` for `fe1987c` is **failed**, not accepted:
[run 36583240823](https://github.com/donaldfilimon/abbey-bot/actions/runs/36583240823)
completed with the annotation "The self-hosted runner lost communication with
the server." Its prerequisite and cargo-audit steps passed, but there is no
completed gate receipt; GitHub log retrieval reported the job log missing.
The cause of the communication loss is unknown. Other successful check names
on that SHA do not substitute for this gate. No runner action or rerun was
performed in this continuation.

### 2026-09-29 FM operator tooling source checkpoint

Final `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0:1553 Rust tests passed,5 ignored; locked release completed. Candidate SHA256 `c56fadf6e15d42589d61c4afab0b3d9af96969cad8b6b4d5b684285a2a0123f9`. Independent operator-tooling review passed after two fix rounds. Token-free candidate identity wiring accepted a synthetic temporary dry-run manifest and refused stale tool-schema identity; this is not live provider qualification. No runner, launchd, production environment, or installed artifact change occurred. Task5 publication/exact-head CI and Tasks6/7 remain open.


### 2026-10-01 owner-directed profile, Activity and server organization

Current: Discord application description was updated and read back successfully
(HTTP 200). The managed bot was transactionally updated after `./check.sh`
exited 0: 1,556 Rust tests passed, five ignored; warnings-denied Clippy, Python
gates, offline Swift audio-tap checks and locked release build completed.
Installed and candidate SHA-256 both equal
`b2c8b6aab6832a4d5cd88978e1f8c560a5d3c73e2a8eb672e781c7b6d0668344`.
The owner environment and voice configuration were preserved. Discord's member
list visibly showed `Playing Bad Idea Court • professionally unserious` after
installation. Read-only status reported Discord ready, scheduler running and
last persistence complete; one bot process was observed.

The install attempt first refused a retained September 29 lock. No installer
process or open lock holder existed; its last candidate binary, plist and env
matched the healthy installed artifacts. The lock was preserved by rename as
`~/.local/share/abbey-bot/install.lock.retained-2026-10-01-court`, leaving
rollback material intact. The documented installer then completed successfully.

Partial: `activity/` now includes Bad Idea Court, an anonymous shared-vote party
game with ephemeral bounded rooms. Its Node multiplayer regression test passed;
two browser clients observed a shared 1–1 vote and hung-jury verdict. A managed
loopback host `com.donaldfilimon.abbey-court` serves the reviewed copy at
`http://127.0.0.1:8791/` (HTTP 200). The existing Activity Entry Point `launch`
(type 4, handler 2) was verified through REST. The new game is NOT published
in Discord: current GitHub Pages still serves the old shell, Pages cannot host
the shared API, no public HTTPS backend route was configured, and Portal mapping
was not changed or accepted. No OAuth secret was added, no Discord message was
sent by the operator tools, and no voice playback/capture acceptance was run.

Current: the owner-authorized MLAI redesign used three independent read-only
agent reviews and serialized REST writes. Snapshot inventory: 49 channels
(including 10 categories), 26 roles. Category order is START HERE, COMMONS,
AI LAB, PRODUCTS, BUILD LOG, VOICE, SOCIAL, SUPPORTER, STAFF, ARCHIVE.
THE STACK was renamed AI LAB; programming-chat and research moved into AI LAB;
off-topic and commands moved into SOCIAL. Twenty-six channel topics were
clarified, retaining product-specific qualification and private-access copy.
All category/child ordering and planned parents were read back and verified.
Discord rejected a bulk request with multiple parent changes (HTTP 400, code
40009); individual parent changes with `lock_permissions=false`, followed by
a position-only bulk reorder, succeeded.

Before/after checks preserved all 49 channel IDs, normalized overwrite sets,
role names/permissions/positions, bot role memberships, channel types, forum
metadata, voice settings, Community special-channel pointers and the exact
onboarding object (two prompts, six default channels). No channels, messages,
roles or members were deleted. No permission synchronization, membership change,
role reorder, commit, push or PR was performed. These configuration checks do
not claim a complete human member roster or human-witnessed conversational voice.
Private before/after snapshots, reviews, execution plan, source-gate log and
preview are retained under
`~/.local/share/abbey-bot/server-redesign-2026-10-01/`.

### 2026-10-01 continuation: onboarding and forums

Current, API readback: the existing Research option now includes research;
Apple Silicon and Site builder now include their respective channels. Prompt
and option IDs, titles, role selections, six default channels, enabled state,
and advanced mode are preserved. Discord reorders channel arrays; verification
compares channel membership rather than treating array order as policy.

Current, API readback: help retains its six tag IDs and adds optional Solved
and Unresolved tags; showcase has Demo, Tool, App, Library, Creative; research
has Paper, Experiment, Replication, Review, Dataset. Help and research use list
layout, showcase gallery, all use recent activity sorting and seven-day default
archive duration for future threads. Existing overwrites, flags, slowmode,
reactions, topic, parent, type and NSFW state were compared and preserved.
No thread messages, existing thread tag assignments or permissions were edited.

The precise permission review corrects an earlier proposal's interpretation:
17179869184 denies MANAGE_THREADS (bit 34), not CREATE_PUBLIC_THREADS (bit 35).
Forum creation requires SEND_MESSAGES; everyone and Member role profiles in
the snapshot can create and reply in all three forums. This is a permission
calculation, not a witnessed action by a human member.

Evidence and pre-change snapshots remain in the owner-private directory
`~/.local/share/abbey-bot/server-redesign-2026-10-01/`, including
`onboarding-links-before.json`, `onboarding-links-after.json`,
`forum-layouts-before.json`, `forum-layouts-after.json`, and
`forum-permission-review.md`. Bot service read-only status remains ready with
Discord ready, scheduler running, and completed persistence.

Partial: connected Sites hosting is available, but its publication workflow
requires a source commit and push, which remains subject to operator approval.
The Activity still runs on the local host; public hosting and the human-gated
Developer Portal URL mapping remain uncompleted.

### 2026-10-01 continuation: forum suggestions and Voice help

Current source and installed artifact: forum suggestions recognize the live
abbey-bot, quesar and AI & ML tags and punctuation-delimited keywords. Solved
and Unresolved are excluded from automatic suggestions; resolution remains an
explicit member choice. The forum permission gap-fill no longer adds
CREATE_PUBLIC_THREADS, which the forum creation endpoint ignores, and preserves
an existing deny for that unrelated bit. No live permission changes were made.

Voice help now explains server-only calls, individual saved consent, music's
separate consent boundary, status and participant/manager stopping controls.
The printed/read owner page fits at 1,996 characters, including every eligible
entry and the footer. Tests cover all eight voice capability combinations for
the owner; ordinary member help still hides operator diagnostics.

Verification: focused forum filter passed 9 tests (including two command forum
tests); catalog suite passed 19; full `./check.sh` exited 0 with 1,560 Rust tests
passed, zero failed and five intentional ignores, offline Swift audio checks,
warnings-denied Clippy and locked release build. Activity test passed 1/1.
The reviewer approved the final bounded diff after the new owner-size regression
caught and corrected overflow. These are source checks, not a human voice test.

Operator-authorized deployment: `./deploy/install-launchd.sh` exited 0 and
reported `installation: ready`. Installed and release SHA-256 both equal
`b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`.
Runtime source fingerprint (321 files) stayed fixed through the gate.
Read-only managed status after installation reports Discord ready, scheduler
running and completed persistence. The exact compiled help was rendered in the
offline test; no Discord test message or human-witnessed voice claim is made.


### 2026-10-01 continuation: Activity recovery and bounded engagement

Current local Activity: disconnect recovery keeps the current case, clears stale
shared totals, preserves subsequent solo votes across failed polls, and replaces
solo state with validated authoritative shared state on reconnection. Solo votes
are not silently uploaded. Malformed round/count/vote responses fall back to
solo mode. The managed local client was updated atomically without restarting
the room server. Its served SHA-256 matches source:
`b1e64c317bb9c27c6c21f42f99adb6333822d47fd169a87d7e74d1b5ed2a1e1a`.

Verification: `npm --prefix activity test` exited 0, three tests passed,
including deterministic client failure/recovery and actual HTTP room behavior.
A browser preview visibly recorded one approval in a connected shared room.
The full source gate before subsequent streaming/style work exited 0 with
1,560 Rust tests passed, zero failed and five intentional ignores, plus offline
Swift audio checks, warnings-denied Clippy and locked release build. Public
HTTPS hosting, Portal mapping and an actual new Court iframe remain Partial.
The reviewed source archive is a Node package, not a deployable Sites Worker.

Voice readiness: the token-free local self-test exited 0 in 50 seconds with
100% synthetic round-trip word recall through Kokoro, Whisper and generation.
This is synthetic audio evidence, not human-witnessed Discord conversation.
Read-only Discord REST at about 08:05 EDT found Abbey in Office Hours,
self-muted and self-deafened; H2 was absent (Unknown Voice State). H2 has no valid
Local policy-1 receipt, so his own `/voice consent` agreement is still needed.
Other present participants need their own valid receipts, followed by manager
join/resume from the call. No receipt or voice state was forged.

Operator-directed engagement: one Abbey-authored invitation was delivered in
H2's existing bot DM at 12:06:29 UTC, message `1555189156974559404`.
One contextual MLAI general invitation was delivered at 12:08:01 UTC,
message `1555189546193256471`. Both returned the bot author and message IDs;
measured REST delivery was 700 ms and 804 ms respectively. These timings do
not measure model generation latency. No reply has been observed yet. No mass
mentions or repeated sends were used. Other installed guilds' default-off
unsolicited policy was preserved; no new installations or permissions changed.


### 2026-10-01 validated source: streaming stability and learning inspection

Current source: progressive streaming edits wait two seconds after successful
send/edit and skip missed ticks; final completion and honest failure replacement
remain immediate. Two paused-time regressions failed before the change at
100 ms and 1.1 seconds spacing, then passed. Quoted/code/blockquoted style
phrases are ignored as member preference, with ordinary contractions preserved.
`/admin addenda list` now shows current-window aggregate supporting/opposing
counts, distinct supporters, quorum, learning status and suppression. No member
identities, hashes or feedback text are exposed; persisted schema and policy
are unchanged. Consumed application evidence is not presented as pending.

Verification: generation suite 16 passed; style signal suite 8 passed; addenda
filter 41 passed, all with exit 0. The actual rendered full status was read and
fits under 2,000 bytes without clamp loss. Independent scoped review approved
these changes. `./check.sh` exited 0: 1,568 Rust tests passed, zero failed,
five intentional ignores, offline Swift 16 tests/release build, warnings-denied
Clippy and locked Rust release build. Source fingerprint remained unchanged:
`4a257548503f8ed8aa5e83f2b6355b7b34dca385d43433c67ae84ab7dc20345e`
(521 source/support files).

The validated release SHA-256 is
`18b1a1794d54001e9c51c83743575664a7e4f2fdc9e566726b6c2b7680f18914`.
At this observation the installed binary remains
`b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`;
these new Rust changes are not claimed as installed. The operator's latest
request moved the next work into architectural brainstorming and planning.
No new production dependency, commit, push or permission change was made.

Correction to earlier read-only coordination: current source DOES call
`try_auto_listen_while_present` from `commands_voice/supervision.rs` for a human
join while PresenceOnly. A claim that the helper has no caller is stale or
incorrect for this tree. It still checks durable all-present consent. The
external Office Hours watcher separately uses restart-based startup retries.
Neither source hook nor synthetic voice probe proves a human voice exchange.


### 2026-10-02 integrated source baseline and retained voice preparation

The integrated Task 9 engagement, maintenance, provider FIFO, and Activity
candidate received independent review and passed
`ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`: 1,883 Rust tests passed,
zero failed, eight ignored; offline Swift suites passed 12 and 16 tests;
Clippy, WDBX conformance, and locked release build passed. Before/after input
hashes matched. The baseline release SHA-256 was
`2b316049f29f4720004ad746eceba53adcfea9e7c24555aaabeb36b2b1119489`.
The parent verification receipt is retained in
`Archive/2026-10-02-workspace-program/bot-qualified-integration-adoption-20261002/parent-verification.json`.
This supersedes the earlier failed combined gate for source qualification;
it does not establish installation or live acceptance.

The next source slice transfers PresenceOnly join preparation from a detached
spawn to the existing retained Voice operation registry. Shutdown invalidates
its start generation and observes cleanup before freeze. Its new regression
failed against detached behavior, then passed; supervision passed 3 tests and
voice-session coverage passed 63 tests. Independent review approved source
integration. The full strict gate for this changed slice is pending.
Consent, media epochs, and existing activation checks are preserved.

Court protocol-v2 candidate has separately passed 20 Node tests, Liquid checks,
and independent review, with two local browser clients witnessing shared votes.
Public HTTPS hosting, Portal mapping, two Discord Activity clients, installed
provider behavior, ordinary-member journeys, and human voice conversation
remain open. No installed service change is claimed by these source records.


## 2026-10-02 recovered candidate source qualification

**Current source; Partial overall acceptance.** Five review fixes, typed outbound certainty and permission-checked forum resolution passed independent source review. `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited 0 with 1,930 Rust tests passed, zero failed and eight explicit operator/live exclusions; Swift suites passed 12 and 16 tests. Required WDBX conformance, warnings-denied Clippy and locked release build passed. Activity: 20 tests; Pages selector: 21 tests; Liquid and module-size checks passed. Accepted security debt remains five vulnerabilities; the audit is not clean.

The complete tracked/nonignored-source manifest was stable before and after the gate: HEAD `a48379ba3e60535d4141ab84958b1e54a03630c6`, 756 inputs, SHA256 `c2017478dfe65fd8dbdd39265e304aa8fdbab44a5b24d78ff529ed573e2db822`. Gate log: `/tmp/abbey-all-20261002-bot-final-gate-3.log`; manifests: `/tmp/abbey-all-20261002-bot-pre-gate-3.json` and `/tmp/abbey-all-20261002-bot-post-gate-3.json`. Candidate SHA256: `9c4be3cc3b110bd05f8a5e1701a6e3161fdc9e64aa9702b777c8a7b0411a38b3`. Installed SHA256: `b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`. They differ. No deployment, restart, provider qualification or human voice acceptance occurred. These record-only additions follow the verified source snapshot.

The nine-workstream program remains in progress. Read-only audits map 39 tasks; source gaps remain in text instrumentation and bounded streaming, voice lifecycle/timing, learning quality, continuity and task-backed initiative. Community shadow/review/appeal qualification and the ABI training-family split invariant are separate gaps. Public Activity, installed identity, provider, Discord, platform and human acceptance remain open wherever required. Existing Engagement source coverage does not close its live acceptance. Actual model training is outside the training-foundation handoff.

The full historical task/requirement audits and controller corrections are preserved in [completion source audit](2026-10-02-completion-source-audit.md). Later Text Task 1 source changes are under independent review and require their own strict gate; the preceding baseline receipt does not certify them.


## 2026-10-02 Text Task 1 source qualification

**Current source; installed/live acceptance open.** Independent review found and the implementer corrected one P2 provider-clock attribution error; fresh scoped re-review approved the adapter execution-start observation. Queue wait, provider-first-text, first-visible, final-delivered and cancellation now have closed content-free measurements at their actual ownership points. Generation completion remains separate from native delivery. This is not a general arbitrary-clone terminal-state machine or qualification of Engagement's separate outreach metrics.

`ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited 0: 1,935 Rust tests passed, zero failed, eight explicit operator/live exclusions; Swift suites 12 and 16 tests, strict WDBX conformance, warnings-denied Clippy and locked release build passed. Focused generation51 and observability5 tests passed. Log: `/tmp/abbey-all-20261002-text-stage-strict-gate.log`. Stable complete-source manifest: HEAD `a48379ba3e60535d4141ab84958b1e54a03630c6`, 759 inputs, SHA256 `4035dcd6f0f442c1440fe666fbf7806a1e311bbbcaf5bd94b444d05ca1806677`; pre/post receipts `/tmp/abbey-all-20261002-text-stage-pre-gate.json` and `/tmp/abbey-all-20261002-text-stage-post-gate.json` matched. Candidate binary SHA256 `f7ff0368d84fe3d7fa7df9b8263da3dba2d0b2a04294a3b53db6be18faac1388`; installed SHA256 `b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9` differs. No installation, provider/live probe or service action occurred. These record-only additions follow the verified snapshot.

Text Task 3 retained bounded producer and Task 4 workload/installed benchmark remain open. Missing installed/provider/Discord/platform/human evidence stays open across the completion program.


## 2026-10-02 Text Task3 source qualification

Current source: retained provider production proceeds independently of blocked outbound delivery; accumulated answer text has an inclusive65,536-byte UTF-8 bound with direct coalesced publication. Caller disappearance, service cancellation and consent withdrawal request cancellation while retained owners observe actual producer completion. Managed nonstream FM carries per-attempt cancellation into its existing protected subprocess owner and waits for actual child termination/private-file cleanup. Returned cancellation has one closed cancelled terminal rather than a fabricated failed authorization. Overflow remains neutral to the provider circuit and cannot dispatch tools or replay/fallback. Validated terminal text is authoritative; empty final output replaces a partial with honest failure.

Independent review initially requested changes for nonstream cancellation and returned shutdown telemetry. Both meaningful regressions failed before their fixes; fresh scoped review approved both corrections, including the803-line FM infrastructure boundary. Synthetic Unix process evidence establishes actual termination/join/file cleanup while the service token remains unset; it is not a vendor FM, Windows, Discord or human witness. The fixture stack overflow was corrected by boxing its nested test future; no runtime stack limit or production lifecycle change was used to conceal it.

Verification: `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`, terminal exit0; Rust1,944 passed/0failed/8ignored,71.58s; offline Swift12+16passed; warnings-denied Clippy, deployment/privacy/security/contracts/Pages checks, required WDBX fixture parity and locked release build passed. Accepted RustSec debt remains five vulnerabilities plus four informational warnings; the audit is not clean. Release build56.19s. Log: `/tmp/abbey-all-20261002-text-retained-strict-gate.log`.

Pre/post gate: HEAD `a48379ba3e60535d4141ab84958b1e54a03630c6`;766 complete tracked/nonignored input paths; SHA256 `944ac8c8ecd38b8271c49e2ef86866eb033deb19c155cdcc73d6605cc3eb1f88`, unchanged. Manifests: `/tmp/abbey-all-20261002-text-retained-pre-gate.json` and `/tmp/abbey-all-20261002-text-retained-post-gate.json`. Required WDBX fixture SHA256 remains `a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90`.

Candidate release SHA256 `4dc7daf6993515afe441f7c836bfd1ae699ffed1ee250a20e1f80f7b768914a3`; installed binary SHA256 `b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`, different. No installation, restart, publication, live provider/Discord action or commit occurred. These completion records were appended after the frozen source gate; they do not self-certify a new gate snapshot.

Focused final fix: generation59passed; actual Unix subprocess1passed; provider compatibility27passed/1ignored; Clippy/fmt/privacy/module/diffpassed. Initial broad focused evidence and its declared short LLM/provider overlap remain historical; final strict gate was serial and qualifies the complete corrected snapshot. Activity independently rerun:20passed/0failed/0skipped, terminal exit0, `/tmp/abbey-all-20261002-activity-recheck.log`.

Partial completion program: Text Task3 source closes here; Task4 benchmark source and installed48+6 witness remain open, followed by Voice/Learning/Continuity/Initiative and separately tracked Community operations/training foundation. Actual model-weight training remains outside that handoff. Public Activity/Portal, installed candidate identity, provider qualification, Discord behavior, platform runtime and human voice acceptance remain separately open. No source gate promotes those layers.


### 2026-10-02 Text Task4 source qualification

Current source: deterministic synthetic-provider48 and explicit six-Discord-witness receipt/report comparison are implemented. Independent review initially found incomplete successful timing coverage and ambient tracing disclosure; both were corrected and fresh scoped re-review approved (2 addressed, no new consequential findings). Every successful applicable stage is required; explicit measurement mode binds both reports to the provider; actual benchmark startup suppresses ambient tracing and returns one content-free JSON document.

Controller `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` terminal exit0: 1,960 Rust tests passed, 0 failed, 8 intentionally ignored, 74.73s; macOS audio-tap suites12+16, warnings-denied Clippy, locked release build and new actual CLI offline startup test (1passed,48 successful synthetic probes/60 localhost fixture calls) passed. WDBX pinned projection fixture parity remains SHA256 a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90. Accepted RustSec debt remains accepted debt, not a clean audit.

Complete pre/post fingerprint: HEAD a48379ba3e60535d4141ab84958b1e54a03630c6;773 inputs; SHA256134863181824f325fa213e35c3a41cecbf0f76abb9e88d7458944c92749cb140, unchanged across gate. Release candidate SHA256 e40658cab0c7873e7680fc200e2e7d23e6363162dd96ab5337f98d622d667d91 differs from installed b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. Gate log /tmp/abbey-all-20261002-text-benchmark-strict-gate.log and review /tmp/abbey-all-20261002-text-benchmark-rereview.md preserved with reports/fingerprints in .superpowers/sdd/2026-10-01-mlai-text-reliability/task4-qualified-evidence/.

Partial overall Task4: actual matched provider48 baselines/candidates, six authorized Discord delivery witnesses, exact installed route/model/hardware attestation, contention/voice measurements and Windows runtime remain open. Local fixture clocks are not performance evidence. No tuning, provider/vendor probing, capability admission, deployment, restart, live Discord action or commit occurred. Operator records do not retroactively qualify a new installed candidate. These completion-record edits follow the frozen gate and do not claim a new source fingerprint.


### 2026-10-02 stable combined Text Task4 / Voice Task1 qualification

Correction to earlier Text Task4 source evidence: a separate authorized review found that ordinal-specific benchmark tool nonces could not satisfy the unchanged FM schema/parser qualification nonce; all12 tool probes were unsatisfiable for FM. The external checkout writer in “Run auto-improve preview” corrected prompt/schema/acceptance to use the existing qualification nonce, retained distinct ordinals/final markers, and versioned the workload to abbey-text-benchmark-v2. Four parser-backed/contract/workload regressions qualify the corrected source; old v1 receipts intentionally refuse comparison. Production FM validation and capability qualification versions are unchanged. Independent initial/scoped review found no actionable issues.

Voice Task1 is Current source: the production-consumed pure upgrade decision requires PresenceOnly, explicit Local opt-in, a nonempty fully consented roster and current permissions. It reuses the existing admission decision rather than a second ledger/watcher. Existing Localpolicy1 persistence and join hook remain; the rendered notice now honestly includes operator-enabled Local auto-listen using saved choices. Focused4auto-listen/11consent tests and independent Task1 review passed. The original-token-before-first-await/dispatch bug remains explicitly Task2.

The first controller VoiceTask1 gate exited0 with1,962Rust tests, but its complete source hash changed during externally authorized nonce edits. That run is preserved as successful command evidence and is NOT stable-source qualification. Controller held all further writes/builds until the external writer finished; no external edits were reverted or overwritten.

Stable combined source gate executed by the nonce owner and verified by this controller: CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh < /dev/null, terminal exit0 (desktop command receipt exec-ff66216a-7c1e-4476-9b50-3d99be98a309). 1,966Rust passed/0failed/8intentionally ignored,279.66s;334printed Python unittest cases plus named contract/scenario checks;Swift12+16;warnings-denied Clippy;locked release build1m58s;actual offline CLI startup1test passed with48successful synthetic probes/60localhost calls. PinnedWDBX fixture parity SHA a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90. Accepted RustSec debt remains debt, not a clean audit.

Complete source identity: HEAD a48379ba3e60535d4141ab84958b1e54a03630c6,773inputs,SHA2569d6e4bb7d0e22f05d06f2db8bbedcfed6aaaacbfba267d62334383da19434ee9, stable. Owner captured767regular inputs before/after; controller reconstructed the complete pre manifest with6excluded instruction/skill inputs from its exact prior manifest, verifying every excluded hash/mode unchanged and modification before the owner pre-gate receipt. This complete773-input pre matches the controller in-gate observation and complete post byte/hash/mode manifest. Reconstruction provenance preserved in the Voice plan workspace; no claim that the owner’s partial manifest was complete.

Release candidate SHA256598a02a58cb8cc75a7a272a3f36e5a03d68222c6836f5af70989b979ad78f629 differs installedb965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. Source/offline qualification does not establish installed identity, actual FM/provider qualification, six Discord delivery witnesses, human voice8cases, measured performance, watcher service readiness or Windows runtime. No deployment/restart/live server/provider action/commit occurred here. Overall program remains Partial. These completion-record edits follow the frozen gate; they do not certify a new complete-input fingerprint.


### 2026-10-02 Voice Task2 source qualification

Current source: original activation token now survives both live permission lookups, retained dispatch before first poll, and startup output-only fallback. Stale attempts cannot reserve or publish Presence after leave, withdrawal, replacement or draining; an old fallback error cannot cancel a newer start. Existing retained owner joins, final persistence at most once, consent policy and Pass/self-deaf behavior remain. Independent spec and quality review approved the eight-file implementation and reviewed the 985-line lifecycle boundary.

Verification: genuine held-future RED failures followed by auto-listen11/fallback4/voice-session65/service62/actual retained-owner1 GREEN. First strict run exited1 on the Pages inventory after the completion audit was externally staged; source remained stable. One expected inventory entry was added,21 selector tests and scoped rereview passed. Corrected strict command `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0:1,977Rust passed/0failed/8ignored,334printed Python cases,Swift12+16,Clippy,required pinned WDBX fixture,locked release4m12 and actual offline benchmark startup1passed (48successful synthetic probes/60localhost calls). Rust suite254.40s. Accepted security debt remains five vulnerabilities; audit is not clean. Full gate log/result and complete per-file before/after manifests preserved in Voice task2-qualified-evidence.

Complete stable snapshot:776inputs,SHA256c398557d35ae0bb23495bcaf0527fdf7a764afe18f18664453bc599996ef1aa2,HEADe742375ff9b3d20b97df09c7fad10c1225ae957c. The HEAD/index changed externally during implementation; this controller performed no Git mutation, preserved that state, and reconciled complete source manifests. Only eight reviewed Voice files, four completion records and the reviewed inventory fix changed since the preceding qualified source snapshot.

Built candidate SHA2569a077724b0de609547e803c30f472455263f363218a3d732ac35ef30cec4d2c7 differs from installedb965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. No deployment/restart/live provider/server/human voice action occurred here. Voice timing/matrix Task3, operator/witness Task4, Learning/Continuity/Initiative and separate Community/Training-foundation source gaps remain open. Installed/provider/Discord/platform/public Activity/human acceptance is separate. These completion-record edits follow the frozen gate and do not certify a new complete-source fingerprint.


### 2026-10-02 Voice Task3 source qualification

Current source: closed content-free recognition, generation and synthesis timing events now report observed success, failure, cancellation and timeout through the existing retained telemetry owner. Generation measures the full attempt, including preparation, guards, queueing and finalization; its event is not proof that a provider ran. Fixed operational replies produce synthesis events without inventing a generation operation. Existing consent/media epochs, Pass/self-deaf reconnect, immediate withdrawal and observed owner joins are preserved. Independent spec and quality review passed; its nonblocking inaccurate header comment was corrected and a fresh scoped rereview closed V3-1.

Genuine RED coverage: eight actual actor/timing fixtures failed for absent terminal events and two production-used timer fixtures failed for typed timeout/cancellation classification; GREEN followed. Final focused voice_local42 and observability6 passed, warnings-denied Clippy and static checks passed. No synthetic fixture qualifies a human conversation.

Strict command `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0 with the default test concurrency:1,989Rust passed/0failed/8intentionally ignored,99.41s;334 printed Python unittest cases,Swift12+16,Clippy,required pinned WDBX fixture,locked release1m25s and actual offline benchmark startup1passed (48successful synthetic probes/60localhost calls). Accepted RustSec debt remains five vulnerabilities and four informational advisories; audit is not clean. Complete before/after manifests and terminal gate/result/reviews are preserved in the ignored Voice task3-qualified-evidence directory.

Stable source:777inputs,SHA256e936af8713d4158f7de1bd20334ddfb845d984e4058c289ebd32d6ae0950bdd9,HEADe742375ff9b3d20b97df09c7fad10c1225ae957c. Candidate SHA256a4eed42b5c606f40c2ac6dd762ff9c3f484e3bf6b8b42f6e21ce48561e173e0f differs installedb965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. No deployment/restart/live provider/server/human voice action or Git mutation occurred here. Task4 watcher/operator decision and eight witnessed human cases remain open, as do later Learning/Continuity/Initiative and Community/Training-foundation source work. These completion-record edits follow the frozen gate and do not certify a new complete-source fingerprint.


### 2026-10-02 Voice Task4 operator checklist and OPEN witness ledger

Current record preparation; Partial Task4 acceptance. Voice Tasks1–3 source qualification is the preceding stable 777-input SHA256 `e936af8713d4158f7de1bd20334ddfb845d984e4058c289ebd32d6ae0950bdd9`, HEAD `e742375ff9b3d20b97df09c7fad10c1225ae957c`: `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0, 1,989 Rust passed/0failed/8ignored at default concurrency. This record-only slice reuses that evidence; it does not claim a new full-source fingerprint or installed qualification.

Current read-only observation on 2026-10-02: `python3 -I deploy/service-status.py` exited0 and reported service/Discord ready, scheduler running, Telegram/Slack disabled and last completed persistence complete. SHA256 comparison reconfirmed candidate `a4eed42b5c606f40c2ac6dd762ff9c3f484e3bf6b8b42f6e21ce48561e173e0f` differs from installed `b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`. `launchctl list` enumerated the bot and Office Hours autolisten labels with running PIDs. That establishes loaded-agent presence only; no watcher restart behavior, consent coverage, voice readiness or installed candidate equivalence was witnessed.

Each row requires its own exact installed SHA256, date/time, willing witness role, current personal receipt/roster/permission check, observed content-free behavior and PASS/FAIL verdict. Record those fields in the Result column when actually witnessed; `OPEN` is neither PASS nor a fabricated failed experiment. Use role labels rather than member/channel IDs. Retain no raw human audio, utterances, transcripts or responses. Historical receipts and source fake PCM do not fill witness fields.

| Case | Required operator action and witness assertion | Result: installed SHA; date; witness role; checks; observation; verdict |
|---|---|---|
| V1 Personal choice | Each willing participant personally reviews and agrees via `/voice consent`; verify current Local policy-1 coverage. Saved valid choices survive restart; membership, silence and manager assertion never substitute. | OPEN — all witness fields absent |
| V2 Current admission | In-channel Manage Server operator runs `/voice join consent:true` or `/voice resume consent:true` as required; check the full current nonempty roster, receipts and permissions before Decode/unmute. | OPEN — all witness fields absent |
| V3 Audible conversation | Willing participant wakes Abbey and witnesses an audible reply; pair the human hearing assertion with the process-memory verification milestones. | OPEN — all witness fields absent |
| V4 Interruption | Willing participant interrupts active playback; witness playback stopping and a current safe continuation. | OPEN — all witness fields absent |
| V5 Unattested arrival | Controlled willing arrival without current receipt pauses/closes processing before later frames; obtain personal choice before current manager resume. | OPEN — all witness fields absent |
| V6 Withdrawal | Participant personally withdraws via `/voice consent`; witness media/processing closes without later processing. | OPEN — all witness fields absent |
| V7 Reconnect | Operator conducts an authorized controlled reconnect; witness current receipt/roster/permission and media epoch checks, safe Pass/self-deafen when output-only, and no retired-session revocation of the new epoch. | OPEN — all witness fields absent |
| V8 Leave and cleanup | Operator runs `/voice leave`; witness call closure and observed retained-owner completion, not merely cancellation requested. | OPEN — all witness fields absent |

Operator sequence, for a separately authorized run:

1. Identify the exact reviewed/gated artifact intended for installation. Any deployment/restart requires explicit operator direction through the documented transaction; compare installed hash afterward. Confirm current service and qualified provider/model/OS identity. The mismatching installed artifact above does not qualify Tasks1–3.
2. Arrange willing participants, an in-channel Manage Server operator for join/resume, and an application owner or Administrator for verification start/report. These are distinct authorization requirements; Manage Server alone does not grant verification access. Inspect current member choices privately. Do not copy identity-bearing consent storage or raw service logs into this ledger. Establish a safe controlled reconnect/arrival scenario before starting.
3. Have the application owner or Administrator arm `/voice verify start` before the consented join. It disables conversational commits and retains only process-memory milestones. Run V1–V8 on one identity-bound run with current checks at every transition, then have that authorized verification actor obtain `/voice verify report` after leave. A process restart clears the report; use a separate identity-bound run/record for restart coverage rather than treating a cleared report as continuity proof.
4. Record the content-free milestone report and each actual human/operator assertion with exact installed hash and date. Missing witness, current receipt checks, actual audible behavior or observed cleanup leaves the corresponding case OPEN. A synthetic audition qualifies only its synthetic provider chain, never V3 or the eight-case human run.
5. Decide watcher retention/retirement only after the installed join path and failure/reconnect matrix pass. Record the operator, date, evidence references, exact target and authorized action. If behavior proves a change necessary, prepare a narrow separately reviewed shell/fake-launchctl-tested change wired into the gate; apply it only on explicit service direction.

Watcher decision: unchanged; retain/retire acceptance OPEN. Source inspection of `deploy/watch-office-hours-auto-listen.sh` found its debounced restart condition is designated participant presence plus Abbey self-mute or self-deafen; it does not consult durable receipt coverage. This is a source observation, not proof of a consent-triggered restart in the running agent. Do not use MissingConsent as a restart reason or add a second consent ledger/controller. Source-qualified while-present activation does not alone justify retiring a loaded fallback watcher. No applied watcher patch is warranted by the available witnessed evidence.

Installed candidate identity, actual provider qualification/performance, Discord/platform runtime and all eight human/operator cases remain OPEN. No installer, restart, watcher invocation, gateway, live Discord REST, provider/voice probe, participant consent creation or human audio capture occurred in this record slice. The available checklist/record work can be reviewed while independent Learning source work proceeds; these proof obligations remain required.


### 2026-10-02 Learning Task1 source qualification

Current source: typed exact/unique/duplicate/ambiguous/expired/unsupported attribution now rejects competing turns, unsupported reactions and unidentified/bot reactors; the actual gateway resolves current reactor identity within its retained framework owner. Scoped active reaction contributions survive canonical save/reopen, duplicate adds and removals are inert, removals reverse only their own recorded contribution, and settled turns cannot reopen through ledger eviction. The existing strictly-older-than-150-second settlement, reward blending and ±3 clamp remain unchanged. Active keys are bounded at4,096; closed markers retire into a compact creation-time floor within300seconds. This does not promise arbitrary event-order correction or fabricated fresh-time replay deduplication after all identity markers retire.

Independent review found LQ1-001 P1: a new pending-row limit rejected valid legacy canonical state. The reviewed compatibility repair preserves all4,097 distinct legacy fixture rows and unrelated settings/facts/reputation through actual load, publication, partial settlement and reopen; it drains each reward exactly once without expanding per-row markers. New tracking is temporarily refused while legacy carryover exists. Missing historical reaction provenance is never invented; legacy reaction additions remain inert. Fresh scoped spec and quality rereview approved the fix and closed LQ1-001. Original recovered and review packages remain preserved.

Verification: meaningful duplicate/removal/competing-turn RED tests, plus actual4,097-row canonical-load RED, followed by43reward/35pipeline+one existing ignored/27persist/one runtime/one native GREEN; warnings-denied Clippy and static checks passed. Strict command `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0 with default concurrency:2,015Rust passed/0failed/8existing ignored in108.64s,334 printed Python cases,Swift12+16,required pinned WDBX fixture,Clippy,locked release1m35s and actual offline text-benchmark startup one test passed (48successful synthetic probes/60localhost calls). Accepted RustSec debt remains five vulnerabilities and four informational advisories; audit is not clean.

Complete before/after stable snapshot:782inputs,SHA2560ab9c7551c2068bc633e6ee3eeb3c12468304ebd15230db345ceb2bf251f1c84,HEADe742375ff9b3d20b97df09c7fad10c1225ae957c. Terminal gate result, complete manifests, logs and independent reports are preserved in ignored Learning task1-qualified-evidence. Candidate SHA256cab941e3a260308ae7b6f83283d4f87528e3efefe71f6b55c4482f790c292c8f differs installedb965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. Installed/provider/Discord/platform/human acceptance remains OPEN. No commit/deployment/service/live provider/server action occurred. Learning Tasks2–5 and downstream source work remain open. These record edits follow the frozen gate and do not certify a new fingerprint.


### 2026-10-02 Learning Task2 source qualification

Current source: canonical Pending stores bounded AskSignature instead of raw original ask. Legacy strings deserialize once into at most32 distinct domain-separated normalized lexical hashes and closed markers; raw asks are absent from subsequent publication. Canonical load/save/reopen, default migration, negative-zero no-feedback settlement and the4,097-row legacy carryover fixture pass. Normalization, marker precedence, overlap thresholds,150-second settlement and reward/reaction behavior remain. The first32distinct tokens are retained; overlap outside that bound can differ from historical unbounded text. This is a bounded lexical representation, not cryptographic anonymity or semantic truth inference.

Production LearningAudit records six saturating aggregate attribution counters without loading DQN, refreshing policy idle state or spending budget. Counters survive policy eviction and reset with process start. Fresh native authorization precedes private /admin brain snapshots and epsilon changes; the registered command fixture verifies denial after prior catalog approval and permission revocation. Diagnostics retain sources/refusal reasons, pending age, guard reason and action-values labeling, with no member/channel/turn identifiers or raw feedback. Actual printed empty/current/maximum copies are517/567/1083characters. Future scoped reset must explicitly include the aggregate audit map; this slice does not implement member erasure or individual aggregate unlearning.

Independent spec/quality review approved all16owned changes and explicitly reviewed pipeline870/runtime869lines. Optional P3 first32selection coverage was added as a test-only follow-up; fresh scoped rereview closed it. Meaningful migration/label RED tests and a caught eager-policy-load admission regression preceded final GREEN. Focused final outcome19/signature3/audit5/registry10/reward43/persist29/commands36/pipeline36+one existing ignored/fresh-authorization1/rendered1 passed; rendered test overlaps the command suite and is not an extra unique test. Clippy/static checks passed.

Strict command `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0 with default concurrency:2,025Rust passed/0failed/8existing ignored in67.93s;334 printed Python cases,Swift12+16,Clippy,required pinned WDBX fixture,locked release1m23s and actual offline benchmark startup one test passed (48successful synthetic probes/60localhost calls). Accepted RustSec debt remains five vulnerabilities and four informational advisories; audit is not clean.

Complete stable before/after:783inputs,SHA2561f27de4f6f13ed33aa87a610058b0a61b799c19c91b8bf26fdba5806989a0ea4,HEADe742375ff9b3d20b97df09c7fad10c1225ae957c. Original/fix source packages, complete manifests, terminal gate result/log and independent reports remain preserved in ignored Learning evidence directories. Candidate SHA25601281e876ab6b854e46eb788b6ae2e80a3b7cc8cb714bf416c40e72d83245748 differs installedb965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. Installed/provider/Discord/platform/human acceptance remains OPEN. No Git mutation, deployment/service change or live provider/server action occurred. Learning Tasks3–5 and dependent source work remain open. These completion-record edits follow the frozen gate and do not qualify a new fingerprint.


### 2026-10-03 Learning Task3 source qualification

Current source: exactly/uniquely attributed explicit nonquoted corrections bind the current native prior reply, scope, guild, asker, creation time and minimized signature. Expired, competing, cross-scope, unidentified, quoted/code/example and bare-no inputs cannot acquire repair authority. The selected production repair and provider fallback use disabled tools and current authorized SourceOnly evidence; correction text and historical assistant output are not factual evidence. Empty evidence gives a fixed honest uncertainty reply without a text-provider call. Native source/settings and existing personal-memory authority are rechecked before evidence capture and through retained generation/delivery, with observed cancellation joins. Reward blending, settlement and the physical-reply heuristic remain unchanged. Erasure/reset remains the next separate task; this slice does not implement it.

Independent spec and quality review approved the ten-file bounded source slice, including explicit review of the889-line production pipeline. A P3 rendered-copy finding was fixed through trusted closed ContextChanged classification, preserving provider-controlled backend errors and uncertain-delivery precedence. A genuine actual-pipeline RED reproduced backend-blaming copy; Revoke/RemoveSource/DisableLearning now render “The context for this answer changed while I was checking it. Please ask again.” Fresh scoped rereview closed F1 and verified all source/package modes, exact index and preserved original evidence. Final fix-focused correction21/ask22/generation59/pipeline44+one existing ignored/llm25/provider-runtime25 and Clippy/static checks passed.

Strict command `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited0 with default concurrency:2,038Rust passed/0failed/8existing ignored in63.65s;334 printed Python cases,Swift12+16,Clippy,required pinned WDBX fixture,locked release1m22s and actual offline text-benchmark startup one test passed (48successful synthetic probes/60localhost calls). Accepted RustSec debt remains five vulnerabilities and four informational advisories; audit is not clean. Complete before/after stable snapshot:785inputs,SHA256bc3bb69faa974609841ee45d3cf45fddbe9067261b745fc9c014ea3a9f733db7,HEADe742375ff9b3d20b97df09c7fad10c1225ae957c. Terminal gate81550, full manifests/logs/reports/artifact hashes are preserved in ignored Learning task3-qualified-evidence.

Candidate SHA25693588cc291010478956dfeed75f28011900bd72ea3cb5f94a27cb9af952a0139 differs installedb965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9. Installed/provider/Discord/platform/human acceptance remains OPEN. An independently authorized cleanup chat removed generated debug artifacts during focused checks; the missing-rmeta attempt was environmental, the canonical rebuild and final gate passed, and no source/archive loss was observed. This chat performed no cleanup, Git mutation, deployment/service change or live provider/server action. Learning Tasks4–5 and downstream source work remain open. These completion-record edits follow the frozen gate and do not qualify a new fingerprint.


### 2026-10-03 Learning Task5 source qualification; human/provider acceptance open

Current source: frozen schema-1 synthetic corpus has 100 distinct cases, five
classes of 20. The pure public evaluator enforces that shape and bounded
field/list/identity preflight before grounding or counting. The production offline
`--learning-quality CORPUS.json --json` caller exits before configured-state,
credential, provider or gateway initialization, uses a 1 MiB regular-file bound
and emits closed counts/provenance/hashes. Dotted tokens are bounded before
expansion; Unix nonblocking open plus opened-handle metadata rejects FIFO races.
DQN imports reject nonfinite values and invalid actions atomically, then replace
replay while preserving capacity and documented legacy width compatibility.
Task4 erasure code and policy weights were not changed by this Task5 repair.

Fresh independent standards/spec review passed, closing LQ5-1/2/3 with no open
source findings. RED evidence includes four import failures and one public-corpus
failure, followed by 242 brain and 5 CLI unit tests passing. The original
corpus/held-out/rollback tests are green; no individual historical RED evidence is
claimed for those three. Full receipt and review links:
[Learning Task5 verification](verification/2026-10-03-learning-quality-v1.md).

Strict `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited 0
(session32980): **2,087 Rust passed / 0 failed / 8 ignored** in106.35s;
**339 Python unittest cases** (333 before Swift, 1 actual text startup, 5 actual
learning CLI startup), separately 16 publication scenarios; Swift **12+16**,
required pinned WDBX conformance, warnings-denied Clippy and locked release
**2m25s** passed. All five learning CLI subprocess cases passed on the rebuilt
release artifact. Accepted RustSec debt remains **5 vulnerabilities +4 informational
advisories** (3 unmaintained,1 yanked); audit is not clean. Nonfatal macOS unwind
and rust-objcopy/libLLVM debug-stripping warnings remain visible in the log.

Synthetic measurements: TP30/FP0/TN70/FN0; correction Repair6/Ignore14/mismatch0;
seed1369907238, topology[18,64,32,3], untrained stay/reply/react92/5/3.
Corpus SHA256 `20237fe9320d68dd1ff60af329c308ec7f67322817e9cf86087fabd308b0fb0f`;
emitted evaluator-source SHA256
`366cb3e7575c0f37ffb3322c3088674d6be455bf596a02af0ff775542783af5d`.
Agent-authored labels are still pending human adjudication; these counts do not
qualify provider answers or supply factual-confidence probabilities.

Stable gate snapshot: **799 files**, no changed files, HEAD
e742375ff9b3d20b97df09c7fad10c1225ae957c, aggregate SHA256
`fc1cc1bb6e5c81457638dede4e712dd5c5a627127d4f51a4364189843677a457`.
Complete manifests/result/log are retained under ignored Learning
`task5-quality-evidence-20261003/fix3-root-run/`; scoped fixes/reviews preserve the
earlier evidence. Candidate SHA256
`f4d76cd9e141b5ad04cba7fc9965d4a05b8b7359933d77a66d65655c1805582a`
differs from installed
`b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`.
No deployment or installed fix3 identity is claimed. No service mutation, live
provider/Discord action, commit or push occurred here.

**Partial overall:** human adjudication of all 100 labels, operator-reviewed
fixed-provider answers/manual support before tuning, installed identity and live
runtime acceptance remain OPEN. These record/plan edits follow the frozen source
gate and receive Liquid/diff checks separately; they do not qualify a new source
fingerprint or close unrelated Learning/whole-bot work.


### 2026-10-03 full-source review; source qualification only

Current repaired source passed independent whole-diff/cross-seam review with no
unresolved blockers. Two P2 defects were repaired through attributable RED/GREEN
regressions: accepted-preview withdrawal now attempts static terminal replacement
without retrying uncertain delivery, and canonical pending actions are validated
before restore/load/publication. Strict gate session 14918 exited 0 with unchanged
800-input SHA256
`6bb0d2d1dcfb1bff572d844b4749692c899b1cd96146f1f865bbcab3afcc0ac9`:
2,090 Rust passed/0 failed/8 ignored,339 Python cases,16 separate publication scenarios,
Swift 12+16,required WDBX parity,Clippy,locked release and rebuilt offline startup.
Accepted RustSec debt is5 vulnerabilities plus 3 unmaintained informational records;
the macOS linker unwind warning remains nonfatal. HEAD/index are preserved.

Learning Task 4 erasure/reset and Task 5 bounded evaluator/import/offline CLI source
are included. The originalTask 4 gate's793→795 input drift remains disclosed; it
was not stable-source qualification. Task 5's later799-file/2,087-test combined
receipt remains historical. All 100 synthetic labels and fixed-provider/manual
support before tuning remain pending. No installed identity, provider latency or
quality, managed-service readiness, live Discord/Activity, Linux/Windows runtime
or human audio acceptance is established. No deployment, restart, live call,
Discord mutation or Git publication occurred.

[Full-source verification](verification/2026-10-03-full-source-review.md) records
repairs, counts, exclusions and evidence. These document updates follow that
repaired gate; the final complete-tree confirmation is external so its fingerprint
can include this receipt without self-reference. This entry adds source evidence
only and does not close live acceptance or future program work.


### 2026-10-03 ContinuityTask1 source snapshot only

Current tested domain24focused passes; independent Abbey review found no blockers.
Strict required-WDBX gate2967 exited0,2,114Rust passed/0failed/8ignored,
339Python cases plus16publication scenarios,Swift12+16,Clippy and locked release;
identical804input fingerprintf1a6e2ce96642ff1555592127805fce6315d0be05a51ef8328344c0c47e7a6a7.
No live continuity feature is claimed: the new domain is cfg(test), andTasks2/3
private human flows/fresh access/prompt barriers/canonical admission/clear/erasure
remain mandatory. All whole-program downstream source and external/human proofs
remain open. This receipt follows the qualified snapshot; no new whole-tree
fingerprint or installed identity is inferred. No service, Git or live mutation.
See [domain evidence](verification/2026-10-03-continuity-domain.md).


## 2026-10-03 Continuity source integration correction

Current source only: native private show/propose/confirm/clear; observed immutable human previews; late native/receipt prompt and delivery checks; validated canonical cards; covered admission and fenced clear/member-erasure recovery. Strict unchanged822-input gate exited0 with2180Rust passes/8excluded. [Source receipt](verification/2026-10-03-continuity-integration.md). The earlier tested-domain-only statement is historical. Installed, provider, Discord and human acceptance remain unverified; no service or live acceptance was performed. Initiative and other completion-program residuals remain open.


## 2026-10-03 Initiative source correction

Current source only: [Initiative receipt](verification/2026-10-03-initiative-source.md), stable851inputs, strictWDBXgate0,2277Rustpasses/8existing exclusions. Native scoped task-follow-up admission/delivery/stop/erasure and private conservative readable-subset counts are tested. Task3 human willing-recipient pilot, actual replies/usefulness, installed hash/provider/Discord behavior, ActivityPortal, Linux/Windows and human voice remain unverified. No production outreach or service mutation was performed. This appended evidence was written after that frozen gate.


## 2026-10-03 Community and Activity focused source correction

Current source: CommunityO7 human-assessed exact-source shadow cases, independent
staff review and subject appeal are implemented. Actual defects were reproduced:
missing native IDs/roles/positive attribution and mentions, unsafe parent child
creation, stopped retry uncertainty, read-only Show mutation, policy/proof checks
after path creation, late staff/hierarchy fallback and wrong command-name copy.
Focused final evidence: shadow45/0, private help115/0, catalog27/0, printed native14/0,
warnings-denied Clippy0. Independent complete-delta review has no blocking findings;
strict qualification of the combined current snapshot is pending.

ActivityTask4 source preparation is implemented with four fixed assets, readonly
manifest-addressed packages, observed existing Node identity, finite proposal-only
ingress and dry-run exact rollback. DNS-label RED preceded repair; actual Python
package17/0, DNS4/0, direct-child probe6/0 and CourtNode20/0/0skipped passed.
The packaged Node test used an ephemeral loopback port and observed termination;
configured8791 and managed services were untouched. Candidate/prior packages and
rollback are retained under `/Users/donaldfilimon/.codex/verification/abbey-bot-continuity-20261003`.
Full stable required-WDBX gate and receipt remain pending for these additions.

Independent classifier/live shadow pilot/appeal utility, O1/O6 live blueprints and
enforcement activation, Initiative willing-recipient pilot, ActivityHTTPS host/
ingress installation/Portal/iframe/two-participant/disconnect/restart/instance
witnesses, installed/provider/operator/platform and human corpus/voice acceptance
remain OPEN. Fresh ABI aggregate-split repair and named Abbey combined source
qualification remain required. No Git/deploy/restart/provider/Discord action occurred.


## 2026-10-03 Community and Activity current source qualification correction

Current: CommunityO7 bounded human-assessed operational shadow/review/appeal and
ActivityTask4 immutable package/finite ingress proposal/dry-run rollback source are
independently reviewed without blocking findings and source-qualified by strict
gate2 actualexit0:2338Rust passed/0failed/8existing exclusions,366Python cases plus
16publication scenarios,Swift12+16,Clippy/locked release. Complete878inputs stable
SHA256c16ab5c39cabd1c3f086318499e0630a14cbb0815c3d461a222b046c8b8eb99b;
HEAD/index and required sibling fixture preserved. [Receipt](../docs/verification/2026-10-03-community-activity-source.md)
records actual RED/GREEN, complete50delta review, real filesystem/retained/native
proof, Activity27Python/20Node cases, five accepted vulnerabilities and remaining
proof gaps. Gate1's terminal exit was not returned; its unchanged success-marked
log remains unqualified. Gate2 has a durable actualreturncode receipt.

These receipt/plan/checklist writes follow frozen gate2 and require their own
final documentation-inclusive source attribution. Community's independent
classifier/live moderation/appeal utility/activation and Activity's publicHTTPS/
finite ingress installation/Portal/iframe/member/recovery/restart/instance witnesses
remain OPEN, as do previously named installed/provider/operator/platform/human
corpus/voice and Initiative willing-recipient acceptance. ABI foundation aggregate
split and named Abbey combined source repair/qualification remain required.
No Git/deploy/service/provider/Discord action occurred. Whole all-plans objective
is Partial; only this bounded source slice is qualified.
