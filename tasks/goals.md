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
