# AGENTS.md

Rust/Serenity/Poise Discord bot, not the separate Swift AbbeyBot product.
This is the canonical agent guide. `CLAUDE.md` must remain only the
`@AGENTS.md` pointer; `README.md` owns commands and configuration, and dated
live evidence belongs in `docs/MLAI-LIVE-ACCEPTANCE.md`.

The `## Boundaries` section is mirrored into
`.cursor/agents/abbey-reviewer.md` and `.codex/agents/abbey-reviewer.toml`.
After changing it, run `python3 scripts/check-instructions.py --write`; never
hand-edit the generated TOML. `check.sh` and `check.ps1` fail on drift.

## Checkout and sources

- Read `README.md`, `.env.example`, `rust-toolchain.toml`, `check.sh`, and
  `.github/workflows/rust.yml` before changing behavior. Module-level `//!`
  headers identify the intended seam and often the design contract.
- `tasks/goals.md` is an append-only ledger, not a current-state database; append
  corrections rather than rewriting history. Read the whole relevant section and
  verify its artifact (`git log`, the gate, or live read-only status) before
  acting on it. `tasks/todo.md` is the companion checklist.
- Other agents share this checkout. Never run `git checkout` here; use
  `git show <ref>:<path>` for another ref, and put any isolated worktree beside
  the repository, never inside it. Check that no other session owns it before
  removal, then remove it when finished.
- On this Mac, prefer `.claude/skills/run-abbey-bot/smoke.sh` for build,
  token-free self-tests, focused tests, and read-only status. Do not start a
  second Discord gateway while the launchd service is loaded.

## Shape and flow

- `src/` is one Rust binary crate, with no library target or workspace.
  `tests/` contains fixtures only; Rust tests are inline or in `src` test
  modules. `deploy/`, `scripts/`, `contracts/`, `blueprints/`, `activity/`, and
  `tools/` are support surfaces.
- `main.rs` parses the CLI and starts `startup.rs`; the normal path builds
  `runtime::AppState`, then wires Discord, optional Telegram/Slack, voice, and
  persistence. `--server-plan`, `--provider-self-test`, and `--voice-self-test`
  are explicit non-gateway modes.
- Gateway adapters turn native events into `platform::SocialEvent`. The common
  `pipeline` decides whether to speak; `generation` streams/delivers and runs
  tools; `engine` owns per-scope transcripts; `llm` and `provider` are the
  backend seams. `brain/` owns the per-guild learning policy.
- Keep policy decisions transport-free and put Discord translation in
  `commands*`/`gateway`. The shell import boundary is `gateway/`, `commands*`,
  `forum`, `startup`, `service/framework`, `voice_session/playback`,
  `server/{run,discord}`, and `main`; `permission_mirror.rs` is a deliberate
  pure exception that imports only the `Permissions` bitflag type. Check it
  with `rg -l '^\s*use (serenity|poise)' src` and the wider
  `rg -l '\b(serenity|poise)::' src` for path-qualified uses.
- Pure policy modules receive time and seeds from callers and do not read the
  wall clock, randomness, Discord, or network. `llm`, `persist`, `runtime`,
  and provider/service adapters are infrastructure seams and may own their
  respective I/O; keep that ownership explicit. `AppState` locks follow the
  documented field order and are never held across a network await.
- Canonical persistence is one atomic `abbey-state.json` plus the rebuildable
  `wdbx.seg.0.jsonl` projection under `ABBEY_DATA_DIR`; consent persistence is
  independent. Scope each DM as `network:dm:<user_id>` for memory and recall;
  never use the generic `network:dm` scope for isolation. Use U+001F rather
  than colon-splitting scoped persistence keys.

## Build and verification

- Rust is pinned by `rust-toolchain.toml` to 1.98.0 stable, edition 2024.
  Keep `--locked` on Cargo commands so manifest/lock drift fails before
  deployment.
- `./check.sh` is the authoritative POSIX gate. It runs
  `cargo fmt --all -- --check`, then Python deployment, privacy, contract,
  instruction, security, and plist checks; offline macOS Swift audio-tap
  tests/build; warnings-denied Clippy; Rust tests; and the locked release
  build. Do not replace it with Cargo-only checks or pipe its output in a way
  that hides the exit status.
- Shell syntax and plist lint are glob-enumerated, but Python gates and tests
  run by explicit name; wire new ones into the relevant gate scripts. The
  privacy and Python-syntax gates have no twin.
- For Activity/docs edits, run `python3 scripts/check-pages-liquid.py`; it
  parses Liquid delimiters even inside code fences.
- On Windows use `./check.ps1`; its POSIX shell, plist, launchd, and Swift
  coverage is intentionally different. `scripts/check-audio-tap.sh` skips off
  macOS and uses synthetic PCM only.
- Focus tests with `cargo test --locked <filter>`. This is a binary crate:
  `--lib`, `-p`, and `--workspace` are meaningless, and a filter matching zero
  tests can still exit 0. Use `.claude/skills/run-abbey-bot/smoke.sh test
  <filter>` when a nonzero test count matters.
- Strict cross-repository evidence is
  `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`; set `ABBEY_WDBX_REPO` when the
  sibling checkout is not `../wdbx`. Without the required flag, a missing
  external fixture is an explicit skip.
- `cargo-audit` is pinned to 0.22.2 and the repository intentionally accepts
  locked debt recorded in `security/rustsec-accepted-debt.json`; the audit is
  not clean. Preserve the reviewed `patches/openmls_rust_crypto-0.5.1` patch and
  the Linux Rustls/WebPKI invariant. Do not change the dependency topology or
  debt policy without rerunning `scripts/check-rustsec-debt.py`.
- Windows test code must use `std::env::temp_dir()` rather than `/tmp` paths,
  build JSON with `serde_json::json!`, set accepted sockets non-blocking, and
  use `mode: disabled` in voice fixtures on non-macOS. Byte-for-byte tracked
  fixtures need LF via `.gitattributes`; cfg-only `mut` bindings need the
  narrow platform lint allowance.
- CI evidence belongs to the exact `headSha`. A short hosted Gate job with zero
  steps is an unmeasured account/billing lock, not a product failure; do not
  change code to satisfy it. Do not stack merges while the exact tip Gate is
  still in progress. Use local gate evidence while the lock persists.
- Serenity 0.12.5 keeps Components V2 crate-blocked; use classic Action Rows,
  buttons, selects, and modals. Bumping poise does not clear the blocker.
- Never treat a source gate as proof of installed artifact identity, provider
  qualification, live Discord behavior, managed-service readiness, or
  human-witnessed voice. Those layers have separate acceptance records.

## Local operation

- `launch.sh` and `run_bot.sh` are untracked owner-only helpers. They source
  `~/.config/abbey-bot/env` and then repo `.env`, so a stale repo `.env` wins.
  The managed service uses its own owner-only env and fixed private data path;
  inspect it with `sh deploy/check-launchd-env.sh ~/.config/abbey-bot/env`,
  not `ps` or the plist.
- A plist is not proof that an agent is loaded. Enumerate live agents with
  `launchctl list | grep com.donaldfilimon.abbey` and installable artifacts
  with `ls deploy/*.plist`. Do not stop, unload, reinstall, or restart them
  without explicit operator direction; the episode gateway must be ready before
  restarting the bot.
- Do not run installers, permission commands, production capture endpoints, or
  live acceptance as source validation. Production voice never retains raw
  audio or transcripts; `--voice-self-test` writes a fresh operator WAV and
  never overwrites one. Provider self-tests use synthetic probes and no
  Discord credentials.
- Keep MLAI server changes additive: preserve AI LAB and VOICE, do not
  mass-rebuild or mass-grant roles, and do not escalate Admin without an
  explicit decision. Prefer Discord REST plus host env credentials over UI
  automation; do not invent Developer Portal actions, and keep Activity URL
  mapping human-gated. Never print or commit tokens.

## Boundaries

- Keep decisions in pure modules and Discord translation in `commands*`/`gateway`.
  `pipeline::Outbound` is the fakeable shell seam; pass time and seeds into
  policy code instead of adding wall-clock or random reads.
- Keep production Rust modules below 1,000 lines; over 800 requires review.
  External test-only modules share the 1,000-line cap without the review note;
  split them while preserving test module paths. No module-wide dead-code or
  unused-import suppressions; `pub` does not exempt a binary-crate item.
- Preserve pipeline guard and budget ordering, load the guild brain on forced
  mention/DM paths, require an explicit `ToolScope`, and keep voice,
  unsolicited generation, and summaries read-only. Missing backends must render
  an honest degraded reply.
- Defer before network work; accept `ChannelId`, not `GuildChannel`; clamp
  every rendered reply. `/voice leave` is the deliberate exception: it closes
  media first, then acknowledges while teardown runs; autocomplete does not
  defer. Clear Poise permission fields in `commands_help::bind_commands` so
  the acknowledged catalog guard owns authorization. Recheck current
  authorization before private fact snapshots.
- Keep intents non-privileged by default. Message content requires both the
  environment opt-in and the Developer Portal toggle. Fetch current member and
  permission facts over REST, match overwrites by snowflake rather than
  display name, use `get_permission_names()`, consult `hierarchy_blocker` for
  acting moderation, and construct every `Action::Timeout` through
  `MAX_TIMEOUT_MINUTES`.
- Decide `/roleplay` admission only in `roleplay_gate.rs`: Aviva is allowed
  only in a bot DM or NSFW guild channel while the durable gate is on; a SFW
  guild channel always refuses. Callers use `RoleplayDecision::persona()`.
- Transcribe ABI/WDBX contracts rather than depending on ABI: `wdbx.rs`,
  `embedding.rs`, `wyhash.rs`, and `persona.rs` are pinned. Put new routing in
  `routing_signals.rs`. Keep Discord prompt copy in `ask.rs` separate from
  hosted Grok instructions: carry voice and honesty rules, but never include
  tip floors, PR/SHA state, or goal-loop/claims-ledger vocabulary. Update the
  inline `ask.rs` assertions with any intentional copy edit; keep routing in
  `persona.rs`.
- Credential-bearing types use redacted `Debug`; provider API keys travel in
  headers, while Telegram's Bot API token follows its URL-path contract and
  is never rendered in errors or logs. LLM base URLs omit `/v1` while vision
  URLs include it; an absent `DISCORD_TOKEN` may fall back to
  `DISCORD_BOT_TOKEN`, but a present blank primary fails, and guild zero is
  invalid. FM qualification uses `ABBEY_FM_CAPABILITY_MANIFEST`, not generic
  discovery metadata.
- `service/` owns admitted work through observed joins: close admission, join
  mutation owners, freeze, then attempt final persistence at most once.
  Dropping a waiter or requesting abort is not cleanup. Managed JSONL accepts
  only closed content-free observability events, and readiness needs actual
  startup evidence.
- Voice is consent- and epoch-gated. Never persist raw audio or transcripts;
  verification mode disables conversational commits; music never grants
  listening consent. Autojoin/output-only reconnect uses Pass/self-deafen.
  Destroy Decode before publishing teardown, use `play_input`, and pass player
  arguments as argv rather than script source.
- The episode gate is default-off and guild-scoped; uncovered scopes behave as
  if no gate is configured. Covered memory writes
  persist only after `appended`; refused queued writes are dropped, not retried;
  model-tool writes queue until drain. Keep receipts, preserve pre-gate
  checkpoints, never propose during shutdown, and charge budgets cumulatively
  for changed checkpoints. Do not add automatic quarantine or contradiction:
  only `/admin quarantine`, `/admin contradict`, and `/admin resolve` emit
  memory edges. Live acceptance is ignored and scratch-gateway-only because it
  permanently charges the test budget; run only the exact live test with
  `ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG=... cargo test --locked
  episode_gate::acceptance::live_memory_path_round_trips_through_a_real_gateway
  -- --ignored --nocapture`.
- Server plans are operator-only and dry-run by default. `--apply` may create
  and edit planned channels, cosmetics, topics, and overwrites, but never
  deletes, reorders roles, or edits existing role permissions. Reveal only
  engine-hidden channels, scope overwrite stages to one category, stop at the
  first failure without rollback, and rerun the dry run afterward. Normalize
  text/forum/announcement names, not voice/stage names. The current adapter has
  no guild PATCH; if one is added, a bare `public_updates_channel_id` update
  can return 200 without effect, so include `features` and `rules_channel_id`
  in the same request.
- Print and read user-visible rendered text before shipping; a green string
  assertion is not evidence that the prose reads well.
