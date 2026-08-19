# Guild Learning Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a guild opt in (`/admin act on`) to Abbey acting unsolicited through the per-guild DQN, bounded by a per-guild hourly budget and the per-channel cooldown, with the policy's decisions and rewards legible in `/admin brain` and the log.

**Architecture:** Two new pure modules (`brain/budget.rs` token bucket, `brain/telemetry.rs` per-guild stats) plug into the existing pure-core/thin-shell shape: `guild.rs` gains two settings, `brain/registry.rs` carries stats beside each loaded brain, `pipeline.rs` reorders its gates (QUIET → act → learning → blank → policy → cooldown → budget → act) and records decisions, `commands_brain.rs` exposes `/admin act|budget` and a richer `/admin brain`. Nothing imports serenity outside the existing shell files; every pure function takes `now: u64`.

**Tech Stack:** Rust nightly (edition 2024), serenity 0.12 / poise 0.6, serde, tokio. Gate: `./check.sh` (fmt, clippy `--all-targets --locked -D warnings`, tests `--locked`).

**Spec:** `docs/superpowers/specs/2026-08-19-guild-learning-loop-design.md`

## Global Constraints

- Pure modules (`brain/*`, `guild.rs`, `memory.rs`, `engine.rs`, `wdbx.rs`, `pipeline.rs` logic) import neither serenity nor poise; `commands_brain.rs`, `gateway.rs`, `main.rs` are the Discord surface.
- Every pure function that needs time takes `now: u64` (unix seconds); `runtime::now()` is the only clock read.
- Clippy runs with `-D warnings` and this is a binary crate: a `pub` item only tests use is a dead-code error — either wire it or `#[cfg(test)]` it; never `#[allow]`.
- Every Discord-bound string goes through `clamp_message` (commands) or the outbound clamp (gateway).
- Mentions, DMs, and slash commands are never budgeted, never cooled down, and never counted as policy decisions.
- `ABBEY_QUIET=1` remains the operator override and wins over any guild setting.
- Defaults from the spec: `unsolicited = false`, `unsolicited_per_hour = 6`, budget clamp `1..=60`, cooldown unchanged (20 s per channel).
- `AGENTS.md` is a verbatim mirror of `CLAUDE.md` except the two header lines — apply every doc edit to both.
- Commit after every task; run `./check.sh` (not `cargo test | tail` — the pipe hides the exit code) before each commit.

## Roadmap (not part of this plan's tasks)

"Finish and complete the app" decomposes into four sub-projects agreed 2026-08-19. **This plan implements #3 only** — the only one with an approved spec. The others each need their own brainstorm → spec → plan before code:

1. Reply quality & speed (local-first; model selection by measurement, length contract, generation queue, progressive edit-in-place, Anthropic-when-present routing) — spec not written.
2. Smarter agent (tools: `remember_fact`, `lookup_reputation`, `recall`, `switch_persona`, `summarize_channel`; OpenAI function-calling + Anthropic `tool_use` shapes; bounded loop; honest degrade) — spec not written.
3. **Guild learning loop — this plan.**
4. Breadth & ops (Telegram/Slack live, vision on a real VLM, launchd service with `ABBEY_DATA_DIR`, CI executing) — spec not written.

Proposed (from `tasks/goals.md`) and out of scope everywhere: Swift companion app, Apple on-device models, voice, Postgres.

## File Structure

- Create `src/brain/budget.rs` — `Budget` token bucket keyed by scoped guild id. One responsibility: "may this guild spend one unsolicited action right now?"
- Create `src/brain/telemetry.rs` — `BrainStats` per guild: last state/Q/action, action histogram, forced count, recent rewards, render.
- Modify `src/brain/mod.rs` — declare the two modules.
- Modify `src/guild.rs` — `GuildSettings.unsolicited`, `GuildSettings.unsolicited_per_hour`, `DEFAULT_BUDGET_PER_HOUR`, `MAX_BUDGET_PER_HOUR`, `clamp_budget`, `render_settings` adds `act` and `budget`.
- Modify `src/brain/registry.rs` — `Loaded<B>` carries `stats: BrainStats`; `stats(&self, g)`, `stats_mut(&mut self, g)`.
- Modify `src/runtime.rs` — `AppState.budget: Mutex<Budget>`; `settle_rewards` records each settled reward into the guild's stats.
- Modify `src/pipeline.rs` — gate order, `Outcome::OverBudget`, decision/forced telemetry, `policy decision` log line.
- Modify `src/commands_brain.rs` — `/admin act`, `/admin budget`, `/admin brain` renders telemetry, `/stats` shows tokens left.
- Modify `README.md`, `CLAUDE.md`, `AGENTS.md`, `tasks/goals.md`, `tasks/todo.md`, `docs/live-test-protocol.md`.

---

### Task 1: `brain/budget.rs` — per-guild refilling token bucket

**Files:**
- Create: `src/brain/budget.rs`
- Modify: `src/brain/mod.rs` (add `pub mod budget;` — keep the list alphabetical)

**Interfaces:**
- Produces: `pub struct Budget` (Default), `pub fn try_take(&mut self, key: &str, capacity_per_hour: u32, now: u64) -> bool`, `pub fn tokens_left(&self, key: &str, capacity_per_hour: u32, now: u64) -> f32`.
- Semantics: a fresh key starts full (`capacity` tokens). Tokens refill continuously at `capacity / 3600` per second, never above `capacity`. `try_take` spends 1.0 if ≥ 1.0 is available. A capacity of 0 never permits.

- [ ] **Step 1: Write the failing tests**

```rust
// src/brain/budget.rs
//! Per-guild budget for unsolicited actions — a refilling token bucket.
//!
//! The cooldown in `guild.rs` stops bursts (one reply per channel per N
//! seconds); this stops volume (at most `capacity` unsolicited actions per
//! guild per hour, refilling continuously). Pure: the clock is injected.

use std::collections::HashMap;

/// One bucket per scoped guild id.
#[derive(Debug, Default)]
pub struct Budget {
    buckets: HashMap<String, Bucket>,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f32,
    last: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_guild_starts_full_and_spends_down_to_empty() {
        let mut b = Budget::default();
        for _ in 0..6 {
            assert!(b.try_take("discord:g", 6, 1_000));
        }
        assert!(!b.try_take("discord:g", 6, 1_000), "seventh in the same second is refused");
        assert!(b.tokens_left("discord:g", 6, 1_000) < 1.0);
    }

    #[test]
    fn tokens_refill_at_capacity_per_hour_and_cap_at_capacity() {
        let mut b = Budget::default();
        for _ in 0..6 {
            assert!(b.try_take("discord:g", 6, 0));
        }
        // 6/h = one token per 600 s.
        assert!(!b.try_take("discord:g", 6, 599));
        assert!(b.try_take("discord:g", 6, 600));
        // A long idle period never overfills.
        let left = b.tokens_left("discord:g", 6, 1_000_000);
        assert!((left - 6.0).abs() < 1e-3, "{left}");
    }

    #[test]
    fn guilds_do_not_share_a_bucket() {
        let mut b = Budget::default();
        for _ in 0..6 {
            assert!(b.try_take("discord:a", 6, 0));
        }
        assert!(b.try_take("discord:b", 6, 0), "guild b is untouched");
    }

    #[test]
    fn zero_capacity_never_permits_and_time_going_backwards_is_harmless() {
        let mut b = Budget::default();
        assert!(!b.try_take("discord:g", 0, 10));
        assert!(b.try_take("discord:g", 6, 100));
        assert!(b.try_take("discord:g", 6, 50), "a clock step back does not panic or refund");
    }
}
```

- [ ] **Step 2: Run to verify they fail to compile**

Run: `cargo test brain::budget 2>&1 | tail -5`
Expected: `error[E0599]: no method named try_take` (the struct exists, the methods do not).

- [ ] **Step 3: Implement**

Insert between the `Bucket` struct and `#[cfg(test)]`:

```rust
impl Budget {
    /// Spend one unsolicited action for `key` if the bucket has a whole token.
    pub fn try_take(&mut self, key: &str, capacity_per_hour: u32, now: u64) -> bool {
        if capacity_per_hour == 0 {
            return false;
        }
        let bucket = self.refilled(key, capacity_per_hour, now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Tokens available right now, for `/admin brain` and `/stats`.
    pub fn tokens_left(&self, key: &str, capacity_per_hour: u32, now: u64) -> f32 {
        let capacity = capacity_per_hour as f32;
        match self.buckets.get(key) {
            None => capacity,
            Some(b) => refill(*b, capacity, now).tokens,
        }
    }

    fn refilled(&mut self, key: &str, capacity_per_hour: u32, now: u64) -> &mut Bucket {
        let capacity = capacity_per_hour as f32;
        let entry = self.buckets.entry(key.to_owned()).or_insert(Bucket {
            tokens: capacity,
            last: now,
        });
        *entry = refill(*entry, capacity, now);
        entry
    }
}

/// Advance a bucket to `now`: add `capacity/3600` per elapsed second, cap at
/// `capacity`. A clock that went backwards adds nothing.
fn refill(mut b: Bucket, capacity: f32, now: u64) -> Bucket {
    let elapsed = now.saturating_sub(b.last) as f32;
    b.tokens = (b.tokens + elapsed * capacity / 3600.0).min(capacity);
    b.last = now.max(b.last);
    b
}
```

Add `pub mod budget;` to `src/brain/mod.rs` (alphabetical: before `dqn`).

- [ ] **Step 4: Run tests**

Run: `cargo test brain::budget 2>&1 | grep "test result"`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Clippy (dead code is expected until Task 5 wires it)**

Run: `cargo clippy --all-targets -- -D warnings -A dead_code 2>&1 | grep -c "^error"`
Expected: `0`. (`as f32` casts on a u32/u64 are not denied by this crate's clippy config; if `cast_precision_loss` ever fires, use `#[expect(clippy::cast_precision_loss, reason = "token counts are tiny")]`, never `#[allow]`.)

- [ ] **Step 6: Commit**

```bash
git add src/brain/budget.rs src/brain/mod.rs
git commit -m "feat(brain): per-guild refilling token bucket for unsolicited actions"
```

---

### Task 2: `brain/telemetry.rs` — per-guild `BrainStats`

**Files:**
- Create: `src/brain/telemetry.rs`
- Modify: `src/brain/mod.rs` (add `pub mod telemetry;`)

**Interfaces:**
- Consumes: `crate::brain::state::BotAction` (`ALL`, `index()`, `from_index`).
- Produces: `pub struct BrainStats` (Default, Debug, Clone, PartialEq) with `pub fn record_decision(&mut self, state: &[f32], q: &[f32], action: BotAction)`, `pub fn record_forced(&mut self)`, `pub fn record_reward(&mut self, reward: f32)`, `pub fn mean_recent_reward(&self) -> Option<f32>`, `pub fn render(&self, view: &BrainView<'_>) -> String`, and `pub struct BrainView<'a> { pub scoped_guild_id: &'a str, pub epsilon: f32, pub learn_steps: u64, pub buffer_len: usize, pub buffer_capacity: usize, pub experiences: u64, pub budget_per_hour: u32, pub tokens_left: f32, pub topology: &'a [usize] }`.
- `RECENT_REWARDS: usize = 20`.

- [ ] **Step 1: Write the failing tests**

```rust
// src/brain/telemetry.rs
//! What the per-guild policy has been doing — the observability the
//! adaptive loop needs to be debuggable rather than a black box
//! (`docs/spec/companionapp.md`: "/admin brain exposes DQN state").
//!
//! In-memory only: these describe the running process. Pure; rendering is a
//! string the command layer clamps.

use std::collections::VecDeque;

use crate::brain::state::BotAction;

/// How many settled rewards the rolling mean covers.
pub const RECENT_REWARDS: usize = 20;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrainStats {
    pub last_state: Vec<f32>,
    pub last_q: Vec<f32>,
    pub last_action: Option<BotAction>,
    /// Indexed by `BotAction::index()`: stay / reply / react.
    pub action_counts: [u64; 3],
    /// Mentions and DMs answered — never policy decisions.
    pub forced_replies: u64,
    pub recent_rewards: VecDeque<f32>,
    pub settled_total: u64,
}

/// The numbers `/admin brain` shows that live outside `BrainStats`.
pub struct BrainView<'a> {
    pub scoped_guild_id: &'a str,
    pub epsilon: f32,
    pub learn_steps: u64,
    pub buffer_len: usize,
    pub buffer_capacity: usize,
    pub experiences: u64,
    pub budget_per_hour: u32,
    pub tokens_left: f32,
    pub topology: &'a [usize],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(stats_guild: &str) -> BrainView<'_> {
        BrainView {
            scoped_guild_id: stats_guild,
            epsilon: 0.1,
            learn_steps: 3,
            buffer_len: 2,
            buffer_capacity: 10_000,
            experiences: 2,
            budget_per_hour: 6,
            tokens_left: 5.0,
            topology: &[18, 64, 32, 3],
        }
    }

    #[test]
    fn decisions_update_last_and_histogram() {
        let mut s = BrainStats::default();
        s.record_decision(&[0.0; 18], &[0.1, 0.7, 0.2], BotAction::Reply);
        s.record_decision(&[1.0; 18], &[0.9, 0.0, 0.0], BotAction::Stay);
        s.record_forced();
        assert_eq!(s.action_counts, [1, 1, 0]);
        assert_eq!(s.forced_replies, 1);
        assert_eq!(s.last_action, Some(BotAction::Stay));
        assert_eq!(s.last_q, vec![0.9, 0.0, 0.0]);
        assert_eq!(s.last_state.len(), 18);
    }

    #[test]
    fn rewards_keep_a_bounded_window_and_a_mean() {
        let mut s = BrainStats::default();
        assert_eq!(s.mean_recent_reward(), None);
        for i in 0..(RECENT_REWARDS + 5) {
            s.record_reward(i as f32);
        }
        assert_eq!(s.recent_rewards.len(), RECENT_REWARDS);
        assert_eq!(s.settled_total, (RECENT_REWARDS + 5) as u64);
        // The window holds 5..=24 → mean 14.5.
        assert!((s.mean_recent_reward().unwrap() - 14.5).abs() < 1e-6);
    }

    #[test]
    fn render_reads_well_and_names_the_winning_action() {
        let mut s = BrainStats::default();
        s.record_decision(&[0.0; 18], &[0.1, 0.7, 0.2], BotAction::Reply);
        s.record_reward(0.8);
        let text = s.render(&view("discord:g"));
        assert_eq!(
            text,
            "**brain — discord:g**\n\
             ε 0.100 · learn steps 3 · replay buffer 2/10000 · experiences 2 · topology [18, 64, 32, 3]\n\
             last decision: reply · q stay 0.10 / reply 0.70 / react 0.20\n\
             decisions: stay 0 · reply 1 · react 0 · forced (mentions/DMs) 0\n\
             rewards settled: 1 · mean of last 1: 0.80\n\
             budget: 5.0 of 6/h left"
        );
    }

    #[test]
    fn render_before_any_decision_says_so() {
        let s = BrainStats::default();
        let text = s.render(&view("discord:g"));
        assert!(text.contains("last decision: none yet"), "{text}");
        assert!(text.contains("rewards settled: 0 · mean of last 0: n/a"), "{text}");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test brain::telemetry 2>&1 | tail -5`
Expected: `error[E0599]` for `record_decision` / `render`.

- [ ] **Step 3: Implement**

Insert between `BrainView` and `#[cfg(test)]`:

```rust
impl BrainStats {
    /// The policy chose `action` for `state` with Q-values `q`.
    pub fn record_decision(&mut self, state: &[f32], q: &[f32], action: BotAction) {
        self.last_state = state.to_vec();
        self.last_q = q.to_vec();
        self.last_action = Some(action);
        self.action_counts[action.index()] += 1;
    }

    /// A mention or DM was answered — not a policy decision.
    pub fn record_forced(&mut self) {
        self.forced_replies += 1;
    }

    /// A reward settled into this guild's replay buffer.
    pub fn record_reward(&mut self, reward: f32) {
        self.recent_rewards.push_back(reward);
        while self.recent_rewards.len() > RECENT_REWARDS {
            self.recent_rewards.pop_front();
        }
        self.settled_total += 1;
    }

    /// Mean of the rolling window, `None` before the first settlement.
    pub fn mean_recent_reward(&self) -> Option<f32> {
        if self.recent_rewards.is_empty() {
            return None;
        }
        Some(self.recent_rewards.iter().sum::<f32>() / self.recent_rewards.len() as f32)
    }

    /// The `/admin brain` body. Read it before shipping a change — a passing
    /// snapshot test is not evidence that output reads well.
    pub fn render(&self, view: &BrainView<'_>) -> String {
        let last = match self.last_action {
            None => "last decision: none yet".to_string(),
            Some(action) => {
                let q = |a: BotAction| self.last_q.get(a.index()).copied().unwrap_or(0.0);
                format!(
                    "last decision: {} · q stay {:.2} / reply {:.2} / react {:.2}",
                    action_name(action),
                    q(BotAction::Stay),
                    q(BotAction::Reply),
                    q(BotAction::React)
                )
            }
        };
        let mean = self
            .mean_recent_reward()
            .map_or_else(|| "n/a".to_string(), |m| format!("{m:.2}"));
        format!(
            "**brain — {}**\n\
             ε {:.3} · learn steps {} · replay buffer {}/{} · experiences {} · topology {:?}\n\
             {last}\n\
             decisions: stay {} · reply {} · react {} · forced (mentions/DMs) {}\n\
             rewards settled: {} · mean of last {}: {mean}\n\
             budget: {:.1} of {}/h left",
            view.scoped_guild_id,
            view.epsilon,
            view.learn_steps,
            view.buffer_len,
            view.buffer_capacity,
            view.experiences,
            view.topology,
            self.action_counts[BotAction::Stay.index()],
            self.action_counts[BotAction::Reply.index()],
            self.action_counts[BotAction::React.index()],
            self.forced_replies,
            self.settled_total,
            self.recent_rewards.len(),
            view.tokens_left,
            view.budget_per_hour,
        )
    }
}

/// Lower-case names for copy; `Debug` would give `Reply`.
pub const fn action_name(action: BotAction) -> &'static str {
    match action {
        BotAction::Stay => "stay",
        BotAction::Reply => "reply",
        BotAction::React => "react",
    }
}
```

Add `pub mod telemetry;` to `src/brain/mod.rs` (alphabetical: after `state`).

- [ ] **Step 4: Run tests**

Run: `cargo test brain::telemetry 2>&1 | grep "test result"`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Commit**

```bash
git add src/brain/telemetry.rs src/brain/mod.rs
git commit -m "feat(brain): per-guild BrainStats telemetry with /admin brain renderer"
```

---

### Task 3: `guild.rs` — `unsolicited`, `unsolicited_per_hour`, `clamp_budget`, render

**Files:**
- Modify: `src/guild.rs` (`GuildSettings` struct ~line 61, `Default` ~line 77, constants near `MAX_COOLDOWN_SECONDS`, `clamp_cooldown` ~line 216, `render_settings` ~line 225, tests at the bottom)

**Interfaces:**
- Produces: `GuildSettings.unsolicited: bool` (default `false`), `GuildSettings.unsolicited_per_hour: u32` (default `DEFAULT_BUDGET_PER_HOUR`), `pub const DEFAULT_BUDGET_PER_HOUR: u32 = 6`, `pub const MAX_BUDGET_PER_HOUR: u32 = 60`, `pub fn clamp_budget(per_hour: i64) -> u32` (clamps to `1..=60`), `render_settings` now ends `· cooldown: 20s · act: off · budget: 6/h`.

- [ ] **Step 1: Write the failing tests** (append inside the existing `mod tests`)

```rust
    #[test]
    fn new_settings_default_to_not_acting_with_six_per_hour() {
        let s = GuildSettings::default();
        assert!(!s.unsolicited, "opt-in, never opt-out");
        assert_eq!(s.unsolicited_per_hour, DEFAULT_BUDGET_PER_HOUR);
    }

    #[test]
    fn an_old_document_without_the_new_fields_still_loads() {
        let old = r#"{"enabled":true,"default_persona":"abbey","learning_enabled":true,"voice_enabled":true,"vision_enabled":true,"reply_cooldown_seconds":20,"epsilon_override":null,"locale":"en"}"#;
        let s: GuildSettings = serde_json::from_str(old).expect("older row loads");
        assert!(!s.unsolicited);
        assert_eq!(s.unsolicited_per_hour, 6);
    }

    #[test]
    fn budget_clamps_to_one_through_sixty() {
        assert_eq!(clamp_budget(0), 1);
        assert_eq!(clamp_budget(-5), 1);
        assert_eq!(clamp_budget(6), 6);
        assert_eq!(clamp_budget(999), MAX_BUDGET_PER_HOUR);
    }
```

And update the existing snapshot test `render_settings_matches_admin_show` so its expected string ends with `· cooldown: 20s · act: off · budget: 6/h` (read the current assertion and append the two new fields to it).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test guild:: 2>&1 | grep -E "error|test result" | head -3`
Expected: compile error — `no field unsolicited`.

- [ ] **Step 3: Implement**

In `GuildSettings`, after `epsilon_override`:

```rust
    /// May Abbey speak unsolicited here (reply/react chosen by the policy)?
    /// Opt-in per guild via `/admin act on`; `ABBEY_QUIET=1` overrides.
    #[serde(default)]
    pub unsolicited: bool,
    /// Hourly budget of unsolicited actions for the whole guild.
    #[serde(default = "default_budget_per_hour")]
    pub unsolicited_per_hour: u32,
```

Near the other constants:

```rust
/// Unsolicited actions per guild per hour when nothing else is configured.
pub const DEFAULT_BUDGET_PER_HOUR: u32 = 6;
/// Ceiling for `/admin budget`.
pub const MAX_BUDGET_PER_HOUR: u32 = 60;

fn default_budget_per_hour() -> u32 {
    DEFAULT_BUDGET_PER_HOUR
}
```

In `Default for GuildSettings`, after `epsilon_override: None,`: `unsolicited: false, unsolicited_per_hour: DEFAULT_BUDGET_PER_HOUR,`.

After `clamp_cooldown`:

```rust
/// `/admin budget` input → `1..=MAX_BUDGET_PER_HOUR`. Zero is not a valid
/// budget: "never" is `/admin act off`, which also stops learning from the
/// guild, and the distinction matters.
pub fn clamp_budget(per_hour: i64) -> u32 {
    per_hour.clamp(1, i64::from(MAX_BUDGET_PER_HOUR)) as u32
}
```

`render_settings` becomes:

```rust
pub fn render_settings(scoped_guild_id: &str, settings: &GuildSettings) -> String {
    format!(
        "**Abbey — {scoped_guild_id}**\npersona: {} · learning: {} · voice: {} · vision: {} · cooldown: {}s · act: {} · budget: {}/h",
        persona_name(settings.default_persona),
        on_off(settings.learning_enabled),
        on_off(settings.voice_enabled),
        on_off(settings.vision_enabled),
        settings.reply_cooldown_seconds,
        on_off(settings.unsolicited),
        settings.unsolicited_per_hour,
    )
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test guild:: 2>&1 | grep "test result"`
Expected: all guild tests pass (12 existing + 3 new = 15).

- [ ] **Step 5: Commit**

```bash
git add src/guild.rs
git commit -m "feat(guild): per-guild act switch and hourly unsolicited budget"
```

---

### Task 4: `brain/registry.rs` — stats travel with each loaded brain

**Files:**
- Modify: `src/brain/registry.rs` (`Loaded<B>` ~line 66, `brain()` ~line 96, add two accessors near `get` ~line 170, tests)

**Interfaces:**
- Consumes: `crate::brain::telemetry::BrainStats`.
- Produces: `pub fn stats(&self, scoped_guild_id: &str) -> Option<&BrainStats>`, `pub fn stats_mut(&mut self, scoped_guild_id: &str) -> Option<&mut BrainStats>` (both `None` when the brain is not loaded; stats are evicted with the brain).

- [ ] **Step 1: Write the failing test** (append inside `mod tests`; the module already has a test `Brain` impl — reuse its constructor and the `InMemoryBrainStore`)

```rust
    #[test]
    fn stats_appear_with_the_brain_and_leave_with_it() {
        let mut reg = BrainRegistry::new(Counter::default, 3600);
        let mut store = InMemoryBrainStore::new();
        assert!(reg.stats("discord:g").is_none(), "unloaded → no stats");
        reg.brain("discord:g", &store, 0);
        reg.stats_mut("discord:g").unwrap().record_forced();
        assert_eq!(reg.stats("discord:g").unwrap().forced_replies, 1);
        reg.persist_and_evict("discord:g", &mut store);
        assert!(reg.stats("discord:g").is_none(), "evicted with the brain");
    }
```

(`Counter` is the test module's existing `Brain` impl; `InMemoryBrainStore::new()` is its store double.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test brain::registry 2>&1 | grep -E "error|test result" | head -3`
Expected: `no method named stats`.

- [ ] **Step 3: Implement**

```rust
use crate::brain::telemetry::BrainStats;

struct Loaded<B> {
    brain: B,
    experience_count: u64,
    last_touched: u64,
    stats: BrainStats,
}
```

In `brain()`'s `or_insert_with`, add `stats: BrainStats::default(),` to the `Loaded { .. }` literal. Add next to `get`:

```rust
    /// The guild's telemetry, if its brain is loaded.
    pub fn stats(&self, scoped_guild_id: &str) -> Option<&BrainStats> {
        self.brains.get(scoped_guild_id).map(|l| &l.stats)
    }

    /// Mutable telemetry, if loaded. Does not touch the idle clock.
    pub fn stats_mut(&mut self, scoped_guild_id: &str) -> Option<&mut BrainStats> {
        self.brains.get_mut(scoped_guild_id).map(|l| &mut l.stats)
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test brain::registry 2>&1 | grep "test result"`
Expected: 9 passed (8 existing + 1).

- [ ] **Step 5: Commit**

```bash
git add src/brain/registry.rs
git commit -m "feat(brain): BrainRegistry carries per-guild BrainStats"
```

---

### Task 5: `runtime.rs` — `budget` on `AppState`, rewards recorded into stats

**Files:**
- Modify: `src/runtime.rs` (`AppState` struct ~line 130, `from_env` ~line 190, `in_memory` ~line 210, `settle_rewards` ~line 252, tests)

**Interfaces:**
- Produces: `pub budget: Mutex<Budget>` on `AppState`; `settle_rewards` calls `stats_mut(guild).record_reward(exp.reward)` for each settled experience.

- [ ] **Step 1: Write the failing test** (append inside `mod tests`)

```rust
    #[test]
    fn settled_rewards_reach_the_guild_stats() {
        let state = AppState::in_memory();
        {
            let mut brains = AppState::lock(&state.brains);
            let stores = AppState::lock(&state.stores);
            brains.brain("discord:g", &*stores, 0);
        }
        AppState::lock(&state.rewards).register_reply(vec![0.0; 18], 1, "m1", "discord:g", 0);
        AppState::lock(&state.rewards).reaction("👍", "m1", true);
        // settle_rewards reads the real clock; the entry is 150 s+ old by any clock.
        state.settle_rewards();
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").expect("loaded");
        assert_eq!(stats.settled_total, 1);
        assert!((stats.mean_recent_reward().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(brains.get("discord:g").unwrap().buffer_len(), 1);
        drop(brains);
        assert!(AppState::lock(&state.budget).try_take("discord:g", 6, 0));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test runtime:: 2>&1 | grep -E "error|test result" | head -3`
Expected: `no field budget`.

- [ ] **Step 3: Implement**

Add `use crate::brain::budget::Budget;`. In `AppState`, after `pub cooldown: Mutex<ReplyCooldown>,`:

```rust
    /// Per-guild hourly budget for unsolicited actions.
    pub budget: Mutex<Budget>,
```

In both constructors (`from_env` and `in_memory`), after `cooldown: Mutex::new(ReplyCooldown::new()),`: `budget: Mutex::new(Budget::default()),`.

Replace the body of the `for (guild, exp) in settled` loop in `settle_rewards`:

```rust
        for (guild, exp) in settled {
            let loaded = brains.get(&guild).is_some();
            tracing::info!(
                guild = %guild,
                reward = exp.reward,
                action = exp.action,
                loaded,
                "reward settled into the replay buffer"
            );
            if let Some(stats) = brains.stats_mut(&guild) {
                stats.record_reward(exp.reward);
            }
            brains.remember(&guild, exp);
        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test runtime:: 2>&1 | grep "test result"`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/runtime.rs
git commit -m "feat(runtime): budget on AppState; settled rewards feed BrainStats"
```

---

### Task 6: `pipeline.rs` — gate order, decision telemetry, `OverBudget`

**Files:**
- Modify: `src/pipeline.rs` (gates ~lines 141–150, policy block ~lines 169–186, cooldown block ~lines 195–205, `Outcome` enum ~line 55, tests)

**Interfaces:**
- Consumes: `GuildSettings.unsolicited`, `.unsolicited_per_hour`; `AppState.budget`; `BrainRegistry::stats_mut`; `DqnAgent::q_values`; `telemetry::action_name`.
- Produces: `Outcome::OverBudget`; `Outcome::Ignored("act off")`; info log `policy decision` with fields `guild`, `action`, `q`, `intent`, `heat`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`)

```rust
    #[tokio::test]
    async fn a_guild_that_has_not_opted_in_is_ignored_before_the_policy() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let outcome = handle(&state, &out, message("lol nice", Some("g"), "u1"), false, None).await;
        assert_eq!(outcome, Outcome::Ignored("act off"));
        assert!(AppState::lock(&state.brains).loaded_guilds().is_empty(), "no brain, no experience");
    }

    fn opt_in(state: &AppState, guild: &str, per_hour: u32) {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update(guild, &mut *stores, |s| {
            s.unsolicited = true;
            s.unsolicited_per_hour = per_hour;
            s.reply_cooldown_seconds = 0;
        });
    }

    #[tokio::test]
    async fn an_opted_in_guild_consults_the_policy_and_records_the_decision() {
        let state = AppState::in_memory();
        opt_in(&state, "discord:g", 6);
        let out = FakeOut::default();
        let outcome = handle(&state, &out, message("lol nice", Some("g"), "u1"), false, None).await;
        assert!(
            matches!(outcome, Outcome::Stayed | Outcome::Reacted | Outcome::OverBudget),
            "{outcome:?} (no backend → reply degrades to Stayed)"
        );
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").expect("brain loaded by the decision");
        assert_eq!(stats.action_counts.iter().sum::<u64>(), 1, "exactly one policy decision");
        assert_eq!(stats.last_q.len(), 3);
        assert_eq!(stats.forced_replies, 0);
    }

    #[tokio::test]
    async fn a_mention_counts_as_forced_not_as_a_decision() {
        let state = AppState::in_memory();
        let out = FakeOut::default();
        let _ = handle(&state, &out, message("abbey?", Some("g"), "u1"), true, None).await;
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").unwrap();
        assert_eq!(stats.forced_replies, 1);
        assert_eq!(stats.action_counts, [0, 0, 0]);
    }

    #[tokio::test]
    async fn over_budget_is_silent_and_unlearned() {
        let state = AppState::in_memory();
        opt_in(&state, "discord:g", 1);
        // Drain the single token up front.
        assert!(AppState::lock(&state.budget).try_take("discord:g", 1, runtime::now()));
        // Force the policy's hand: ε = 1 would still be random, so pin the
        // brain to prefer React by lowering ε and retrying until it picks a
        // non-Stay action; the budget must refuse every one of them.
        let out = FakeOut::default();
        let mut saw_over_budget = false;
        for i in 0..40 {
            let mut m = message("lol nice", Some("g"), "u1");
            m.native_message_id = format!("m{i}");
            match handle(&state, &out, m, false, None).await {
                Outcome::OverBudget => {
                    saw_over_budget = true;
                    break;
                }
                Outcome::Stayed => continue,
                other => panic!("acted past the budget: {other:?}"),
            }
        }
        assert!(saw_over_budget, "the policy never picked reply/react in 40 tries — raise the loop or seed");
        assert!(out.reacted.lock().unwrap().is_empty());
        assert!(out.sent.lock().unwrap().is_empty());
        // Every OverBudget left no experience: buffer holds only the silence experiences.
        let brains = AppState::lock(&state.brains);
        let stats = brains.stats("discord:g").unwrap();
        let stays = stats.action_counts[BotAction::Stay.index()];
        assert_eq!(brains.get("discord:g").unwrap().buffer_len() as u64, stays);
    }
```

Also update the existing test `quiet_and_learning_off_gate_unsolicited_speech_before_the_policy`: its second half (`learning_enabled = false` → `Ignored("learning off")`) must now opt the guild in first (call `opt_in(&state, "discord:g", 6)` before setting `learning_enabled = false`, in the same `update` or a second one), because `act off` is checked first. And `unsolicited_text_consults_the_policy_and_never_speaks_without_a_backend` must `opt_in` first and accept `Outcome::OverBudget` in its `matches!`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test pipeline:: 2>&1 | grep -E "error|test result" | head -3`
Expected: compile error — `no variant named OverBudget` / `no field unsolicited`.

- [ ] **Step 3: Implement**

Add `OverBudget,` to `Outcome` after `CooledDown,` with doc `/// The guild's hourly budget is spent; nothing sent, nothing learned.`.

Replace the two gate `if`s (the block beginning `// Two hard gates on unsolicited speech`) with:

```rust
    // Gates on unsolicited speech, checked before the policy so nothing is
    // learned from a message Abbey was never allowed to answer — in order:
    // the operator's `ABBEY_QUIET=1` (wins over any guild), the guild's own
    // `/admin act on` (opt-in, default off), and `/admin learning off`.
    if !forced && state.quiet {
        return Outcome::Ignored("quiet");
    }
    if !forced && !settings.unsolicited {
        return Outcome::Ignored("act off");
    }
    if !forced && !settings.learning_enabled {
        return Outcome::Ignored("learning off");
    }
```

Replace the `let action = { ... };` block with:

```rust
    let action = {
        // Load the guild's brain even on the forced path: the reward for a
        // mention/DM reply settles 150 s later into `BrainRegistry::remember`,
        // which drops experiences for guilds that are not loaded.
        let mut brains = AppState::lock(&state.brains);
        let stores = AppState::lock(&state.stores);
        let brain = brains.brain(&scoped_guild, &*stores, now);
        if let Some(eps) = settings.epsilon_override {
            brain.set_epsilon(eps);
        }
        if forced {
            if let Some(stats) = brains.stats_mut(&scoped_guild) {
                stats.record_forced();
            }
            BotAction::Reply
        } else {
            let q = brain.q_values(&encoded);
            let chosen =
                BotAction::from_index(brain.select_action(&encoded)).unwrap_or(BotAction::Stay);
            if let Some(stats) = brains.stats_mut(&scoped_guild) {
                stats.record_decision(&encoded, &q, chosen);
            }
            tracing::info!(
                guild = %scoped_guild,
                action = crate::brain::telemetry::action_name(chosen),
                q = ?q,
                intent = ?intent,
                heat,
                "policy decision"
            );
            chosen
        }
    };
```

Replace the cooldown block (`// Unsolicited output is rate-limited per channel…` through its closing brace) with:

```rust
    // Unsolicited output is rate-limited twice: per channel (cooldown, the
    // burst guard) and per guild (hourly budget, the volume guard). Mentions
    // and DMs bypass both. Over budget, the decision is not acted on and not
    // learned — silence was not the policy's choice.
    if !forced {
        let permitted = AppState::lock(&state.cooldown).permitted(
            &scoped_channel,
            settings.reply_cooldown_seconds,
            now,
        );
        if !permitted {
            return Outcome::CooledDown;
        }
        let within_budget = AppState::lock(&state.budget).try_take(
            &scoped_guild,
            settings.unsolicited_per_hour,
            now,
        );
        if !within_budget {
            return Outcome::OverBudget;
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test pipeline:: 2>&1 | grep "test result"`
Expected: all pass (the ignored live test stays ignored).

- [ ] **Step 5: Full gate (dead code should now be zero — budget/telemetry are wired)**

Run: `./check.sh > /tmp/claude-501/gate.log 2>&1; echo "EXIT: $?"`
Expected: `EXIT: 0`. If clippy reports an unused `BrainView` or `render`, that is Task 7's consumer — proceed to Task 7 before committing, or commit with the warnings noted and fix in Task 7 (the gate must be green by the end of Task 7).

- [ ] **Step 6: Commit**

```bash
git add src/pipeline.rs
git commit -m "feat(pipeline): per-guild act gate, hourly budget, policy decision telemetry"
```

---

### Task 7: `commands_brain.rs` — `/admin act`, `/admin budget`, richer `/admin brain`, `/stats` tokens

**Files:**
- Modify: `src/commands_brain.rs` (`admin` subcommand list ~line 380, `admin_brain` ~line 499, `stats` ~line 340, add two subcommands after `admin_cooldown`)

**Interfaces:**
- Consumes: `guild::clamp_budget`, `telemetry::{BrainStats, BrainView}`, `AppState.budget`, `BrainRegistry::{stats, stats_mut}`.
- Produces: slash subcommands `admin act state:on|off`, `admin budget per_hour:1–60`; `/admin brain` body is `BrainStats::render`.

- [ ] **Step 1: Add the two subcommands** (after `admin_cooldown`)

```rust
/// Let Abbey speak unsolicited in this server (the per-guild policy decides).
#[poise::command(slash_command, guild_only, ephemeral, rename = "act")]
pub async fn admin_act(
    ctx: Context<'_>,
    #[description = "on | off"] state: OnOff,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let on = state.is_on();
    let Some(_) = update_settings(ctx, |s| s.unsolicited = on) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(if on {
        "Abbey may now speak unsolicited here — bounded by the cooldown and the hourly budget (`/admin budget`). `ABBEY_QUIET=1` on the host still silences her."
    } else {
        "Abbey will only answer mentions, DMs, and commands here."
    })
    .await?;
    Ok(())
}

/// Unsolicited actions allowed per hour in this server (1–60).
#[poise::command(slash_command, guild_only, ephemeral, rename = "budget")]
pub async fn admin_budget(
    ctx: Context<'_>,
    #[description = "1–60"] per_hour: i64,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let n = guild::clamp_budget(per_hour);
    let Some(_) = update_settings(ctx, |s| s.unsolicited_per_hour = n) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!("Unsolicited budget: **{n}/h** for this server."))
        .await?;
    Ok(())
}
```

Register both in the `admin` attribute's `subcommands(...)` list: add `"admin_act", "admin_budget",` after `"admin_cooldown",`. Forgetting this compiles fine and ships nothing (CLAUDE.md).

- [ ] **Step 2: Replace the `let text = { ... };` block in `admin_brain`**

```rust
    let text = {
        let now = runtime::now();
        let (settings, tokens_left) = {
            let mut stores = AppState::lock(&state.stores);
            let settings = AppState::lock(&state.guilds).config(&g, &mut *stores);
            let tokens_left = AppState::lock(&state.budget).tokens_left(
                &g,
                settings.unsolicited_per_hour,
                now,
            );
            (settings, tokens_left)
        };
        let mut brains = AppState::lock(&state.brains);
        let stores = AppState::lock(&state.stores);
        let brain = brains.brain(&g, &*stores, now);
        if let Some(eps) = override_eps {
            brain.set_epsilon(eps);
        }
        let (eps, steps, buffer) = (brain.epsilon(), brain.step_count(), brain.buffer_len());
        let experiences = brains.experience_count(&g).unwrap_or(0);
        let view = BrainView {
            scoped_guild_id: &g,
            epsilon: eps,
            learn_steps: steps,
            buffer_len: buffer,
            buffer_capacity: runtime::REPLAY_CAPACITY,
            experiences,
            budget_per_hour: settings.unsolicited_per_hour,
            tokens_left,
            topology: &runtime::TOPOLOGY,
        };
        let stats = brains.stats(&g).cloned().unwrap_or_default();
        format!("{}\nact: {}", stats.render(&view), if settings.unsolicited { "on" } else { "off" })
    };
```

Add `use crate::brain::telemetry::BrainView;` at the top and remove the now-unused `use crate::brain::state::BotAction;` if nothing else in the file uses it (check with `grep -n BotAction src/commands_brain.rs`).

- [ ] **Step 3: `/stats` — one extra line**

In `stats`, after `let pending = ...;`, add:

```rust
    let budget_line = {
        let mut stores = AppState::lock(&state.stores);
        let settings = AppState::lock(&state.guilds).config(&g, &mut *stores);
        let left = AppState::lock(&state.budget).tokens_left(&g, settings.unsolicited_per_hour, runtime::now());
        format!(
            "act: {} · budget {left:.1} of {}/h left",
            if settings.unsolicited { "on" } else { "off" },
            settings.unsolicited_per_hour
        )
    };
```

and append `\n{budget_line}` to the `format!` that builds `text` (pass `budget_line` as an argument).

- [ ] **Step 4: Gate**

Run: `cargo fmt --all && ./check.sh > /tmp/claude-501/gate.log 2>&1; echo "EXIT: $?"; grep "test result" /tmp/claude-501/gate.log`
Expected: `EXIT: 0`, all tests pass, zero warnings.

- [ ] **Step 5: Read the rendered output** (CLAUDE.md: a passing snapshot test is not evidence that output reads well)

Run: `cargo test brain::telemetry::tests::render_reads_well -- --nocapture` and read the string in the test; confirm it is what a moderator would want to see. Adjust wording in `render` + its test together if not.

- [ ] **Step 6: Commit**

```bash
git add src/commands_brain.rs
git commit -m "feat(admin): /admin act, /admin budget, telemetry in /admin brain and /stats"
```

---

### Task 8: Docs, ledger, PR

**Files:**
- Modify: `README.md` (Commands table `/admin` row; "The learning loop" design note; env table `ABBEY_QUIET` row), `CLAUDE.md` + `AGENTS.md` (the "Two hard gates" rule paragraph; the "Commands that recommend do not act" exception sentence), `docs/live-test-protocol.md` (Phase D), `tasks/goals.md`, `tasks/todo.md`.

- [ ] **Step 1: README**

In the `/admin` row, change `/admin show|persona|learning|vision|cooldown|brain|flush|export|reset` to `/admin show|persona|learning|vision|cooldown|act|budget|brain|flush|export|reset` and append to its description: ` \`act on\` opts the server in to unsolicited replies (default off); \`budget\` caps them per hour (default 6).`

In "The learning loop" note, replace the sentence beginning `Unsolicited output is rate-limited per channel` with:

```
Unsolicited output needs the server's opt-in (`/admin act on`, default off — on
a token that sits in 58 servers nothing speaks up until an admin asks), then is
bounded twice: per channel by the cooldown (`/admin cooldown`, default 20 s)
and per server by an hourly budget (`/admin budget`, default 6/h; over budget
the policy's choice is neither acted on nor learned). `/admin learning off`
pins a server to mentions and commands, and `/admin brain` shows ε / steps /
buffer, the last decision's Q-values, the action histogram, recent reward mean,
and budget left — so the loop is inspectable rather than a black box.
```

`ABBEY_QUIET` row: append ` Wins over every server's \`/admin act on\`.`

- [ ] **Step 2: CLAUDE.md and AGENTS.md (identical edit in both)**

Replace the paragraph beginning `**Two hard gates on unsolicited speech, checked before the policy.**` with:

```
**Unsolicited speech is gated four times, in this order, before the policy is
consulted: `ABBEY_QUIET=1` (operator, wins over everything) → the guild's
`/admin act on` (opt-in, default off) → `/admin learning off` → the
blank-content guard.** After the policy picks reply/react: per-channel cooldown,
then the per-guild hourly budget (`brain/budget.rs`, default 6/h); over budget
returns `Outcome::OverBudget` and records **no** experience — silence was not
the policy's choice, so it must not be taught as one. Mentions, DMs, and
commands bypass all of it and are counted as `forced_replies` in the guild's
`BrainStats` (`brain/telemetry.rs`), never as decisions. Every policy decision
logs one `policy decision` line with the Q-values.
```

And in the "Commands that recommend do not act" paragraph, replace `per-channel cooldown (default 20 s), \`/admin learning off\` pins a server to mentions and commands` with `the server's own opt-in (\`/admin act on\`), the per-channel cooldown (default 20 s), the per-guild hourly budget (default 6), and \`/admin learning off\``.

Verify the mirror: `diff CLAUDE.md AGENTS.md | wc -l` must print `8` (the two header lines only).

- [ ] **Step 3: Live protocol Phase D** — replace Phase D in `docs/live-test-protocol.md` with:

```
## D — the policy acting (requires MESSAGE_CONTENT in the Dev Portal)
Restart with `ABBEY_MESSAGE_CONTENT=1` and **without** `ABBEY_QUIET`. In the
sandbox guild only: `/admin act on`, `/admin budget 6`, `/admin show` confirms.
Send 5–10 ordinary messages (no mention) across a couple of minutes. Expect:
one `policy decision … action=… q=[…]` log line per message; at least one
`Reacted` or `Replied` (the reply references the message); `/admin brain`
shows the last decision's Q-values and a non-zero histogram; `/stats` shows
budget tokens decreasing; after 7+ actions in an hour `OverBudget` in the log
and no further output; **every other guild's messages log `Ignored("act off")`**.
React 👍/👎 on her unsolicited replies; after 150 s `reward settled` and the
mean in `/admin brain` moves. Record what was and was not observed.
```

- [ ] **Step 4: Ledger** — in `tasks/goals.md` add a new `## Guild learning loop acts in opted-in servers (sub-project 3)` section with `status: in_progress`, one bullet pointing at the spec + plan, and one bullet to be filled by the live acceptance. In `tasks/todo.md` add the plan's eight tasks as checkboxes and tick the ones done.

- [ ] **Step 5: Gate, branch, PR**

```bash
./check.sh > /tmp/claude-501/gate.log 2>&1; echo "EXIT: $?"
git checkout -b feat/guild-learning-loop   # if not already on a feature branch
git add README.md CLAUDE.md AGENTS.md docs/live-test-protocol.md tasks/
git commit -m "docs: per-guild act/budget gates, telemetry, live protocol phase D"
git push -u origin feat/guild-learning-loop
gh pr create --fill --body "Sub-project 3 (spec: docs/superpowers/specs/2026-08-19-guild-learning-loop-design.md). Per-guild /admin act opt-in, hourly budget, BrainStats telemetry in /admin brain. Gate green. Live acceptance (Phase D) pending MESSAGE_CONTENT in the Dev Portal."
```

---

### Task 9: Live acceptance (manual, after merge)

Not code. Prerequisite: Donald enables MESSAGE_CONTENT for the app in the Developer Portal.

- [ ] Merge the PR; `git checkout main && git pull && cargo build`.
- [ ] Stop the running bot (`pkill -INT -f target/debug/abbey-bot` — SIGINT so it persists), then start: `set -a; . ./.env; set +a; ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434 ABBEY_BOT_LLM_MODEL=gemma4:12b ABBEY_VISION_ENDPOINT=off ABBEY_MESSAGE_CONTENT=1 ABBEY_DATA_DIR=<data dir> RUST_LOG=info,abbey_bot=debug nohup ./target/debug/abbey-bot >> /tmp/claude-501/live.log 2>&1 &` (note: no `ABBEY_QUIET`).
- [ ] Follow `docs/live-test-protocol.md` Phase D in guild `1275617641620443146`; capture log lines and screenshots.
- [ ] Record outcomes in `tasks/goals.md` (observed vs not observed) and the "verified live" sections of README/CLAUDE.md/AGENTS.md; mark the goal `done` only if an unsolicited action, a budget refusal, and an other-guild `act off` ignore were all seen.
