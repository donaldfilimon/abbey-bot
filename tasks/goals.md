# Goals

## Program 1 stable-Rust contract conformance
status: done
- Local C1 source evidence on 2026-08-22: `abbey-bot` vendors the exact 81-artifact,
  88,328-byte Abbey Program 1 corpus from ABI revision
  `348754bdaaf59a40fbb858380f925e0aba95a23b`, pinned to aggregate SHA-256
  `72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e`.
  The Python gate independently rejects lock, inventory, byte, digest, and privacy-taxonomy
  drift using closed reason codes and corpus-relative paths. The stable Rust 1.98.0 test
  decoder independently verifies the lock, per-artifact and aggregate commitments, bounded
  local schemas, all 52 fixtures across seven taxonomies, authority-unknown rejection,
  tolerant extension preservation, semantic fail-closed outcomes, and the complete redacted
  synthetic operator-verification report classified only as `local_test`.
- The earlier 656-test ledger snapshot is historical, not a current stabilization gate result.
  The current Rust 1.98.0 source still requires the isolated strict gate and locked release build
  after this documentation cycle. Contract validation remains data-only and does not establish
  production federation, deployment, provider qualification, a real grant or approval,
  participant consent, live Discord, WDBX episode writes, or installed-artifact identity.

## Full-duplex Abbey voice in Discord Engineering
status: in_progress
- Design: `docs/superpowers/specs/2026-08-20-live-voice-design.md`. Delivery now follows the
  canonical checkout on `main`; dated branch names are archival context, not current guidance.
- The source contract remains participant-attested and fail-closed: bounded Songbird media,
  explicit join/resume/leave/status controls, read-only voice cognition, immediate cancellation on
  consent or media revocation, and no provider prose as authority for voice state.
- Privacy-safe historical evidence from 2026-08-20 and 2026-08-22 is retained only by commit and
  artifact identity. Commits `089b1cb`, `8f57b9c`, `2e3c772`, `644cbd8`, `cd4b404`, and
  `cae32d0`, with recorded artifact hashes
  `e9051aa1fe7978f3c6e97ac10ef528d99640ac7da42be5af058b7eaf88281e30`,
  `6ce8f99088feec4f93f3fa866f763e2f71b9d338d5698156ebd98cc052df27b1`,
  `745d4f25d2e62074c857b23bf48ce977834e09363a2aafa6c34ad75c10792f84`,
  `366ef4b9204896a9227eec553d59bb91ea79dc3a43feb29ea4b95b0d991cda21`, and
  `5e41b477e04812a6933f2542da9258cfa8228146c270d2f56f146d344f088bf6`, covered the
  categories safe presence, bounded output, restart recovery, permission revocation,
  content-free lifecycle verification, and offline speech-chain checks. Those dated results do
  not qualify the current source, manually launched process, provider, installation, or live
  voice behavior.
- Current source publishes a guild-keyed coarse Inspect state from central lifecycle transitions:
  `off`, `presence`, `awaiting-consent`, `active`, or `paused`. Consent revocation, media
  revocation, actor failure, leave, and shutdown cannot leave a stale `active` state; DMs and
  other guilds observe `off`. No identity, participant count, epoch, model, counter, timestamp,
  audio, media detail, or transcript is exposed.
- The existing manually launched process remains untouched and unqualified (see 2026-09-04 reconciliation: now a launchd service, still unqualified). Fresh acceptance is
  pending for exact pushed source, provider qualification, installed artifact identity, two-guild
  isolation, unanimous current consent, a human-witnessed audible wake/reply, barge-in,
  membership-change pause, renewed consent, written stop, and final leave with no remaining media.
- Stream/video ingestion remains outside this voice goal; only explicitly supplied still images
  are part of the documented vision surface.
- **2026-09-04 03:5x — wake names are operator-configurable; nothing about acceptance changes.**
  `ABBEY_VOICE_WAKE_WORDS` replaces the built-in list (`abbey`, `abby`, `aviva`, `abi`) when set.
  Words are lowercased and must be ASCII-alphabetic and at most 32 bytes, which is exactly what
  the matcher's tokenizer can produce, so a configured word can always be spoken into a match. A
  blank, absent, or fully invalid value keeps the default rather than leaving Abbey unaddressable
  — a typo in the operator env is not allowed to silently deafen the wake gate. The wake gate
  itself, the continuation window, and the speaker scoping are unchanged, so this adds no live
  evidence: it changes which names open a turn, not whether a turn was ever witnessed. The
  duplicated `contains_wake_name` in `voice_local.rs` and `voice_self_test.rs` was collapsed to
  one `voice::contains_wake_name`, so the self-test and the live session can no longer drift.
  Everything listed above as pending acceptance is still pending.
- **Not landed, and deliberately so: `/voice mode`.** An untracked working tree from 00:16 added a
  MANAGE_GUILD `/voice mode` that wrote a `pending_voice_mode` nothing read, and whose OpenAI arm
  refused unconditionally — a command that promised a switch it could not perform. It was dropped
  rather than completed here. A real switch requires `VoiceConfig` to retain complete-but-unselected
  backends so a switch can be validated against something real; that work is in flight on
  `cursor/voice-mode-multi-backend` and is owned there.

- **2026-09-04 reconciliation.** Two corrections, both verified rather than inferred. (1) The
  line above describing "the existing manually launched process" is stale: `launchctl list` on
  this Mac shows `com.donaldfilimon.abbey-bot` running as a **launchd** service at PID 66700
  (alongside `com.donaldfilimon.abbey-mlx-audio` at PID 21413). This was observed read-only; the
  service was not started, stopped, or reloaded. The PID differs from the 26416 recorded in
  `docs/MLAI-LIVE-ACCEPTANCE.md`, so the agent has restarted since 2026-09-03 ~21:50 ET. The
  qualification claim is unaffected - it remains unqualified - but it is no longer a manual
  foreground process. The machine-level `~/CLAUDE.md` already records this correctly (it
  marks the deployment LIVE as of 2026-09-03 23:45 at the same PID, and files the 2026-08-21
  teardown as superseded provenance); only this repository's ledger was stale. (2) The pending live
  acceptance in this section is blocked one layer earlier than stated:
  `docs/live-test-protocol.md:35-41` requires green exact-SHA three-platform CI before stage 0,
  and that CI is currently **red**, not merely unrun (see the Complete Abbey section).

- **2026-09-06 20:5x — the CI precondition above is CLEARED; nothing else about this goal moved.**
  `docs/live-test-protocol.md:3-5` is the actual gate ("Begin only after the final
  provider-routing commit equals `origin/main` and Ubuntu, macOS, and Windows CI are green for
  that exact SHA"); the `:35-41` citation in the correction above points at the launchd
  transaction paragraph instead, so read line 3 as the requirement. That gate now holds:
  `HEAD == origin/main == a40e035`, and its `Rust` workflow is **success on all three platforms**
  (Gate (Windows), Gate (macOS), Gate (Ubuntu)). It was genuinely red earlier today and took four
  commits to clear, each failure masking the next because the gate short-circuits at the first
  stage on every runner: `f45b7b2` (formatting, a `private_interfaces` denial on
  `AppState::host_music`, and a voice-status assertion left behind when `6a3000c` moved
  `voice_status`/`voice_diagnostics` to per-guild wording), `566a449` (a Windows-only
  `-D unused-mut` on the cfg-dependent `fs::DirBuilder` binding in `voice_registry`), and
  `d5f00da` (two Windows-only test-portability defects that only became visible once Windows
  first reached the suite). This is a **precondition**, not progress: it permits stage 0 to
  begin and is not evidence that any stage ran.

  The precondition is SHA-bound. Any later push to `main` invalidates it until that new SHA is
  green on all three platforms again; do not carry `a40e035`'s result forward.

- **2026-09-06 — the referenced voice PRs are all merged, and the work is present in source.**
  The line above saying #82's "merge is Donald's" is stale: #76 merged 2026-09-04, #79
  2026-09-04, #85 2026-09-04, #82 2026-09-05, and `abbey-bot` has no open PRs. Verified in the
  tree rather than inferred from PR state: `VoicePhase::accepts_backend_change`
  (`voice_session.rs:67`, used by `voice_session/activation.rs:104` and covered at
  `voice_session/tests.rs:262,276`), the per-fixture songbird `Scheduler::new`
  (`voice_local/turn_tests.rs:172`), `available_local`/`available_openai` (`voice.rs:401,409`),
  and the single `voice::contains_wake_name` (`voice.rs:595`) with `ABBEY_VOICE_WAKE_WORDS`
  (`voice.rs:221`). No new live evidence follows from any of it.

  **Still pending, unchanged, and not startable from this machine alone:** provider
  qualification, installed artifact identity, two-guild isolation, unanimous current consent, a
  human-witnessed audible wake/reply, barge-in, membership-change pause, renewed consent, written
  stop, and final leave with no remaining media. Every one needs two operator-supplied sandbox
  guilds, consenting test users, and a person listening. No source check substitutes for them.
- **2026-09-04 evidence toward this goal, not closure: `/voice mode` is switchable at runtime in
  draft PR #76.** Before it, `VoiceConfig::from_values` retained only the selected backend, so
  after startup the process held no credentials for any other mode; the command early-returned on
  an equal mode, `local()`/`openai()` were `None` for every other one, and only `/voice mode
  disabled` reached a write that nothing read. Switching was unrepresentable, not unimplemented.
  #76 retains complete-but-unselected backends inert behind `available_*` accessors (a present key
  still never selects cloud audio; `destination_defaults_to_local_even_when_a_cloud_key_exists`
  passes unchanged), keeps the effective mode in an `AtomicU8` outside the documented lock order,
  and makes `start_voice` take one backend snapshot that it threads through the Songbird decode
  mode, the public consent notice, the actor it spawns, and the reply, so a switch mid-join cannot
  make the notice describe a different backend than the one that connects. Review caught that the
  OpenAI actor still re-read the startup selection; `ca03d65` gives `OpenAiSession` the snapshot.
  Gate at `ca03d65`: 798 passed / 0 failed / 2 ignored, clippy clean. Not live-verified, and #76's
  three-platform matrix was still pending when this was written. Recorded follow-ups: autojoin
  leaves the runtime `PresenceOnly`, which the `Disconnected`-only guard rejects, and
  `activate_inner` does not re-check the mode between arming and activation.
- **2026-09-04 (later): #76 is merged as `4681509`; its follow-ups are in PR #82.** The two
  items above are closed there, plus one more of the same class found on review: the `/voice
  mode` phase check became a pure `voice_session::mode_switch_blocker` sharing one idle predicate
  (`VoicePhase::accepts_backend_change`: Disconnected, PresenceOnly, Failed) with `/voice verify
  start`, so the autojoin presence no longer blocks a switch; leaving local is refused while a
  verification run is armed, closing the arm-then-switch-then-join window at the switch; and
  `/voice verify start` now holds `transition` through the arm so the two admin commands cannot
  interleave into a run armed under OpenAI. `activate_inner` was left untouched by analysis (both
  the switch and the join hold `transition` through activation). #82 is green on all three
  platforms at `21fa7cd` with 828 passed / 0 failed / 2 ignored; merge is Donald's. Still not
  live-verified. The timeout in `audible_playback_still_stops_on_speech` (from #79) is diagnosed,
  not a lost wakeup: the turn-test module failed about one parallel run in two (5 of 10 at
  `be2915f`, 3 of 6 at `ac909b9`), never serially (5 of 5) and never alone (2 of 2). Probes showed
  a freshly played track's `stop()` returning `Err(Finished)` about 100 µs after the handle was
  stored, with no `End` event, so the speech-triggered stop correctly counted no barge-in and the
  assertion waited for a count that could not come. The only condition observed is many standalone
  drivers created and dropped concurrently on songbird's process-global default scheduler; the
  scheduler-internal mechanism is not pinned. PR #85 gives each fixture a private `Scheduler`
  (8 of 8 parallel runs clean, full suite 835 passed). Test-only; the actor is unchanged.

- **2026-09-08 05:5x EDT — voice classic Action Row UX design is locked on `main`;
  implementation has not shipped.** Squash #104 (`df6a3b6`) landed
  `docs/superpowers/specs/2026-09-08-voice-classic-ux-design.md` plus minimal
  pointers in `docs/superpowers/README.md` and
  `docs/discord-application-api-roadmap.md`. Locked decisions: phased classic
  Action Rows **A** (post-join status Refresh+Leave) → **B** (leave Confirm/Cancel,
  no teardown until Confirm) → **C** (Play/Stop/Skip when playable); short custom
  ids `abbey:v:{sid}:{act}` with a server-side session store under
  `ABBEY_DATA_DIR`; fail-closed like `/admin` / `/pending`; `consent:true` stays
  **slash-only**. Explicit non-goals: Components V2, Portal/OAuth, bot Go Live,
  consent via components. **Implementation requires a separate writing-plans
  pass and Gate-phased PRs per the spec** — design lock is not code delivery.
  A→B→C Rust work is in flight on another branch (`feat/voice-classic-ux-abc`);
  this ledger does **not** claim it shipped. Live acceptance residuals for this
  goal are unchanged: provider qualification, installed artifact identity,
  two-guild isolation, unanimous current consent, human-witnessed audible
  wake/reply, barge-in, membership-change pause, renewed consent, written stop,
  and final leave with no remaining media.

- **2026-09-08 06:1x EDT — PR #105 MERGED on `main` (source landed); NOT
  live-accepted.** After #104 design lock (`df6a3b6`) and intermediate docs tip
  `85d0e18` (addressed #104 design-review findings; docs-only), PR #105
  (`feat(voice): classic Action Row UX Phases A+B+C`) merged as
  `1b4822d1a3b017b65be6cf0afccf8b85e10890c5` (merge of `6f5fbba` into tip after
  #106). **Source is on `main`.** That is **not** live acceptance: human-witnessed
  audible wake/reply, barge-in, consent, two-guild isolation, and related
  human-gated boxes stay open. Live Abbey **redeploy is in flight separately** —
  **do not** claim installed artifact identity from this ledger pass. Tip Gate
  for `1b4822d` was not three-platform green at write time (see tip subsection).

- **2026-09-08 06:4x EDT — live Abbey redeployed with Action Row UX binary; acceptance still open.**
  Verified on this Mac (read-only observation; service not started/stopped/reloaded by this
  pass): `com.donaldfilimon.abbey-bot` PID **58490**, started **2026-09-08 ~06:27:54 ET**
  (process start; Discord `discord_ready` / `connector_state=ready` at **~06:27:59 ET** in
  `~/Library/Logs/abbey-bot/abbey-bot.events.jsonl`). Installed binary
  `/Users/donaldfilimon/.local/libexec/abbey-bot/abbey-bot` mtime Sep 8 06:27, SHA-256
  prefix **`6084c7f2`** (full
  `6084c7f2dc71e27cdb7bae06345e5c8b920d4d7897005ce9c1655836bf9b012b`). Binary strings include
  classic UX markers (`abbey:v:`, "Refresh voice & music", "Confirm leave") matching #105
  source on `main` (`src/voice_ux.rs` / `src/commands_voice/ux.rs`). **Action Row UX binary is
  live.** That is **not** live acceptance and **not** installed-artifact identity
  *qualification*: human-witnessed audible wake/reply, barge-in, consent observations,
  two-guild isolation, and the formal identity-qualification checklist remain open.
  #106 / #107 / #108 are also **MERGED** (ledger + agents docs). Tip Gate for current
  `origin/main` tip `7137d86` is **not** claimed three-platform green here (see tip
  subsection: may still be in progress / cancelled chain).


## Implement the discord-abbey spec suite in Rust (abbey-bot)
status: done
- Captured 2026-08-19 from the approved Rust specification program; specs copied to
  `docs/spec/` (brain, adaptive-learning, multi-guild, platforms, vision, bot-architecture,
  apple-intelligence, companion-app, discordbm-api, SKILL).
- 2026-08-19 slice landed (gate `./check.sh` green, 282 tests, was 84): pure modules brain/{nn,replay,dqn,intent,state,reward,social,registry}, guild, memory, engine, wyhash (188 Zig ref vectors), embedding (pinned to abi-ai's own vectors), wdbx (WDBX v1 JSONL + guild-namespaced recall), platform (Discord/Telegram/Slack translation), vision (OpenAI-compatible VLM seam), persist (atomic JSON store), pipeline (SocialRouter behind an `Outbound` trait, tested); shells gateway.rs (serenity events, Telegram long-poll, Slack Socket Mode), commands_brain.rs (/remember /forget+autocomplete /recall /reputation /summarize /see /ocr /stats /admin×9), runtime.rs (AppState + scheduler), main.rs wiring (opt-in MESSAGE_CONTENT, ctrl-c persist). Binary verified to start, fail fast, write and reload `ABBEY_DATA_DIR`, and refuse a corrupt state file.
- At this slice, model-initiated tools were still proposed; PR #19 and the "Smarter agent — tools" section below supersede that residual. The remaining out-of-scope items are the Swift companion app + Apple on-device models, voice (no `voice.md` supplied), Postgres/Fluent (file store instead), and Slack HTTP Events (Socket Mode implemented instead).
- Privacy-safe historical evidence from 2026-08-19 records the result categories gateway-ready,
  command success, generated DM and guild-mention replies, quiet-guard enforcement, and delayed
  reward settlement. PRs #10–#14 own that dated snapshot; no Discord identifiers, participant
  identities, prompts, replies, or raw session text are retained here.

## DMs work end-to-end and the smart features are exercised live on Discord
status: done
- Captured 2026-08-19 from the approved DM and smart-feature live-acceptance goal.
- Acceptance: a DM to Abbey gets a generated (not template) reply; the memory/reputation/admin/stats commands answer in a real guild; the pipeline's reply/react/reward path is observed at least once; everything observed is recorded here with what was *not* observed.
- Historical provider check on 2026-08-19: the local loopback backend produced bounded timing and
  failure categories, `ABBEY_BOT_LLM_MODEL` was added, the local budget became 4,096 tokens, and
  reasoning-only replies gained an explicit result category.
- 2026-08-19 slice (PR #12): DMs are one-person namespaces (`discord:dm:<user>`), `/persona` + memory/stats commands DM-capable, `/persona ask` shares the engine transcript + context, forced path loads the brain (rewards no longer dropped), honest failure reply on mention/DM, typing keepalive, mention stripping, `ABBEY_QUIET`, `/admin learning off` now really gates the policy, persona-reviewed prompt wording. **Verified:** DM round-trip against the real model through the pipeline (`cargo test live_dm -- --ignored`: 3 turns, transcript survives). At that dated checkpoint, a mainline binary was observed live under quiet mode; that is not current process, installation, or provider evidence.
- 2026-08-19 outcome: DMs generate replies (3 turns observed, transcript carried, honest failure line on a backend timeout), `/admin export` + `/recall` answered in a guild, guild mentions answered, 👍 → reward settled into the DM user's replay buffer. Not individually exercised: `/reputation`, `/stats`, `/remember` (same command path; the DM slash picker had not yet propagated the re-registration), Telegram, Slack, vision — recorded, not claimed.
- 2026-08-19 privacy-safe result categories: desktop-controlled DM and guild-mention replies,
  `/admin export`, and `/recall` were observed; reaction settlement was still pending at that
  checkpoint. The ledger retains no prompt, reply, participant, or concrete Discord identifier.

## Guild learning loop acts in opted-in servers (sub-project 3 of "improve all")
status: in_progress
- Spec: `docs/superpowers/specs/2026-08-19-guild-learning-loop-design.md`; plan: `docs/superpowers/plans/2026-08-19-guild-learning-loop.md`. Decisions: per-guild `/admin act on` opt-in (default off), per-guild hourly budget (default 6, 1–60), in-memory BrainStats in `/admin brain`, `ABBEY_QUIET` still wins. At capture time, sub-projects 1, 2, and 4 still needed their own records; the sections below supersede that planning note.
- Privacy-safe 2026-08-19 evidence for PRs #16–#17 and commit `101dd96` records the result
  categories opt-in enforcement, policy decisions, reactions, cooldown refusal, delayed rewards,
  and persisted enablement in the sandbox role. The 2026-08-20 checkpoint records a successful
  aggregate brain-state read. An actual `OverBudget` refusal remains pending external acceptance;
  no Discord identifier, participant identity, prompt, reply, or raw session text is retained.

- **2026-09-04 source re-verification.** The `OverBudget` refusal remains correctly recorded as
  pending external acceptance; nothing here is stale. Source wiring re-confirmed at this commit:
  `Outcome::OverBudget` is defined at `src/pipeline.rs:75`, returned at `src/pipeline.rs:169` and
  `:178`, and covered by `src/pipeline/tests.rs:624`. The gap is live observation only, not
  implementation.

## Reply quality & speed (sub-project 1 of "improve all")
status: done
- Spec `docs/superpowers/specs/2026-08-19-reply-quality-speed-design.md`; the dated `docs/benchmarks/2026-08-19-local-models.md` ranked gpt-oss:20b first, gemma4:e4b second, and measured gemma4:12b at 32–94 seconds with heavy reasoning. The latest operator choice supersedes that recommendation and the interim e4b choice: `gemma4:12b` is now the operational default/deployment intent, while every benchmark result remains historical evidence. Landed: tidy_reply shape/length contract; one generation slot per local backend + honest busy line; streaming local replies with post-early/edit-in-place (`stream_reply`, `Outbound::edit` on Discord/Telegram/Slack); one-shot Anthropic→local fallback. Gate 317 tests.
- Privacy-safe 2026-08-19 evidence at `b41783e` records the result categories streaming
  post-early/edit-in-place, tidy output shape, and bounded local latency. Concurrent DM
  serialization and the Anthropic fallback remained source-tested only.

## Smarter agent — tools (sub-project 2 of "improve all")
status: done
- Spec `docs/superpowers/specs/2026-08-19-tools-design.md`. Landed: pure `tools.rs` (5 tools, both wire shapes, both parsers, dispatch/ToolHost), tool-aware `ChatTurn`/request builder/`extract_turn`/streamed tool-call merging in `llm.rs`, `ToolScope` host in `runtime.rs`, `pipeline::generate` loop (max 3 rounds, streams locally, 4xx degrade), `/persona ask` on the same loop, `ABBEY_BOT_LLM_TOOLS`. Gate 326 tests.
- Privacy-safe 2026-08-19 evidence records one validated `remember_fact` call followed by a
  grounded recall result. `switch_persona`, `lookup_reputation`, `recent_messages`, the Anthropic
  wire shape, and 4xx degradation remained unobserved live.

## Breadth & ops (sub-project 4 of "improve all")
status: in_progress
- 2026-08-19: vision works on a local VLM — ollama `gemma4:e4b` described a screenshot correctly; `/v1` needed a 1,024-token budget (reasoning first) and a reasoning-exhausted error is now honest. launchd user agent for this Mac: `deploy/com.donaldfilimon.abbey-bot.plist` + `deploy/install-launchd.sh` (build, install, load; `--uninstall`).
- Historical 2026-08-20 installation evidence recorded the categories atomic launchd replacement,
  owner-only environment and data state, successful gateway/DM/reward paths, persistence reload,
  final-persist handling, permission repair, and rollback. It does not qualify the current source
  or the currently manual process, and retains no raw logs, prompts, replies, or identities.
- 2026-08-20 command-evidence reconciliation: the durable interaction ledger records successful `/stats`, `/remember`, `/reputation`, `/summarize`, `/whois`, `/perms`, `/modcall`, `/server`, `/voice status`, and `/voice leave` executions. `/forget`, `/ocr`, and `/webhook` remain unobserved. A live `/see` invocation reached the older path but failed on attachment MIME/decoding; current source fully decodes JPEG/PNG/WebP/GIF under 8192×8192-pixel and 96 MiB allocation ceilings, preserves validated JPEG/PNG/WebP, and normalizes GIF's first frame to PNG before transport, but needs a fresh live `/see` after deployment. Memory slash commands are self-only by default; cross-member `/remember`, `/forget`, and `/recall` require Manage Messages, Manage Guild, or Administrator, and new facts are normalized, non-empty, and capped at 300 Unicode characters.
- Not done / needs external credentials or live acceptance: Telegram and Slack live (tokens); `/forget`, `/ocr`, and `/webhook`; the hardened `/see` attachment path after deployment; an actual `OverBudget` refusal. GitHub Actions is no longer listed as blocked here: the exact stable-toolchain gate executed successfully on PR #24.
- The 2026-08-27 and 2026-08-28 zero-vulnerability reports were lockfile snapshots and are
  superseded; they are not current audit-clean evidence. Commit `9895734` remains historical
  maintenance provenance only.
- **Current 2026-09-02 TLS debt is explicitly not audit-clean.** The portable Linux tree excludes
  `native-tls`, `openssl`, and `openssl-sys`, but Serenity 0.12.5 still selects
  `tokio-tungstenite` 0.21, Rustls 0.22.4, and `rustls-webpki` 0.102.8. Exactly these four
  vulnerability records remain accepted and visible:
  - `RUSTSEC-2026-0049` / `GHSA-pwjx-qhcg-rvj4`
  - `RUSTSEC-2026-0098` / `GHSA-965h-392x-2mh5`
  - `RUSTSEC-2026-0099` / `GHSA-xgp8-3hg3-c2mh`
  - `RUSTSEC-2026-0104` / `GHSA-82j2-j2ch-gfr8`
- The accepted records are bound by the gate to the exact package, version, source, checksum,
  aliases, patched and unaffected ranges, categories, severity metadata, and dependency identity.
  The malformed-CRL panic advisory stays visible. Any added, missing, or changed vulnerability
  fails closed. The `cargo-audit` 0.22.2 pin is report-format tooling, not accepted debt.
- **2026-09-08 04:4x — the Serenity/Rustls re-review trigger was checked and is NOT met.**
  This section standing-orders a re-review "when Serenity publishes a compatible
  Rustls/WebSocket edge or any accepted advisory evidence changes". Discharged
  with a measured negative rather than left implicit. Serenity's latest release
  on crates.io is still `0.12.5` (published 2025-12-20), so no edge exists and
  the four accepted records stay correctly accepted. Verified unchanged:
  `security/rustsec-accepted-debt.json` still binds exactly `RUSTSEC-2026-0049`,
  `-0098`, `-0099`, `-0104`, all to `rustls-webpki 0.102.8`, and the pinned
  chain still resolves to `serenity 0.12.5`, `tokio-tungstenite 0.21.0`,
  `rustls 0.22.4`, `rustls-webpki 0.102.8`.
- **Correction, and it saves a wasted upgrade: the "serenity 0.12.x / poise 0.6.x"
  framing of this blocker is misleading, because poise is not a blocker at all.**
  `poise 0.7.0` was published 2026-09-06 and is available, but it requires
  `serenity ^0.12.5`, so adopting it cannot clear the TLS debt and cannot unlock
  Components V2. `songbird 0.6.0` likewise requires only `serenity ^0.12.0`.
  `cargo tree --locked --offline -i rustls@0.22.4` shows the whole chain descends
  from Serenity alone: `serenity 0.12.5 -> tokio-tungstenite 0.21.0 ->
  tokio-rustls 0.25.0 -> rustls 0.22.4`; poise and songbird only reach it
  *through* serenity. **Serenity is the sole blocker.** A future session must not
  bump poise expecting either outcome. Whether to adopt `poise 0.7.0` on its own
  merits is an open decision for Donald, not implied by this finding, and no
  version change was made here (`Cargo.toml` and `Cargo.lock` verified clean).
  Note `AGENTS.md`/`CLAUDE.md` still carry the "pinned serenity 0.12.x / poise
  0.6.x" wording for Components V2; correcting those mirrored twins is a separate
  documentation change and was deliberately not made in this ledger-scoped pass.
- The unrelated informational unmaintained warnings for `derivative`, `instant`, and
  `proc-macro-error2` are reported separately. Re-review is required when Serenity publishes a
  compatible Rustls/WebSocket edge or any accepted advisory evidence changes; no local
  cryptographic fork is claimed.
- Direct compatible majors moved to `sha2` 0.11, `base64` 0.23, and `tokio-tungstenite` 0.30.
  Reqwest remains on 0.12 for Serenity feature compatibility, and Symphonia remains on 0.5 for
  Songbird playback/error type compatibility.
- **2026-09-04 03:5x — context menus shipped (code); two roadmap rows corrected.** The Discord
  roadmap's gap table listed context menus as **Missing** with no `context_menu` anywhere in
  `src/`. Both now exist: "Abbey: profile" (USER) renders the same summary as `/whois` through a
  shared `member_profile`, and "Ask Abbey" (MESSAGE) routes a message's own text through the same
  `answer_question` path `/persona ask` uses, so identical text cannot get two different answers,
  cooldowns, or transcript scopes. Both are ephemeral. The message menu does **not** commit to the
  channel transcript (`Commit::No`): an ephemeral exchange must not steer a conversation nobody
  saw it enter, and a right-click must not pull a third party's words into Abbey's context in a
  guild where Abbey holds no message-content access of its own. Empty resolved content is reported
  plainly instead of answered. This is source evidence only — no live invocation is claimed, and
  the unobserved-command list (`/forget`, `/ocr`, `/webhook`, post-deploy `/see`) is unchanged.
- **`/webhook` is not an unimplemented gap; it is a refusal.** The roadmap row read "Guide only,
  no create-webhook call", which reads like unfinished work. `commands::webhook` emits setup steps
  on purpose: a bot-minted webhook URL is a credential the bot would then hold. Reclassified in
  the roadmap as "Guide only, by decision" so no later session "finishes" it.
- **User-install (P3) is not a pure code change, and the roadmap now says why.** Crate support
  exists in the pinned poise 0.6.2 (`Command::install_context` / `interaction_context`), but a
  global bulk overwrite carrying `USER_INSTALL` is rejected until the Developer Portal enables
  User Install — and that overwrite runs in the `ready` callback, so a rejection breaks command
  registration for the running service. Any slice must be off by default behind an operator env
  flag, and must decide per command what a user-installed invocation may touch: `persona ask`
  writes a channel-scoped transcript, so invoking it inside a guild Abbey was never installed in
  would create context for a server that never consented. Unstarted.

- **2026-09-04 gate evidence: the accepted TLS debt is unchanged and the re-review trigger has
  NOT fired.** Verified by running the repository's own gate rather than re-reading the bullet
  above. `scripts/check-rustsec-debt.py` exits 0 with "accepted temporary debt matches: 4
  vulnerabilities remain; audit is NOT clean" and reports the 3 informational unmaintained
  warnings separately (`derivative` 2.2.0, `instant` 0.1.13, `proc-macro-error2` 2.0.1), exactly
  as recorded. `scripts/check-linux-tls-tree.py` reports "linux TLS dependency tree: OK
  (Rustls/WebPKI; native TLS and OpenSSL absent)". `cargo audit --json` returns exactly the four
  accepted records, all against `rustls-webpki` 0.102.8. The cause is still upstream and there is
  no action available: `serenity` is pinned 0.12.5 (`Cargo.toml:19`) and **0.12.5 is still the
  latest published release**, so no newer Serenity exists to move off the vulnerable Rustls. The
  lock resolves both stacks side by side - vulnerable via Serenity (`rustls` 0.22.4,
  `rustls-webpki` 0.102.8, `tokio-tungstenite` 0.21.0) and current for direct dependencies
  (`rustls` 0.23.43, `rustls-webpki` 0.103.15, `tokio-tungstenite` 0.30.0). GitHub's Dependabot
  surface independently reports 4 vulnerabilities on the default branch, consistent with the
  accepted set. No bump was attempted.
- **2026-09-04: `main` has no branch protection, which is the mechanism behind the cancelled-run
  history.** `GET repos/donaldfilimon/abbey-bot/branches/main/protection` returns 404 "Branch not
  protected", so there are no required status checks. The Rust matrix does run on pull requests,
  but cannot block a merge. Observed consequence: PRs #68-#71 merged between 04:00Z and 04:03Z
  and each merge cancelled the previous `main` run (`33835401266`, `33835225139`, `33834312740`
  all `cancelled`), so a formatting break landed and stayed hidden until run `33835423344` was
  the first allowed to finish. Recorded as an observation only; enabling protection is Donald's
  decision and was deliberately not changed.
- **2026-09-04 correction to a contradicting checklist entry.** `tasks/todo.md` carried a
  **checked** box for Telegram/Slack marked "(live)", which contradicts this section, the same
  file's own later entries, and `docs/MLAI-LIVE-ACCEPTANCE.md` ("Telegram / Slack tokens |
  missing | missing"). The box has been unchecked in this pass. Live connector acceptance
  remains blocked on credentials.
- **2026-09-04: `check.ps1` is not full parity with `check.sh`, and only part of the gap is
  documented.** `check.sh` runs `deploy/test-check-launchd-env.py`,
  `deploy/test-smoke-mlx-vlm-tool-deltas.py`, and `deploy/test-patch-mlx-vlm-tool-encoding.py`;
  `check.ps1` runs none of the three. Its header documents only the POSIX/plist omission, so a
  green Windows gate covers less than a green POSIX gate. Recorded, not changed.

- **2026-09-08 05:5x EDT — residual inventory still open on this goal (no status
  flip).** After #103 (ledger) and #104 (voice classic UX **design**), the
  operator/human-gated work this section still does not claim done:
  - **Portal Activity URL map (P0)** — Developer Portal click; bot tokens cannot
    set it (`docs/discord-application-api-roadmap.md` / `docs/activities.md`).
  - **OAuth secret host (P2)** — authorize / authenticated channel name /
    `setActivity` wait on an operator-hosted exchange; secret never in git/Pages.
  - **Components V2** — still crate-blocked on pinned `serenity 0.12.5` /
    `poise 0.6.2` (classic Action Rows only); #104 deliberately stayed classic.
  - Telegram/Slack live tokens; unobserved `/forget`, `/ocr`, post-deploy `/see`;
    an actual `OverBudget` refusal. None of these flip this goal to `done`.

- **2026-09-08 06:1x EDT — residual inventory restated after #106 + #105 merge;
  no status flip.** Tip of `main` is now
  `1b4822d1a3b017b65be6cf0afccf8b85e10890c5` (#105 merge). #106 (`143fc79`) and
  intermediate docs tip `85d0e18` sit beneath it. #105 source landing does
  **not** close any of:
  - **Portal Activity URL map (P0)**
  - **OAuth secret host (P2)**
  - **Components V2** (crate-blocked; classic Action Rows only)
  - Telegram/Slack live tokens; unobserved `/forget`, `/ocr`, post-deploy `/see`;
    an actual `OverBudget` refusal.
  Voice classic UX A→B→C is **merged source** on `main`, with live redeploy in
  flight and **no** installed-artifact or human-witnessed acceptance claim.

- **2026-09-08 06:4x EDT — residual inventory restated after live Action Row UX
  redeploy; no status flip.** Tip of `main` at this writing is
  `7137d866ccc7c4edea5c13a1371d9b5af22b733a` (#107 ledger merge). Beneath it:
  #108 (`6f678ac`, agents docs), help harden `b6358c7`, #105 merge `1b4822d`,
  #106 `143fc79`. Live binary SHA prefix `6084c7f2` / PID 58490 / Discord connected
  is recorded above. Redeploy does **not** close any of:
  - **human-witnessed voice** (audible wake/reply, barge-in, leave/stop)
  - **two-guild** isolation observations
  - **consent** observations (fresh unanimous agreement)
  - **installed artifact identity qualification** (redeploy observed ≠ qualified)
  - **Portal Activity URL map (P0)**
  - **OAuth secret host (P2)**
  - **Components V2** (crate-blocked; classic Action Rows only)
  - Telegram/Slack live tokens; unobserved `/forget`, `/ocr`, post-deploy `/see`;
    an actual `OverBudget` refusal.


## Self-learning hardening (continuation of "improve all")
status: done
- 2026-08-19: replay buffer (last 1,000 experiences) and pending rewards persist across restarts; rolling channel summaries for opted-in guilds/DMs every 30 messages (scheduler, 10 min), grounding replies. Gate 331 tests. Live observation of a refreshed summary needs 30 new messages in an invited channel — not yet seen.

## Modernize the Rust codebase and harden network boundaries
status: done
- Captured 2026-08-19 from the approved modernization and mainline-delivery goal. Acceptance:
  exact supported Rust toolchain, compatible lock refresh, deployed release build in the gate,
  generated Discord mentions disabled, bounded/no-redirect model and vision responses, bounded
  shared Discord attachment downloads, pure hierarchy policy, clone-independent hooks,
  secret/data ignores, claim-honest docs, full green gate, and integration on `main`.
- 2026-08-20 outcome: implementation commit `8da4717` merged through PR #24 as `24c95d2`; the exact stable Rust 1.97.1 GitHub gate passed (fmt, Clippy `-D warnings`, 335 passed + 1 intentionally ignored live-model test, locked release build). Startup rejects blank Discord tokens and remote plaintext model endpoints. `cargo audit` still reports the four documented `rustls-webpki` 0.102.8 advisories blocked on Serenity's Rustls 0.22 path, plus Poise's transitive unmaintained `derivative` warning; Docker/systemd and live mention/cooldown behavior remain explicitly unverified.
- 2026-08-20 dependency recheck (Modernize goal): Rust 1.97.1 remains the current stable release and `cargo update --dry-run` resolves no compatible changes. Serenity 0.12.5 and Poise 0.6.2 remain current on crates.io; upgrading only reqwest/tokio-tungstenite would duplicate TLS stacks without removing Serenity's rustls 0.22 path. The complete locked gate passed again before the launchd restart. The four upstream `rustls-webpki` advisories therefore remain explicit rather than being hidden behind an unsafe fork or a deploy-stack TLS rewrite.

## Complete all unfinished .md files
status: done
- Scanned all .md files in the repository for "TODO", "FIXME", "unfinished", "TBD", "incomplete", and "..." markers.
- Verified the content of all `docs/spec/` and `docs/superpowers/` files.
- No obviously unfinished documentation or placeholders were found; all structural and architectural references are complete.

## Complete Abbey: MLX Gemma 4 12B, vision, tools, voice, and cross-platform support
status: in_progress

- Captured 2026-08-20 from a full written specification. One coarse intention: make the pinned
  MLX-VLM `mlx-community/gemma-4-12B-it-4bit` sidecar the qualified macOS primary for text,
  structured tool calls, `/see`, `/ocr`, and voice cognition; add a capability-gated Apple
  Foundation Models secondary; keep Linux and Windows on the portable OpenAI-compatible Gemma
  contract; and preserve the shared Discord/Telegram/Slack pipeline. Voice stays Discord-only and
  read-only (no tool or memory mutation from spoken turns).
- Explicit non-goals, recorded so later readers do not infer them: Discord Go Live / stream video
  is **not** ingested. "Vision" means uploaded JPEG, PNG, WebP, and GIF plus OCR — not continuous
  capture. Windows is foreground execution plus CI, not a Windows Service. No new local
  cross-platform STT/TTS stack: non-Mac voice remains the explicitly selected
  `ABBEY_VOICE_MODE=openai` backup, never auto-selected merely because a key exists.
- Relationship to the existing `## Full-duplex Abbey voice in Discord Engineering` goal: that goal
  keeps ownership of the live consent/epoch/barge-in acceptance. This goal owns the provider
  architecture, FM gating, vision/tool safety, and cross-platform surface around it. Neither is
  closed by the other's evidence.
- The FM provider is gated per capability, never wholesale. The observed FM server accepted text
  and image requests but silently turned an OpenAI custom-tool request into prose with no
  `tool_calls`, so its server endpoint must never be advertised as tool-capable; tools go through
  the schema-constrained `fm respond` adapter, and Abbey's allowlist/validation stays authoritative.
  Provider prose that merely claims an action, without a validated tool request, must not mutate
  memory or report success.
- **2026-08-20 slice 1 of 5 complete — reconcile and stabilize the concurrent candidate.** Verified,
  not assumed: branch `codex/live-voice-20260820` @ `ed7dc66`, 26 modified + 6 untracked files
  (+3548/-553), quiescence confirmed (no non-`.git` file touched in the preceding 10 minutes) before
  any inspection. The dirty worktree was preserved byte-for-byte — this slice made no source edits.
  MLX-VLM subsystem audited against all five required properties and all five hold: requirements are
  reproducibly hash-locked (1294 `--hash=sha256:` entries from `uv pip compile --generate-hashes
  --only-binary=:all:`); the model revision is exact (`73bcf09092aa277861d5a191b989b666f7f32e8f`,
  and the runner fails closed when that snapshot is absent); runtime is offline (`HF_HUB_OFFLINE=1`,
  `TRANSFORMERS_OFFLINE=1`, telemetry disabled); the endpoint binds `127.0.0.1` only and unsets
  every proxy variable so loopback traffic cannot be intercepted; and rollback is non-destructive —
  the previous install is **moved** to `.backup.$STAMP`, failed candidates to `.failed.$STAMP`, and
  an un-completable rollback fails loudly with retained backups rather than deleting anything. The
  sole destination-touching `rm -f` is inside the explicit `--uninstall` branch, which still retains
  the model cache and venv. `AGENTS.md` and `CLAUDE.md` are already verbatim mirrors (0 diff lines
  beyond their titles) — no reconciliation was needed. Memory bounds match the 24 GB M5 constraint:
  `--max-num-seqs 1`, `--vision-cache-size 4`, `--max-tokens 4096`.
- **Gate rerun after the MLX-VLM changes (supersedes the pre-MLX-VLM 405-test evidence):**
  `sh check.sh` green end to end — fmt, deploy syntax (including the three new MLX-VLM shell files,
  the smoke `ast.parse`, and `plutil -lint` on all three plists), Clippy `--all-targets --locked
  -D warnings`, **408 passed / 0 failed / 1 ignored** (the intentional live-backend test), and the
  locked release build, with in-log `== ok ==` and `GATE_EXIT: 0`.
- Open and honestly unclaimed: MLX-VLM semantic smoke (streamed text, forced tool call,
  tool-result continuation, vision and OCR fixtures, offline restart) has **not** been executed
  here — the source gate is not semantic evidence. The FM provider, capability layer, and
  `--provider-self-test` are unimplemented. Cross-platform CI (`macos`/`ubuntu`/`windows`) and the
  PowerShell gate are not yet added. All live Discord acceptance — fresh consent, audible reply,
  barge-in, membership pause/resume, authoritative stop, leave — requires human participants and an
  authorized manager, and will remain pending rather than be substituted with source tests, MLX
  access logs, or historical consent. `cargo audit` stays deliberately non-green: the
  `rustls-webpki` and DAVE/OpenMLS/libcrux advisories remain documented, not hidden behind a
  hand-maintained cryptographic fork.
- **2026-08-20 slice 2 — memory relevance (branch `codex/memory-revision-20260820`).** Fixed a
  concrete defect rather than adding a layer: `PersonaContext::render` joined *every* stored fact
  into every prompt, so at the hundred-fact cap an unrelated query carried up to 30,000
  characters of biography. New pure `src/recall.rs` ranks facts against the
  message being answered — lexical overlap weighted by term rarity across that user's own facts —
  with no embedding call, no network, and nothing on the hot path that can stall. Ranking is not
  forgetting, enforced three ways: the prompt discloses held-back facts as "(+N more remembered
  facts not shown for this message)" so the model never mistakes a partial view for the whole
  file; a short fact list renders whole regardless of wording; `/recall` still lists everything and
  `/forget` remains the only deletion. A snapshot test caught a real flaw in the ranking itself —
  rarity weighting made a stopword look maximally distinctive because it appeared in exactly one
  fact, floating an unrelated fact to the top; the cheap fix (drop tokens
  under three characters) would have destroyed `go`, `ai`, `js`, `c`, and `os` as retrieval keys,
  so it is an explicit stopword list with regression tests in both directions. Gate: 428 passed /
  0 failed / 1 ignored, exit 0 (was 415).
- **2026-08-27 fact supersession landed (the item above is now built, as a hybrid).** An
  explicit `replaces` on `/remember` is AUTHORITATIVE and removes the named fact atomically;
  a model-proposed `supersedes` on the `remember_fact` tool only QUEUES a `PendingSupersession`
  and removes nothing, until a human confirms via `/pending confirm`. There is deliberately no
  model-callable path to the authoritative method — a model must not confirm its own contested
  claim. `PendingSupersession` lives only in the JSON `UserMemory`, never on `FactRecord`, whose
  bytes are pinned by a fixture the sibling `../wdbx` repo also owns; `MEMORY_PROJECTION_VERSION`
  is deliberately NOT bumped because the field is additive with `serde(default)`, so an older
  binary loses advisory proposals but never facts. `./check.sh` green, 655 tests.
  An independent review of the landed diff confirmed the central property — no model-callable
  path deletes a fact — and found three real issues, all since fixed: a stale-premise gap where
  confirming a proposal whose REPLACEMENT had been separately forgotten would have left the user
  holding neither fact (now refused as `PremiseGone`, with a regression test); a rollback branch
  that mutated `stores` and returned without reconciling the WDBX projection; and two literal
  runs of whitespace in user-facing strings. The review also proved the rollback restore itself
  cannot fail, since after the forget the list is de-duplicated and one under the cap.

- **2026-08-27 embedding-backed recall ranking: MEASURED AND REJECTED, not deferred again.**
  This had been carried as "a later slice." It is now closed as not viable on this embedding,
  with data rather than opinion. `src/embedding.rs`'s algorithm and `src/wyhash.rs` were
  reimplemented and validated against all 5 pinned wyhash reference values and all 3 pinned
  golden embedding vectors (to 3e-8) before any measurement was trusted. Across 1,200 unrelated
  fact/query pairs the cosine distribution runs median 0.133, p95 0.429, max 0.611 — while
  genuine paraphrases score as low as 0.108 (`uses rust` vs `I use Rust`) and 0.110
  (`moved to zig` vs `switched to Zig`), the two examples the feature existed to catch. One pair
  sharing its single most distinctive term (`kubernetes`) scored NEGATIVE at -0.071. The
  populations overlap with no separating threshold: a floor of 0.10 admits 62% of pairs sharing
  zero meaning, and a floor of 0.40 discards 14% of genuinely related ones. No floor preserves
  both the paraphrase case and the exact-zero property that `unrelated_facts_still_fill_remaining_slots_newest_first`
  and `a_short_fact_list_is_never_trimmed_by_focusing` depend on. This is a property of a
  non-learned 32-dimensional feature hash with signed bucket collisions, so it cannot be tuned
  away; raising `EMBED_DIM` would change the persisted vector format shared with abi. Retrieval
  stays lexical. Re-opening this requires a different embedding, not a threshold.

- **Deliberately not built, with the reason recorded:** automatic contradiction detection over
  free-text technology-preference updates. Deterministic supersession over free text mis-supersedes
  real facts, and silently losing a user's memory is a worse failure than showing one stale line.
  It needs either an explicit `replaces` parameter or a model-judged path — each its own decision,
  neither a guess to slip in under "smarter memory".
- **2026-08-20 parallel dispatch (three isolated worktrees, results pending):** (1) a routing
  signal layer composing *on top* of the canonical `persona.rs` — its keyword table, weights,
  prior, and tie order are a verbatim abi-ai transcription and must not drift, so distress and
  urgency detection is additive and explicit selectors stay absolute; (2) a delayed-outcome reward
  path in `src/brain/*`, since the DQN currently learns only from its own immediate heuristic and
  never observes what the human did next; (3) a lexical grounding guard flagging specifics
  (versions, dates, statistics, quotes) asserted in a reply but absent from the supplied grounding
  — explicitly a lexical check, not a hallucination detector, and required to test the
  false-positive direction because a guard that flags numbers the user supplied is worse than none.
- **2026-08-28 reconciliation: items (1) and (3) above landed; the "results pending" framing is
  superseded for those two.** Verified against source, not assumed. Both feature branches are
  merged into `main`: `e3422ef` ("Merge branch 'codex/routing-signals-20260820'") and `ac05930`
  ("Merge branch 'codex/grounding-guard-20260820'"). (1) `src/routing_signals.rs` (885 lines from
  that merge) exists and is wired in: `src/main.rs:73` declares `mod routing_signals;` and
  `src/pipeline.rs:99` calls `let composed = routing_signals::route(text, None);`. (3)
  `src/grounding.rs` (1,177 lines from that merge) exists and is wired in: `src/engine.rs:20`
  imports `crate::grounding::Grounding`, `PreparedTurn` holds a `grounding: Grounding` field
  (`src/engine.rs:77`) populated via `Grounding::from_sources` at `src/engine.rs:131`; consumption
  lives in `src/generation.rs`, where `apply_grounding` (line 250) and `finalize_reply` (line 255)
  call `grounding::check`/`grounding::hedged` and are invoked from both the streaming and
  non-streaming tool-round paths. Item (2), the delayed-outcome reward path, is not part of this
  reconciliation — it landed separately via merge `4c85646` ("Merge branch
  'feat/delayed-outcome-reward'") and was already correctly accounted for elsewhere in this file.
- **2026-08-21 ledger reconciliation.** PRs #25–#31 landed after the entries above without those
  entries being updated, so several "Open and honestly unclaimed" claims from 2026-08-20 slice 1
  were stale by the time this was checked. Verified against actual source and a real binary run,
  not assumed: (1) `.github/workflows/rust.yml` runs the `ubuntu-24.04` / `macos-15` /
  `windows-2025` matrix on every push and PR — the "cross-platform CI … not yet added" line is
  false as of this repo state; (2) `--provider-self-test primary|fm|all --json` is implemented and
  was run here under `env -i` (no inherited environment, no `DISCORD_TOKEN`, no `ABBEY_DATA_DIR`):
  `primary` correctly reports `configured:false` and exits 2 without touching Discord or a data
  directory; `fm` with `ABBEY_FM_MODE=system` against this Mac's real `/usr/bin/fm` (macOS 27,
  build `26A5416b`) reports `text`/`structured_output`/`tools` as `pass`, bound to
  `cli_sha256`/`abbey_binary_sha256` identity, and `vision`/`ocr` as `fail` with
  `category":"semantic_vision"`/`"semantic_ocr"` — the self-test fails closed on the real semantic
  check rather than a bare-connectivity pass, which is the FM vision/OCR gating requirement working
  as designed. This closes both remaining `--provider-self-test` and FM-vision/OCR-gating items in
  `tasks/todo.md`. It does **not** mean FM vision/OCR are production-qualified on this build — they
  are not, per the same evidence, and must not be advertised as such.
- **Cross-platform evidence is commit-specific.** The older `588cbe6` / Actions run
  `33025176982` result is historical only. Immediately before this stabilization wave,
  `origin/main` was `9716f00`; Actions run `33218303755` supplied Ubuntu, macOS, and Windows
  source-gate evidence for that pre-stabilization baseline only. Neither run proves the current
  local stabilization commits, final pushed head, provider/runtime qualification, installed
  artifact identity, live connectors, managed deployment, or consented voice. Fresh exact-head
  three-platform CI is pending after the normal push to canonical `main`.
- **2026-09-02 Core + Inspect source surface.** Production offers exactly seven tools in stable
  order whenever the global tools policy is enabled: the original five Core tools followed by
  `inspect_status` and `list_facts`. `abbey_tools()` remains the byte-compatible five-tool corpus.
  Both OpenAI-compatible/Anthropic and Foundation Models decision schemas expose the seven-tool
  production surface. The partial Inspect-only toggle no longer exists; only the global
  tools-off boundary can hide tools. Inspect is read-only, guild/user scoped, non-provisioning,
  snapshot-consistent, and returns only effective routable provider capabilities with safe
  configured-versus-qualified provenance. HTTP and Discord acting tools remain deferred.
- **Current evidence boundary and delivery.** The source-level coarse voice state is wired to
  central lifecycle transitions and limited to `off`, `presence`, `awaiting-consent`, `active`,
  or `paused`. The existing manual foreground process remains untouched and unqualified (see 2026-09-04 reconciliation: now a launchd service, still unqualified). The
  isolated strict gate, locked release build, non-divergence review, normal push from canonical
  `main`, and exact-head Ubuntu/macOS/Windows CI remain pending. Provider qualification,
  installation, two-guild live acceptance, managed-service acceptance, and consented voice are
  separate pending layers.
- **Live protocol roles.** With operator-supplied sandbox inputs, Guild A starts with learning and
  acting enabled under a small budget/cooldown while Guild B remains default-off. Exercise all
  seven tools and bounded policy/provider behavior, prove no cross-guild leakage or unsolicited
  Guild B behavior, swap the Guild A/B roles, repeat the isolation-sensitive subset, and restore
  both guilds' initial settings. No concrete Discord identifier belongs in this ledger.
- **2026-09-04 reconciliation. Three claims in this goal are STALE and are corrected here; three
  have CHANGED.** Verified against source and live command output, not assumed. Earlier bullets
  are left intact above; this entry supersedes them where they conflict.
  - **STALE:** "The FM provider, capability layer, and `--provider-self-test` are unimplemented."
    All three exist - `src/main.rs:72` declares `mod provider_self_test;`, dispatch is at
    `src/main.rs:621-634`, and the full `src/provider/` module is present. The 2026-08-21 entry
    already reconciled this, but the original bullet was never annotated, so a reader hitting it
    first is misled.
  - **STALE:** "Cross-platform CI (`macos`/`ubuntu`/`windows`) and the PowerShell gate are not yet
    added." Both exist. `.github/workflows/rust.yml` runs `ubuntu-24.04` and `macos-15` through
    `./check.sh` and `windows-2025` through `./check.ps1`; `check.ps1` is 52 lines and is
    genuinely invoked. Runner labels have not drifted from the values recorded on 2026-08-21.
  - **STALE:** "`cargo audit` stays deliberately non-green: the `rustls-webpki` **and
    DAVE/OpenMLS/libcrux** advisories remain documented." Only the four `rustls-webpki` records
    remain. The `[patch.crates-io]` entry for `openmls_rust_crypto` (`Cargo.toml:44-49`) removed
    the HPKE/libcrux advisory path, so naming those alongside the accepted set overstates current
    debt.
  - **CHANGED:** "MLX-VLM semantic smoke ... has **not** been executed here." It has been executed
    and partly failed, which is a stronger and more useful result than "unrun". Per
    `docs/MLAI-LIVE-ACCEPTANCE.md` (2026-09-03 ~17:15 ET), `probe_status` was forced and
    `MLX_READY` streamed; tool-result continuation loops `<|channel>thought` into content until
    `finish_reason=length`, so that item is a recorded **FAIL**, and `:8282` stays unpublished
    with the installer failing closed. The vision fixture, OCR fixture, and offline-restart items
    remain genuinely unknown - no result was found either way.
  - **CHANGED:** "The existing manual foreground process remains untouched and unqualified." It is
    a launchd service, confirmed read-only via `launchctl list`: `com.donaldfilimon.abbey-bot` at
    PID 66700. Still unqualified; no longer manual. See the voice section for the full note.
  - **CHANGED:** "Fresh exact-head three-platform CI is pending." It is **red**, not merely unrun.
    At `057e6b1` (PR #71, adaptive routing wave, +754 lines across `src/provider/`), run
    `33835423344` failed on Gate (Ubuntu), Gate (macOS), and Gate (Windows), all at step 1,
    `cargo fmt --all -- --check`, on `src/provider/adapters.rs`, `src/provider/routing.rs`, and
    `src/provider/routing_tests.rs`. A formatting-only fix is in draft PR #72; the full gate at
    that tree is green end to end - fmt, deploy/privacy validation (81 contract artifacts, 3
    plists), clippy `--all-targets --locked -D warnings`, **786 passed / 0 failed / 2 ignored**,
    locked release build, `== ok ==`, exit 0. Formatting was the only defect in `057e6b1`. **This
    item closes only when `main` itself is green at its exact head after #72 merges; a green PR
    run does not close it.**
  - **Still correct, unchanged:** the seven-tool production surface and its stable order
    (`src/tools.rs:99-187`, consumed by `src/generation.rs:657` and
    `src/generation/foundation_models.rs:101`, with `abbey_tools()` still the five-tool
    byte-compatible corpus); the coarse voice Inspect vocabulary (`src/inspect.rs:14-32`, exactly
    five variants, `render_voice` at `:227-229` emitting only the label); and "installed artifact
    identity pending" - the deployed binary is `15c0f15`, behind both `ec2901a` and `057e6b1`.
- **2026-09-04 evidence-boundary note: a gate run from a git worktree is one layer thinner than a
  canonical-checkout run.** `scripts/check-wdbx-conformance.py` reports SKIP under
  `.claude/worktrees/<name>/` because the sibling resolves as `<worktree>/../wdbx`, i.e.
  `.claude/worktrees/wdbx`, not `~/dev/active/wdbx`. The repository-local writer pin stays active
  so the run remains valid, but a worktree gate must never be cited as external WDBX fixture
  evidence.
- **2026-09-04 CLOSED: exact-head three-platform CI is green.** #72 (`e0825b9`) restored the gate,
  #73 (`f4a338b`) and #74 (`cd37eb9`) followed, and `main`'s head `cd37eb9` completed run
  `33850790233` with Gate (Ubuntu), Gate (macOS), and Gate (Windows) all passing. As predicted by
  the workflow's `cancel-in-progress` concurrency group, `f4a338b`'s own run (`33850515754`) was
  cancelled by the next merge; the evidence is the run at the *final* head, which is the one that
  matters. This closes the "exact-head three-platform CI" item in this goal and unblocks stage 0 of
  `docs/live-test-protocol.md`. It does not qualify the provider, the installed artifact, live
  connectors, managed deployment, or consented voice — those remain separate pending layers.
- **2026-09-04 root cause for the recorded tool-result continuation FAIL, in draft PR #77**
  (`docs/superpowers/specs/2026-09-04-mlx-vlm-tool-continuation-diagnosis.md`). Diagnosis only:
  no sidecar was started and no 12B checkpoint was loaded. Verified against the pinned snapshot's
  own `chat_template.jinja`, two defects compose. (1) The template's thought-suppressor (the
  pre-closed empty `<|channel>thought` block at line 362) is gated on `prev_message_type` not being
  `tool_response`, but the reset at line 218 sits inside the `role != 'tool'` guard at 217, so a
  tool message never clears it and the suppressor is skipped: the model opens its own thought
  channel. That is why the plain `MLX_READY` probe passes and only continuation fails. (2) The
  server's thought-splitter is a one-shot latch, so every block after the first is emitted as
  content with markers intact. Ruled out with reasons: a client `stop` sequence is silently ignored
  (`extra="allow"`), `enable_thinking`/thinking-budget cannot act while thinking is off, and
  Rust-side marker stripping alone still burns the budget and returns `length`. The spec ranks the
  candidate fixes and records two gaps: the smoke's JSON fixture is not representative of Abbey's
  prose tool results, and `configure-mlx-primary.py` gates on a manifest's self-declared `tools:
  pass` rather than the exact-marker assertion. One open question stands: whether the recorded
  "generation-prompt experiments also failed" already covered the prefill-after-tool-response
  candidate. `tasks/todo.md:108` still closes only on a live `deploy/smoke-mlx-vlm.py` run passing
  `TOOL_CONTINUATION_READY`; not on the spec, and not on a template patch with a green unit test.
- **2026-09-04 (later) CLOSED again at a new head: `main` `be2915f` is green on all three
  platforms (run `33882682404`) and the Pages build is green (run `33882680758`).** The path there
  is the lesson. #77 (`cff28c3`), #76 (`4681509`), and #78 merged in sequence after Donald's #79
  (`12df0e4`), each green against its own base. `main` at `4681509` was then **red on all three
  platforms** (run `33879118089`): #79 built `VoiceConfig` with a struct literal and #76 made two
  of its fields private the same morning, so the test binary no longer compiled. The Pages build
  failed at the same head because the diagnosis spec from #77 quotes Jinja that Jekyll's Liquid
  parser rejects (a Jinja `set` tag), and this repository has no `_config.yml` or `.nojekyll`. #80
  (`be2915f`) fixed both: the fixture now uses `VoiceConfig::selected_only`, and the two quoted
  blocks sit inside Liquid raw tags. Two individually green PRs producing a red `main` is the concrete
  argument for requiring the three Gate checks on `main`; that is a repository setting and
  Donald's call. As before, this closes only the exact-head CI item; nothing here qualifies the
  provider, the installed artifact, live connectors, managed deployment, or consented voice.
  **Superseded before this line merged:** `main` then moved to `ac909b9` (#83, Donald's
  member-consent feature) and run `33885035333` is **red on Gate (Windows)** in
  `audible_playback_still_stops_on_speech` (Ubuntu and macOS passed). That is the shared-scheduler
  test defect diagnosed in the voice section; PR #85 restores the gate and its merge is Donald's.
  That exact-head item closed at `53836c0`: run `33891829308` passed Ubuntu, macOS, and Windows
  after #85 landed. The merge of this ledger repair will create a newer `main` head whose push
  gates remain a separate requirement.
  **And again, same mistake, same day:** the Pages build failed at `bb65c81` (#84) and `a4ca221`
  (#85) because the ledger bullet above quoted the Jinja tag and the Liquid raw tag literally, and
  Jekyll parses `tasks/goals.md` too. Fixed by rewriting those tokens as prose and by adding
  `scripts/check-pages-liquid.py` to the gate, which fails on any Liquid-looking token in Markdown
  outside a raw span, so the gate catches this before Pages does.
  The exact merged repair head `20e9d83` then closed both upstream layers: Pages run
  `33894994514` succeeded, and Rust run `33894995541` passed Ubuntu, macOS, and Windows. A
  follow-up selector audit found that the first preventive gate modeled only `.md` and scanned
  indexed metadata that Jekyll does not render. The follow-up now pins the observed Pages v232
  source set, covers all five case-insensitive Jekyll Markdown suffixes, mirrors default entry and
  optional-front-matter/readme-index behavior, and fails closed when configuration, front-matter
  routing, symlinks, or Jekyll magic directories invalidate that model. Its merge will again create
  a newer head whose hosted gates must be evaluated separately.
- **2026-09-04 (evening): the newer head `3c13589` is green on both hosted gates, as the line
  above requires.** Rust run `33932415417` passed Ubuntu, macOS, and Windows; Pages run
  `33932414418` passed. Windows matters most here: it is the runner that failed twice
  (`ac909b9`, `bb65c81`) on `audible_playback_still_stops_on_speech`, and it has now passed at
  `53836c0`, `20e9d83`, and `3c13589` carrying #85's private-scheduler fixture, so the
  shared-scheduler diagnosis holds on the platform that exposed it rather than only locally.
  Open at the time of writing: #82 (the `/voice mode` idle guard, switch serialization, and the
  Codex P1 fix, green on all three platforms at `8da591c`, merge Donald's). As before, this
  closes only the exact-head CI item; nothing here qualifies the provider, the installed
  artifact, live connectors, managed deployment, or consented voice.
- **2026-09-04 night: the exact-head item is OPEN again, and this is the third direct-push break
  in one day.** The bullet above is dated evidence, not current state: `main` has been red since
  `e39e13c`, a direct push adding the private `/help` surface and a macOS audio tap. It failed the
  gate four separate ways, and the first one hid the rest because `cargo fmt` is step one of
  `check.sh`. (1) `src/audio_tap.rs` declared `#[cfg(test)] mod tests;` without committing the
  file, so the test binary would not compile; repaired in #89, which supplies ten tests covering
  the endpoint validator, the one-frame push contract, the s16-to-f32 conversion, the queue bound,
  close, the freshness window, and the error strings. (2) 27 `cargo fmt` diffs across six files;
  repaired in #91. (3) `clippy::chunks_exact_to_as_chunks` in `audio_tap.rs`, which is **not** dead
  code and would therefore survive the wiring; #91 was merged at its first commit before that fix
  was pushed, so it is still open as #93. (4) clippy dead code for `AudioTapClient`, `TapStream`,
  `Script`, `gate`, `play`, and `pause`: `PcmBuffer` has a consumer in `voice_session/music.rs` but
  the client half has none, because the command that would drive the tap is not written. That
  fourth one is a design decision rather than a repair and is deliberately untouched; after #93 it
  is the only gate failure left. The exact-head item closes again on a green run at a head that
  carries both.

  Counting the pattern, because it is the argument: `057e6b1` (#71) landed unformatted; `4681509`
  went red because #79 and #76 were each green against their own base and broke only in
  combination; `e39e13c` went red four ways. Three breaks in one day, all on a branch with no
  protection, two of them from pushes that never ran the documented gate. Requiring the three Gate
  checks on `main` remains a repository setting and Donald's call, now with three dated instances
  behind it rather than one.
- **2026-09-04 night: a review of the merged `/voice mode` work found and fixed a real defect in
  it (#92, `975c295`).** `/voice mode` checked "is a start pending" twice through two mechanisms,
  one of which the code itself called advisory: `snapshot.start_pending` in a pure blocker that
  returned a rendered sentence, and `pending_start_generation` in a `bool`-returning writer. The
  writer discarded its reason, so the caller had to invent a sentence, and the identical refusal
  text ended up written twice and kept in sync by hand. `switch_effective_mode` now checks every
  rule and writes in one critical section, returning `Result<(), ModeSwitchRefusal>`; the
  reservation, the media epoch, and the verification run are all read under `activation_gate`
  rather than from a snapshot. The refactor surfaced a case the old shape could not express, since
  activation clears the pending-start token: an open media epoch is now its own refusal with its
  own test. Net lines are flat; the reduction is in concepts. Standing follow-up, not yet done:
  `src/voice_session.rs` is 1283 lines and the mode-switching cluster belongs in
  `src/voice_session/mode.rs` beside the existing `control.rs`, `playback.rs`, and
  `verification.rs`. That is a pure move, and it is being held until the in-flight music feature
  stops editing the same file.
- **2026-09-05 00:40: `b9df963` went red on all three platforms with the Mac gate fully green,
  which is a different failure class from the three direct-push breaks above.** That merge landed
  the music wiring and cleared the dead-code blocker; `./check.sh` passed end to end here before the
  push (940 tests, release build). CI run `33944219413` then failed three ways that a macOS gate is
  structurally unable to see. (1) `voice::tests::music_command_channel_config_is_optional_nonzero_and_requires_voice_scope`
  built its fixture from `destination()`, which selects the default `local` mode, and `local` fails
  closed off macOS before the music channel is ever compared; Ubuntu and Windows both panicked at
  the unwrap. The fixture now pins `mode: disabled`, the one mode every platform accepts, which is
  honest because the channel id is parsed before the mode. (2) `command_catalog::tests::readme_generated_region_matches_catalog_exactly`
  compares `include_str!("README.md")` byte for byte with the generator's LF output; the Windows
  runner checks out with `autocrlf`, so the whole file arrived as CRLF. `.gitattributes` now pins
  `README.md text eol=lf` beside the existing fixture rule. (3) `tools/abbey-audio-tap`'s
  `ServerTests.swift:126` destructured a tuple straight out of `NSLock.withLock`'s generic result,
  which the local Swift 6.4 infers and the runner's older compiler does not. The binding now carries
  an explicit `(DispatchQueue?, DeferredSource?)` type. That one is verified against a real older
  compiler rather than reasoned: Swift 6.3.2 rejects the original line at the same column and
  accepts the fixed one, and the package passes on both 6.3.2 and 6.4. The exact-head item stays
  open until a three-platform run is green at a head that carries this fix; the local gate is the
  Mac layer of the evidence ladder, and this is the recorded instance of why it is not the last rung.

- **2026-09-08 04:4x — the standing `voice_session.rs` follow-up rests on a STALE
  line count, and that changes its priority rather than just its wording.** The
  bullet above defers moving the mode-switching cluster to
  `src/voice_session/mode.rs` and gives the reason as "`src/voice_session.rs` is
  1283 lines". Measured now: it is **887 lines**. Decomposition already happened
  around it, and `src/voice_session/` now holds `activation.rs`, `control.rs`, a
  `control/` directory, `music.rs`, `ownership.rs`, `playback.rs`,
  `verification.rs` and `tests.rs`. `python3 scripts/check-rust-module-size.py`
  reports the whole tree passing, with `voice_session.rs` in the 800–1000
  review-advisory band (alongside `commands.rs` 854, `provider.rs` 962,
  `provider/manifest.rs` 900, `offline_voice.rs` 818), not in violation of the
  <1000 hard rule.
  So the move is no longer needed to satisfy any gate; it is an optional tidy.
  The mode cluster is still in place (`ModeSwitchRefusal` at :247,
  `effective_mode` at :416) and `mode.rs` still does not exist, so the follow-up
  is genuinely undone — it is the justification that expired, not the work.
  The original blocking condition IS now clear: the follow-up was held "until the
  in-flight music feature stops editing the same file", and both
  `src/voice_session/music.rs` and `src/voice_session.rs` were last touched
  2026-09-06 (`c22a4e6` / `df0a5ad`), with the tree idle at this reading (no
  source file modified in 60 minutes, no `cargo`/`rustc` running, no merge or
  rebase in progress).
  **Deliberately not performed in this pass, and this is a named stop, not an
  oversight.** It is a pure move inside consent- and epoch-sensitive voice code
  whose only remaining motive is tidiness; it cannot be closed by the Mac gate
  alone and would need its own three-platform run, which this ledger records four
  separate times as the rung that actually catches things. Whether that CI cycle
  is worth an optional decomposition is Donald's call, not a gap to be quietly
  filled by a session whose mandate was ledger continuation.
- **The exact-head item in this section was satisfied at `eeb717b`, which is the
  parent of this ledger commit and not whatever `main` becomes when it lands.**
  Its closing sentence reads "The exact-head item stays open until a
  three-platform run is green at a head that carries this fix". `b9df963` is an
  ancestor of `eeb717b` (`merge-base --is-ancestor` verified), and `eeb717b` is
  green on macOS, Ubuntu and Windows (run `34179977713`, recorded under the
  modernization goal below). So the three fixes that pass carries — the
  `mode: disabled` music fixture, `README.md text eol=lf`, and the explicit
  `(DispatchQueue?, DeferredSource?)` binding — are confirmed on a green
  three-platform head. Scope, corrected after review: this is SHA-bound per
  lines 99–100 and `docs/live-test-protocol.md` stage 0, so the head produced by
  merging this PR needs its own run and inherits nothing from `eeb717b`. It also
  closes that CI rung only and establishes nothing about live acceptance.

- **2026-09-08 05:5x EDT — `#103` and `#104` landed on `main`; neither closes
  live acceptance.** `9abab67` is the #103 ledger squash (green three-platform
  evidence bound to parent `eeb717b`, Serenity re-review negative, stale
  `voice_session` line-count correction). `df6a3b6` is the #104 voice classic
  UX **design** squash (docs-only; A→B→C Action Row UX locked). Exact-SHA
  three-platform CI for tip `df6a3b6` is **not** claimed green in this pass —
  see the modernization CI subsection. Residuals unchanged: installed artifact
  identity, provider qualification, two-guild member/manager checks, fresh
  unanimous consent, human-witnessed audible voice. Voice A→B→C implementation
  is in flight elsewhere and is **not** recorded as shipped here.

- **2026-09-08 06:1x EDT — `#106` then `#105` on `main`; source landed, live
  acceptance not closed.** `143fc79` is the #106 ledger squash. `1b4822d` is the
  #105 merge (classic Action Row UX Phases A+B+C source). Parent chain now:
  `1b4822d` → `143fc79` → `85d0e18` → `df6a3b6` → `9abab67` → `eeb717b` (last
  completed three-platform **success** on record). Exact-SHA three-platform CI
  for tip `1b4822d` is **not** claimed green — see tip subsection. #105 is
  **merged source**, not live-accepted: redeploy in flight; do **not** claim
  installed artifact identity; human-witnessed voice boxes stay open. Residuals
  unchanged: provider qualification, two-guild checks, consent, Portal Activity
  P0, OAuth P2, Components V2 crate-block, episode-gate human approval.

- **2026-09-08 06:4x EDT — `#107` + `#108` on `main`; live Action Row UX
  redeployed; acceptance not closed.** `7137d86` is the #107 ledger merge.
  `6f678ac` is the #108 agents-docs merge. #105 (`1b4822d`) source remains on
  `main`; live Abbey was redeployed ~06:28 ET (PID 58490, binary SHA-256 prefix
  `6084c7f2`, Discord connected; Action Row UX markers present in the live
  binary). Exact-SHA three-platform CI for tip `7137d86` is **not** claimed
  green — tip Gate may still be in progress / cancelled chain (see tip
  subsection). **No** status flip to done for live residuals: human-witnessed
  voice, two-guild, consent observations, installed artifact identity
  qualification, Portal P0, OAuth P2, Components V2 crate-block, episode-gate
  human approval.


## Route guild operations through the WDBX episode gate
status: in_progress
- 2026-09-06 first slice (Donald chose "subprocess to the abi binary" over a gRPC
  client or a contract amendment): `src/episode_gate.rs` mirrors `/admin learning
  on|off` into the constitutional ledger as a content-free `proposal` event via
  `abi wdbx episode propose --json`, default-off behind `ABBEY_EPISODE_GATE_CONFIG`.
  Pure builders (guild ref, keyed principal ids, request/operation ids, the write)
  are unit-tested; the runner is tested against fake `abi` shell scripts (append,
  refusal, hang, missing binary, environment isolation); the JSON transcription is
  pinned byte-for-byte to a fixture generated from `abi-wdbx::v3::episode`.
  The fixture was accepted by a real `abi-wdbx-gateway --episode-policy` through the real
  `abi` CLI, invoked exactly as the bot's child would be (`env -i`, allowlist only, no
  `ABI_WDBX_PATH`): `decision=appended`, nothing under `~/.abi` touched, and the replay refused
  with `AlreadyExists: episode_replay`. The bot's own runner is proven only against fake
  scripts, and no gateway is deployed.
- 2026-09-06 04:0x: **pushed** (`dbe4b88..a7b39fb` on `origin/main`, Donald's yes; `a7b39fb`
  fixed three ordering gaps found in review: tombstones are proposed only for a confirm that
  will remove, `replaces` is resolved before proposing, receipts are dropped after the local
  delete). Then, on Donald's two calls: the cumulative payload charge stays and checkpoint
  guilds get sized budgets (AGENTS.md), and the model's `remember_fact` tool now **queues**
  instead of refusing while the gate is configured (`memory_gate::{enqueue, drain}`,
  `AppState.memory_queue`, cap 64, drained after each pipeline reply and before each gated
  persist; a refused item is dropped, not retried; tests cover the dead-gate and no-gate
  paths). Gate green (1011 passed); `ba61202`, pushed on Donald's yes.
- 2026-09-06 04:3x: **GATE ON FOR MLAI, LIVE.** On Donald's four yeses: (1) the gateway agent was
  bootstrapped by the installer's first execution (`com.donaldfilimon.abbey-wdbx-gateway`, pid
  10660, loopback 50051, store `~/.local/share/abbey-bot/wdbx-gateway`, readiness verify answered
  `found=false`); (2) `ABBEY_EPISODE_GATE_CONFIG=/Users/donaldfilimon/.config/abbey-bot/episode-gate.json`
  appended to the live env file (backup `env.before-episode-gate-20260906-043018`, names-only
  checker green) and the bot restarted with `launchctl kickstart -k` at 04:30:24 EDT: the old
  process (95630) persisted twice on SIGTERM (`overall="complete"`), the new one (11750) accepted
  the credential, registered commands, and connected, and its environment carries the key (read by
  `ps -E` as the single `ABBEY_EPISODE_GATE_CONFIG=` assignment; the startup env-presence log line
  reports a fixed voice/LLM key set and does not mention it); (3) budget kept at 64 MiB,
  watched on `/inspect`; (4) `6bf3252` pushed (`ba61202..6bf3252`). Proof of the gate in effect
  awaits the first *changed* MLAI checkpoint or memory write (unchanged rows are not proposed);
  the production ledger was 0 bytes at restart. This ledger entry is a docs-only local commit.
- 2026-09-06 04:2x: **gate scoped to one guild, gateway deployment prepared, memory path
  accepted live (local commit, not pushed).** The gate config gains an optional `guilds` list
  (scoped guild ids; absent = every scope) and `AppState::gate_for` is the single decision point:
  an uncovered scope is byte-identical to no gate on `/remember`, `/forget`, confirm, the model
  tool, the learning-toggle mirror, and checkpoints (`checkpoint_gate::{plan,restrict_to_admitted}`
  take the predicate, so the six non-MLAI brain rows are neither proposed nor substituted).
  `/inspect`'s gate line now says `all guilds` or `N guild(s)`. Live acceptance is
  `src/episode_gate/acceptance.rs`, an `#[ignore]` test run on purpose against a scratch
  gateway: real `abi` binary + real `abi-wdbx-gateway` + the real policy file, through the tool
  host, the queue and drain, slash-style admit with `replaces`, forget, and `persist_all_gated`;
  every receipt re-verified by `abi wdbx episode verify` from a separate process; one live
  refusal (a covered scope the policy does not list). Result: 4 records in the scratch ledger
  (fact 44 B, superseding fact 42 B, tombstone 0 B, experience checkpoint 71 B), counters
  appended 4 / rejected 1 / unavailable 0; the production store at
  `~/.local/share/abbey-bot/wdbx-gateway` was opened once by the gateway and answered
  `found=false` for the MLAI guild, ledger 0 bytes. Deployment files (outside the repo):
  `~/.config/abbey-bot/{episode-policy.json,episode-gate.json,episode-gate-acceptance.json,
  episode-gateway-token}` (0600); binaries + both `libabi_*.dylib` shims in
  `~/.local/libexec/abbey-bot`. In the repo: `deploy/com.donaldfilimon.abbey-wdbx-gateway.plist`
  and `deploy/install-wdbx-gateway-launchd.sh` (readiness = zero-digest verify answers
  `found=false`; **syntax-checked only, never executed: its first run is the bootstrap**, and
  nothing it does before `launchctl bootstrap` touches the bot), `check-launchd-env.sh` names
  `ABBEY_EPISODE_GATE_CONFIG`. Startup trap, read from `runtime.rs:523`: a gate config the bot
  cannot validate is a `StartupError`, i.e. a crash loop under `KeepAlive`, so the env line must
  be the literal absolute path and the file must validate before the restart. Discord auth runs
  before that validation, so the binary cannot preflight it with a placeholder token (tried:
  both a good and a broken config die at auth); the preflight is the `#[ignore]` test
  `preflight_the_gate_config_named_by_the_environment`, which ran green on the production
  `episode-gate.json` (coverage 1) and red on a broken file. Policy sizing:
  MLAI `discord-1275617641620443146` token_budget 10,000,000 and storage_budget_bytes 64 MiB
  (the store's hard cap); acceptance guild 100,000 / 4 MiB. Honest arithmetic: MLAI's row is
  ~52 KB and learning is on there, so 64 MiB is a lifetime of ~1,290 admitted checkpoints
  (worst case 288/day = ~4.5 days if the row changes every tick; longer in practice, since an
  unchanged row is not re-proposed). Not done, by rule: nothing added to the live env file, no
  launchd bootstrap of the gateway agent, no bot restart; those are the named stop and wait
  for Donald's yes. Gate green (1014 passed, 3 ignored including the acceptance test).
- 2026-09-06 03:4x: **memory-candidate adapter landed (amendment step 3 of 3, local commit,
  not pushed).** `episode_gate.rs` transcribes `MemoryClass`/`RetentionClass`/`MemoryCandidate`
  and the `memory_candidate` event (pinned by `tests/fixtures/episode_write_memory_candidate.json`,
  copied byte-for-byte from wdbx's golden), builds candidates with `memory_candidate_write`
  (SHA-256 over the payload, never the payload), and counts appended/rejected/unavailable/
  ungated-forgets for `/inspect`. `memory_gate.rs`: `/remember`, `/forget`, `/pending confirm`
  propose before writing and write only on `appended`; receipts in `Stores.memory_receipts`
  (`#[serde(default)]`, old state files still load). The model's `remember_fact` tool refuses
  while the gate is configured (sync host cannot propose). `checkpoint_gate.rs`: per-guild
  `BrainRow` proposed as an `experience` candidate once per changed persist, superseding the
  last admitted digest; refused rows are substituted by the last admitted (or pre-gate
  on-disk) row; the sync shutdown persist writes only admitted rows. Default-off path is
  byte-identical. Not done: no end-to-end test against a live gateway from this repo (the
  gate is exercised through fixtures and pure functions; the abi CLI test covers the wire),
  and `operational` retention emits no `forgets` on checkpoint replacement (the supersede
  chain is the lifecycle). Budget trap named in AGENTS.md: payload bytes are charged
  cumulatively and never refunded.
- **OWNED MISTAKE, 2026-09-06 03:1x:** the direct push of `f3c0ba8` turned the Windows gate red
  (5 `episode_gate` unit tests): the tests hardcoded POSIX absolute paths, which are relative on
  Windows, and built JSON by string formatting, so a Windows temp path's backslashes made the
  config invalid. Ubuntu and macOS were green, and the local Mac gate cannot catch this class.
  Fixed here: paths derive from `temp_dir()` and the JSON is built with `serde_json::json!`.
  This is the fourth direct-push break the ledger above argues about; the argument stands.
  Honest scope: wired and tested;
  only the proposal stage is emitted (approval needs a distinct human approver the
  bot cannot supply); DQN/memory-bank writes are *not* routed through the gate,
  because the gate's vocabulary is operation lifecycle, not memory vectors.

- **2026-09-08 04:4x — gate continuity re-measured; NOT a new finding, and a
  correction to how I first wrote it.** I initially filed this as "stale bullet
  corrected", claiming this section still said the gateway was never
  bootstrapped. That was my error: the 2026-09-06 04:3x **GATE ON FOR MLAI,
  LIVE** bullet above already records the bootstrap, Donald's four yeses and the
  `launchctl kickstart -k` restart at 04:30:24 EDT. I had read only this
  section's last 35 lines, and its bullets are not in time order, so an earlier
  "not done, waiting for Donald's yes" line sitting near the tail read as
  current. The lesson is the ledger contract's own: read the whole section, not
  its tail.
  What is genuinely new is continuity, two days on. Measured read-only, nothing
  started, stopped or reloaded: `com.donaldfilimon.abbey-wdbx-gateway` is still
  loaded at **the same pid 10660** recorded on 2026-09-06, last exit 0, beside
  `com.donaldfilimon.abbey-bot` (pid 64772, exit 0). So the gateway has not
  crashed, been respawned or been reinstalled since it went live.
  `sh deploy/check-launchd-env.sh ~/.config/abbey-bot/env` still reports
  `present: ABBEY_EPISODE_GATE_CONFIG`, and scope is unchanged at MLAI-only:
  `~/.config/abbey-bot/episode-gate.json` lists exactly one guild,
  `discord:1275617641620443146`, `policy_version` `abbey_mlai_v1`,
  `contract_revision` 2, `evidence_level` `c0`. Read with `token_file` redacted;
  no secret printed or copied.
  Residuals unchanged, so the goal stays `in_progress`: no end-to-end test
  against the live gateway from this repo, proposal stage only (approval needs a
  distinct human approver the bot cannot supply), DQN/memory-bank vector writes
  deliberately outside the gate, and `operational` retention still emitting no
  `forgets` on checkpoint replacement. A loaded agent is not an accepted
  transaction.

- **2026-09-08 05:5x EDT — episode approval residual restated; no status flip.**
  Continuity from the 04:4x re-measurement still holds as the last measured
  gateway observation in this file. Approval remains a distinct human step the
  bot cannot supply; proposal-only stage is what is live. This `/goal continue`
  does not re-probe launchd or claim a new transaction.

- **2026-09-08 06:1x EDT — episode approval residual restated; no status flip.**
  Still proposal-only; human approval is a distinct step the bot cannot supply.
  This `/goal continue` does not re-probe launchd, claim a new transaction, or
  invent WDBX progress beyond what is already recorded.

- **2026-09-08 06:4x EDT — episode approval residual restated; no status flip.**
  Still proposal-only; human approval is a distinct step the bot cannot supply.
  This `/goal continue` records the live Action Row UX redeploy elsewhere and
  does not re-probe the episode gateway or claim a new WDBX transaction.


## Build the MLAI server from a plan file (`--server-plan`)
status: done
- 2026-09-06 04:5x: **manual steps taken by hand on Donald's choice ("also trim Team and
  Moderator"), outside the engine as its 3.B.8 boundary requires.** Three REST role PATCHes with an
  audit-log reason: @everyone gained Send Polls (nothing removed); Team and Moderator were set to
  exactly the plan's permission sets. Net loss after @everyone inheritance, stated so it is not
  discovered later: Team lost Change Nickname, Create Events, Create Private Threads, Manage
  Events, Manage Nicknames, View Audit Log; Moderator lost those plus Mute/Deafen/Move Members
  (which the plan reserves to Team). Role snapshots before and after are in
  `~/Archive/2026-09-06-mlai-discord-snapshot/mlai-roles-{before,after}-hand-pass-2026-09-06.json`
  (0600). The reveal dry run afterwards: `changes (0)`, manual steps down from four to one, the
  remaining one being Member's guild-level extras, which Donald chose to leave. Goal stays `done`.
- 2026-09-06 04:36: **OVERWRITES STAGE APPLIED, blueprint fully applied; goal closed.** Dry runs of
  all eight categories: six already `changes (0)`, BUILD LOG and PRODUCTS one change each (deny
  @everyone View Channel on #ci-and-deploys and #ops-console). Applied on Donald's yes, one run
  per category, each `verified: 1 change(s) applied`, each followed by an independent
  `changes (0)` dry run. Every stage of `blueprints/mlai-community.toml` (additive 03:06, reveal
  04:33, overwrites 04:36) now matches the live guild. Transcripts (0600) in
  `~/Archive/2026-09-06-mlai-discord-snapshot/mlai-{dry-run,apply}-overwrites-*-2026-09-06.txt`.
  Residual, by design and unchanged: the engine's review notes (existing-role permission diffs
  incl. @everyone missing Send Polls, and role order) are hand decisions the engine never makes
  (`diff::Change` has no permission edit); they are Donald's, not a gap in this goal.
- 2026-09-06 04:33: **STAGE REVEAL APPLIED to the live MLAI guild** on Donald's "confirm all".
  Fresh dry run first (`changes (52)`, 0 blockers, no deletes: overwrite sets/clears, role
  hoist/colour edits, topic edits, one forum and one announcement edit), then `--apply`:
  `verified: 52 change(s) applied; the guild now matches stage reveal of the plan.` An
  independent dry run afterwards reports `changes (0)`. Transcripts (0600, outside the repo):
  `~/Archive/2026-09-06-mlai-discord-snapshot/mlai-{dry-run-reveal-2026-09-06-0433,apply-reveal-2026-09-06,dry-run-reveal-after-2026-09-06}.txt`.
  Review notes the engine leaves by design (role permission diffs, role order) are unchanged.
  Remaining stage: overwrites (per category, `--category`).
- 2026-09-06: `server/{plan,observe,diff,apply,discord,run}` + `blueprints/mlai-community.toml`
  land the engine the 2026-09-04 design asked for, with the boundary moved into the type
  system: `diff::Change` has no delete variant and no role-permission edit (a test enumerates
  the variants). Stages mirror the proposal's rollout (additive → reveal → overwrites per
  category); `reveal` changes @everyone visibility only from engine-hidden (the hide marker is
  the channel's only overwrite) to visible, and a plan role matching an integration-managed
  role blocks every stage. Preflight: Manage Channels/Manage Roles unless Administrator,
  bot top role above any role it edits, COMMUNITY for forum/announcement/stage, and the bot
  may only grant/deny permissions it holds. `--apply` re-reads and re-diffs to verify.
- Evidence: 62 `server::` unit tests (fake guild proves every stage idempotent and that
  `SetOverwrite` never removes another entry), full `./check.sh` green on `233b2df` (1001
  passed, 2 ignored, no other cargo process in the tree), and a read-only dry run against
  the live MLAI guild as the bot (33 additive changes, no blockers after renaming the
  colliding interest role to "Personas"). Two live findings fixed here: the `@me` guild-member
  route is user-token only, and `NEVER_FOR_EVERYONE` listed a name serenity never emits.
- 2026-09-06 03:1x: **`--stage additive` was applied to the live MLAI guild** on Donald's explicit
  yes (Stage 0 history export waived by him): 33/33 applied, 0 failed, exit 0, the engine's
  verification re-diff empty, and an independent post-apply dry run `changes (0)`; guild
  21 roles / 28 channels -> 31 / 51, nothing pre-existing touched. Transcript in
  `~/Archive/2026-09-06-mlai-discord-snapshot/mlai-apply-additive-2026-09-06.txt`. A reveal
  dry run afterwards shows 55 changes and no blockers; **reveal has not run** and needs its
  own yes after the 7 manual steps (role permissions and order) that sit between the stages.
- Honest scope: only the additive stage has ever run against a real guild. Role permissions,
  role order, Community settings, onboarding, AutoMod, and every deletion remain human steps
  by design. Forum tags and slowmode are creation-time only (not diffed afterwards).
- 2026-09-06 03:0x: `233b2df` fixed a reveal non-idempotence the advisor caught (a plan with no
  topic kept emitting an empty edit for a channel that had one, so `--apply` could never
  verify clean on the topic-less archetype plans). The three commits `dac71a7`, `f74bb7a`,
  `233b2df` are local only, not pushed.


## 2026-09-06 full modernization integration
status: in_progress

Historical modernization checkpoint; its results belong to `ff5d594`. The reviewed
source implements guided help and private full-fact browsing, scoped statistics
and fixed operator advice, plus managed readiness/logging and retained service,
voice and persistence ownership. The fresh WDBX-required strict gate passed on
tested source `ff5d594`: 1,183 Rust tests, zero failures, four intentional live
ignores, warnings-denied all-target locked Clippy, formatting, 26 offline installer
tests and the locked Rust release build and offline Swift release build. Deployment/Python, privacy, 81-artifact contracts,
required WDBX, Linux TLS, module size and Swift groups of 12 and 16 passed.
RustSec policy passed with four accepted vulnerabilities and three unmaintained
warnings; this is not a clean audit.

- Completed: reviewed decomposition, 19 compatible transitive dependency updates,
  source documentation and the fresh strict gate with the locked Rust release build and offline Swift release build.
- Closed by review and passing regressions: per-press pending authorization,
  ACK-before-gate ordering, stale displayed-row rejection, subprocess/music
  cancellation ownership, startup/reconnect readiness and truthful shutdown events.
- At that checkpoint, canonical integration, push and exact-head hosted CI were
  not yet established. Current release observations are maintained separately in
  `.superpowers/completion-20260906/delivery.json`.
- Completed: all ten sanitized release entrypoint cases (five managed startup/
  privacy/unsafe-log and five legacy argument/no-backend cases), without requests.
- That checkpoint did not establish installed artifact identity or managed transaction
  acceptance, provider qualification, live two-guild Discord/UI
  checks, fresh unanimous consent and human-witnessed voice acceptance.
- Historical live evidence and recovery material remain preserved. This source
  validation pass performed no live transition.

See `docs/live-test-protocol.md` for lifecycle/status semantics and
`docs/MLAI-LIVE-ACCEPTANCE.md` for dated live observations. The source verification
above belongs to the tested revision, not a later documentation-only commit.

### Command-workflow follow-through

The reviewed workflow implementation was handed off at `5724bf6`. It includes
all five help tasks, typed capability readiness shared with execution, canonical
availability decisions, shared delivery observation outside startup, effective
administration guidance and active voice wake/stop instructions. Independent
behavioral and structural reviews approved the corrections. The integrated Rust
suite passed 1,222 tests with zero failures and five intentional ignores before
the final small catalog lint fixes; the affected 18-test catalog suite and
warnings-denied all-target Clippy then passed on the handoff source.

The release slice also fixes selection of Cargo's actual external-target binary
and makes two deliberately insecure permission fixtures independent of umask.
Linux installer fixtures use a private scratch parent, and the Pages inventory
includes the approved workflow documents. These repairs preserve production
artifact validation and the Windows fixture line-ending repair. Final strict-gate,
exact remote-SHA CI, transactional installation, registered-schema and installed-provider
results are revision-bound observations in the canonical delivery record, rather
than conclusions inherited from the earlier test counts. The earlier delivery
record is preserved privately as historical evidence.

Live acceptance is tracked by role and workflow in two operator-supplied sandbox
guilds. Member access, manager access, global/DM command scope, current unanimous
voice agreement and human audible confirmation each need their own observation.
No source test or synthetic speech probe substitutes for those checks. See
`docs/live-test-protocol.md` for the workflow matrix and restoration requirements.

### Red-main repair and branch/worktree consolidation (2026-09-06 11:2x EDT)

`6a3000c` (guild-scoped voice sessions, pushed by a concurrent session) left
`main` red on all three CI platforms. Repaired at `f45b7b2`: `src/host_music.rs`
formatting, the `private_interfaces` denial on `AppState::host_music` (widened
`HostMusic` to `pub`, matching every other type reachable from a public
`AppState` field), and the `actual_unconfigured_voice_status_reaches_explanatory_handler`
assertion, which still expected the pre-scoping startup-global wording after
`voice_status` / `voice_diagnostics` deliberately moved to the per-guild text.
Behaviour was not reverted; only the stale assertion was corrected.

Strict gate green on the pushed tree: `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`
with `ABBEY_WDBX_REPO=../wdbx` — 1,249 tests passed, 0 failed, 5 intentional live
ignores, warnings-denied all-target locked Clippy, formatting, WDBX
cross-repository fixture parity, locked Rust release build, offline Swift
audio-tap build.

Exact-SHA CI on `f45b7b2` then showed what the Mac gate structurally cannot:
macOS passed, Ubuntu passed, **Windows failed** on `-D unused-mut` at
`src/voice_registry.rs:376`, where the `fs::DirBuilder` binding is `mut` only
for the unix `DirBuilderExt::mode` call. Repaired at `566a449` with
`#[cfg_attr(not(unix), allow(unused_mut))]`, scoped to the affected targets so
unix still enforces the lint there. Verified by compiling a minimal
reproduction for three targets: the old form fires the lint on
x86_64-pc-windows-msvc only; the new form is clean on Windows, Linux and
macOS. A full local cross-compile is impossible (`ring` needs MSVC C headers),
so hosted Windows CI remains the authority and the exact-SHA three-platform
run on `566a449` is the closing evidence. The Mac gate does not substitute.

Repository consolidation the same session: `abbey-bot` already had only `main`
(the `abbey-bot-wt-*` worktrees were gone before this pass) and is level with
`origin/main`. All 6 stashes were dropped on Donald's explicit instruction after
capture to `~/at-risk-bundles/abbey-bot-stashes-20260906/`; stashes 0 and 5 touch
`src/voice.rs` / `src/voice_local.rs` and may overlap the voice work above.

Sibling `dev/active/AbbeyBot` (Swift) was consolidated to `main` at `dae6c42`:
`codex/quasar-linux-process-groups` and `cursor/quasar-completion-20260906`
merged, `Scripts/verify-all.sh` green twice, 2 local and 4 remote branches
deleted at zero unique commits, and the live `AbbeyBot-wt-quasar-completion-20260906`
worktree removed on Donald's explicit call while its session was still
committing. Its CI and the `aae40b4` Linux verification remain outstanding there.

#### Exact-SHA three-platform CI: GREEN on `d5f00da`

Closing evidence for the red-main repair. `Rust` workflow run on `d5f00da`:
Gate (Ubuntu) success, Gate (macOS) success, Gate (Windows) success.

Reaching it took four commits, because each failure masked the next: `cargo fmt`
short-circuited `check.sh`/`check.ps1` before clippy on every runner, and clippy
short-circuited before the suite.

1. `f45b7b2` — formatting, the `private_interfaces` denial on
   `AppState::host_music`, and a stale voice-status assertion left behind when
   `6a3000c` moved `voice_status`/`voice_diagnostics` to per-guild wording.
2. `566a449` — `-D unused-mut` on the cfg-dependent `fs::DirBuilder` binding in
   `voice_registry`, Windows-only.
3. `d5f00da` — the two Windows-only test-portability defects that only became
   visible once Windows first reached the suite: a hardcoded POSIX path literal
   in `managed_env`, and an accepted socket inheriting the listener's
   non-blocking mode in `pipeline::tests::memory_outcomes` (WSAEWOULDBLOCK).

Standing lesson: the macOS gate cannot observe the Windows lane, and a failing
early stage hides every later one, so "the Mac gate is green" says nothing about
the platforms and nothing about stages after the first failure. Only the
exact-SHA three-platform run closes this.

Not established by this pass, and unchanged: installed artifact identity,
provider qualification, live two-guild member/manager Discord checks, fresh
unanimous consent and human-witnessed audible voice acceptance.

#### Three-platform CI: GREEN on `eeb717b`, the pre-merge parent of this commit (2026-09-08 04:3x EDT)

**This is parent-SHA evidence and it does NOT clear stage 0 for the head that
results from landing this ledger commit.** Corrected after review: an earlier
draft of this entry called it "the current standing CI observation", which
contradicts the SHA-bound rule this very file states at lines 99–100 ("Any later
push to `main` invalidates it until that new SHA is green on all three platforms
again") and the stage 0 requirement in `docs/live-test-protocol.md`, which asks
for the Ubuntu, macOS and Windows job results whose `headSha` is *exactly* the
SHA under test. Merging this PR produces a new `main` SHA for which `eeb717b`'s
run is, by that rule, only the parent's result. That new head needs its own
three-platform run before anything here is treated as current, and this entry
must not be carried forward to it.

What it does establish, scoped precisely: `eeb717b` — the tree as it stood
before this ledger commit, and the head that carries #97–#102 — is green on all
three platforms. In that narrow sense it supersedes `d5f00da`, which is 16
commits behind and predates `main` going red and being repaired, so `d5f00da`
describes neither `eeb717b` nor anything later.

Measured, not inferred. `Rust` workflow run `34179977713`, `headSha`
`eeb717b370c0ac37e24b89acac274c79dd8255a2`, which was `main` and level with
`origin/main` at the time of reading (`rev-list --left-right --count
main...origin/main` = `0 0`), and is the parent of this ledger commit:

- Gate (macOS) success, Gate (Ubuntu) success, Gate (Windows) success.
- The Windows lane genuinely reached the suite rather than short-circuiting:
  its `Gate — fmt, privacy, locks, clippy, tests, release build` step succeeded
  over a 24m35s run (02:25:25Z to 02:50:00Z). Checked because this section's own
  standing lesson is that an early failing stage hides every later one, so a
  workflow-level `success` alone is not sufficient evidence.
- Method: `gh run view <id> --json headSha,conclusion,jobs`, reading per-job and
  per-step conclusions. No local gate was run and none is claimed; per the
  standing lesson the Mac gate cannot observe the Windows lane.

This closes the red-`main` episode opened by `6a3000c` and repaired across
`f45b7b2`, `566a449`, `d5f00da`, and later `dd102dc` (#101, the redundant
`must_use` on `resolve_select_action` that turned `main` red again after the
`/admin show` page-select work landed in #99).

Branch hygiene the same session: all six local branches whose upstreams were
`gone` were deleted after proving each merge was a content no-op
(`git merge-tree --write-tree main <branch>` equalled `main`'s tree for all six,
plus a per-file byte compare). The work had landed as squash merges #97, #98,
#99, #101, #102, so no merge commits were created. The only branch-only lines
anywhere were four roadmap lines on `feature/p3-forum-helpers`, which #99 had
deliberately rewritten. Ancestry alone would have misled here: none of the six
was an ancestor of `main`.

Still not established by this pass, and unchanged: installed artifact identity,
provider qualification, live two-guild member/manager Discord checks, fresh
unanimous consent and human-witnessed audible voice acceptance. Those are live
observations; no source test, CI run or synthetic probe substitutes for them.


#### Tip `df6a3b6` (#104 design) — three-platform Gate NOT yet re-cleared (2026-09-08 05:5x EDT)

Executed as `/goal continue` after #103/#104 landed. Tip of `origin/main` at
this writing is `df6a3b6cca406ce2e87ae515d9336b2afd755240` (voice classic UX
design squash #104). Parent chain: `df6a3b6` → `9abab67` (#103 ledger) →
`eeb717b` (last SHA with a completed three-platform **success** on record).

**Honest CI claim for `df6a3b6`:** Rust workflow run `34212047816`
(`headSha df6a3b6…`) was still **`in_progress`** when this ledger entry was
written — Gate (macOS), Gate (Ubuntu), and Gate (Windows) all `in_progress`,
no job `conclusion` yet. Therefore this tip does **not** clear
`docs/live-test-protocol.md` stage 0, and the SHA-bound precondition is **not
yet re-cleared** for `df6a3b6`. Do not carry `eeb717b`'s green forward to this
tip or to `9abab67`.

Method: `gh run view 34212047816 --json status,conclusion,headSha,jobs`. No
local gate was run and none is claimed. The #103 push run on `main` was
observed `cancelled` when #104 landed (`34212033633`), which is exactly why
parent-SHA greens never transfer.

Still not established, and unchanged by #103/#104: installed artifact identity,
provider qualification, live two-guild member/manager Discord checks, fresh
unanimous consent, human-witnessed audible voice acceptance, Portal Activity
URL map (P0), OAuth secret host (P2), Components V2 (crate-blocked), and
episode-gate human approval.

#### Tip `1b4822d` (#105 merge) — three-platform Gate NOT yet re-cleared (2026-09-08 06:1x EDT)

Executed as `/goal continue` after #106 landed and #105 merged on `main`. Tip of
`origin/main` at this writing is `1b4822d1a3b017b65be6cf0afccf8b85e10890c5`
(merge of PR #105, classic Action Row UX Phases A+B+C). Intermediate docs tip
`85d0e18` addressed PR #104 design review findings (docs-only) between `df6a3b6`
and #106. Parent chain: `1b4822d` → `143fc79` (#106 ledger) → `85d0e18` →
`df6a3b6` → `9abab67` → `eeb717b` (last SHA with a completed three-platform
**success** on record, run `34179977713`).

**#105 status:** **MERGED** (source on `main`). Quality review had been PASS on
the PR rollup before merge. This is **not** live acceptance. Live Abbey redeploy
is in flight separately — **do not** claim installed artifact identity. Human-
witnessed audible voice acceptance (and related human-gated boxes) remain open.

**Honest CI claim for `1b4822d`:** Rust workflow run `34214591720`
(`headSha 1b4822d…`) was **`in_progress`** when this ledger entry was written —
Gate (Ubuntu), Gate (macOS), and Gate (Windows) all `in_progress`, no job
`conclusion` yet. Therefore this tip does **not** clear
`docs/live-test-protocol.md` stage 0, and the SHA-bound precondition is **not
yet re-cleared** for `1b4822d`. Do not carry `eeb717b`'s green forward.

Method: `gh run view 34214591720 --json status,conclusion,headSha,jobs` and
`gh pr view 105 --json state,mergeCommit,mergedAt`. No local gate was run and
none is claimed.

Still not established, and unchanged: installed artifact identity (redeploy in
flight), provider qualification, live two-guild member/manager Discord checks,
fresh unanimous consent, human-witnessed audible voice acceptance, Portal
Activity URL map (P0), OAuth secret host (P2), Components V2 (crate-blocked),
and episode-gate human approval.

#### The final-head rule has produced no completed `main` run since 03:00 UTC (2026-09-08 07:1x EDT)

Measured, not inferred, and it explains why both "NOT yet re-cleared" subsections
above are still true: `gh run list --branch main --workflow Rust`. The last
Rust run on `main` to reach a conclusion is `eeb717b` (`34179977713`, success,
02:25:04 to 03:00:06 UTC, 35 minutes end to end; the Windows lane alone is
about 25). Every run since has been cancelled by the next merge, ten in a
row: `9abab67`, `df6a3b6`, `85d0e18`, `143fc79`, `1b4822d`, `b6358c7`,
`6f678ac`, `7137d86`, `3c740eb`, `52562c9`. That covers 19 commits: nine
merged PRs (#103, #104 and #106 squashed; #105, #107, #108, #109, #110 and
#112 as merge commits) plus two direct pushes (`85d0e18`, `b6358c7`), counted
with `git log --first-parent eeb717b..origin/main`, because `--merges` alone
misses the squashes and an earlier draft of this entry said "6 merged PRs" on
that basis. At this reading `df1f5d8`'s run (`34217913098`) started 10:54:40
UTC and is 16 minutes in, so any merge before roughly 11:30 UTC cancels it too.

What this changes. The rule recorded on 2026-09-04 ("the evidence is the run
at the final head, which is the one that matters") assumes a final head
eventually finishes. For the last eight hours it has not, because merges from
several concurrent sessions land more often than every 35 minutes, which is
faster than the `cancel-in-progress` group lets a run complete. So stage 0 of
`docs/live-test-protocol.md` has been un-clearable for every `main` head since
03:00 UTC, including the head the live service was redeployed from at 06:28
ET. Nothing here says `main` is broken. Measured on the PR branches rather
than assumed (`gh run list --workflow Rust`, non-`main` heads, 11:18 UTC):
seven of the nine merged PRs had a green Rust run at the exact head that
merged (`f36f646`, `874a929`, `6f5fbba`, `1a4b3b8`, `7595a9d`, `9301016`,
`fd50529`). The other two, #109 (`0bf6a3d`) and #112 (`506869a`), were merged
at 10:54 UTC with their branch runs still in progress, and both runs were
still running at 11:18, so neither of those heads is proven green or proven
broken. Either way `main` itself carries no three-platform evidence at any
SHA newer than `eeb717b`, and will not until a head survives 35 minutes
unmerged.

Levers, all Donald's decision and none taken here: hold merges for one
35-minute window so a head completes; or change the workflow's concurrency
group so `main` runs are not cancelled (keep `cancel-in-progress` for PR
branches); or enable branch protection, which the 2026-09-04 entry already
records as his call. Until one of these happens, every "NOT yet re-cleared"
subsection stays true by construction, and no live-acceptance rung that
requires stage 0 can be started honestly.

#### Tip `7137d86` (#107 ledger + #108 agents) — three-platform Gate NOT claimed green (2026-09-08 06:4x EDT)

Executed as `/goal continue` after live Action Row UX redeploy. Tip of
`origin/main` at this writing is `7137d866ccc7c4edea5c13a1371d9b5af22b733a`
(merge of PR #107). Parent chain includes #108 (`6f678ac`), help harden
`b6358c7`, #105 merge `1b4822d`, #106 `143fc79`, docs tip `85d0e18`, #104
`df6a3b6`, #103 `9abab67`, then `eeb717b` (last SHA with a completed
three-platform **success** on record, run `34179977713`).

**Live redeploy (verified this pass, ~06:28 ET):** PID **58490**; installed
binary SHA-256 prefix **`6084c7f2`**; Discord `discord_ready` /
`connector_state=ready` in events jsonl; Action Row UX binary markers live
(`abbey:v:`, Refresh / Confirm leave). #105 / #106 / #107 / #108 **MERGED**.

**Honest CI claim for `7137d86`:** Rust workflow run `34215740641`
(`headSha 7137d86…`) was **`in_progress`** when this ledger entry was written —
Gate (macOS) had `conclusion=success`; Gate (Ubuntu) and Gate (Windows) were
still `in_progress` (no full three-platform success yet). Prior push Gates on
the cancelled chain include `34215468939` (#108, cancelled), `34215190160`
(help harden, cancelled), `34214591720` (#105, cancelled). Therefore this tip
does **not** clear `docs/live-test-protocol.md` stage 0, and three-platform
green is **not** claimed for `7137d86`. Tip Gate may still be in progress /
cancelled chain; do not carry `eeb717b`'s green forward.

Method: `gh run view 34215740641 --json status,conclusion,jobs`,
`gh pr view` for #105–#108, `ps`/`shasum`/`launchctl print` read-only, and
events jsonl. No local gate was run and none is claimed.

Still not established, and unchanged (no status flips to done): human-witnessed
voice, two-guild isolation, consent observations, installed artifact identity
qualification, provider qualification, Portal Activity URL map (P0), OAuth
secret host (P2), Components V2 (crate-blocked), and episode-gate human
approval.
