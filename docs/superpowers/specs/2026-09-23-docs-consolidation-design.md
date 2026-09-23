# Docs + tasks consolidation design (Phase A)

Owns: the approved design for Phase A of the docs + tasks consolidation (target
tree, task breakdown, gate changes). Does not own: current-state documentation
content itself (see the individual consolidated docs once written); the guild
blueprint redesign (Phase B, a separate design record). Last verified:
2026-09-23 against `~/.claude/plans/plan-all-superpowers-brainstorming-temporal-squid.md`.

Status: approved 2026-09-23; implementation on codex/docs-consolidation.

## Context

Donald chose, in order: docs first, consolidate hard into fewer and larger
documents with dated history kept in appendices rather than dropped; and
`tasks/` ledgers consolidated in place into one current-state file, with older
history recoverable through git rather than carried forward as prose. This
document is that design, approved before implementation, replacing a separate
per-file spec round.

## Phase A: docs + tasks consolidation (`codex/docs-consolidation`, from `codex/docs-launchd-and-ledger`)

Base prep: cherry-pick #177's `45f9365` and `6abd39a` (Phase A also edits the
check.sh/check.ps1 block #177 edits; identical patches fold at merge). Phase A owns all
AGENTS.md prose, including rewriting "blueprint fully applied, changes(0)" to "live guild
matches blueprint `f74bb7a` as of 2026-09-07; redesign on `codex/blueprint-quesar`, not
applied", so Phase B never edits AGENTS.md. Any edit inside `## Boundaries` requires
`python3 scripts/check-instructions.py --write` and committing the regenerated
`.cursor`/`.codex` reviewer files (planned: none). The design is also committed as
`docs/superpowers/specs/2026-09-23-docs-consolidation-design.md`.

Record the pre-consolidation commit (current branch HEAD) as `LEDGER_BASE`; every old
`goals:NNNN` / `todo:NNNN` / `MLAI:NNN` citation resolves via `git show
LEDGER_BASE:<path>`.

Target tree (each file opens with a 3-line "owns / does not own / last verified"
header; no counts that can rot):

| New file | Absorbs | Notes |
|---|---|---|
| `docs/README.md` | docs/README.md + docs/superpowers/README.md | Index by question ("how do I launch the Activity?" -> file#section); no counts |
| `docs/discord-application.md` | activities.md, discord-application-api-roadmap.md, ops/monetization-portal-checklist.md | Current state first (shipped / partial / blocked / never), then Portal operator steps, then Appendix: dated acceptance notes verbatim. Keeps the five checker marker strings |
| README.md `## Commands` | ops/slash-command-catalog.md | AGENTS.md: README owns commands. Fold only content README lacks (permission model); discord-application.md links there |
| `docs/live-acceptance.md` | live-test-protocol.md (rules), MLAI-LIVE-ACCEPTANCE.md (current gaps + runbook), superpowers/plans completion-progress, catalog-audio-tap-evidence, voice-play-evidence, benchmarks/, research/ | Rules, then current gap table, then runbook; Appendix A dated evidence (newest first, verbatim), Appendix B benchmark + research notes with their dates and a "superseded by" line where stale. The mlx-vlm diagnosis spec is NOT touched (W4 appended its closure note there) |
| `docs/brand.md` | unchanged content | Kept: small, current, owns the claims policy |
| `docs/spec/{brain,adaptivelearning,multiguild,platforms,vision}.md` | unchanged | Rust-cited behavior references; header says "Swift-lineage port; Rust truth is the citing module's `//!` header" |
| `docs/spec/swift-lineage.md` | botarchitecture, companionapp, discordbmapi, appleintelligence (one section each) | Cited sections keep anchors the `src/` cites point at |
| (deleted) | docs/spec/SKILL.md | Stale copy of a claude.ai account skill (paths do not match this machine), uncited in src; remove it and the `SUPPORTED_FRONT_MATTER_PATH` special case (gate + twin, same commit) |
| `docs/superpowers/{specs,plans}/*` | kept as immutable design records minus the evidence files moved above; `2026-08-10-abbey-ai-backend-proposal.md` moves to `specs/` | Fix only status lines (shipped / merged / superseded) |

Tasks: `tasks/goals.md` becomes the single current-state ledger: per goal
(17 -> grouped) status, open items with blocker class (live Discord, voice/human, Mac
service, hardware, billing lock, upstream crate, Donald decision), done items with one
evidence pointer (commit/PR), plus a "Ledger history" line naming `LEDGER_BASE`.
`tasks/todo.md` is removed (its open items fold into goals.md). `session-log.md`
untouched. This session's outcomes (#176, #177, W2, W4, launchd measurement, docs and
blueprint work) are entered as current state.

Reference updates (same commit as the moves): `src/**` `//!` cites (22, mostly
unchanged paths; the three Swift-lineage cites repoint to `swift-lineage.md#section`),
`activity/app.js`, `activity/README.md`, `activity/server/token-exchange.example.mjs`,
`README.md` (6 links incl. the live-test-protocol anchor), `AGENTS.md` (docs/spec
ownership line; ledger rules rewritten: goals.md is current-state, corrected in place,
history via git at `LEDGER_BASE`; drop todo.md mentions), `QUALITY_ASSESSMENT.md`,
`.claude/settings.json` / `.codex/hooks.json` Stop-hook text if it names todo.md (both
are tracked; read first, change only the path text).

Gates changed with their twins in the same commit: Pages allowlist
(`scripts/test-check-pages-liquid.py`), front-matter path
(`scripts/check-pages-liquid.py`), activity checker path + test
(`deploy/check-activity-url-map.py`, `deploy/test-check-activity-url-map.py`). Add one
small gate `scripts/check-docs-links.py` + twin (every relative `.md` link and anchor in
docs/, README.md, AGENTS.md, tasks/ resolves; every docs file is linked from
docs/README.md), wired into check.sh and check.ps1.

Tasks for subagents (sequential, one branch): A1 skeleton + moves of whole files and
reference/gate updates (mechanical, sonnet); A2 write `discord-application.md` (opus);
A3 write `live-acceptance.md` (opus); A4 `swift-lineage.md` + spec headers + superpowers
status lines + `docs/README.md` + README commands fold (sonnet); A5 tasks/goals.md
consolidation + AGENTS.md prose (opus, highest risk: every open item in LEDGER_BASE's
goals.md and todo.md must map to an entry, verified by a reviewer diffing the item
lists); A6 link gate (sonnet). PR body lists Pages URLs that stop resolving
(`/docs/activities.html`, `/docs/discord-application-api-roadmap.html`,
`/docs/MLAI-LIVE-ACCEPTANCE.html`, `/docs/live-test-protocol.html`, `/tasks/todo.html`,
the ops/ and research/benchmark pages) and confirms nothing in the repo links to them.
