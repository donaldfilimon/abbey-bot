# Guild learning loop — per-guild opt-in, budget, telemetry

**Date:** 2026-08-19 · **Status:** approved design (brainstorm with Donald) · **Sub-project 3 of 4** ("improve all": reply quality/speed, tools, learning loop, breadth/ops).

## Goal

Let the per-guild DQN act unsolicited (reply / react / stay) in guilds that opt in, safely bounded, and make what it does legible — so the learning loop designed in `docs/spec/adaptivelearning.md` can be watched and tuned on a real server. Today it never acts: blank content without MESSAGE_CONTENT, and `ABBEY_QUIET=1` blocks everything.

## Decisions

1. **Gating is per guild, opt-in, default off.** `GuildSettings.unsolicited: bool` (default `false`), toggled by `/admin act on|off` (Manage Server). `ABBEY_QUIET=1` stays as the operator's global override and wins over everything. No operator allow-list.
2. **Budget is per guild.** A refilling token bucket keyed by scoped guild id, capacity `GuildSettings.unsolicited_per_hour` (default 6, `/admin budget <1–60>`), refilling at capacity/3600 per second, clock injected. Cooldown (20 s, per channel) stays in front as the burst guard. Over budget → the policy's Reply/React is not executed **and no experience is recorded** (silence was not the policy's choice).
3. **Telemetry is per guild, in memory.** `BrainStats { last_state, last_q, last_action, action_counts{stay,reply,react}, forced_replies, recent_rewards (last 20), settled_total }`; `/admin brain` renders it; one info log line per unsolicited decision.
4. Mentions, DMs, and commands are unchanged: always answered, never budgeted, never counted as policy decisions (counted as `forced_replies`).

## Pipeline order (non-forced message)

blank-content guard (first — nothing to learn from) → `ABBEY_QUIET` → `settings.unsolicited` (else `Ignored("act off")`) → `settings.learning_enabled` (else `Ignored("learning off")`) → encode state → policy (`select_action`, record decision) → Stay ⇒ silence experience → else cooldown (`CooledDown`) → budget (`OverBudget`, unrecorded) → act → register reward.

## Modules

- `brain/budget.rs` (new, pure): `Budget { buckets: HashMap<String, (tokens: f32, last: u64)> }`, `try_take(key, capacity_per_hour: u32, now) -> bool`, `tokens_left(key, capacity, now) -> f32`.
- `brain/telemetry.rs` (new, pure): `BrainStats` + `record_decision(state, q, action)`, `record_forced()`, `record_reward(r)`, `render(&self, ...) -> String`.
- `brain/registry.rs`: `stats: HashMap<String, BrainStats>` alongside brains; `stats_mut(guild)`, `stats(guild)`; evicted with the brain.
- `guild.rs`: `GuildSettings.unsolicited` (default false), `unsolicited_per_hour` (default 6, clamp 1..=60); `render_settings` shows `act: on|off · budget: N/h`; serde `#[serde(default)]` so old documents load.
- `pipeline.rs`: gate order above; `Outcome::OverBudget`; decision/forced/reward telemetry calls; info log per decision.
- `runtime.rs`: `budget: Mutex<Budget>`; `settle_rewards` feeds `record_reward`.
- `commands_brain.rs`: `/admin act`, `/admin budget`; `/admin brain` and `/stats` render telemetry + tokens left.
- Docs: README ("act is per guild; QUIET is the operator override; budget per guild"), CLAUDE.md/AGENTS.md rules section, `.env.example` unchanged.

## Testing

Pure: bucket refill/cap/empty math; gate order (`act off`, `learning off`, `CooledDown`, `OverBudget` with no experience recorded, Stay records silence); telemetry counts and `render` snapshot; settings serde default for old documents. Gate stays offline.

Live acceptance: Donald enables MESSAGE_CONTENT in the Dev Portal; restart with `ABBEY_MESSAGE_CONTENT=1` (QUIET unset); `/admin act on` in sandbox guild `1275617641620443146`; observe `policy … action=… q=[…]` lines, an unsolicited reply or react, `/admin brain` showing Q-values/histogram, the budget refusing past 6/h, and **no other guild speaking** (their `act` is off).

## Out of scope

Tools, backend routing, reward-shape changes, persisting telemetry, vision, Telegram/Slack.
