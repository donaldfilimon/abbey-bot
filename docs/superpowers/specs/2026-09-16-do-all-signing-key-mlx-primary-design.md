# Do all: land, sign episodes, stage MLX-VLM as primary, live acceptance

Date: 2026-09-16
Status: **approved design (Donald, 14:0x EDT), implementation authorized in the
order below.** Sections 1 and 2 are repository work; sections 3 and 4 are
operations on this Mac and in Discord that this document authorizes once,
with the stop rules stated per section. It does not claim any of them done:
evidence lives in `tasks/goals.md`, appended per section.
Scope: the four items Donald selected for "do all" on 2026-09-16 after the
morning's `/goal continue` passes had exhausted agent-only slices. Out of
scope and unchanged: the Developer Portal Activity URL map, the GitHub
billing lock, the `serenity` 0.12.5 ceiling, guild-level Discord settings.

## Why now

Three facts from today's measurements drive this:

- The WDBX gateway learned to sign episodes this morning (wdbx `56767f7`),
  and `abi wdbx episode verify --json` now reports `signature_status`. Every
  live and scratch receipt reads `unsigned` because neither the deployed plist
  nor the installer passes `--episode-signing-key`. Unsigned records are a
  weaker evidence level for the constitutional ledger than the code now
  supports.
- MLX-VLM is not installed at all (no venv, no snapshot, agent not loaded),
  while the deployed primary is the Ollama backend on `127.0.0.1:11434`, which
  took 236 s for three DM turns in the live test today. The staged installer,
  smoke, and primary-switch tooling already exist in `deploy/` and have gate
  tests; they have never been run on this host.
- Two ignored live tests had drifted from product behaviour (fixed today,
  uncommitted), and the learning loop's post-deploy `step_count` growth was
  measured from persisted state. Those need landing, and the remaining
  residuals need a human in Discord.

## Section 1: land today's verified work

Commit `src/episode_gate/acceptance.rs`, `src/pipeline/tests.rs`,
`tasks/goals.md`, this spec, its inventory pin in
`scripts/test-check-pages-liquid.py`, and its index line in
`docs/superpowers/README.md` on local `main`; push by ref to a topic branch
(never `git checkout` in the shared checkout); open the PR; merge with a
**merge commit** so the checkout fast-forwards; `git pull --ff-only`.
Gate: the tree already passed `./check.sh` (exit 0, 1,306 tests) before the
docs were added; the pages-liquid test and checker rerun after.

## Section 2: episode signing key support (Proposed until 3a applies it)

**Interfaces.**

- `deploy/com.donaldfilimon.abbey-wdbx-gateway.plist`: the exec string gains
  `--episode-signing-key "$HOME/.config/abbey-bot/episode-signing-key"`.
- `deploy/install-wdbx-gateway-launchd.sh`: new `SIGNING_KEY_PATH` beside
  `BEARER_PATH`; a provisioning step after the token and policy checks and
  before `launchctl bootstrap`:
  - absent → write exactly 32 bytes from `/dev/urandom` to `$path.new` under
    the existing `umask 077`, `chmod 600`, `mv -f` into place (atomic, never a
    partially written key);
  - present → must be a regular file that is not a symlink, mode `600`, size
    exactly 32 bytes, and byte-different from
    `$STORE_DIR/gateway-membership/signing.key` when that file exists (the
    gateway refuses the membership key as the episode key; failing here gives
    a named reason instead of a bootstrap crash loop);
  - any violation exits 1 with the path and the rule, never the bytes.
- `--uninstall` leaves the key in place and says so, like the token.
- Header layout comment and the final "next:" line mention the key.
- README `## Deploying`, the WDBX gateway bullet: one sentence naming the key
  file and that the installer generates it.

**Test.** New `deploy/test-install-wdbx-gateway-launchd.py`, run by name from
`check.sh` beside the other `deploy/test-*.py` entries. It is a text pin, and
says so in its docstring: the installer cannot run under a test because it
talks to launchd. It asserts (a) the plist's `ProgramArguments` string carries
`--episode-signing-key` pointing at the same `$HOME`-relative path the
installer's `SIGNING_KEY_PATH` names; (b) the installer contains the 32-byte
generation, the mode-600 check, the size check, the symlink refusal, and the
membership-key comparison; (c) `--uninstall` still leaves the key (the
uninstall branch does not `rm` it). `sh -n` and `plutil -lint` continue to
cover syntax through the existing globs.

**Proof before live.** Full `./check.sh`. Then a scratch gateway, as run
earlier today (unused loopback ports, empty store, fresh token, a copy of the
live policy, a scratch 32-byte key), the ignored acceptance test against it,
and `abi wdbx episode verify --json` on one of its receipts reading
`signature_status: valid` with a `signer_key_id`. The scratch store and key
are deleted afterwards.

**Error handling.** The gateway validates the key file itself
(`validate_regular_file(path, private=true)`), so the installer's checks are
the same rules stated earlier with a readable reason. A gateway that refuses
the key fails the installer's readiness verify within 10 s and the installer
exits 1 with a pointer to the gateway log; the previous plist is already
replaced at that point, so recovery is rerunning the installer after fixing
the key, which the ledger records as the known rollback shape.

## Section 3: deploy steps on this Mac

Run in order; any failure stops the section and is recorded before anything
else happens. Nothing here is run as source validation; it is the deployment
Donald authorized.

**3a. Apply the signing key.**

1. Precondition: SHA-256 of `~/.local/libexec/abbey-bot/{abi,abi-wdbx-gateway}`
   equals the same files in `~/dev/active/abi/target/release`, so the installer
   rerun changes the launch arguments and nothing else. A mismatch stops 3a;
   which build should be live is Donald's call, not a silent upgrade.
2. `./deploy/install-wdbx-gateway-launchd.sh ~/dev/active/abi/target/release`.
   The gateway is unloaded and re-bootstrapped; covered memory writes fail
   closed for those seconds by design. Success is the installer's own
   readiness verify (`{"found":"false"}` for the zero digest).
3. Post-checks: the key file exists at mode 600 and 32 bytes; `launchctl list`
   shows all five `com.donaldfilimon.abbey-*` agents; the bot's
   `readiness.json` still reads ready (the bot is not restarted here).
4. Live signing evidence is the first real record after this step, produced in
   section 4 (`/remember`, then `verify --json` on its receipt). Not claimed
   before that.

**3b. MLX-VLM: install, smoke, switch primary.**

1. Disk precondition: at least 15 GiB free (43 GiB measured). The pinned
   snapshot is `mlx-community/gemma-4-12B-it-4bit` at revision
   `73bcf09092aa277861d5a191b989b666f7f32e8f` (from the installer).
2. `./deploy/install-mlx-vlm-launchd.sh`, run detached with its output to a
   log file because the model download exceeds a single tool call's window;
   progress is read from the log, never inferred. The installer stages a venv
   and the exact revision, smoke-tests text, tools and vision on a temporary
   loopback port, and rolls back on failure before any live service changes.
3. `python3 deploy/smoke-mlx-vlm.py` against `http://127.0.0.1:8282`: the six
   `tasks/todo.md` rungs (streamed text with terminal marker, one forced tool
   call with exact arguments, tool-result continuation, colour/scene vision,
   OCR recovering exact text, offline restart from the pinned snapshot). Each
   rung's result is recorded; a failing rung stops before the switch.
4. `python3 deploy/configure-mlx-primary.py --apply` with `--model-dir`
   (the snapshot directory), `--manifest` (the capability manifest the
   installer publishes), and `--binary` (the installed bot binary), which
   atomically rewrites the owner-only env (`ABBEY_BOT_LLM_ENDPOINT` →
   `http://127.0.0.1:8282`, model = snapshot dir, tools on, vision remote on
   `/v1`), backs up the previous env under `~/.local/share/abbey-bot/env-backups`,
   kickstarts the bot through the pinned `/bin/launchctl`, and requires a
   stable new PID. Run `--check` first and read its report.
5. Post-checks: `readiness.json` `phase: ready`, `discord: ready`; installed
   bot binary SHA unchanged from `3c017f0c…`; `deploy/check-launchd-env.sh`
   lists the endpoint and model names present; `/v1/models` on 8282 answers
   200; the Ollama service is left installed and untouched as the manual
   fallback the todo names. No env value is printed at any point.
6. Rollback: the env backup plus a kickstart restores the Ollama primary;
   `install-mlx-vlm-launchd.sh --uninstall` removes the sidecar. Both are
   recorded as the rollback shape, not executed unless a post-check fails.

## Section 4: live Discord acceptance

Donald present in the MLAI Office Hours voice channel; the agent observes
`readiness.json`, `~/Library/Logs/abbey-bot/`, and numeric fields of the
persisted state, and records each rung in `tasks/goals.md` with the time.
The agent sends nothing to Discord and runs no installer during this section.

Rungs, in order:

1. `/remember` one fact in the MLAI guild, then `abi wdbx episode verify
   --json` on the receipt the bot stored for it (read from the state document
   by receipt only): `signature_status: valid` closes 3a's evidence gap.
2. Voice, the `tasks/todo.md` list: `/voice status` (deployed local mode,
   inactive media, exact model ids, no credentials); fresh notification and
   explicit agreement from every human present; `/voice join consent:true`;
   a wake-name turn with attributed transcription and audible reply; barge-in
   during playback; a membership change closing the epoch; `/voice resume
   consent:true` as a new epoch after re-notification; written `stop
   listening`; `/voice leave` with no voice presence and no UDP socket.
3. Learning: one `/admin act` decision observed; `/admin brain` showing
   `step_count` above the 12 measured at 13:29; an `OverBudget` refusal only if
   the hourly budget can be driven there without spamming the guild.

Each rung is Current only when observed; a rung not reached stays open in the
todo with the reason.

## Testing summary

| Layer | Evidence |
|-------|----------|
| Repository | `./check.sh` exit 0 after section 2; the new deploy test in the run |
| Scratch gateway | acceptance test ok; one receipt `signature_status: valid` |
| Mac deploy | installer readiness verify; readiness.json; SHA and env-name checks |
| Discord | rungs observed by Donald and recorded with times |

## Non-goals

No automatic quarantine or contradiction emitter (a separate slice another
session owns), no change to `AGENTS.md`/`CLAUDE.md` (the five-agent list and
the gate description are unchanged), no Ollama uninstall, no guild-level
Discord settings, no code edits to satisfy the billing-locked hosted lanes.
