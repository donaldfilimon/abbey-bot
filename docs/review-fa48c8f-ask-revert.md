# Review: fa48c8f + uncommitted src/ask.rs revert — Abbey system prompt skill compliance

**Date:** 2026-09-24
**Refs:** `fa48c8f` (HEAD, `fix: harden agent guidance and local verification`), uncommitted working-dir `src/ask.rs` (revert of that commit's ask.rs toward `HEAD~1`), skill `~/.agents/skills/abbey-system-prompt/SKILL.md`, `AGENTS.md` Boundaries (mirrored to `.cursor/.codex/abbey-reviewer.*`), `src/generation/capability_guidance.rs`, `.claude/skills/run-abbey-bot/smoke.sh`

No edits made. No commit created. Working directory left dirty (`M src/ask.rs`) as found.

---

## Verdict

**Revert to Discord-specific. Do not keep the transport-neutral `fa48c8f` ask.rs as the base.**

`fa48c8f` deliberately makes `src/ask.rs` transport-neutral (`local-first chat companion`, `do not promise a roleplay control`, no `/roleplay` in `system_prompt`, `chat conversation` / `external systems`, degraded copy mentioning `ANTHROPIC_API_KEY`). That directly violates the frozen Abbey system prompt skill contract for Discord and `AGENTS.md`'s own boundary, while duplicating work already done correctly in `capability_guidance.rs`. The uncommitted revert on disk (restoring `local-first Discord companion` and `/roleplay` in all three contracts) is the correct direction and should be landed as a proper commit after fixing the two minor residual nits below.

The skill and `AGENTS.md` carve the surface explicitly: `ask.rs` is **the Discord prompt**; transport variation belongs in `generation/capability_guidance.rs` (scope-gated suffix). The file header's "intentionally drift from abi-ai" (`src/ask.rs:12-14`) authorizes drift **from `abi/crates/abi-ai/src/identity.rs`**, not from the skill's Discord contract. It does not license erasing `Chief of Staff`, `Dual CoS + companion`, `MLAI`, or Discord framing from the Discord bot. #162→a0ad563 rot is exactly why the tip floor is banned — both versions correctly ban it, but only the Discord-specific base satisfies the rest of the contract.

**If transport-neutral is still wanted**, it must be proposed as a **skill + AGENTS.md amendment first**, not shipped as a silent copy edit. The skill currently pins the Discord phrases verbatim (see analysis §3).

---

## 1) Skill compliance — current HEAD (fa48c8f) is non-compliant, working-dir revert restores compliance

### [HIGH] `src/ask.rs:48` (fa48c8f) — `local-first chat companion` breaks skill pin

- **Severity:** HIGH — contract violation, test-enshrined pin.
- **Location:** `src/ask.rs:48` at fa48c8f (`contract_description(Persona::Abbey)`).
- **Description:** Skill requires Abbey's description to contain `local-first Discord companion` verbatim (skill § "What the tests pin"). `fa48c8f` changes it to `local-first chat companion`. The revert on disk restores `local-first Discord companion`, which is correct.
- **Banned-term check:** Both versions pass the banned list (`Tip floor`, `a0ad563`, `Partial / Proposed / Blocked`, `Current` as claim label, `empty goal`, `Cross-lane`, `Quesar`) — no hosted vocabulary leaked into prompts. `grep -rn` confirms zero prompt hits. Comment-only mentions in `src/ask.rs:19-20` are intentional documentation of what's *excluded* and are not emitted in prompts (test `discord_prompts_carry_no_goal_loop_or_ledger_vocabulary` / `prompts_carry_no_goal_loop...` enforces).
- **Suggestion:** Keep `local-first Discord companion`. If a loopback-only generic surface truly needs a neutral base, introduce it as a **separate** `*_chat` constructor, not by mutating the Discord contract.
- **Status:** FAIL at HEAD, PASS on working-dir revert.

### [HIGH] `src/ask.rs:48,51,54,73,95` (fa48c8f) — `/roleplay` erased from base prompt, skill requires it in all three

- **Severity:** HIGH.
- **Location:** fa48c8f `contract_description` (all three personas) and `contract_character(Abbey)` + `system_prompt` framing.
- **Description:** Skill pins "`/roleplay` named in all three" and the exact handoff phrasing `hand NSFW roleplay to Aviva through /roleplay`, `deep runtime ... to Abi`, `WDBX is substrate`. `fa48c8f` replaces them with `do not promise a roleplay control` and `current surface offers an approved one`. `system_prompt` drops `Discord conversation` → `chat conversation` and `server` → `external systems`. At HEAD the tests were also edited to assert `!system_prompt.contains("/roleplay")` — a self-consistent but skill-breaking change.
- **Why it matters:** `capability_guidance.rs` (added in fa48c8f) **already** does the scope gating correctly: `discord:` scopes get `/roleplay` + `/help` + voice text, other scopes get `does not expose Discord's adult roleplay control` with no `/roleplay` leak. Duplicating that gating inside `ask.rs` strips the Discord base of its actual command name; a Discord-only model loses the concrete door it must point at.
- **AGENTS.md cross-check:** `AGENTS.md:162-166` says "Keep Discord prompt copy in `ask.rs` separate from hosted Grok instructions: carry voice and honesty rules, but never include tip floors, PR/SHA state, or goal-loop/claims-ledger vocabulary. Update the inline `ask.rs` assertions with any intentional copy edit; keep routing in `persona.rs`." Transport-neutral is explicitly called "Discord prompt copy" — fa48c8f mislabels it "shared chat surfaces."
- **Suggestion:** Revert to Discord-specific: `Discord conversation`, `cannot see or change the server`, `point people to /roleplay ... in bot DMs or NSFW channels where an operator has turned it on`, `Abi coordinates across lanes`. Let `capability_guidance.rs` append the neutral fallback for non-Discord scopes.
- **Status:** FAIL at HEAD, PASS on working-dir revert (which restores the three `/roleplay` asserts).

### [MEDIUM] `src/ask.rs:73` — Abbey character line loses pinned phrases

- **Severity:** MEDIUM.
- **Location:** `contract_character(Persona::Abbey)` fa48c8f vs skill pins.
- **Description:** Skill pins: `lead with the answer`, `when I'm not sure`, `local and consent-aware`, `can't verify`, `hand NSFW roleplay to Aviva through /roleplay`, `deep runtime claims to Abi`, `ABBEY_VOICE_MODE=local`, `not OpenAI Realtime`, `music mirroring is not listen consent`, plus the longer CoS phrases `Soft Mat craft never absorbed`, `Dual CoS + companion`, `Chief of Staff`, `Manages his other bots`, `Abi coordinates across lanes`. fa48c8f drops `manage other bots and surface decisions`, `Dual CoS + companion —`, `mirror Donald's latest move ... particular detail`, `Soft Mat craft`. The revert restores them.
- **Suggestion:** Keep the full character line. If brevity is needed for a neutral surface, add a separate neutral character — don't amputate the Discord character.
- **Status:** FAIL at HEAD, PASS on working-dir revert.

### [LOW] `src/ask.rs:12-14` header — stale Discord label on a neutral file

- **Severity:** LOW (doc drift).
- **Location:** `src/ask.rs:1` and `src/ask.rs:12`.
- **Description:** At HEAD the module header still says `Abbey's Discord voice (2026-09-16) follows Donald's Grok Bot Abbey` and `Aviva/Abi remain product chat modes` while the contracts claim transport neutrality. That's internally inconsistent. The revert correctly says `Pure prompt assembly and reply shaping for /persona ask` / `product Discord modes` / `Discord pipeline`. If neutrality were kept, that header would need rewriting and `AGENTS.md` would need a matching boundary edit — neither was done.
- **Status:** OPEN.

### [LOW] `src/ask.rs:107` degraded copy — `ANTHROPIC_API_KEY` mention

- **Severity:** LOW.
- **Location:** `degraded_reply` fa48c8f vs revert.
- **Description:** Skill pins `degraded_reply` verbatim and the honesty contract says the Discord bot is loopback-only (`ABBEY_BOT_LLM_ENDPOINT` → Ollama/mlx-lm). fa48c8f broadens it to `configuring a provider, such as ... or ANTHROPIC_API_KEY`. That's a product-factual drift; the Discord binary's degraded path has never accepted Anthropic as a loopback. Not a secret leak, but a claim change that should be gated by a real provider seam change.
- **Suggestion:** Keep the loopback-only degraded copy unless the binary actually wires an Anthropic fallback for that path.
- **Status:** OPEN.

---

## 2) `src/ask.rs:13` vs skill — intended drift or violation?

- **Finding:** No violation by citing `src/ask.rs:13`. The line **documents intentional drift from `abi-ai` sibling contracts** ("warm, sharp, result-first, local-first, claim-honest — and may intentionally drift from abi-ai until that tree is updated to match."). That's the correct seam: `ask.rs` transcribes rather than depends on `abi`. It does **not** document or authorize drift from the **hosted Grok vs Discord** split. The hosted-vs-Discord split is governed by the skill rule "never include tip floors, PR/SHA state, or goal-loop/claims-ledger vocabulary" and `AGENTS.md:162-166`. Both HEAD and revert obey that ban.
- **Therefore:** `local-first Discord companion` vs `local-first chat companion` is **not** an abi-ai drift question — it's a skill/AGENTS boundary question. The skill's verbatim pins make `Discord companion` required; the revert is correct, the transport-neutral edit is out-of-bound without a skill amendment.
- **Similarly:** `/roleplay named in all three` vs `do not promise a roleplay control` — the latter belongs in `capability_guidance.rs` (where it now correctly lives at `src/generation/capability_guidance.rs:32`). Putting it in `ask.rs` violates the single-responsibility split the skill describes: `ask.rs` is Discord-specific base, capability layer adds transport suffix.

---

## 3) No hosted text copied verbatim

- Checked: `rg "Tip floor|a0ad563|Partial.*Proposed|Current.*goal|empty goal|Cross-lane|Quesar"` over `src/ask.rs` hits only the **banned-term test array** (`src/ask.rs:281,289-290`), which is the enforcement mechanism, not prompt content. `system_prompt` for all three personas contains none of those strings — confirmed by `cargo test ask::tests::discord_prompts_carry_no_goal_loop_or_ledger_vocabulary` (PASS on both revisions). No hosted PR tip (`a0ad563`) or ledger labels leak into prompts.
- `Quesar` is correctly absent from `src/ask.rs` in both versions (banned everywhere per skill + brand). Allowed docs mentions (`docs/`, `tasks/todo.md:330`, `tasks/goals.md:40,1738,1760`, `activity/README.md:15`, `docs/brand.md:13`) are brand-scoped and correctly segregated — e.g. `src/commands_forum.rs:5` explicitly says "never Quesar."

---

## 4) smoke.sh hardening — PASS

`fa48c8f` hardens `.claude/skills/run-abbey-bot/smoke.sh` substantially. No credential or persistence leak found.

- **Literal env parsing, no `source`:** Old version used `set -a; . "$ENV_FILE"` with trace suppression. New ` _read_env` (`smoke.sh:111-166`) trims, validates `KEY=VALUE`, supports `export`, rejects non-identifier keys, duplicate keys, oversized or NUL-containing files, and **exports only** the explicit allowlist (`_env_key_allowed`, `smoke.sh:66-99`). Hosted token only loaded when `load_env token` is used (`do_plan` path), and only DISCORD_* pass through `plan_env`.
- **Filesystem hygiene:** `load_env` (`smoke.sh:168-218`) refuses symlink (`-L`), requires regular readable file (`-f` + `-r`), checks mode exactly `400`/`600` via `stat -c %Lp` → fallback `stat -f %Lp`, checks owner `$(id -u)`, size ≤ 65536 and `tr -d '\000'` NUL check. Error path is uniform `refusing invalid or unsafe environment file` — no secret echo.
- **Allowlist discipline:** `clean_env` (`smoke.sh:223-268`) is `env -i` with an explicit `ABBEY_BOT_LLM_*`, `ABBEY_VISION_*`, `ABBEY_FM_*`, `ABBEY_VOICE_*` allowlist; `DISCORD_*` **not** in that allowlist. `plan_env` (`smoke.sh:270-298`) is the only `env -i` that forwards `DISCORD_TOKEN`/`DISCORD_BOT_TOKEN` and is only called from `do_plan`.
- **Foreground + timeout + direct exit capture:** Every mode runs in foreground with `timeout 600` (provider/voice) or `timeout 300` (plan), and `rc` is captured without `| tail`. `do_args` asserts exit 2 for bad argv, `do_test` fails on `running 0 tests`, `do_provider` validates JSON via `python3`.
- **Residual notes (not a fail):** `ENV_FILE="$HOME/.config/abbey-bot/env"` permission check being in `smoke.sh` rather than a separate gate is intentional per `AGENTS.md` local-operation guidance. `TMP_ROOT`/`OUT` handling (`${TMPDIR:-/tmp}`) is Windows-incompatible but that's out-of-scope for this POSIX driver (`check.ps1` has its own lane).

---

## 5) Retired voice docs

- None retired in `fa48c8f`. Voice code surfaces (`src/voice*.rs`, `src/voice_session/`, etc.) remain intact. Docs change is limited to `docs/README.md` clarifying `AGENTS.md` vs `CLAUDE.md` roles — correct, aligns with AGENTS.md canonical pointer rule. No operator voice acceptance doc was removed.

---

## 6) Other fa48c8f seams not in scope — briefly noted for completeness

- `src/generation/capability_guidance.rs` addition at fa48c8f is **correctly scoped** and tested: discord scopes get `/roleplay`/`/help` text, non-discord scopes get the explicit "does not expose" denial. No secret leak, no banned vocabulary.
- `AGENTS.md` rewrite (515 lines shorter) preserves the `Keep Discord prompt copy in ask.rs separate from hosted ... tip floors` boundary; mirrors in `.cursor/.codex` were regenerated (commit diff shows formatter churn, not semantic drift).
- No credential-bearing `Debug` regressions introduced; `provider API keys travel in headers` invariant untouched.

---

## Action (no edits performed)

1. Adopt the working-dir revert as the base: restore `src/ask.rs` to the Discord-specific contract (`Donald’s Chief of Staff ... local-first Discord companion`, `/roleplay` in all three, `Discord conversation`/`server`, loopback `degraded_reply`, full `Soft Mat`/`companion turns` character). That's what the skill currently requires.
2. Keep `src/generation/capability_guidance.rs` as landed — it is the right place for transport gating.
3. If a shared-surface abstraction is still desired, open a separate proposal: amend `~/.agents/skills/abbey-system-prompt/SKILL.md` and `AGENTS.md:162-166` first, then introduce a new `system_prompt_for_scope` wrapper rather than mutating the Discord base. Update the `ask.rs` assertions in the same commit per the skill's "change copy and assert together" rule.
4. Do **not** merge the current `fa48c8f` `ask.rs` without that amendment — it would silently violate a frozen contract that CI now enforces via the banned-term test and the verbatim pins.

---

*Reviewer tail: `cargo test --locked ask::tests::discord_prompts_carry_no_goal_loop_or_ledger_vocabulary` passes on working-dir revert; both HEAD and revert pass the banned-term ban but only the revert passes the skill's Discord-specific pin set (`local-first Discord companion`, `/roleplay` in all three). `rg -l '^\s*use (serenity|poise)' src` boundary intact per AGENTS.md.*
