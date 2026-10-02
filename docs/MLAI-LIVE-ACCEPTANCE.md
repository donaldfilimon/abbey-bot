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
