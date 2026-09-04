# Discord Activities (in-voice apps)

Abbey is an Activities-enabled app. The auto-created Entry Point command
`launch` (type `PRIMARY_ENTRY_POINT`, handler `2` = Discord Launch Activity)
is how members start Abbey from the rocket / App Launcher in a voice channel.
Global command registration **must** keep that Entry Point
(`register_globally_keeping_entry_point` in `src/main.rs`). Never bulk-overwrite
commands without it. Deleting `launch` disables the Activity.

Conversational voice is still consent-gated: `/voice join consent:true` from an
in-channel manager after a wake name (Abbey / Abby / Aviva / Abi). AUTOJOIN is
muted/self-deafened presence only.

## What Discord allows vs does not

| Capability | Humans | Abbey bot |
|---|---|---|
| Connect / Speak | Yes, with channel overwrites | Yes (Songbird + DAVE) |
| Stream (`1 << 9`, Go Live / screenshare) | Yes when allowed | **No.** Discord has no bot Go Live API. Granting Stream does not make Abbey start a stream. |
| Use Embedded Activities (`1 << 39`) | Yes | Lets the app Activity launch in that VC |
| Entry Point `launch` | Rocket / App Launcher | Already live (type 4, handler 2). Preserve it. |

Do **not** fake screenshare. In-voice visuals go through Activities.

## Live overwrite snapshot (2026-09-03 ~18:55–19:00 ET)

Guild `1275617641620443146`. Office Hours `1495755277859815595` plus all other
type-2 VOICE channels (Community Lounge, Pair Programming, Gaming, Town Hall,
Chill / AFK):

- **PUT only to add missing allows** (no wipe) for Abbey bot member
  `1147940171099152464`, Abbey's roles, and Member `1545150308244521044`.
- Bits ensured: View, Send, Connect, Speak, Stream (`1<<9`), Use Embedded
  Activities (`1<<39`).
- Office Hours: roles mostly needed Send Messages; bot member overwrite gained
  Stream + Use Embedded Activities. Post-verify: all targets
  Connect+Speak+Stream+Use Embedded Activities = true.
- Other voice channels: similar gap-fill (48 successful `204` PUTs total).
- `#bot-ops` message `1545205023808430150` recorded the mutation.

`/voice` join/supervision fail-closed requires:
`View Channel, Send Messages, Connect, Speak, Stream, Use Embedded Activities`.

## Launch Abbey from a voice channel

1. Join Office Hours (or another VOICE channel with Use Embedded Activities).
2. Enable Discord Developer Mode if the rocket shelf hides unpublished apps:
   User Settings → App Settings → Advanced → Developer Mode.
3. Click the **rocket** on the RTC panel / Center Control Tray.
4. Launch **Abbey**. That invokes Entry Point `launch`. Do not look for a bot
   Go Live button.

Spoken turns still need `/voice join consent:true` and a wake word.

## Activity web client (this repo)

Static page: `activity/` (shows **Abbey**, `ready()`, channel/guild UI, participant count).
No OAuth client secret is invented or committed. Application ID
`1147940171099152464` is public.

After this lands on `main`, GitHub Pages (already live from `main` `/`) serves:

`https://donaldfilimon.github.io/abbey-bot/activity/`

Discord does **not** load that URL directly. The Activity iframe is
`https://1147940171099152464.discordsays.com/` and only reaches the page after
a Developer Portal URL mapping.

### P2 — Real Embedded Activity (client + token architecture)

Shipped in `activity/` toward roadmap P2 (not full OAuth until a secret host exists):

1. **Pre-auth context** — after `ready()`, the UI shows voice **channel** and
   **guild** from Discord-injected query params (`channel_id`, `guild_id`).
   Known MLAI labels (Office Hours / MLAI Community) are display-only.
2. **Truthful mode** — local UI mode is `idle` or `waiting` only. Never claims
   Go Live / screenshare.
3. **Participants (no OAuth)** — after `ready()`, the client calls
   `GET_ACTIVITY_INSTANCE_CONNECTED_PARTICIPANTS` /
   `getInstanceConnectedParticipants` and subscribes to
   `ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE`. Discord documents **no scopes** for
   these. The UI shows participant **count** (+ names when present).
4. **authorize() scaffolding** — client probes `GET /api/token/health` →
   `{ ok: true }` (or Activity URL `?oauth=1`) before calling `authorize` +
   server code exchange + `authenticate`. Without that host, it stays on
   pre-auth context and does **not** pop OAuth.
5. **After auth** (when exchange is mapped) — `getChannel` for the live voice
   channel **name** (needs `guilds`), and `setActivity` with truthful idle /
   waiting copy under Abbey / IWL branding (needs `rpc.activities.write`).
   Never invents Go Live.
6. **Server secret** — `DISCORD_CLIENT_SECRET` stays in operator env. Example
   route: `activity/server/token-exchange.example.mjs` (not on Pages).

**OAuth scopes requested when the exchange host is live:**
`identify`, `guilds`, `applications.commands`, `rpc.activities.write`.

**Operator env (when you host the exchange):**

| Var | Where | Notes |
|---|---|---|
| `DISCORD_CLIENT_ID` | exchange host | Defaults to public app id `1147940171099152464` |
| `DISCORD_CLIENT_SECRET` | exchange host only | Portal → OAuth2; **never** git / Pages / chat logs |
| Redirect URI | Portal OAuth2 | Required by Discord; SDK returns users to the Activity |

**Hosting options for the exchange (pick one):**

- Same-origin: serve `activity/` static **and** `POST /api/token` from one host,
  then point Portal TARGET at that host, **or**
- Split: keep Pages for static; add a Portal PREFIX mapping so
  `/.proxy/api/token` reaches the secret host (Discord CSP blocks unmapped
  origins).

Preferred SDK rebuild source: `activity/src/main.js`. Pages continues to ship
`activity/app.js` without a bundler.

## Remaining Developer Portal clicks (Donald)

Bot tokens cannot set URL mappings. Donald must click these:

1. Open [Abbey application](https://discord.com/developers/applications/1147940171099152464).
2. Left sidebar → **Activities** → **URL Mappings**.
3. Add/save exactly:
   - **PREFIX:** `/`
   - **TARGET:** `donaldfilimon.github.io/abbey-bot/activity`
   - No `https://`. Target must be a directory, not `index.html`.
4. **Activities** → **Settings** / **Supported Platforms**: enable **Desktop**
   and **Web** (Mobile optional). The rocket shelf hides the app on platforms
   that are unchecked.
5. Confirm **Entry Point** `launch` is still present. Do not delete it.
6. Do **not** create or paste a Client Secret into git. `ready()` does not
   need one. For P2 `authorize()`, put the secret only in operator env on the
   token-exchange host (see § P2 above), never in the repo.
7. Join Office Hours → rocket → Abbey. First load after mapping can take a
   minute while Pages + the proxy cache.

Optional later mappings (only if the iframe needs extra hosts): add a PREFIX
for each host; Discord CSP blocks unmapped origins.

## Related

- Voice permission gate: `src/commands_voice/discord.rs`
- Entry Point preserve: `src/main.rs` → `register_globally_keeping_entry_point`
- Live acceptance: `docs/MLAI-LIVE-ACCEPTANCE.md`
- Application API roadmap: [`docs/discord-application-api-roadmap.md`](discord-application-api-roadmap.md)
