# Voice classic Action Row UX (status → leave confirm → play)

Date: 2026-09-08
Status: **locked design only.** Approaches and architecture below record Donald's
brainstorm decisions on 2026-09-08. This document does **not** authorize Rust or
source implementation. Implementation requires a separate writing-plans pass and
Gate-phased PRs after this design lands.
Scope: classic Discord Action Row controls for in-guild voice operator UX on the
Abbey Rust bot. Extends existing `/voice join|leave|status` and play surfaces;
does not change consent grammar, voice lifecycle authority, or Components V2
expectations.

## Intent

After a successful `/voice join consent:true`, operators need durable, clickable
controls for status refresh, safe leave, and (when a playable session exists)
play/stop/skip — without stuffing guild/user into custom ids, without starting
STT or consent from buttons, and without a mega-panel or modal-heavy flow.

Hard constraints locked with Donald:

- Do **all three** surfaces: (A) post-join status panel, (B) leave Confirm/Cancel,
  (C) in-VC play controls.
- `consent:true` stays **slash-only** — never a consent button.
- Approach: **phased classic Action Rows** (A then B then C), not a mega-panel,
  not modal-heavy.
- Custom ids: **short form** `abbey:v:{sid}:{act}` with a **server-side session
  store** (guild, user, channel, expiry, mode) under `ABBEY_DATA_DIR` — not
  packing guild/user into the custom id.
- Fail-closed like `/admin` and `/pending`: wrong user/guild/expired → ephemeral
  deny; successful paths ack with `UpdateMessage`.
- Out of scope for this design and its later implementation: Components V2,
  Portal/OAuth, bot Go Live, consent via components.

## Approaches considered

1. **Phased classic Action Rows (selected).** Ship Phase A status row first after
   join; Leave swaps the same message to Phase B Confirm/Cancel without tearing
   down the voice session; Phase C play controls appear only when a playable
   session exists. Matches shipped `/pending` Confirm/Dismiss and `/admin show`
   classic select patterns under serenity 0.12.5 / poise 0.6.2 (classic Action
   Rows only). Smallest risk surface per Gate phase; each phase is independently
   testable and revertible.

2. **Mega-panel.** One message with status + leave + play + mode always present.
   Rejected for this slice: denser permission/disable matrix, harder phased Gate
   acceptance, and more ways to accidentally expose play or leave actions when
   the session is not playable or leave is mid-fail.

3. **Modal-heavy.** Leave and play behind modals or multi-step modal wizards.
   Rejected: slower operator path, worse 3s interaction budget, and unnecessary
   relative to the locked Confirm/Cancel swap already proven on `/pending`.

Components V2 layouts remain crate-blocked (see
`docs/discord-application-api-roadmap.md`); this design deliberately stays on
classic Action Rows.

## Locked architecture

```
/voice join consent:true succeeds
        │
        ▼
 create voice UX session under ABBEY_DATA_DIR
   (sid, guild, user, channel, expiry, mode, …)
        │
        ▼
 send Phase A Status Action Row
   Refresh + Leave [+ optional Mode glance]
        │
        ├── Refresh → load sid → re-read live voice registry → UpdateMessage
        │
        └── Leave → swap to Phase B (no teardown yet)
                      Confirm → existing leave path once → UpdateMessage
                      Cancel  → restore Phase A status row
        │
        └── (when playable) Phase C Play/Stop/Skip on the status surface
              disabled or ephemeral when N/A; never starts STT/consent
```

Authority boundaries:

- **Voice session / registry** remains the source of truth for joined state,
  playable state, and leave teardown (existing `/voice leave` path).
- **Voice UX session store** is presentation + authorization binding only: who
  may click, which guild/channel the controls belong to, expiry, and which
  phase/mode the message is showing. It does not replace consent epochs or
  media gates.
- **Consent** stays on slash `consent:true` (+ existing public notice). Buttons
  never start STT, never open a listening epoch, never act as consent.

## Component surfaces

### Phase A — post-join status panel (ships first)

Trigger: after successful `/voice join consent:true`, follow up with a Status
Action Row on a bot-authored message the invoker can operate.

Controls:

| Control | Custom id act | Behavior |
|---------|---------------|----------|
| Refresh | `ref` | Load session by `sid`; reject if missing/expired/wrong user/guild; re-read live voice registry; `UpdateMessage` with current status copy and button enablement. |
| Leave | `leave` | Swap the same message to Phase B Confirm/Cancel. **Does not** tear down voice. |
| Mode glance (optional) | n/a or disabled label-only | Disabled / label-only glance at current mode; not a mutation control. |

Custom ids: `abbey:v:{sid}:ref` and `abbey:v:{sid}:leave`.

### Phase B — leave Confirm/Cancel

Entered only from Phase A Leave (or equivalent restore path).

| Control | Act | Behavior |
|---------|-----|----------|
| Confirm | `ok` (or locked short act in plan) | Runs the **existing** leave path **once**; then `UpdateMessage` to a terminal/left state (or clears controls). |
| Cancel | `cancel` | Restores the Phase A status row via `UpdateMessage`; no leave. |

Leave mid-fail: message shows failed state + Refresh so the operator can re-read
live registry without inventing a second teardown path.

### Phase C — in-VC play controls

Shown only when a **playable** session exists (music/play path available per
existing voice-play design and live registry). Controls: Play / Stop / Skip.

- Enabled only when playable; otherwise disabled and/or ephemeral explain.
- Play with nothing playable → disabled or ephemeral deny; no speculative start.
- Never starts STT or consent from these buttons.
- Custom ids remain `abbey:v:{sid}:{act}` with short acts (plan locks exact act
  tokens; examples: `play`, `stop`, `skip`).

Phasing for implementation (not this PR): A lands and Gates first; B builds on
A's session + dispatcher; C attaches when playable detection is solid. Design
acceptance covers all three; code ships in Gate-phased PRs.

## Session store and custom id grammar

### Custom id grammar

```
abbey:v:{sid}:{act}
```

- Prefix `abbey:v:` — voice UX protocol (classic Action Rows).
- `{sid}` — opaque session id minted at join-success UX creation; ASCII, short
  enough that the full id stays within Discord's 100-character custom id limit
  with margin for act tokens.
- `{act}` — short action token (`ref`, `leave`, confirm/cancel acts, play acts).
- **Do not** embed guild id, user id, channel id, or expiry in the custom id.
  Those live in the server-side store and are checked on every click.

Malformed ids, unknown acts, and unknown protocol versions fail closed (ephemeral
deny; no mutation; no registry write).

### Server-side session store

Location: under `ABBEY_DATA_DIR` (exact filename/layout fixed in the later plan;
this design requires durable binding, not a specific JSON schema yet).

Minimum fields per session:

| Field | Role |
|-------|------|
| `sid` | Opaque key matching the custom id |
| `guild` | Exact guild id; must match interaction guild |
| `user` | Invoking owner/operator id; must match clicker |
| `channel` | Voice (and/or status message) channel binding as plan defines |
| `expiry` | Absolute expiry; past expiry → deny |
| `mode` / phase | Which row is currently intended (A status, B confirm, C play-capable) |

Store rules:

- Create on successful join UX follow-up; do not create on failed join.
- Load by `sid` on every click before any voice mutation.
- Reject if missing, expired, wrong user, or wrong guild (and wrong channel if
  the plan binds channel strictly).
- Expiry is not extended by Refresh or Cancel (parity with guided memory browser:
  navigation does not refresh trust windows unless a later plan explicitly
  justifies a renew path).
- Destroy or mark terminal on successful Confirm leave; Refresh after foreign
  teardown must render honest "not in VC" / left state rather than resurrecting
  controls.

## Data flow

1. `/voice join consent:true` succeeds (existing consent + join path unchanged).
2. Create voice UX session under `ABBEY_DATA_DIR`.
3. Send Phase A status Action Row (follow-up / message as plan chooses within
   ephemeral vs channel-visible operator norms already used by voice status).
4. Click → parse `abbey:v:{sid}:{act}` → load session by `sid` → reject if
   missing/expired/wrong user/guild → then dispatch act.
5. Refresh re-reads the **live voice registry** (not a stale snapshot alone) and
   `UpdateMessage`.
6. Leave swaps to Phase B **without** teardown until Confirm.
7. Confirm runs existing leave **once**; Cancel restores Phase A.
8. Phase C acts call existing play/stop/skip paths only when playable.
9. **Never** start STT or consent from buttons.

Ack policy (parity with `/pending` / `/admin` components):

- Authorized, understood clicks: `UpdateMessage` so the 3s interaction ack is
  never missed.
- Auth / bind failures: ephemeral `Message` deny; no partial leave; no play
  start.

## Error handling

| Condition | Behavior |
|-----------|----------|
| Stale / foreign / expired / missing session | Ephemeral deny; no mutation |
| Wrong user or wrong guild | Ephemeral deny; no mutation |
| Missing perms / not in VC | Disable controls and/or explain; do not tear down spuriously |
| Leave mid-fail | Failed state + Refresh; do not claim left |
| Play with nothing playable | Disabled and/or ephemeral; no start |
| Double Confirm / replay after leave | Fail closed / no-op second teardown; honest terminal UI |
| Consent-shaped button (must not exist) | Out of scope; dispatcher must not treat any voice UX act as consent |

Never log tokens, raw audio, or full custom-id+PII dumps beyond existing redaction
norms. Session files hold Discord snowflakes; treat like other `ABBEY_DATA_DIR`
operator state.

## Testing (design acceptance — implement in later PRs)

Design acceptance criteria for the eventual implementation (not this docs PR):

- **Unit:** parse custom id; bind session fields; reduce phase transitions
  (A→B→A, A→B→left, playable→C enablement) as pure functions where possible.
- **Dispatch fixtures:** wrong user, wrong guild, expired, missing sid →
  ephemeral deny and zero voice mutations.
- **Leave confirm once:** Confirm invokes existing leave path exactly once;
  Cancel never leaves; Refresh never leaves.
- **Play enablement:** Play/Stop/Skip enabled only when playable session exists;
  otherwise disabled/ephemeral.
- **Gate green** on each phased implementation PR.
- Later: roadmap + `MLAI-LIVE-ACCEPTANCE.md` note when live UX is witnessed
  (separate from this design PR).

This design PR itself only adds documentation; no new Rust tests land here.

## Acceptance (this design document)

- Records all three surfaces A/B/C and the phased classic Action Row approach.
- Locks short custom ids + server-side session store (no guild/user stuffing).
- Locks fail-closed bind checks and `UpdateMessage` ack policy.
- Keeps `consent:true` slash-only; lists explicit non-goals.
- States that **implementation requires a separate writing-plans + Gate-phased
  PRs** — no voice UX code in the design PR.

## Explicit non-goals

- Components V2 (Container / Section / `IS_COMPONENTS_V2`) — crate-blocked.
- Portal / OAuth / Linked Roles flows for voice UX.
- Bot Go Live / Embedded Activity visuals as a substitute for these rows.
- Consent via components (no consent button, no modal consent attestation).
- Replacing `/voice leave` slash or changing consent epoch / media-gate
  semantics.
- Mega-panel or modal-heavy leave/play wizards.
- Stuffing guild, user, channel, or expiry into the custom id.
- Any Rust/source implementation, Gate run for new code, or `tasks/goals.md`
  ledger edit in the design PR (ledger may reference this spec later in a
  dedicated ledger PR if #103 or a successor owns that file).

## Implementation sequencing (future)

1. Land this design (docs-only).
2. Writing-plans: Phase A plan with exact store path, act tokens, message
   visibility, and Gate checklist; then B; then C.
3. Implement A → Gate → PR; B → Gate → PR; C → Gate → PR.
4. Optional roadmap / `MLAI-LIVE-ACCEPTANCE.md` live note after witnessed UX.

Until step 2–3 exist, treat any voice UX button code as out of scope.
