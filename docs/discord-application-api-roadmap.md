# Discord Application API roadmap (Abbey)

Phased plan for Discord Application / Interactions / Activities features beyond
the guild bot gateway. Complements [`activities.md`](activities.md) (operator
how-to for rocket launch + Portal URL mapping). This file is the product
backlog; implement in order unless a phase is explicitly deferred.

**Hard rules (all phases):**

- **Never bot Go Live.** Discord has no bot screenshare / Stream start API.
  In-voice visuals go through Embedded Activities only.
- **No secrets in git.** Application ID is public. Client Secret, bot token,
  OAuth refresh material, and monetization keys stay in operator env /
  launchd / secret store — never committed, never pasted into chat logs that
  land in the repo.
- Preserve Entry Point `launch` (`PRIMARY_ENTRY_POINT`, handler Discord Launch
  Activity). Global registration must keep it
  (`register_globally_keeping_entry_point` in `src/main.rs`).

---

## 2026-09-06 source integration update

Guided private help, the `Abbey: memory` member menu, four-fact private browser,
scoped `/stats`, and fixed persistence/provider recovery advice are implemented
in the completion worktree. Browser pages recheck owner, exact server or bot-DM
context, original expiry and current server permissions before reading a fresh
snapshot; browsing performs no memory writes. The existing command factories,
classic admin controls and Entry Point preservation remain registered.

The fresh WDBX-required strict gate passed on tested source `ff5d594` with
1,183 Rust tests, zero failures and four intentional live ignores, warnings-denied
all-target Clippy and locked release builds. Source implementation and review
are complete. Canonical integration, push, exact-head hosted CI and live Discord
UX acceptance remain separate pending layers. Final delivered identity/CI/cleanup
proof belongs in canonical `.superpowers/completion-20260906/delivery.json`.
The historical statuses below do not establish deployment of these additions.

## Already shipped

| Area | Status | Where |
|---|---|---|
| Gateway + intents, listening presence | Live | `src/main.rs` |
| Poise slash commands (chat input) | Live | `src/commands_*`, `src/main.rs` |
| Voice consent gate (`/voice join consent:true`), wake names | Live | `src/commands_voice/` |
| Entry Point `launch` preserved on global register | Live | `src/main.rs` |
| Activity client (`ready()` + channel/guild UI + participants) | Live on Pages after merge | `activity/` → `https://donaldfilimon.github.io/abbey-bot/activity/` |
| Stream (`1<<9`) + Use Embedded Activities (`1<<39`) overwrite gap-fill; `/voice` fail-closed on missing bits | Live | `src/commands_voice/discord.rs`, `docs/activities.md` |
| `/pending list` classic Confirm/Dismiss buttons + collector | Live (code) | `src/commands_brain.rs` (P1; Components V2 blocked on crates) |
| `/admin show` + dashboard classic page String Select | Live (code) | `src/commands_brain.rs`, `src/admin_dashboard.rs` (P1 leftover) |
| Context menus: "Abbey: profile" (USER), "Ask Abbey" (MESSAGE) | Live (code) | `src/commands.rs`, registered in `src/main.rs` |
| Configurable wake names (`ABBEY_VOICE_WAKE_WORDS`) | Live (code) | `src/voice.rs`; falls back to the default list rather than leaving Abbey unaddressable |

Shipped means code + docs exist and Gate can pass. Live rocket iframe still
needs Donald’s Portal URL mapping (P0). Live spoken turns still need a human
in Office Hours with consent (see `MLAI-LIVE-ACCEPTANCE.md`).

---

## Current gap inventory (2026-09-04)

Relative to Discord Application + Interactions surface. Source: live checkout + docs.

| Surface | Status | Notes |
|---|---|---|
| Chat-input `/` commands | **Done** | Full set in `src/main.rs` (persona, voice, admin, memory, vision, …) |
| Context menus (USER / MESSAGE) | **Done** | `commands::profile_context_menu` (USER), `commands::ask_context_menu` (MESSAGE) |
| Buttons / selects / modals | **Partial** | `/pending list` Confirm/Dismiss buttons; `/admin show` + dashboard page String Select; slash confirm/dismiss remain |
| Components V2 layouts | **Blocked (crates)** | serenity 0.12.5 / poise 0.6.2 expose classic Action Rows only — no Container/Section/IS_COMPONENTS_V2 builders |
| Interaction HTTP endpoint | **N/A (by design)** | Gateway + poise defer/follow-up only |
| Entry Point `launch` preserve | **Done** | `register_globally_keeping_entry_point` |
| Activity `ready()` shell | **Done** | Pages `/activity/`; Portal map still P0 |
| Activity authorize / channel / participants / setActivity | **Partial (P2 slice)** | Pre-auth channel/guild + participants (no OAuth); authorize/setActivity wait on secret host |
| Stream + Use Embedded Activities gate | **Done** | Fail-closed on `/voice` |
| Privileged intents (PRESENCES, members) | **Partial** | `non_privileged` + `GUILD_VOICE_STATES`; optional `MESSAGE_CONTENT` |
| Webhook create API | **Guide only, by decision** | `/webhook` emits setup steps. A bot-minted URL is a credential the bot then holds; that is a deliberate refusal (`commands::webhook` doc comment), not an unimplemented gap. |
| Polls / Stage / SKUs | **Out / deferred** | See Later + Out of scope |
| Bot Go Live | **Never** | No Discord API |

---

## P0 — Portal URL map (operator-gated)

Bot tokens cannot set Activity URL mappings. Donald clicks once in the
[Abbey application](https://discord.com/developers/applications/1147940171099152464):

1. **Activities → URL Mappings**
   - PREFIX: `/`
   - TARGET: `donaldfilimon.github.io/abbey-bot/activity` (no `https://`, directory not `index.html`)
2. **Activities → Settings / Supported Platforms:** Desktop + Web (Mobile optional).
3. Confirm Entry Point `launch` still present.
4. Join Office Hours → rocket → Abbey. First load after mapping may cache for ~1 min.

**Automatable (local only):** `deploy/check-activity-url-map.py` encodes the
canonical PREFIX/TARGET contract, verifies `activity/index.html` + `activity/app.js`
(+ Pages markdown inventory for `activity/README.md`), and prints PASS/FAIL.
Tests: `deploy/test-check-activity-url-map.py` (also wired into `check.sh` /
`check.ps1`). Checker PASS does **not** mean Portal is configured — API cannot
read URL mappings.

**Done when:** Donald confirms the discordsays iframe loads the Abbey shell and
shows ready status inside Discord (not only in a plain browser tab on the Pages
URL). Status remains **operator-gated** until that iframe confirmation.

Detail: [`activities.md`](activities.md) § Remaining Developer Portal clicks and
§ Canonical mapping contract.

---

## P1 — Components V2 / interactions UX

Improve interaction surfaces that already ride the bot token (no OAuth secret).

- Adopt Discord **Components V2** layouts where poise/serenity support them
  (containers, sections, media galleries, etc.) for admin, help, and voice
  status replies — progressive enhancement, not a rewrite of every command.
- Tighten interaction acknowledge / follow-up / ephemeral patterns so long
  LLM or voice ops never hit the 3s token timeout.
- Modal + button/select flows for consent and settings that today are
  slash-option only, when that UX is clearer than another subcommand.
- Keep signature verification and fail-closed permission checks; no new
  privileged intents without an explicit ops decision.

### Acceptance note (2026-09-03)

- **Components V2 crate blocker:** pinned `serenity 0.12.5` + `poise 0.6.2`
  (Dependabot ignores majors until a coordinated pair lands) only ship classic
  Action Row builders (`CreateButton` / `CreateSelectMenu` / modals). There is
  no `IS_COMPONENTS_V2` / Container / Section API in these crates yet — do not
  bump majors casually.
- **Shipped path:** `/pending list` defers ephemeral, then attaches Confirm /
  Dismiss buttons (one Action Row per proposal, max 5). Clicks are scoped to
  the invoker via `ComponentInteractionCollector` (requires serenity
  `collector` feature), re-check memory authorization, and
  `UpdateMessage` so the 3s interaction ack is never missed. Slash
  `/pending confirm|dismiss` + autocomplete remain for overflow / power users.
- **Defer policy:** every ordinary slash command calls `defer` /
  `defer_ephemeral` before any guard, network, or other command path. The sole
  exception is `/voice leave`: it authorizes synchronously from the interaction
  payload, closes the media gate before its first await, and then polls its
  acknowledgement concurrently with transition-lock acquisition and teardown;
  its configuration and authorization guard paths answer directly inside the
  three-second window. Component button handlers acknowledge via
  `UpdateMessage` (or ephemeral `Message` on auth failure).
- **Not in this slice:** `/voice` consent button (keep explicit
  `consent:true` slash gate + public notice), Components V2 layouts, modals —
  follow-ups once Components V2 lands or classic UX still wins.

- **Design (2026-09-08):** classic voice Action Row UX locked in [`docs/superpowers/specs/2026-09-08-voice-classic-ux-design.md`](superpowers/specs/2026-09-08-voice-classic-ux-design.md) (A status → B leave confirm → C play). Consent remains slash-only.

### Acceptance note (2026-09-08) — classic voice Action Row UX (A/B/C)

- **Source + live:** classic voice UX Phases A→B→C are on `main` and the live binary was restarted 2026-09-08. Human-witnessed Discord acceptance remains open (do not treat restart alone as acceptance).
- **Shipped path:** after successful `/voice join|resume consent:true`, Abbey follows up with a classic Status Action Row (`Refresh` + `Leave`). Leave swaps to Confirm/Cancel without teardown; Confirm runs the existing leave path once. When a playable voice session exists, Play/Stop/Skip appear on the status surface; Play reuses the empty-query `/voice play` path for the selected player (default Spotify).
- **Protocol:** short custom ids `abbey:v:{sid}:{act}` with acts `ref|leave|ok|cancel|play|stop|skip`; guild/user/channel/expiry live in the server-side session store under `ABBEY_DATA_DIR` (`voice-ux-sessions.json`), not in the custom id.
- **Fail-closed:** wrong user/guild/expired/missing/malformed → ephemeral deny and zero voice mutations; authorized clicks ack with `UpdateMessage` (or Defer/Acknowledge+edit for leave teardown and Play).
- **Unchanged:** `consent:true` stays slash-only (never a consent button; buttons never start STT); no Components V2 / bot Go Live in this slice. Portal P0 remains Donald-owned.


### Acceptance note (2026-09-07) — `/admin show` classic page select

- **Shipped path:** `/admin show` stays Manage-Server + ephemeral, still prints
  `guild::render_settings`, and now attaches a classic String Select
  (`abbey:admin:v1:…:page-select`) whose option values are navigable
  `View(...)` slugs. Choosing a page reuses the existing admin dashboard
  dispatcher (ack → owner/guild/expiry bind → fresh Manage Server check →
  render + page actions). Dashboard navigation itself uses the same select
  (frees the former four-button nav row). Action rows remain classic buttons.
- **Fail-closed:** non-nav option values, wrong component kinds, foreign
  owner/guild, expiry, and missing Manage Server reject without mutation.
- **Unchanged:** Entry Point `launch` preservation; `/voice` `consent:true`
  slash gate + public notice (no consent button replacement); no secrets in
  git; never bot Go Live. Components V2 still crate-blocked on serenity
  0.12.5 / poise 0.6.2.

**Done when:** at least one high-traffic command path uses Components V2 (or
documents why serenity/poise cannot yet), and interaction timeout / ephemeral
policy is consistent in code + a short acceptance note.

**Status:** met via classic components (`/pending list` buttons + `/admin show`
/ dashboard page select) + crate-blocker note (this section).

---

## P2 — Real Embedded Activity

Graduate `activity/` from `ready()` shell to a useful in-VC client via the
Embedded App SDK (source of truth: `activity/src/main.js` +
`@discord/embedded-app-sdk`).

Suggested order:

1. **authorize()** — OAuth2 code exchange through a **server-side** token
   endpoint (operator secret in env only). Scopes minimal
   (`applications.commands` / `identify` / activity-related as required).
2. **Get channel + guild context** from the SDK; show Office Hours / current
   VC name in the UI.
3. **Subscribe to participants** (join/leave/speaking proxies the SDK exposes);
   mirror consent / wake state at a glance — do not start STT from the iframe
   without the same consent gate the bot enforces.
4. **setActivity** (rich presence / activity instance metadata) so the rocket
   shelf and invite surfaces show Abbey’s current mode (listening / idle /
   waiting for consent) without inventing a bot Go Live stream.
5. Optional: same-origin API routes under the mapped PREFIX if the iframe needs
   backend; extra hosts require additional Portal PREFIX mappings (CSP).

### Acceptance note (2026-09-04) — P2 progress

GitHub Pages cannot hold `DISCORD_CLIENT_SECRET`, so token exchange is
**architected + stubbed**, not hosted in this PR. Rocket iframe still needs
Donald’s Portal URL map (P0).

- **Working UI (no OAuth):** `activity/index.html` + `activity/app.js` after
  `ready()` show status, truthful mode (`idle` / `waiting`), channel + guild
  from iframe query params (Office Hours / MLAI Community labels), and
  **participant count** via `GET_ACTIVITY_INSTANCE_CONNECTED_PARTICIPANTS` +
  `ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE` (SDK: no scopes). Brand Abbey / IWL only.
  Never Go Live.
- **OAuth hooks:** client probes `GET /api/token/health` (or `?oauth=1`) before
  `authorize` → `POST …/api/token` → `authenticate`. Scopes:
  `identify`, `guilds`, `applications.commands`, `rpc.activities.write`.
  No modal spam when the secret host is absent.
- **After auth:** `getChannel` for live channel name; `setActivity` with idle /
  waiting copy (not stream).
- **Server design:** `activity/server/token-exchange.example.mjs` exchanges the
  code with Discord using env-only `DISCORD_CLIENT_SECRET`. Operator maps a
  PREFIX or same-origin host for `/.proxy/api/token`.
- **SDK source:** `activity/src/main.js` mirrors the same flow for an optional
  bundled rebuild (`package.json` `build` script). Pages ships `app.js` as-is.
- **Still open for P2 “done when”:** live secret endpoint + Portal mapping so
  authenticated channel name + truthful `setActivity` run inside Discord.

**Done when:** launching Abbey from the rocket shows authenticated channel +
participant context and a truthful `setActivity` state; secrets remain out of git.

**Status:** partial — pre-auth UI + participants subscribe + authorize/token
architecture landed; full auth / `setActivity` blocked on operator-hosted
secret endpoint (not Pages).

---

## P3 — User-install, role connections, forum helpers

- **User-installable app** (user-scoped installation) alongside guild install:
  commands that make sense in DMs / user context without requiring Member role
  in MLAI Community. Respect existing onboarding-safe guild locks.
- **Linked Roles / role connections** metadata endpoint so Discord can gate a
  role on verified external state (e.g. Residents holder) without manual grants
  where product policy allows.
- **Context menus** — **shipped 2026-09-04.** "Abbey: profile" (USER) renders the
  same summary as `/whois`; "Ask Abbey" (MESSAGE) routes a message's own text
  through the shared `answer_question` path `/persona ask` uses, so identical
  text cannot get two different answers. Both are ephemeral: a right-click is a
  private lookup, not an announcement about the person or the author. The
  message menu reports empty resolved content plainly instead of answering a
  blank question, and points at `/see` for images.
- **Forum / channel helpers:** **shipped 2026-09-07.** `/forum draft` suggests
  tags + previews Question/Bug/Build/General first-post templates (IWL brand,
  no Quesar). `/forum post` creates a thread in `#help` (or a chosen forum) via
  `CreateForumPost`, applies available tags (explicit list or auto-suggest), and
  mentions the requester. `/forum perms` (Manage Server) snapshots the bot
  member overwrite then gap-fills only missing required bits
  (`View Channel`, `Send Messages`, `Send Messages in Threads`,
  `Create Public Threads`, `Embed Links`, `Attach Files`, `Read Message History`)
  — never wipes unrelated allow/deny or other overwrite targets. Classic slash
  only; Components V2 still crate-blocked.

**Done when:** user-install path documented + gated commands registered;
  role-connection endpoint (if adopted) behind operator env; forum helpers have
  a live checklist entry without breaking Onboarding constraints.

### Acceptance note (2026-09-07) — forum helpers

- Code: `src/forum.rs` (pure suggest/template/gap-fill) + `src/commands_forum.rs`
  (`/forum draft|post|perms`), catalog leaves under Server, Entry Point `launch`
  preservation unchanged.
- **Still open in P3:** user-install (Portal Installation Contexts) and Linked
  Roles / role connections — skipped until Donald enables Portal + decides
  per-command consent. OAuth secret host remains P2.

### Operator prerequisite for user-install (read before writing the code)

User-install is **not** a pure code change. Sending `integration_types` with
`USER_INSTALL` in the global bulk overwrite is rejected unless the application
has User Install enabled under **Installation → Installation Contexts** in the
Developer Portal. The bulk overwrite runs in the `ready` callback
(`register_globally_keeping_entry_point`), so a rejected overwrite breaks
command registration for the running service — the same failure mode the Entry
Point preservation exists to avoid. Any user-install slice must therefore be
**off by default and behind an operator env flag**, and it must decide, per
command, what a user-installed invocation is allowed to touch: `persona ask`
writes a channel-scoped transcript, so invoking it inside a guild Abbey was
never installed in would create memory for a server that never consented.
`persona route` and `server` are pure and carry no such question.

**Status:** forum helpers shipped (this section). User-install and Linked Roles
remain unstarted; crate support confirmed (`poise::Command::install_context`
/ `interaction_context` exist in the pinned 0.6.2), Portal click and the
per-command consent decision are not.

---

## Later — Monetization (explicitly deferred)

- SKU / entitlements / premium app subscriptions only after P0–P2 are solid and
  product asks for them.
- Never store Stripe/Discord payment secrets in the repo.
- Monetization must not weaken consent, privacy, or brand boundaries
  (Abbey / Intelligence Without Limits — not Quesar on companion surfaces).

---

## Out of scope / never

| Idea | Why not |
|---|---|
| Bot Go Live / screenshare | No Discord API for bots; do not fake it |
| Committing Client Secret or bot token | Operator env only |
| Bulk-overwrite global commands that drop `launch` | Disables Activity |
| Wiping channel overwrites / roles to “fix” perms | Gap-fill only |
| Dual brand taglines / Quesar on Abbey Activity UI | Brand freeze |

---

## Related

- [`activities.md`](activities.md) — rocket launch, Portal mapping, permission bits
- [`MLAI-LIVE-ACCEPTANCE.md`](MLAI-LIVE-ACCEPTANCE.md) — live voice + MLX evidence checklist
- [`live-test-protocol.md`](live-test-protocol.md) — Guild A/B privacy boundary
- `activity/` — static Embedded App shell
- `src/main.rs` — Entry Point–preserving global registration
- `src/commands_voice/discord.rs` — Stream + Use Embedded Activities fail-closed gate
