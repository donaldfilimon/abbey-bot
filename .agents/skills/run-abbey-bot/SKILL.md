---
name: run-abbey-bot
description: Build, run, smoke-test and drive the abbey-bot Rust/serenity Discord bot on this Mac without touching the live launchd service. Use when asked to run abbey-bot, start it, build it, self-test the provider or voice chain, check the deployed service status, dry-run a server plan, or run a focused cargo test.
---

abbey-bot is a headless Discord bot whose only interactive surface is the
Discord gateway, and on this machine the launchd service
`com.donaldfilimon.abbey-bot` already owns that token. So "run the app" means
driving the release binary through its **token-free modes** with
`.agents/skills/run-abbey-bot/smoke.sh`, which runs non-plan modes with an
explicit nonsecret environment allowlist and a timeout. The only credentialed
mode is the read-only `plan` REST diff, which refuses `--apply`. The artifacts
that stand in for a screenshot are the provider JSON report and the Kokoro WAV
it writes. Never start a second gateway here (see Gotchas).

All paths are relative to the repo root.

## Prerequisites

Rust 1.98.0 stable comes from `rust-toolchain.toml` via rustup. The
self-tests require the configured loopback services. This setup expects
Ollama on `127.0.0.1:11434` with `gemma4:12b` available (reasoner + vision) and
the MLX-Audio sidecar on `127.0.0.1:8181` (Whisper + Kokoro). Their endpoints are
read from the owner-only `~/.config/abbey-bot/env` file as literal data; the
driver never evaluates that file as shell code or prints its values. Recheck
model availability before each run; a reachable endpoint alone does not prove
the configured model is available.

```bash
curl -s -m 5 http://127.0.0.1:11434/api/tags | python3 -c 'import sys,json; print([m["name"] for m in json.load(sys.stdin)["models"]])'
curl -s -m 5 -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8181/v1/models
```

## Build

```bash
cargo build --locked --release
```

Keep `--locked`. The driver's `build` step also prints the binary's SHA-256,
which the provider report echoes back as `abbey_binary_sha256`, so you can
prove the report came from the binary you built.

## Run (agent path)

```bash
.agents/skills/run-abbey-bot/smoke.sh all
```

That runs `build`, `args`, `provider primary`, `voice`, `status` in order and
stops at the first nonzero step. Run individual modes to collect the remaining
diagnostics after a failure. On success it prints the output directory
(`$TMPDIR/run-abbey-bot-<timestamp>/`). Historical 2026-09-08 result:
exit 0, provider `overall_pass: True` in 44 s, WAV written
in 11 s with 100% round-trip word recall, live service reported `ready`.

| command | what it does | result/exit contract |
|---|---|---|
| `smoke.sh build` | `cargo build --locked --release`, prints binary sha256 | 0 |
| `smoke.sh args` | proves `--bogus`, a `--provider-self-test` without `--json`, and a bare `--voice-self-test` all exit 2 with usage text | 0 |
| `smoke.sh provider [primary\|all]` | `--provider-self-test … --json` with synthetic fixtures; writes `provider-<target>.json` and summarises each lane/mode | CLI: 0 required probes pass; 1 required probe fails; 2 invalid/missing configuration or arguments. Wrapper timeout/parser failures can return other statuses. |
| `smoke.sh voice` | `--voice-self-test <fresh>.wav`: Kokoro TTS → Whisper STT → Abbey generation → Kokoro; writes `audition-<timestamp>.wav` on success | 0 on success; nonzero on failure |
| `smoke.sh status` | read-only evidence about the live service: `deploy/service-status.py`, `deploy/check-launchd-env.sh` (names only), `launchctl list` | 0 |
| `smoke.sh plan GUILD_ID [--stage S] [--category C]` | dry-run diff of `blueprints/mlai-community.toml` against a live guild over REST; refuses `--apply` | 0 |
| `smoke.sh test FILTER` | `cargo test --locked FILTER`; **fails if the filter selects zero tests** | 0 / 1 |

Every non-plan mode runs with an explicit nonsecret environment allowlist;
persistence and episode-gate paths are blank, so no gateway opens, nothing
persists, and no proposal reaches the WDBX ledger. The `plan` mode passes only
the Discord token and bounded read-only configuration it needs. Set
`RUN_ABBEY_BOT_OUT=/some/dir` to choose the output directory.

Artifacts land in `$TMPDIR/run-abbey-bot-<timestamp>/`:
`build.log`, `provider-<target>.json` (+ `.stderr`), `voice.stdout`,
`audition-<timestamp>.wav`, `plan.txt`, `test.log`.

### Direct invocation for PR work

Most PRs touch `src/commands_voice/`, `src/commands_help/`,
`src/command_catalog/`. Tests live in the binary, so the direct path is a
filtered `cargo test`:

```bash
.agents/skills/run-abbey-bot/smoke.sh test voice_ux::
```

Verified: `running 10 tests … 10 passed`, exit 0. The driver exits 1 when a
filter matches nothing, because plain cargo prints `running 0 tests` and
exits 0, which reads exactly like a pass.

### Server-plan dry run (touches the live guild, read-only)

```bash
.agents/skills/run-abbey-bot/smoke.sh plan 1275617641620443146
```

Verified on the MLAI Community guild: `changes (0)` plus two manual-review
notes about `Moderator`/`Member` role permissions, exit 0 in about a second.
A dry run that shows changes means the guild drifted, not that work is
pending. `--apply` is an operator action per README; the driver refuses it.

## Run (human path)

`cargo run` with `DISCORD_TOKEN` set opens a gateway session. **Do not do
that on this machine while `launchctl list | grep abbey-bot` shows the
service loaded**: the live process holds the same application token, so a
second session double-registers commands and double-replies in the MLAI
guild. Deployment goes through `deploy/install-launchd.sh` only, on Donald's
say-so; not verified here and not part of this skill.

## Test

Full gate (fmt, deploy/privacy checks, Swift audio-tap tests, locked
clippy, locked tests, locked release build):

```bash
gate_log="$(mktemp "${TMPDIR:-/tmp}/abbey-bot-gate.XXXXXX")"
if ./check.sh > "$gate_log" 2>&1; then
    gate_exit=0
else
    gate_exit=$?
fi
printf 'EXIT: %s; log: %s\n' "$gate_exit" "$gate_log"
(exit "$gate_exit")
```

Verified 2026-09-08: exit 0 in 163 s, `1277 passed; 0 failed; 5 ignored`.
Never run it under `nohup`/`&`: the installer signal tests inherit `SIG_IGN`
and fail. Claude's PostToolUse hook in `.claude/settings.json` runs the gate
after Rust edits; Codex sessions must invoke the gate explicitly.

## Gotchas

- **The release binary in `target/` and the deployed one in
  `~/.local/libexec/abbey-bot/` are separate files.** Rebuilding `target/`
  never touches the live service; `smoke.sh status` reads the live one.
- **`provider all` has no fixed expected exit.** Read `overall_pass` and every
  `fm_cli_modes` entry in `provider-all.json`. `fm_cli` records only the first
  mode; the driver's console summary labels each available mode. A passing
  capability in one mode does not qualify all capabilities or publish/admit a
  route. Use `primary` for its primary probe and `all` to include the FM lanes;
  check the report's exact identities and the runtime's admitted route separately.
- **Self-test scope differs from gateway routing.** The primary probe uses
  the legacy endpoint backend directly; FM modes are separate probes. The voice
  self-test uses only the local LLM backend and does not select FM. Its
  `NoConfiguredProvider` result does not diagnose FM admission.
- **`sh -x smoke.sh …` is safe for the owner env path.** The driver reads
  `~/.config/abbey-bot/env` as literal data, keeps tracing disabled while
  parsing it, and restores tracing only after the sanitized child environment
  is built. The separate `~/.config/abbey-bot/load.sh` helper has no such
  guard, so never trace a shell that sources it.
- **Historical observation, 2026-09-08:** `GET /v1/models` on 8181 returned
  `{"data":[]}` and that voice self-test passed. Recheck the selected models
  and run the actual voice self-test; inventory alone does not qualify voice.
- **The voice mode refuses to overwrite.** Against an existing WAV it exits 1
  in about a second with `StartupError("the voice self-test refuses to
  overwrite <path>")`, so the driver timestamps the path.
- **`$TMPDIR` ends in `/` on macOS.** The driver strips it; if you build
  paths by hand you get `//` in output, harmless but confusing in logs.
- **A filtered `cargo test` that matches nothing exits 0** (measured:
  `running 0 tests … ok`). This is a single binary crate; use
  `cargo test --locked FILTER` and read the `running N tests` line, or use
  `smoke.sh test`, which rejects an empty selection.
- **An inherited `ABBEY_EPISODE_GATE_CONFIG` can enable ledger writes.**
  Covered memory writes need the configured WDBX gateway or fail closed;
  the driver blanks the config, so its self-tests never propose to the ledger.

## Troubleshooting

- **`unknown argument "--bogus"; usage: abbey-bot [...]`, exit 2**: correct
  behaviour; the binary validates argv before building a runtime.
- **`usage: abbey-bot --provider-self-test primary|fm|all --json`, exit 2**:
  `--json` is mandatory and must be the last argument.
- **`Error: StartupError("the voice self-test refuses to overwrite …")`,
  exit 1**: the output WAV already exists. Pick a fresh path.
- **`refusing --apply from the driver`**: by design. Run the README command
  by hand with the env loaded if you actually intend to mutate the guild.
- **`filter 'X' selected no tests`, exit 1**: the filter matched nothing;
  check the module path (`voice_ux::`, `commands_help::`, …).
