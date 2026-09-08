# CLAUDE.md

Rust/Serenity/Poise bot, not the separate Swift `../AbbeyBot` product.
`AGENTS.md` and `CLAUDE.md` are verbatim mirrors except for the first heading;
edit both bodies together. `README.md` owns commands, configuration and feature
details; `docs/MLAI-LIVE-ACCEPTANCE.md` owns dated live evidence, not this file.

## Verification

- Rust **1.98.0 stable**, edition 2024 (`rust-toolchain.toml`); single binary crate.
- Gate: `./check.sh`. It runs fmt, deployment/privacy/Pages/contracts/security
  checks, offline macOS Swift audio-tap tests/build, then locked all-target Clippy
  with `-D warnings`, locked Rust tests and a locked release build.
- Keep `--locked`: the gate must catch manifest/lock drift before deployment.
  Do not pipe gate output through a command that hides its exit status.
- Focus: `cargo test --locked moderation::` or `cargo test --locked <filter>`;
  tests live in the binary, not a library or workspace member. `-p` and
  `--workspace` are therefore meaningless here and `--lib` matches nothing while
  still exiting 0: a selector that quietly picks no tests reads exactly like a
  pass.
- Strict release evidence: `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`.
  `scripts/check-wdbx-conformance.py` compares the frozen v1 fixture to `../wdbx`;
  `ABBEY_WDBX_REPO` overrides that path. Missing external fixtures otherwise skip.
- `cargo-audit` must be exactly **0.22.2**; accepted debt is pinned in
  `security/rustsec-accepted-debt.json`. The Linux TLS check rejects native TLS;
  preserve the reviewed `patches/openmls_rust_crypto-0.5.1` dependency patch.
- Windows CI uses `./check.ps1`: POSIX/plist, launchd execution and Swift checks
  are not equivalent coverage. `scripts/check-audio-tap.sh` skips off macOS;
  on macOS it uses Xcode Swift, clears `TOOLCHAINS`, and tests synthetic PCM only.
- Markdown check: `python3 scripts/check-pages-liquid.py`. Pages parses template
  delimiters even in code fences; use Liquid raw spans or avoid those delimiters.
- Source gates do not prove installed artifact identity, provider qualification,
  Discord behavior or audible consented voice. Never run installers, permission
  commands or production capture endpoints as source validation.
- To check what the *deployed* service actually has in its environment, run
  `sh deploy/check-launchd-env.sh ~/.config/abbey-bot/env`. Do not infer it from
  the launchd plist and do not infer it from `ps`: the plist is generated from
  `deploy/com.donaldfilimon.abbey-bot.plist`, whose `EnvironmentVariables` holds
  only `RUST_LOG` and by design never carries secrets, while the real values live
  in the env file and `main.rs` injects them with `std::env::set_var` after exec,
  so they never reach the initial environ block that `ps eww` prints. Both
  surfaces report a variable that IS set as unset.

## Architecture

`README.md` `## Design notes` owns the reasoning and `docs/spec/*.md` owns the
ported design (`brain.md`, `adaptivelearning.md`, `platforms.md`,
`botarchitecture.md`, `appleintelligence.md`); this section is only the map.
Almost every module opens with a `//!` header naming its own seam and the spec
section it ports. Read that header before the code.

- **Shell versus pure, by module.** Only these import serenity or poise:
  `gateway/`, `commands*`, `forum`, `startup`, `service/framework`,
  `voice_session/playback`, `server/{run,discord}`, `main`. Everything else,
  including `pipeline`, `platform`, `brain/`, `engine`, `memory`, `tools`,
  `persist`, `wdbx`, `llm/`, and `server.rs` itself, is pure: no locks, no
  network, no clock, with `now` and seeds injected by the caller. Nothing
  enforces this but the convention and review, and it is what lets the entire
  decision path run in tests behind a recording `Outbound`.
- **Inbound path.** A native event becomes a `platform::SocialEvent`; `pipeline`
  decides *whether* Abbey speaks (triage, intent, 18-dimension state encoding,
  the guild's policy, cooldown, hourly budget) and `generation` decides *how*
  (stream, post early, edit in place, run model tools, repeat). `engine` holds
  the per-scope multi-turn session and renders the prompt;
  `llm/{dialect,transport,protocol,stream}.rs` is the wire. `pipeline` is
  written once for every network, which is why the learning loop is identical
  across Discord, Telegram and Slack.
- **State.** `runtime::AppState` is one `Arc` with a `Mutex` per registry, taken
  briefly and never held across an await that touches the network.
  `persist::Stores` writes one JSON document plus one WDBX segment under
  `ABBEY_DATA_DIR`, atomically (temp file then rename), so a crash mid-persist
  leaves the previous document intact.
- **Learning.** `brain/` is the whole loop and is entirely pure: `state.rs`
  (action space and encoder), `dqn.rs` with `nn.rs`, `registry.rs` (one policy
  per guild), `reward.rs` (delayed settlement), `social.rs` (reputation),
  `budget.rs`, `replay.rs`.
- **Gates are code with tests.** Each `scripts/check-*.py` has a
  `scripts/test-check-*.py` beside it and `check.sh` runs both. Change a gate and
  its test in the same commit.

## Boundaries

- Keep decisions in pure modules and Discord translation in `commands*`/`gateway`.
  `pipeline::Outbound` is the fakeable shell seam; pass time and seeds into pure
  policy code rather than adding wall-clock/random reads.
- Transcribe, never depend on ABI: `wdbx.rs`, `embedding.rs`, `wyhash.rs` and
  `persona.rs` are pinned by golden contracts. Do not substitute the wyhash crate.
  Keep `persona.rs` frozen; compose new routing in `routing_signals.rs`. Preserve
  Abbey's distinct voice in `ask.rs`, not a generic help-desk prompt.
- Production Rust modules must be below 1,000 lines; above 800 requires review
  (`scripts/check-rust-module-size.py`). External test-only modules are exempt,
  inline tests count. Preserve test module paths; no module-wide unused/dead-code
  suppression. A binary crate's `pub` does not exempt it from dead-code linting.
- Defer interactions before network calls; accept `ChannelId`, not `GuildChannel`
  (the latter fetches before the body). `/voice leave` instead closes media and
  music gates before its first await. Clamp rendered replies with `clamp_message`.
- Default intents remain non-privileged; message content needs both the env opt-in
  and Dev Portal enablement. Fetch member/permission facts over REST, not cache.
  Acting moderation consults `hierarchy_blocker`; match overwrites by snowflake,
  not display name, and use `get_permission_names()`, not `Debug` flag output.
- DMs scope as `network:dm:user`, never one shared DM guild. Recall filters both
  scoped guild and user; persistence uses U+001F, not colon-splitting scoped IDs.
  Browser navigation rechecks current authorization before a fresh fact snapshot.
- Preserve pipeline guards/budget ordering and forced-path brain loading; tools
  require an explicit `ToolScope`. Voice, unsolicited generation and summaries
  remain read-only. Missing backends render an honest degraded reply.
- LLM base endpoints exclude `/v1` (`llm/dialect.rs` appends it); vision bases
  include it. Optional blank backend values differ from a blank `DISCORD_TOKEN`,
  which fails rather than falling through to `DISCORD_BOT_TOKEN`. Reject guild 0.
  Credential-bearing types need redacted `Debug`, and keys travel in headers.
- FM qualification consumes `ABBEY_FM_CAPABILITY_MANIFEST`, not the generic
  `ABBEY_PROVIDER_MANIFEST`; discovery metadata alone is not an executable adapter.
- `service/` owns admitted work through observed joins; dropping a waiter or
  requesting abort is not cleanup. Close admission, join mutation owners, then
  freeze and attempt final persistence at most once. Consent persistence is
  independent. Managed JSONL accepts only closed `observability` types, never raw
  tracing/content/identifiers/errors; readiness needs actual startup evidence.
- Voice transitions advance the cancellation epoch before work; raw audio and
  transcripts are not persisted. Verification mode disables conversational commits.
  Music never grants listening consent. Destroy Decode before publishing teardown
  for that epoch; output-only reconnect uses Pass/self-deafen. Use `play_input`,
  not `play_only_input`, so TTS cannot destroy the independently owned music track.
  Music arguments are argv data, never interpolated AppleScript source.
- Episode gating is default-off and scoped through `AppState::gate_for`. Covered
  memory writes require `appended`; synchronous model tools queue until drain,
  and refused items are dropped. Checkpoints persist only admitted or pre-gate
  on-disk rows; shutdown must not propose. Preserve receipts and regenerate episode
  fixtures from canonical types, not by hand (`episode_gate/tests.rs`). Budgets
  charge each candidate cumulatively without refunds; size for changed checkpoints,
  not current live state. Keep the existing accounting policy.
- Live episode acceptance is deliberately ignored: run
  `ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG=... cargo test --locked acceptance -- --ignored --nocapture`
  only against a scratch gateway. It permanently charges that guild's budget;
  see `src/episode_gate/acceptance.rs` for both required scopes.
- Server plans are dry-run by default; `--apply` mutates and re-verifies, stopping
  on failure without rollback. Keep `Change` additive (no deletes/role-permission
  edits), reveal only engine-hidden channels, and scope overwrite stages to one
  category. No mass Member grants without Donald.
