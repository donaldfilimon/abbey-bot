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

Static page: `activity/` (shows **Abbey**, calls Embedded App SDK `ready()`).
No OAuth client secret is invented or committed. Application ID
`1147940171099152464` is public.

After this lands on `main`, GitHub Pages (already live from `main` `/`) serves:

`https://donaldfilimon.github.io/abbey-bot/activity/`

Discord does **not** load that URL directly. The Activity iframe is
`https://1147940171099152464.discordsays.com/` and only reaches the page after
a Developer Portal URL mapping.

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
   need one. If you later add `authorize()`, put the secret only in operator
   env, never in the repo.
7. Join Office Hours → rocket → Abbey. First load after mapping can take a
   minute while Pages + the proxy cache.

Optional later mappings (only if the iframe needs extra hosts): add a PREFIX
for each host; Discord CSP blocks unmapped origins.

## Related

- Voice permission gate: `src/commands_voice/discord.rs`
- Entry Point preserve: `src/main.rs` → `register_globally_keeping_entry_point`
- Live acceptance: `docs/MLAI-LIVE-ACCEPTANCE.md`
