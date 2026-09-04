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

## Already shipped

| Area | Status | Where |
|---|---|---|
| Gateway + intents, listening presence | Live | `src/main.rs` |
| Poise slash commands (chat input) | Live | `src/commands_*`, `src/main.rs` |
| Voice consent gate (`/voice join consent:true`), wake names | Live | `src/commands_voice/` |
| Entry Point `launch` preserved on global register | Live | `src/main.rs` |
| Activity shell (`ready()` handshake only) | Live on Pages | `activity/` → `https://donaldfilimon.github.io/abbey-bot/activity/` |
| Stream (`1<<9`) + Use Embedded Activities (`1<<39`) overwrite gap-fill; `/voice` fail-closed on missing bits | Live | `src/commands_voice/discord.rs`, `docs/activities.md` |
| `/pending list` classic Confirm/Dismiss buttons + collector | Live (code) | `src/commands_brain.rs` (P1; Components V2 blocked on crates) |

Shipped means code + docs exist and Gate can pass. Live rocket iframe still
needs Donald’s Portal URL mapping (P0). Live spoken turns still need a human
in Office Hours with consent (see `MLAI-LIVE-ACCEPTANCE.md`).

---

## Current gap inventory (2026-09-04)

Relative to Discord Application + Interactions surface. Source: live checkout + docs.

| Surface | Status | Notes |
|---|---|---|
| Chat-input `/` commands | **Done** | Full set in `src/main.rs` (persona, voice, admin, memory, vision, …) |
| Context menus (USER / MESSAGE) | **Missing** | No `context_menu` in `src/` |
| Buttons / selects / modals | **Partial** | `/pending list` Confirm/Dismiss Action Rows (classic); slash confirm/dismiss remain |
| Components V2 layouts | **Blocked (crates)** | serenity 0.12.5 / poise 0.6.2 expose classic Action Rows only — no Container/Section/IS_COMPONENTS_V2 builders |
| Interaction HTTP endpoint | **N/A (by design)** | Gateway + poise defer/follow-up only |
| Entry Point `launch` preserve | **Done** | `register_globally_keeping_entry_point` |
| Activity `ready()` shell | **Done** | Pages `/activity/`; Portal map still P0 |
| Activity authorize / channel / participants / setActivity | **Missing** | P2 |
| Stream + Use Embedded Activities gate | **Done** | Fail-closed on `/voice` |
| Privileged intents (PRESENCES, members) | **Partial** | `non_privileged` + `GUILD_VOICE_STATES`; optional `MESSAGE_CONTENT` |
| Webhook create API | **Guide only** | `/webhook` docs, no create-webhook call |
| Polls / Stage / SKUs | **Out / deferred** | See Later + Out of scope |
| Bot Go Live | **Never** | No Discord API |

---

## P0 — Portal URL map (operator, no code)

Bot tokens cannot set Activity URL mappings. Donald clicks once in the
[Abbey application](https://discord.com/developers/applications/1147940171099152464):

1. **Activities → URL Mappings**
   - PREFIX: `/`
   - TARGET: `donaldfilimon.github.io/abbey-bot/activity` (no `https://`, directory not `index.html`)
2. **Activities → Settings / Supported Platforms:** Desktop + Web (Mobile optional).
3. Confirm Entry Point `launch` still present.
4. Join Office Hours → rocket → Abbey. First load after mapping may cache for ~1 min.

**Done when:** iframe loads the Abbey shell and shows ready status inside Discord
(not only in a plain browser tab).

Detail: [`activities.md`](activities.md) § Remaining Developer Portal clicks.

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
- **Defer policy:** long ops (LLM, voice join/leave/status, memory mutate,
  admin mutate) call `defer` / `defer_ephemeral` before awaits; instant
  validation failures may `say` without defer. Component button handlers
  acknowledge via `UpdateMessage` (or ephemeral `Message` on auth failure).
- **Not in this slice:** `/voice` consent button (keep explicit
  `consent:true` slash gate + public notice), `/admin show` selects, modals,
  context menus — follow-ups once Components V2 lands or classic UX still wins.

**Done when:** at least one high-traffic command path uses Components V2 (or
documents why serenity/poise cannot yet), and interaction timeout / ephemeral
policy is consistent in code + a short acceptance note.

**Status:** met via classic components + crate-blocker note (this section).

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

**Done when:** launching Abbey from the rocket shows authenticated channel +
participant context and a truthful `setActivity` state; secrets remain out of git.

---

## P3 — User-install, role connections, forum helpers

- **User-installable app** (user-scoped installation) alongside guild install:
  commands that make sense in DMs / user context without requiring Member role
  in MLAI Community. Respect existing onboarding-safe guild locks.
- **Linked Roles / role connections** metadata endpoint so Discord can gate a
  role on verified external state (e.g. Residents holder) without manual grants
  where product policy allows.
- **Forum / channel helpers:** thread create, tag suggest, first-post templates
  for `#help` and similar — API-first, gap-fill permissions, no wipe of existing
  overwrites (same discipline as voice overwrite work).

**Done when:** user-install path documented + gated commands registered;
  role-connection endpoint (if adopted) behind operator env; forum helpers have
  a live checklist entry without breaking Onboarding constraints.

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
