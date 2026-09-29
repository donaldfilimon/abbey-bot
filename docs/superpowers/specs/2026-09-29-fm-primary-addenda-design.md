# Apple FM primary, auto-deploy, and capped style addenda

Status: approved by Donald on 2026-09-29 (plan approval in session). This spec
records his decisions and the design built on them. It supersedes the earlier
FM invariants only where it says so explicitly.

## Decisions

1. **Text backend order:** `fm --model pcc` first, `fm --model system` second,
   the configured OpenAI-compatible endpoint (MLX Gemma) last. The endpoint
   stays the vision and OCR route, because FM vision and OCR failed
   qualification on build 26A5416b.
   *This supersedes* "PCC remains intentionally unqualified"
   (`src/provider/qualification.rs`) and "FM is never ambiently selected"
   (`docs/superpowers/specs/2026-09-04-provider-runtime-modernization-design.md`).
   PCC is now qualified exactly like system.
2. **Auto-update commands:** a self-hosted CI job deploys after
   `Gate (macOS)` passes on a push to `main`. Slash commands already
   bulk-overwrite on every start, so every deploy republishes them. Donald
   granted standing authorization for this job to restart the live service.
3. **Learn and update self:** automatic, capped, per-guild **style addenda**.
   They come only from guilds with `learning_enabled` (default off), from a
   closed vocabulary, and are rendered from fixed templates. Model-authored
   or user-authored free text never enters the prompt through this path.
4. **Shipping:** push directly to `main` per slice after a full green
   `./check.sh`, then wait for the exact-head `Gate (macOS)` before the next
   push.

## Design

### Provider route

- **`FmRoute` config.** `ABBEY_FM_MODE` accepts an ordered, deduplicated
  comma list (`pcc,system`); a single value still parses. The route expands
  into one `FmConfig` per mode, and `FmConfig` stays single-mode, so
  `FoundationModels`, the identity helpers and qualification keep their
  current shape.
- **Role.** `ABBEY_FM_ROLE=fallback|primary` sets FM's place in the order.
  `ABBEY_FM_FALLBACK=1` stays as the alias for `fallback`.
  `ABBEY_FM_PCC_TIMEOUT_SECS` overrides the PCC timeout.
- **Registration.** There is one CLI provider per mode. System keeps the
  existing IDs (`foundation-models-cli`, manifest record `foundation-models`);
  PCC uses `foundation-models-cli-pcc` and manifest record
  `foundation-models-pcc`. With role `primary`, the FM providers register
  ahead of `primary`/`local-fallback`, so `legacy_order` picks PCC, then
  system, then the endpoint.
- **Fallback.** Fallback stays one hop per request, as pinned by
  `routing_tests.rs`. A failing PCC opens its circuit, so later turns start
  at system.
- **Degrade instead of refusing to start.** A missing, stale,
  identity-changed or refused manifest no longer fails startup for text
  routing. Each mode records a typed `FmQualificationState`; unqualified
  modes register unadmitted, and the state is visible in status surfaces.
  - Why: an installer rollback, or an OS update that changes `fm`, would
    otherwise crash-loop the live service.
  - What still protects it: the per-call `fm` SHA recheck keeps refusing a
    changed executable.
  - FM vision keeps its strict startup requirement.
- **PCC under launchd.** Qualification is published by the deploy job on
  the self-hosted runner, which is a user LaunchAgent in the same GUI domain
  as the bot, so a passing PCC probe there is evidence for the service
  context. If PCC refuses, it stays unqualified, system serves, and that is
  recorded. It is never worked around.

### Prompt

- `--instructions` carries the static Abbey system prompt
  (`ask::system_prompt`) plus rendered addenda: template text only, never
  user data, because argv is visible to same-user `ps`.
- Facts, tool guidance and the transcript stay on stdin.
- A pure `prompt_budget::fit` trims deterministically for providers with
  small windows (fm system): oldest turns first while keeping the latest user
  turn, then the least relevant facts. It never trims the persona core,
  capability guidance or addenda.
- The budget is re-applied after a fallback switches providers.

### Style addenda

- `brain::style_signal::classify(text)` maps a closed lexicon to
  `TooLong | TooShort | TooFormal | TooCasual | NoEmoji | MoreEmoji | PreferCode`.
- `brain::addenda::AddendaLedger`:
  - `observe(user, signal, now)`, `tick(now)`, `revert(knob, now)`, `render()`.
  - Policy: 5 signals from at least 3 distinct users within 7 days; 14-day
    TTL; at most 4 addenda and 400 rendered bytes per guild.
  - A revert suppresses the same knob for the TTL.
- Wiring, only in guilds with `learning_enabled`:
  - observe beside the outcome classification in `pipeline`;
  - tick from `learn_all`;
  - render after the persona core for guild scopes;
  - persist in `abbey-state.json` with `#[serde(default)]`.
- `/admin addenda list|revert|clear` inspects and reverts addenda. The
  capability guidance says honestly that style can adapt within fixed limits
  and that Abbey does not rewrite her own code.

### Deployment

- `deploy/configure-fm-primary.py` switches the owner env (allowlisted
  `ABBEY_FM_*` keys only), modelled on `configure-mlx-primary.py`.
- `publish-provider-qualification.py --target fm` emits one record per mode.
- **CI job `deploy-macos`:**
  - Triggers: `needs: gate-macos`; runs on a push to `main` or on
    `workflow_dispatch` (requalify after an OS or `fm` update).
  - Concurrency: group `abbey-deploy`, no cancel-in-progress.
  - Steps: stable target dir, then the locked release build, then publish
    the manifest against that binary, then `install-launchd.sh` in the
    foreground, then assert the installed SHA equals the manifest SHA, then
    `service-status.py`.
  - The job holds no secrets.

## Non-goals

Streaming FM replies (the CLI does not stream); FM vision/OCR; changing the
command catalog beyond `/admin addenda`; free-text self-modification; any
change to ABI/WDBX boundaries or the persistence format beyond an additive
field.

## Acceptance

- Source: every slice gate-green, with exact-head `Gate (macOS)` success.
- Deploy: the job log shows the SHA match and a ready status.
- Live, by Donald:
  - a PCC reply in Discord;
  - a system fallback observed;
  - an addendum applied, then reverted with `/admin addenda revert`.
- The goal stays `in_progress` until the live acceptance happens.
