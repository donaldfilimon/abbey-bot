# Apple FM primary, auto-deploy, and style addenda: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: use
> superpowers:subagent-driven-development to implement this plan task by
> task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Abbey answers Discord through `fm --model pcc`, falls back to
`fm --model system` and then the MLX endpoint, deploys herself from a green
`main`, and adapts her style within fixed, revertible limits.

**Architecture:**
- Provider route: an ordered `FmRoute` expands into single-mode `FmConfig`s,
  registered ahead of the endpoint when `ABBEY_FM_ROLE=primary`.
- Prompt: a pure `prompt_budget` fits small windows.
- Style addenda: a pure `brain::addenda` ledger turns closed-lexicon feedback
  into template-rendered style lines.
- Deploy: a self-hosted CI job qualifies, installs and verifies after the
  gate.

**Tech stack:** Rust 1.98 (edition 2024, `--locked`), serenity 0.12.5 plus
poise, Python 3 deploy tooling, GitHub Actions on the self-hosted macOS
runner `abbey-bot`.

**Spec:** `docs/superpowers/specs/2026-09-29-fm-primary-addenda-design.md`

## Global constraints

- `AGENTS.md` governs:
  - production Rust modules under 1,000 lines; 800 or more needs a review note;
  - pure policy modules take `now` and seeds from callers;
  - no module-wide `dead_code`/`unused` allows;
  - Discord translation only in `commands*`/`gateway`;
  - `AppState` locks never held across an await.
- Gate: `./check.sh >| "$SCRATCH/gate.log" 2>&1; echo EXIT:$?`, run in the
  foreground only (never `&`, `nohup` or a pipe). Read the verdict from the
  log.
- Focused tests: `.claude/skills/run-abbey-bot/smoke.sh test <filter>` (a
  nonzero count matters).
- New docs must be added to the Pages inventory in
  `scripts/test-check-pages-liquid.py`.
- Persistence changes are additive with `#[serde(default)]`; old
  `abbey-state.json` documents must load.
- Nothing user-authored or model-authored reaches argv or the addenda text.
- Every slice lands dormant: live behaviour changes only at Task 7 (env
  switch).
- Ship each task: gate green, commit, push `main`, and wait for exact-head
  `Gate (macOS)` success (`gh api repos/donaldfilimon/abbey-bot/commits/<sha>/check-runs`).

## Review focus

1. **PCC refuses** (quota, attribution, network): the turn falls back to
   system once and later turns skip PCC while its circuit is open. Test in
   Task 1.
2. **Stale manifest after an OS update or installer rollback**: the service
   starts, serves via system or the endpoint, and status says
   `stale`/`identity_changed`. Test in Task 1.
3. **One user spamming "too long"** never creates an addendum (distinct-user
   threshold). Test in Task 4.
4. **Old `abbey-state.json` without `addenda`** loads, and a round-trip keeps
   the field. Test in Task 4.
5. **Long guild transcripts on fm system**: the latest user turn and the
   persona core survive trimming, and output is deterministic. Test in
   Task 2.

---

### Task 0: Spec, plan and ledger (docs only)

- [ ] Add both docs to the Pages inventory list in
  `scripts/test-check-pages-liquid.py`.
- [ ] Append the goal `## Apple FM primary, auto-deploy, capped style
  addenda` to `tasks/goals.md`, with `status: in_progress` and a dated
  bullet, and a matching `##` checklist (Tasks 1–7) to `tasks/todo.md`.
- [ ] Gate, commit, push, then wait for exact-head CI.

### Task 1: Two FM modes and degrade instead of refusing to start

**Files:** `src/provider.rs`, `src/provider/tests.rs`,
`src/runtime/provider_setup.rs`, `src/runtime.rs`,
`src/provider/runtime.rs`, `src/provider/runtime/tests.rs`,
`src/provider/qualification.rs`, `src/provider/manifest.rs`,
`src/provider_self_test.rs`, `src/startup.rs`,
`src/commands_brain/dashboard.rs`, `src/commands_help/tests.rs`,
`src/runtime/tests.rs`

**Interfaces produced:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmRole { Fallback, Primary }

/// Ordered, deduplicated FM modes; each expands to one single-mode FmConfig.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FmRoute { pub role: Option<FmRole>, pub instances: Vec<FmConfig> }

impl FmRoute {
    pub fn from_values(mode, endpoint, cli, fallback, role, timeout_secs, pcc_timeout_secs: Option<String>)
        -> Result<Option<Self>, String>;
    pub fn from_env() -> Result<Option<Self>, String>; // reads ABBEY_FM_ROLE, ABBEY_FM_PCC_TIMEOUT_SECS too
}
// FmConfig keeps `mode: FmMode` and `fallback: bool`, which now means "admitted for text routing"
// (role Fallback or Primary). It gains `primary: bool`. The endpoint (fm serve) attaches to the
// first instance only. FmConfig::from_values(5 args) keeps compiling and returns the first instance.

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FmQualificationState { Qualified, Missing, Stale, IdentityChanged, Refused(String) }
```

**Behaviour:**
- **Parsing.** `ABBEY_FM_MODE=pcc,system` gives instances [pcc, system];
  duplicates and `off` inside a list are errors. `ABBEY_FM_ROLE=primary`
  with `ABBEY_FM_FALLBACK=1` is accepted; any other role/fallback conflict
  is an error.
- **Runtime.** `ProviderRuntime::legacy(.., fm: Vec<FoundationModels>, ..)`
  registers `foundation-models-cli-pcc` (PCC, `PublicRemote`) and
  `foundation-models-cli` (system, `SameHost`).
  - With role Primary, these register **before** `primary`/`local-fallback`.
  - `self.fm` becomes a `Vec<Arc<FoundationModels>>`, and
    `foundation_models()` returns the first qualified one (for vision and the
    dashboard).
  - PCC is authorized explicitly wherever explicit IDs are validated
    (`apply_configuration`).
- **Qualification.**
  - Remove the PCC refusal in `verify_fm_manifest`.
  - A V2 record ID is `foundation-models` for system and
    `foundation-models-pcc` for PCC (add `FOUNDATION_MODELS_PCC_PROVIDER_ID`
    in `manifest.rs`).
  - For text routing, `provider_setup::load_fm_qualification` returns a
    per-mode `(Option<VerifiedFmCapabilities>, FmQualificationState)`
    instead of `StartupError`. `apply_fm_qualification` in `runtime.rs`
    follows the same rule.
  - FM **vision** (`ABBEY_VISION_PROVIDER=fm`) keeps the strict error.
- **Status and self-test.**
  - `startup.rs` logs each mode's state; the dashboard shows it.
  - `provider_self_test.rs` drops the `pcc_not_qualified` short-circuit and
    probes every instance.

**Tests:**
- [ ] Write the failing tests:
  - `provider::tests`: `fm_route_parses_ordered_list`,
    `fm_route_rejects_duplicates_and_off_in_list`,
    `single_mode_config_still_parses`, `role_primary_and_fallback_alias`,
    `pcc_timeout_override`.
  - `provider::runtime::tests`: `primary_role_registers_pcc_then_system_before_endpoint`
    (assert the order of `inspect_status()` IDs),
    `fallback_role_keeps_endpoint_first`,
    `pcc_failure_falls_back_to_system_once`.
  - `qualification` tests: `pcc_manifest_record_verifies`,
    `stale_manifest_degrades_not_errors`,
    `fm_vision_still_requires_manifest`.
- [ ] Run them to confirm they fail, implement, run them to green, then
  gate, commit (`feat(provider): route FM pcc then system, degrade on
  unqualified manifest`), push and wait for CI.

### Task 2: Abbey prompt via `--instructions`, and the prompt budget

**Files:** create `src/prompt_budget.rs` (with an inline `#[cfg(test)] mod
tests`); modify `src/provider/foundation_models.rs`, `src/engine.rs`,
`src/generation.rs`, `src/provider/runtime/conversation.rs`, `src/main.rs`
(the `mod` line).

**Interfaces produced:**

```rust
pub struct PromptParts { pub core: String, pub guidance: String, pub addenda: String,
                         pub facts: Vec<(f32 /*relevance*/, String)>, pub turns: Vec<Turn> }
pub struct Budget { pub max_chars: usize }   // derived from tokens * CHARS_PER_TOKEN
pub fn fit(parts: &PromptParts, budget: Budget) -> PromptParts; // pure, deterministic
pub const CHARS_PER_TOKEN: usize = 3;       // conservative; calibrated with `fm count-tokens`
// ProviderConversation::prompt_budget(&self) -> Option<Budget>: Some(~4096 tokens minus a 1024 output reserve) for fm system; None otherwise.
```

**Behaviour:**
- **Trim order.** Drop the oldest turns first, always keeping the last user
  turn, then the lowest-relevance facts. Never touch core, guidance or
  addenda. If core + guidance + addenda + the last turn still overflow,
  return as-is: the provider's own error then triggers the honest degraded
  reply.
- **FM argv.** `CliInvocation::new` passes `--instructions` = core + addenda
  (static text only). `system_policy` on stdin carries guidance + facts.
- **Where the budget applies.** In `generate_conversation` (`generation.rs`)
  the budget is applied after every `reserve()`, so a fallback re-fits.

**Tests:**
- [ ] Write the failing tests:
  - `prompt_budget::tests`: `under_budget_is_identity`,
    `drops_oldest_turns_first_keeps_last_user_turn`,
    `drops_low_relevance_facts_after_turns`, `never_trims_core_or_addenda`,
    `fit_is_idempotent`.
  - `provider::tests`: `cli_argv_contains_instructions_but_no_facts_or_transcript`.
  - `generation::tests`: `budget_reapplied_after_fallback_to_small_window`.
- [ ] Implement, get to green, then gate, commit, push and wait for CI.

### Task 3: Style signals and the addenda ledger (pure), plus wiring

**Files:** create `src/brain/style_signal.rs`, `src/brain/addenda.rs` and
`src/brain/addenda/tests.rs`; modify `src/brain.rs` (the `mod` lines),
`src/pipeline.rs` (observe at the outcome classification around L280, only
when learning is on), `src/runtime.rs` (`learn_all` ticks addenda for
enabled guilds; add an accessor for rendered addenda), `src/engine.rs`
(render after core for `discord:guild:` scopes) and `src/persist.rs`
(`Stores.addenda: BTreeMap<String, AddendaLedger>` with `#[serde(default)]`).

**Interfaces produced:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StyleSignal { TooLong, TooShort, TooFormal, TooCasual, NoEmoji, MoreEmoji, PreferCode }
pub fn classify(text: &str) -> Option<StyleSignal>; // closed lexicon, lowercase, whole phrases, no substring of other words

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StyleKnob { Length, Formality, Emoji, Code }
pub struct Policy { pub signals: usize /*5*/, pub distinct_users: usize /*3*/, pub window_secs: u64 /*7d*/,
                    pub ttl_secs: u64 /*14d*/, pub max_addenda: usize /*4*/, pub max_bytes: usize /*400*/ }
#[derive(Default, Serialize, Deserialize)]
pub struct AddendaLedger { /* bounded observations, active addenda, suppressions */ }
impl AddendaLedger {
    pub fn observe(&mut self, user: &str, signal: StyleSignal, now: u64);
    pub fn tick(&mut self, policy: &Policy, now: u64) -> Vec<AddendumChange>; // applied/expired
    pub fn revert(&mut self, knob: StyleKnob, policy: &Policy, now: u64) -> bool;
    pub fn clear(&mut self, now: u64);
    pub fn active(&self) -> Vec<Addendum>;
    pub fn render(&self) -> String; // fixed template per (knob, direction), joined, <= max_bytes
}
```

**Behaviour:**
- **Observations.** Opposing signals on one knob cancel. Observations are
  bounded (at most 64 per guild) and pruned outside the window.
- **Users.** User IDs are stored as keyed hashes (reuse the existing wyhash
  helper) so the ledger holds no raw IDs.

**Tests:**
- [ ] Write the failing tests:
  - `brain::addenda::tests`: `five_signals_three_users_applies`,
    `single_user_spam_never_applies`, `expires_after_ttl`,
    `revert_suppresses_reapplication`, `caps_count_and_bytes`,
    `render_is_template_only` (no observed text in the output),
    `opposing_signals_cancel`.
  - `brain::style_signal` tests for positive and negative lexicon cases.
  - `persist` tests: `pre_addenda_document_loads`, `addenda_round_trip`.
  - `pipeline` test: `style_signal_ignored_when_learning_disabled`.
- [ ] Implement, get to green, then gate, commit, push and wait for CI.

### Task 4: `/admin addenda list|revert|clear`

**Files:** create `src/commands_brain/addenda.rs`; modify
`src/commands_brain.rs` (the subcommand group under `admin`),
`src/command_catalog/data.rs` (three rows, following the three-level
"work recall show" pattern), `src/command_catalog/tests.rs` (freeze 87 →
90), `README.md` (regenerate the catalog region from `render_readme()`),
`src/generation/capability_guidance.rs:29` and its pinned tests.

**Behaviour:**
- Same authorization as `/admin learning`.
- Defer first; replies are clamped and ephemeral.
- `list` shows each knob, its direction and the time left.
- `revert <knob>` and `clear` record suppression.
- The capability guidance says: "In servers that enabled learning, Abbey may
  adjust tone and length within fixed limits; admins can review or revert
  with /admin addenda. Abbey does not rewrite her own code."

**Tests:**
- [ ] Catalog count, README render, the 1:1 poise tree
  (`commands_help/tests.rs`), and `commands_brain` tests for
  authorization and reply rendering.
- [ ] Print and read the three rendered replies before committing.
- [ ] Gate, commit, push and wait for CI.

### Task 5: Operator tooling

**Files:** create `deploy/configure-fm-primary.py` and
`deploy/test-configure-fm-primary.py`; modify
`deploy/publish-provider-qualification.py` (`--target fm` emits the system
and PCC records), `deploy/test-publish-provider-qualification.py`,
`check.sh` and `check.ps1` (run the new test by name), `README.md`
(*Deploying*), `.env.example` and the `AGENTS.md` deploy bullet.

**Behaviour:**
- Modelled on `deploy/configure-mlx-primary.py`: dry run by default; the
  `--apply` path takes the same lock, backs up, writes owner-only, and
  restarts through the existing service transaction with rollback.
- Manages only `ABBEY_FM_MODE=pcc,system`, `ABBEY_FM_ROLE=primary`,
  `ABBEY_FM_CLI` and `ABBEY_FM_CAPABILITY_MANIFEST`. It preserves every
  other key and never prints values.
- Refuses when the manifest lacks a qualified record for the first mode.

**Tests:** dry run leaves the file untouched; apply preserves unrelated keys
and mode 0600; the refusals.
- [ ] Gate, commit, push and wait for CI.

### Task 6: CI auto-deploy

**Files:** `.github/workflows/rust.yml` (new job `deploy-macos`),
`scripts/check-rust-release.py` and its test if they pin the job set,
`docs/ops/self-hosted-runner.md`.

**Behaviour:**
- Triggers: `needs: gate-macos`;
  `if: github.event_name == 'push' && github.ref == 'refs/heads/main' || github.event_name == 'workflow_dispatch'`.
- Concurrency: group `abbey-deploy`, `cancel-in-progress: false`;
  `timeout-minutes: 45`.
- Steps:
  1. Stable `CARGO_TARGET_DIR=$HOME/.cache/abbey-bot-deploy-target`.
  2. `cargo build --release --locked`.
  3. Back up the manifest.
  4. `publish-provider-qualification.py --binary <release> --target fm`.
  5. `./deploy/install-launchd.sh` in the foreground.
  6. Assert the installed SHA equals the release SHA.
  7. `python3 -I deploy/service-status.py`.
- The job has no secrets; `actionlint` must be clean.
- The first run exercises the unchanged env as a rehearsal.

- [ ] Gate, commit, push. Then wait for **both** `Gate (macOS)` and
  `deploy-macos` success on the exact head, and read the job log for the SHA
  match and `ready`.

### Task 7: Switch to FM primary (live)

- [ ] `python3 deploy/configure-fm-primary.py` (dry run), then review, then
  `--apply`.
- [ ] Verify:
  - `service-status.py` exits 0;
  - `abbey-bot --provider-self-test fm --json` against the installed binary;
  - the per-mode state in the dashboard/log.
- [ ] Record the evidence in `docs/MLAI-LIVE-ACCEPTANCE.md` and the ledger.
- [ ] **Stop:** Donald's live Discord acceptance (a PCC reply, fallback, and
  an addendum applied and then reverted).
