# Finishing plan — abbey-bot (2026-08-19)

> Not an SDD task plan — every automatable item below is already merged (PR #22). This is the record of what remains, who can close it, and what would reopen a sub-project.

**State:** `main` = 6980f1c, gate `./check.sh` green (332 tests), bot live from `main` (gpt-oss:20b, streaming, tools, vision on gemma4:e4b, MESSAGE_CONTENT, sandbox `act on`).

## Closed in this pass (automatable)
- Latent bug: member-join `welcome()` bypassed QUIET/`act` gates → fixed + test.
- Docs: CLAUDE.md/AGENTS.md refresh (live-run recipe, verification pointer, design records); README honesty note + benchmark table; protocol header/model; `.env.example` model; guild-loop spec gate order; SKILL.md open decisions annotated (incl. voice research); todo.md stale lines; `/admin show` no longer renders the dead `voice` field; launchd installer + README name the full env set.
- Research record: `docs/research/2026-08-19-voice-dave-entrypoint-tools.md`.

## Human-gated (one Discord session closes most)
1. In MLAI Community: `/admin brain` (telemetry read), a few ordinary messages until an `OverBudget` appears (7 actions/hour), 30 messages in an invited channel (rolling summary), `/whois` `/perms` `/modcall` `/server` `/webhook` `/remember` `/forget` `/reputation` `/summarize`, and `/see` `/ocr` with an attachment. Then sub-project 3 → `done`, goal "Breadth & ops" narrows to tokens/CI.
2. Tokens/keys you hold: `TELEGRAM_BOT_TOKEN`, `SLACK_BOT_TOKEN`+`SLACK_APP_TOKEN` (adapters wired), `ANTHROPIC_API_KEY` (routing/fallback/tool shape built, recording-transport-tested).
3. Ops: `~/.config/abbey-bot/env` (nine vars — see `deploy/install-launchd.sh` message) then `./deploy/install-launchd.sh` (stop the session bot first with `pkill -INT -f abbey-bot`). CI: register a self-hosted runner for `donaldfilimon/abbey-bot` (the existing one is for `abbey`) and switch `runs-on`, or unlock Actions billing.

## Would reopen a sub-project (needs a spec first)
- Voice: now known to require DAVE (songbird 0.6 + `davey`); out of scope until a `voice.md`-equivalent spec is written.
- Scoped-ID column rename (SKILL.md's second "6."): Donald's call.
