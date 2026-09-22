# AGENTS.md

Rust/Serenity/Poise bot, not the separate Swift AbbeyBot product (archived to `~/Archive/experimental-2026-09-18/AbbeyBot` on 2026-09-18).
`AGENTS.md` and `CLAUDE.md` are verbatim mirrors except for the first heading;
edit both bodies together. No gate enforces the mirror, so verify it yourself
with `diff <(tail -n +2 CLAUDE.md) <(tail -n +2 AGENTS.md)` before committing
either file. `README.md` owns commands, configuration and feature details;
`docs/MLAI-LIVE-ACCEPTANCE.md` owns dated live evidence, not this file.
The Boundaries below are restated in **two** editor-side files —
`.cursor/agents/abbey-reviewer.md` and `.codex/agents/abbey-reviewer.toml` — which
makes four copies of these rules in the repo counting this file and its twin. Both
restatements had drifted the same way and were corrected on 2026-09-16 (each described
a `gateway.rs` that does not exist, a five-file Discord shell where 33 non-test files
import serenity/poise, and a Gate Checklist of four cargo commands that never mentioned
`./check.sh`). Trust this file over both; a rule changed here must be changed in **both**
of them in the same commit, and the `.codex` one is easy to forget because editors load
it silently. It is TOML with a `"""` block, so a regex backslash written into it is an
invalid escape — verify with `python3 -c "import tomllib,sys;tomllib.load(open(sys.argv[1],'rb'))"`.

## Working in this checkout

- Other agents work this same checkout concurrently (a `.codex/` directory is
  present, and the reflog shows checkouts and commits that are not yours), and
  HEAD belongs to whoever is typing in it. Never `git checkout`
  here. Push refs, read another ref's files with `git show <ref>:<path>`, and
  when you need a second branch checked out, `git worktree add` it **beside**
  the repo (`../abbey-bot-wt-<topic>`), then remove it when done. Never create
  one inside the repo root. `.gitignore` covers `/abbey-bot-wt-*` as a backstop,
  because a nested worktree is an embedded repo that `git add -A` would
  otherwise commit into this tree as a gitlink; but the ignore also hides it
  from `git status`, so the placement rule is the real protection, not the
  pattern.
- `tasks/goals.md` is the goal ledger and `tasks/todo.md` its checklists. The
  ledger is append-ordered by writing session, not by time, so the newest text
  in a section is not the newest state and a stale "not done" line can sit at
  the tail below a later "done". Read the whole section and verify the artefact
  (`git log`, the gate, `launchctl`) before acting on any claim in it, and
  correct by appending, never by editing history.
- CI evidence is SHA-bound. The three-platform run counts only for the exact
  `headSha` it ran on; a parent's green run says nothing about the child, and
  the `cancel-in-progress` group means a merge burst leaves about half of
  `main`'s SHAs with a cancelled run (15 of the last 30 on 2026-09-08). The
  evidence is the run at the final head.
- A `Gate` job that completes in 2–10 s with **0 steps** (`gh run view <run>
  --json jobs` shows `steps: []`) is not red, it is unmeasured: its check-run annotation
  (`gh api repos/donaldfilimon/abbey-bot/check-runs/<job id>/annotations`) reads
  `The job was not started because your account is locked due to a billing
  issue.` Every hosted job on this account has carried it since 2026-09-08, and
  `main`'s run for `8dbdb18` at 2026-09-16 03:50Z still does, which is why all
  three lanes on every open PR are red at once. All three lanes here are
  GitHub-hosted (`ubuntu-24.04` / `macos-15` / `windows-2025`), so this repo has
  no CI evidence at all while the lock holds; the evidence is a local
  `./check.sh` run with its exit code read from the log. Never edit code to
  satisfy a locked check. Clearing it is GitHub billing settings, Donald's.
- Do **not** stack merges onto `main` while the tip SHA's three-platform Rust
  Gate is still `in_progress`. Each merge cancels in-flight tip evidence
  (`cancel-in-progress`). Open improve PRs in parallel; leave merge to a
  human/parent after the tip Gate succeeds.

## Layout and local run

`src/` is the entire product: one binary crate, roughly 250 `.rs` files (count
with `find src -name '*.rs' | wc -l`), no library target and no workspace. Everything else is support — `deploy/` the launchd installers
plus the Python tests that gate them (and a systemd unit, `abbey-bot.service`,
which with the root `Dockerfile` is documented in README but never exercised
on this host), `scripts/` the `check-*.py` gates and
their `test-check-*.py` twins, `contracts/` the frozen transcription corpus and
its lockfile, `blueprints/` the guild server plans, `activity/` the Discord
Activity web client published to Pages, `tools/abbey-audio-tap/` the macOS Swift
sidecar, `docs/spec/` the ported design, `tasks/` the ledger. `tests/` holds
**only fixtures**: there is no integration-test target, and `tests/fixtures/*`
is data read by inline `#[cfg(test)]` modules and by the `scripts/check-*.py`
and `deploy/*.py` gates.

Run it locally with `./launch.sh` (`run_bot.sh` execs the same script). Both
are **untracked**, owner-only local wrappers: absent from a fresh clone or
worktree and deliberately outside `check.sh`'s shell-syntax loop, so the
tracked recipe is README `## Running` (`cargo run` with the variables that
`.env.example` documents). `launch.sh` sources `~/.config/abbey-bot/env` first
and then repo `.env`, which **overrides** — a stale repo `.env` silently beats
the deployed credentials. Check
`launchctl list` for `abbey` before starting one: the deployed service is
managed and normally running, the repo `.env` here does set `DISCORD_TOKEN`, and
if the two resolve to the same identity Discord delivers to both sessions. README `## Running` owns
the flags, `## Deploying` the service.

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
- The macOS gate cannot see the Windows lane, and these exact classes have each
  turned `main` red after a fully green local run. In tests: derive paths from
  `std::env::temp_dir()` (a hardcoded `/tmp/...` literal is a relative path on
  Windows); build JSON with `serde_json::json!`, never by string formatting (a
  Windows temp path's backslashes make the document invalid); set an accepted
  socket's blocking mode explicitly (it inherits the listener's non-blocking
  mode and fails with `WSAEWOULDBLOCK`); pin voice fixtures to `mode: disabled`
  (the default `local` mode fails closed off macOS before the assertion runs);
  and any byte-for-byte compare against a tracked file needs `text eol=lf` in
  `.gitattributes`, because the Windows runner checks out with `autocrlf`. In
  source, a `mut` binding needed only under `cfg(unix)` wants
  `#[cfg_attr(not(unix), allow(unused_mut))]`, scoped so unix still lints it.
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
  decision path run in tests behind a recording `Outbound`. Check it in one
  line before adding a module:
  `grep -rlE '^\s*use (serenity|poise)' src/` must list only the modules
  above, plus test-only files (on 2026-09-08, four: `command_registration_tests.rs`,
  `commands_help/dispatch_tests.rs`, `commands_help/workflows/dispatch_tests.rs`,
  `commands_voice/acknowledgement_tests.rs`), plus one named exception:
  **`permission_mirror.rs` is pure and stays pure.** It imports
  `serenity::all::Permissions`, the bitflag *type*, not the client, so the grep
  reports it while the boundary holds. Do not "fix" it by moving it into the
  shell list, and expect the same shape from any future pure module that needs a
  Discord newtype. The check that decides it is a different grep:
  `grep -nE 'async|\.await|Http|Client' src/permission_mirror.rs` must come back
  empty (it did on 2026-09-16). An `.await` in such a file is a real breach.
  Do not widen that pattern to `Context` — the module's own `ActionContext`
  matches it 18 times and the check then looks failed when it is not. A new
  decision module belongs off
  that list, with its Discord edge in a `commands*` file, so the grep is the
  boundary check to run before the first import. It is necessary and not
  sufficient: it matches `use` lines only, so a module reaching serenity through
  a path-qualified attribute never appears in it — `voice_session/playback.rs`
  is listed as shell above and is invisible to that grep because it writes
  `#[serenity::async_trait]`. Widening it to `\b(serenity|poise)::` adds a
  further set of files — re-derive it with
  `comm -13 <(narrow|sort) <(wide|sort)` rather than trusting a count here; any
  number written down goes stale within a day. What matters is not the number but that every one of them is
  either test-only or already inside a module listed above, which is what makes
  the wide pattern a no-leak confirmation at the cost of noise. Read the new
  module either way.
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
- **Gates are code with tests, mostly.** Most `scripts/check-*.py` gates have a
  `scripts/test-check-*.py` twin; `check-privacy.py` and `check-python-syntax.py`
  do not, so a change there has no test to catch it. `check-systemd-unit.py` is the only
  thing that reads `deploy/abbey-bot.service` (no systemd on this Mac), so a
  hardening change there must update its pinned table too. The `deploy/*.py` service
  modules are covered by `deploy/test-*.py`. Since 2026-09-16 `check.sh`
  enumerates shell syntax (`sh -n` over `deploy/*.sh` and `scripts/*.sh`) and
  plist lint (`deploy/*.plist`) by glob, so a new script or plist in those
  directories is parsed the day it lands (the loop assumes `#!/bin/sh`; a bash
  script needs a shebang dispatch first). Every Python gate and test twin is
  still run by explicit name, so a new `.py` gate or test is dead until it is
  added there. Where a twin exists, change the gate and its test in the same
  commit.

## Boundaries

- Keep decisions in pure modules and Discord translation in `commands*`/`gateway`.
  `pipeline::Outbound` is the fakeable shell seam; pass time and seeds into pure
  policy code rather than adding wall-clock/random reads.
- `/roleplay` admission is decided only in pure `roleplay_gate.rs`: Aviva is
  allowed in a bot DM, or in an NSFW guild channel, and only while the durable
  gate is on (`/nsfw` in DMs, `/admin nsfw` in guilds). A SFW guild channel
  always refuses and never falls back to Abbey roleplay. The caller takes the
  persona from `RoleplayDecision::persona()`, not from its own match (#171
  removed exactly that duplicate). Widening the six-case admission table is
  Donald's decision.
- Transcribe, never depend on ABI: `wdbx.rs`, `embedding.rs`, `wyhash.rs` and
  `persona.rs` are pinned by golden contracts. Do not substitute the wyhash crate.
  Keep `persona.rs` frozen; compose new routing in `routing_signals.rs`. Preserve
  Abbey's distinct voice in `ask.rs`, not a generic help-desk prompt.
- Production Rust modules must be below 1,000 lines; above 800 requires review
  (`scripts/check-rust-module-size.py`). External test-only modules (every
  `tests.rs` and `*_tests.rs`) share the 1,000-line cap without the review note;
  split an oversized one into child modules by the surface under test, keeping
  every test name and the old module path as a prefix (`cargo test --locked
  voice_session::tests::` still selects all of them). Inline tests count. Preserve test module paths; no module-wide
  unused/dead-code suppression. A binary crate's `pub` does not exempt it from
  dead-code linting.
- Defer interactions before network calls; accept `ChannelId`, not `GuildChannel`
  (the latter fetches before the body). `/voice leave` instead closes media and
  music gates before its first await. Clamp rendered replies with `clamp_message`.
- Default intents remain non-privileged; message content needs both the env opt-in
  and Dev Portal enablement. Fetch member/permission facts over REST, not cache.
  Acting moderation consults `hierarchy_blocker`; match overwrites by snowflake,
  not display name, and use `get_permission_names()`, not `Debug` flag output.
  Every `Action::Timeout` is constructed through the `MAX_TIMEOUT_MINUTES` clamp
  in `moderation.rs` (Discord's 28-day ceiling); do not build one around it.
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
  not current live state. Keep the existing accounting policy. Memory-edge
  episodes (quarantine/contradict/resolve, amendment 2026-09-16) are emitted
  only by `/admin quarantine`, `/admin contradict` and `/admin resolve`: a
  quarantine is recorded by the service against the fact's receipt, a
  contradiction by the service between two receipts of the same member (the
  write sorts the pair, WDBX admits one encoding), a resolution by the
  reviewing human's keyed principal (the one write under a human principal),
  and none hides or deletes a fact. Add no automatic quarantine or
  contradiction without Donald's decision.
- Live episode acceptance is deliberately ignored: run
  `ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG=... cargo test --locked acceptance -- --ignored --nocapture`
  only against a scratch gateway. It permanently charges that guild's budget;
  see `src/episode_gate/acceptance.rs` for both required scopes.
- Server plans are dry-run by default; `--apply` mutates and re-verifies, stopping
  on failure without rollback. Keep `Change` additive (no deletes/role-permission
  edits), reveal only engine-hidden channels, and scope overwrite stages to one
  category. No mass Member grants without Donald. Discord itself rewrites text
  and forum channel names and leaves voice names alone, so the plan engine
  compares through `server::normalize_text_name` gated on
  `ChannelKind::normalizes_name` — a diff that ignores that reports permanent
  phantom drift.
- A green assertion over rendered user-visible text is not evidence that it
  reads well. Print the rendered string and read it before shipping. (There is
  no `proptest`/`quickcheck` dependency here; these are hand-rolled loops.)

## Learned User Preferences

- Prefer Discord Bot REST API plus host env credentials over Discord Electron UI automation; never paste bot tokens into chat or agent output.
- Evolve the MLAI Community guild additively (keep AI LAB and VOICE categories intact); do not wipe or mass-rebuild structure.
- Do not invent Developer Portal clicks; Activity URL mapping stays Donald human-gated.
- No mass Member role grants and no Admin→Administrator escalation without Donald’s explicit decision.

## Learned Workspace Facts

- MLAI Community guild id is `1275617641620443146` (categories include START HERE, COMMUNITY, AI LAB, VOICE, STAFF).
- Live Abbey is deployed via `deploy/install-launchd.sh` as the `com.donaldfilimon.abbey-bot` managed launchd service; use `deploy/check-launchd-env.sh` for real env checks.
- Developer Portal Activity URL map remains P0 and Donald human-gated; GitHub Pages activity URL is already live.
- Components V2 is blocked on serenity 0.12.5 alone; ship classic Action Rows / buttons / selects / modals only. poise is not part of the blocker: `poise 0.7.0` exists but requires `serenity ^0.12.5`, and `cargo tree --locked --offline -i rustls@0.22.4` shows the accepted TLS debt also descends from serenity alone (`serenity 0.12.5 -> tokio-tungstenite 0.21 -> tokio-rustls 0.25 -> rustls 0.22.4`). Bumping poise unlocks neither; only a Serenity release does.
- `/forum draft|post|perms` shipped for `#help` (`src/forum.rs`, `src/commands_forum.rs`).
- Live `/voice` 8/8 acceptance still requires Donald in the Office Hours VC on the launchd-locked process.
- **Guild-LEVEL settings are outside this crate today, and there is a Discord API trap waiting
  if that ever changes.** `src/server/discord.rs` mutates roles and channels only
  (`create_role`, `edit_role`, `create_channel`, `edit_channel`); it sends no guild PATCH, and
  `blueprints/mlai-community.toml` describes no guild-level fields. The trap, learned during
  manual REST administration of the MLAI guild rather than from this code: a bare
  `public_updates_channel_id` PATCH is **silently ignored** — it returns 200 and changes
  nothing — and needs `features` and `rules_channel_id` in the same request. It cost a
  debugging session that read the 200 as proof. Recorded here because this is where anyone
  extending the apply path to guild settings would look, not because the current code path
  hits it.
- The MLAI guild blueprint is **fully applied**, so every `--server-plan` stage now re-diffs
  to `changes (0)`. A dry run that shows changes means the guild drifted, not that work is
  pending. `--apply` is additive-only and has no delete variant; role-permission and
  role-order decisions stay manual.
- **Five** launchd agents are live (corrected 2026-09-16 03:1x; this line has now read
  one, two and four — each correct when written and stale within a week, which is the
  point of the enumeration below): `com.donaldfilimon.abbey-bot`,
  `com.donaldfilimon.abbey-wdbx-gateway`,
  `com.donaldfilimon.abbey-mlx-audio`, `com.donaldfilimon.abbey-audio-tap`, and
  `com.donaldfilimon.abbey-oh-autolisten`. **A plist in `deploy/` is not a running
  service:** `com.donaldfilimon.abbey-mlx-vlm` has both a plist and an installer there
  and was NOT loaded at that reading, so enumerate the live set with
  `launchctl list | grep com.donaldfilimon.abbey` and the installable set with
  `ls deploy/*.plist` — they are different questions and the answers differ.
  Never trust any count written here. The gateway must be up before the bot is restarted.
  They carry `KeepAlive`, so a plain `kill` respawns rather than stops them. Do not stop,
  unload, or reinstall any of them on your own initiative.
