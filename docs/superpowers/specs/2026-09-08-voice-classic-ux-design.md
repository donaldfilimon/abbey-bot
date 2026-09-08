# Voice classic Action Row UX (status → leave confirm → play)

Date: 2026-09-08
Status: **locked design only.** Approaches and architecture below record Donald's
brainstorm decisions on 2026-09-08. This document does **not** authorize Rust or
source implementation. Implementation requires a separate writing-plans pass and
Gate-phased PRs after this design lands.
Review revision: resolves the design findings on PR #104 against merged `main`
`df6a3b6`. PR #105 (`6f5fbba`) is separate implementation work, not acceptance
evidence; its plan and code must reconcile with these requirements before Gates.
Scope: classic Discord Action Row controls for in-guild voice operator UX on the
Abbey Rust bot. Extends existing `/voice join|leave|status` and play surfaces;
does not change consent grammar, voice lifecycle authority, or Components V2
expectations.

## Intent

After a successful `/voice join consent:true` or `/voice resume consent:true`,
operators need owner-bound, clickable
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
  store** (guild, user, channel, expiry, mode) persisted under `ABBEY_DATA_DIR`
  when configured; memory-only when unset. Never pack guild/user into the id.
  This UX fallback does not relax the separate durable listening-consent ledger.
- Fail-closed like `/admin` and `/pending`: wrong user/guild/expired → ephemeral
  deny; immediate successful paths ack with `UpdateMessage`; slow paths defer
  the component update promptly and then edit the same ephemeral panel.
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
/voice join or resume consent:true succeeds
        │
        ▼
 replace prior UX session; persist when configured, otherwise memory-only
   (sid, guild, user, channel, expiry, mode, …)
        │
        ▼
 send Phase A Status Action Row
   Refresh + Leave [+ optional Mode glance]
        │
        ├── Refresh → load sid → re-read live voice registry → UpdateMessage
        │
        └── Leave → swap to Phase B (no teardown yet)
                      Confirm → atomic Leaving claim + close gates
                                → ack Leaving alongside teardown → edit result
                      Cancel  → restore Phase A status row
        │
        └── (when playable + music channel allowed) Phase C Play/Stop/Skip
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

Trigger: after successful `/voice join consent:true` **or**
`/voice resume consent:true`, including disabled-mode presence-only success,
send a new **ephemeral** bot-authored Status Action Row to that invoker. Mint a
new sid and expiry and invalidate the prior panel for that guild voice session,
even when the operator changes. Failed join/resume does not issue or replace a
panel. A resume never extends the old owner's trust window. Panel delivery or
storage failure is reported as a UX failure without undoing successful voice
work or claiming the join/resume itself failed.

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
| Confirm | `ok` (or locked short act in plan) | Recheck current leave authorization; atomically claim ConfirmLeave → Leaving for this sid and bound voice session; synchronously close media/music gates, then acknowledge Leaving concurrently with existing teardown; edit the same panel to Left or Failed. |
| Cancel | `cancel` | Restores the Phase A status row via `UpdateMessage`; no leave. |

Only the atomic claim winner may invoke leave. Confirm/Cancel/Refresh replays
cannot reset Leaving, Failed, or Left to an actionable phase. No registry lock
is held over Discord work. A slow transition lock or Songbird teardown must not
delay the first acknowledgement: close authorized media/music gates before the
first await, poll the Leaving `UpdateMessage` alongside teardown as the slash
leave path does, then edit the original message with the actual result. If an
earlier defer was needed for asynchronous authorization, edit that same panel;
do not send a second initial response. Delivery failure never reopens media or
releases the claim for replay.

Leave mid-fail settles the claim to Failed with Refresh-only inspection of live
state; it does not report Left or automatically retry teardown. A new explicitly
authorized slash operation is the recovery path. Refresh cannot resurrect old
controls or operate on a replacement voice session.

### Phase C — in-VC play controls

Shown only when a **playable** session exists (music/play path available per
existing voice-play design and live registry), and the interaction text channel
passes `music::command_channel_gate`. If `ABBEY_MUSIC_COMMAND_CHANNEL_ID` is set,
suppress Phase C outside that exact channel; A/B still work there. Never spoof
the configured channel when dispatching a click. Use the actual interaction
channel and repeat the gate before each music mutation. Controls remain
Play / Stop / Skip with these explicit semantics:

- **Play:** reuse the existing native-player play/resume path for the already
  selected player/current selection, with an empty query. With no selected
  player or eligible output session, disable or deny; track selection remains
  `/voice play`. A button labelled Play must actually perform this operation.
- **Stop:** reuse stop-music semantics, stopping independently owned music
  capture/playback without changing listening consent.
- **Skip:** a **new Phase C operation**, not an existing slash-command path on
  merged `main`. Add a pure native next-track script for Spotify and Music,
  executed through the same owned, cancellable player adapter and host-music
  lease as existing controls. Advance the selected player once; do not create
  a queue or open a listening epoch. Missing player, ownership loss, cancelled
  work or adapter failure must not claim success. Gate C requires golden script
  tests for both players and dispatch tests proving authorization, ownership,
  cancellation and honest failure outcomes. An unmerged `next` helper is not
  evidence that these acceptance requirements pass.

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

Use the server-side in-process registry for every panel. Persist UX bindings
under `ABBEY_DATA_DIR` when configured (exact layout fixed in the later plan).
When unset, retain the same A/B protocol in memory, including supported
`ABBEY_VOICE_MODE=disabled` joins. Missing optional UX storage must not break a
successful voice operation. Configured-store errors are reported and fail closed
for affected controls; they are not silently treated as absent configuration.

Bind UX sessions to a process incarnation and current voice-session identity.
After restart, old sids, including persisted Leaving claims, are invalid and
cannot replay teardown; new successful join/resume issues fresh bindings.
Persistence never restores consent, media authority or an actionable old claim.
The later plan must keep storage I/O off the interaction acknowledgement path.

Minimum fields per session:

| Field | Role |
|-------|------|
| `sid` | Opaque key matching the custom id |
| `guild` | Exact guild id; must match interaction guild |
| `user` | Invoking owner/operator id; must match clicker |
| `voice_channel`, `text_channel`, `message` | Separate exact voice and originating panel bindings; validate against current runtime and interaction |
| `voice_session`, `incarnation` | Bind to the current voice-session identity and process; replacements/restarts invalidate old controls |
| `expiry` | Absolute expiry; past expiry → deny |
| `mode` / phase | Status, ConfirmLeave, Leaving (claimed), Left, Failed; C enablement is derived from current music gates |

Store rules:

- Create/replace on successful join or resume, including presence-only success;
  invalidate the previous owner-bound session, not only its visible message.
- Load by `sid` on every click before any voice mutation.
- Reject if missing, expired, wrong user/guild/text channel/message, or no longer
  bound to the current voice session/process. Stored ownership is necessary but
  is **not current authorization**.
- Before **every Confirm/Play/Stop/Skip mutation**, recheck the matching command
  policy using current Discord facts. Confirm requires current configured-VC
  presence **or** Manage Server (fresh component permission payload and current
  voice state, matching slash leave). Music requires **both** Manage Server and
  current configured-VC presence plus the actual command-channel, host-platform,
  playable and host-music-ownership gates. Fetch current member/roles over REST
  for music as `play::authorized` does; never trust roles saved at panel creation.
  Missing/unavailable authorization facts deny without mutation. After async
  checks, revalidate session/phase and runtime identity immediately before acting.
- Expiry is not extended by Refresh or Cancel (parity with guided memory browser:
  navigation does not refresh trust windows unless a later plan explicitly
  justifies a renew path).
- Atomically compare-and-set ConfirmLeave → Leaving before any teardown or
  await in the authorized leave transition. Only the winner owns teardown; a
  second click cannot also win. Settle to Left or Failed exactly once. Do not
  hold the registry lock across network work or reset the claim after ack failure.
- Successful leave or foreign teardown makes the old binding terminal; Refresh
  renders honest "not in VC" / left state without resurrecting controls.

## Data flow

1. `/voice join|resume consent:true` succeeds through its existing consent path,
   including presence-only mode where supported.
2. Invalidate the previous UX binding and mint a fresh owner-bound session in
   memory; persist when configured without turning UX failures into voice failure.
3. Send an **ephemeral** Phase A follow-up. All later updates retain that
   visibility. Member consent choice and authorization-bounded channel details
   must never appear in a channel-visible panel; the public consent notice is a
   separate existing lifecycle message.
4. Click → parse → validate owner/guild/channel/message/expiry/session/phase →
   acknowledge promptly → obtain any required fresh Discord facts → revalidate
   and dispatch. For immediately authorized Confirm, claim and close gates
   synchronously before the first await, as specified in Phase B.
5. Refresh re-reads live state and current channel-visibility authorization;
   Leave only swaps to Phase B; Cancel restores A only from unclaimed Phase B.
6. Confirm uses the atomic Leaving claim, early acknowledgement and existing
   teardown; settle the claim and edit the same message to the observed result.
7. Phase C repeats all current music authorization gates, dispatches existing
   play/stop semantics or the explicitly new next-track operation, and edits the
   result. Buttons never start STT or grant consent.

Ack policy:

- Immediate updates use `UpdateMessage`. Any REST lookup or player operation
  that could exceed the interaction deadline requires a prompt deferred
  **component update** before that work, followed by an edit of the same panel.
  A defer does not authorize mutation. Confirm preserves the special gate-close
  and concurrent-ack ordering above; no storage or transition-lock wait may
  consume the initial response window.
- Immediate auth/bind failures use an ephemeral `Message` deny. Failures found
  after deferral edit the ephemeral panel or use an ephemeral follow-up; never
  attempt a second initial response. No partial leave or play on denied auth.

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
- **Leave race/deadline:** deliver two Confirm clicks concurrently behind a
  blocked teardown; only one claim/invocation wins. Hold transition-lock and
  Discord teardown beyond 3 seconds and verify gates closed immediately, ack
  sent before deadline, and terminal edit only after completion. Test ack failure,
  teardown failure, Cancel/Refresh during Leaving, and restart/replay of a claim.
  Cancel and Refresh never leave; Failed Refresh never rearms Confirm.
- **Authorization changes:** revoke manager role and move owner out of the VC
  between rendering and clicking. Each music mutation denies unless both current
  conditions hold; Confirm allows presence or manager and denies when neither
  holds. Unavailable REST facts deny. No mutation reaches a replacement runtime.
- **Music channel:** join outside the configured music text channel, verify A/B
  remain available but C is suppressed; forged/replayed music acts are denied.
  Test unset channel configuration too.
- **Play/Stop/Skip:** verify actual play/resume and stop effects, next-track golden
  scripts for both players, ownership/cancellation/failure paths, and disabled or
  denied controls when no eligible player/session exists.
- **Visibility:** initial join/resume, Refresh, Confirm, failures and deferred
  edits remain ephemeral; never copy member consent or hidden channel data into
  public messages.
- **Storage:** disabled-mode successful join with unset `ABBEY_DATA_DIR` produces
  usable memory-only A/B controls. Test configured-store failure separately and
  restart invalidation; no UX error changes consent or misreports voice success.
- **Resume:** a different manager resumes after the first panel expires; issue a
  fresh owner/expiry/sid, reject the prior owner's old panel and leave expiry
  unchanged on Refresh/Cancel. Failed resume does not replace the binding.
- **Gate green** on each phased implementation PR.
- Later: roadmap + `MLAI-LIVE-ACCEPTANCE.md` note when live UX is witnessed
  (separate from this design PR).

This design PR itself only adds documentation; no new Rust tests land here.

## Acceptance (this design document)

- Records all three surfaces A/B/C and the phased classic Action Row approach.
- Locks short custom ids + server-side session store (no guild/user stuffing).
- Locks current authorization, ephemeral visibility, atomic leave claims and
  immediate-update/deferred-component acknowledgement policy.
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
2. Writing-plans: Phase A fixes exact store path, session identity, renewal and
   memory-only fallback; B specifies atomic claims and ack/deadline fixtures; C
   specifies music-channel gates and new next-track operation. Visibility is
   already locked ephemeral. Reconcile any existing plan (including PR #105)
   against this revision; unchecked requirements remain implementation work.
3. Implement A → Gate → PR; B → Gate → PR; C → Gate → PR.
4. Optional roadmap / `MLAI-LIVE-ACCEPTANCE.md` live note after witnessed UX.

Until step 2–3 exist, treat any voice UX button code as out of scope.
