# Voice classic Action Row UX (A status → B leave confirm → C play) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** After successful `/voice join consent:true`, ship durable classic Action Row controls for status refresh, safe leave confirm/cancel, and play/stop/skip when playable — fail-closed like `/admin` and `/pending`, with short `abbey:v:{sid}:{act}` ids backed by a server-side session store under `ABBEY_DATA_DIR`.

**Architecture:** Pure `voice_ux` owns custom-id grammar, phase reduce, playable enablement, and status copy. `voice_ux_store` owns durable sid→{guild,user,channel,expiry,phase} binding under `ABBEY_DATA_DIR` (in-memory when unset). Thin `commands_voice/ux.rs` owns Discord Action Rows, post-join follow-up, and component dispatch (`UpdateMessage` on success; ephemeral `Message` deny). Leave Confirm reuses the existing authorize→close-media→teardown leave path once. Consent stays slash-only; buttons never start STT.

**Tech Stack:** Rust 1.98, Serenity 0.12.5, Poise 0.6.2, Tokio, existing voice registry / music gates / `getrandom`.

**Spec:** `docs/superpowers/specs/2026-09-08-voice-classic-ux-design.md` (locked; included on this branch).

## Global Constraints

- Custom ids: exact `abbey:v:{sid}:{act}` ASCII; acts `ref|leave|ok|cancel|play|stop|skip`; never embed guild/user/channel/expiry in the id.
- Fail-closed: wrong user/guild/expired/missing/malformed → ephemeral deny; zero voice mutations.
- Success clicks ack with `UpdateMessage` (or Defer + edit when teardown needs time); never miss the 3s window.
- `consent:true` slash-only — no consent button; never start STT/listening from buttons.
- Out of scope: Components V2, Portal/OAuth, bot Go Live.
- Expiry does not renew on Refresh or Cancel (parity with memory browser).
- Confirm leave invokes existing leave path **once**; Cancel restores Phase A; Refresh never leaves.
- Play/Stop/Skip enabled only when playable; otherwise disabled and/or ephemeral explain.
- Dispatch `abbey:v:` **before** `commands_help::dispatch_component` (help claims other `abbey:` prefixes).
- Keep production modules under 1,000 lines; leave unrelated dirty files alone.
- Never paste Discord tokens.

## File map

- `docs/superpowers/specs/2026-09-08-voice-classic-ux-design.md`: locked design (included from docs PR).
- `docs/superpowers/plans/2026-09-08-voice-classic-ux.md`: this plan.
- `src/voice_ux.rs`: pure session model, custom-id parse/format, phase reduce, playable/button matrix, status text helpers + unit tests.
- `src/voice_ux_store.rs`: `ABBEY_DATA_DIR` session store (atomic JSON) + unit tests.
- `src/commands_voice/ux.rs`: Discord rows, post-join panel, `dispatch_ux_component`, leave-confirm / play wiring.
- `src/commands_voice.rs` / `src/commands_voice/start.rs`: export dispatch; send Phase A after successful join.
- `src/commands_voice.rs` leave path: share teardown helper callable from Confirm.
- `src/player_control.rs`: pure `next` (skip) scripts for Spotify/Music.
- `src/startup.rs`: register UX component dispatch ahead of help.
- `src/commands_help.rs`: belt-and-suspenders skip for `abbey:v:`.
- `src/main.rs`: `mod voice_ux;` / `mod voice_ux_store;`.
- `docs/discord-application-api-roadmap.md`: brief classic voice UX acceptance note.

### Task 1: Plan + design on feature branch

**Files:** design spec + this plan.

- [x] Branch `feat/voice-classic-ux-abc` from `origin/main`; include design spec (PR #104 Gate not green yet).
- [x] Write this plan; commit plan (+ design if not already committed).

### Task 2: Pure voice UX protocol (TDD)

**Files:** Create `src/voice_ux.rs`; register in `src/main.rs`.

**Interfaces:**

```rust
pub const SESSION_SECONDS: u64 = 15 * 60;
pub const CUSTOM_ID_PREFIX: &str = "abbey:v:";

pub enum Phase { Status, ConfirmLeave, Left }
pub enum Act { Ref, Leave, Ok, Cancel, Play, Stop, Skip }
pub enum Rejection { Malformed, Missing, ForeignOwner, ForeignGuild, Expired, WrongPhase }

pub struct Session {
    pub sid: String,
    pub guild: u64,
    pub user: u64,
    pub channel: u64,
    pub expiry: u64,
    pub phase: Phase,
}

pub fn format_custom_id(sid: &str, act: Act) -> String;
pub fn parse_custom_id(id: &str) -> Result<(String, Act), Rejection>;
pub fn authorize(session: &Session, actor: u64, guild: Option<u64>, now: u64) -> Result<(), Rejection>;
pub fn reduce(phase: Phase, act: Act) -> Result<Phase, Rejection>;
pub fn playable(has_runtime: bool, media_enabled: bool, phase_failed: bool, start_pending: bool) -> bool;
pub fn status_buttons(playable: bool) -> &'static [Act]; // ref, leave [, play, stop, skip]
pub fn confirm_buttons() -> &'static [Act]; // ok, cancel
```

- [ ] Write failing unit tests: custom-id round-trip; malformed/unknown act; authorize wrong user/guild/expired; reduce A→B→A, A→B→Left; play acts rejected off Status; playable matrix disables play row.
- [ ] Implement until tests pass. `cargo test --locked voice_ux::`

### Task 3: Session store (TDD)

**Files:** Create `src/voice_ux_store.rs`; register in `src/main.rs`.

- [ ] Store path: `{ABBEY_DATA_DIR}/voice-ux-sessions.json` (atomic temp+rename); `None` dir = memory-only.
- [ ] Tests: create/load/update phase/destroy; missing sid; expiry not extended on update phase; concurrent-safe mutex.
- [ ] `cargo test --locked voice_ux_store::`

### Task 4: Discord adapter + post-join Phase A

**Files:** `src/commands_voice/ux.rs`, wire `start.rs` success path, `commands_voice.rs` mod/export, `startup.rs`, `commands_help.rs` skip.

- [ ] After successful join/resume activation (not presence-only / failed paths), mint sid, persist session, send ephemeral follow-up with Phase A rows (Refresh+Leave; Play/Stop/Skip disabled unless playable).
- [ ] `dispatch_ux_component`: prefix `abbey:v:`; load store; fail-closed ephemeral Message; success `UpdateMessage`.
- [ ] Refresh → live registry status text + button enablement; Leave → Phase B; Cancel → Phase A; Confirm → shared leave once → Phase Left / clear controls; Play/Stop/Skip → existing music paths when playable else ephemeral.
- [ ] Focused tests for deny paths and reduce wiring where pure; adapter smoke via existing patterns if feasible.

### Task 5: Leave confirm once + playable play/stop/skip

**Files:** extract shared leave teardown from `voice_leave`; `player_control::next`; ux play handlers.

- [ ] Confirm calls shared leave exactly once; Cancel/Refresh never call it (unit/integration assertions on a counter/probe where practical).
- [x] Skip uses pure `next` AppleScript; Play with empty query mirrors current selection (existing `play`).
- [ ] Tests: leave confirm once; cancel restores; play disabled when not playable.

### Task 6: Docs, Gate checks, PR

- [ ] Update `docs/discord-application-api-roadmap.md` acceptance note (classic voice UX A/B/C landed).
- [ ] `cargo fmt`; `cargo clippy -D warnings` on touched crates; focused `voice_ux` / `voice_ux_store` / voice command tests; prefer `./check.sh` if time allows.
- [ ] Push `feat/voice-classic-ux-abc`; open PR (do not merge; no force-push main).

## Gate checklist

- Unit: custom-id round-trip; authorize deny matrix; phase reduce; playable enablement.
- Dispatch: wrong user/guild/expired/missing → ephemeral deny, zero mutations.
- Leave confirm once; Cancel restores A; Refresh never leaves.
- Play/Stop/Skip enabled only when playable.
- No consent button; no STT start from buttons.
- Fmt + clippy `-D warnings` + locked tests green.
