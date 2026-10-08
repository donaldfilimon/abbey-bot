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

### Canonical mapping contract (automatable)

Bot tokens still cannot write Portal URL mappings. What we *can* automate is the
local contract + static shell check:

| Field | Canonical value | Notes |
|---|---|---|
| PREFIX | `/` | Exact; no `/*`, no `/activity` |
| TARGET | `donaldfilimon.github.io/abbey-bot/activity` | No `https://`, no trailing slash, directory form (not `index.html`) |
| Pages browser URL | `https://donaldfilimon.github.io/abbey-bot/activity/` | Plain-browser smoke only |
| Discord iframe origin | `https://1147940171099152464.discordsays.com/` | Where rocket launch actually loads |

Run from repo root (read-only; exits non-zero on contract/asset fail):

```
python3 deploy/check-activity-url-map.py
```

Unit coverage: `python3 deploy/test-check-activity-url-map.py`.

**After Donald clicks Portal**, verify like this (operator-gated; no bot can
close this gate):

1. Plain browser: open the Pages URL above — confirms GitHub Pages is serving
   `activity/` (shell HTML/JS). The client status should read **Pages shell only**
   / **No Discord parent (plain browser)**. This alone does **not** mean Portal
   is mapped.
2. In Discord: Office Hours → rocket → Abbey. Confirm Abbey `ready()` inside the
   **discordsays iframe** (not only a browser tab). First load may take ~1 min
   while Pages + proxy cache. If the iframe shows **ready() timeout**, treat it
   as a wait/cache/map troubleshooting hint — not as automated Portal proof either
   way.
3. Do **not** mark P0 done from the checker PASS lines (including the local
   `activity client copy markers` check and the docs verify-markers check). The
   checker never sees Portal state; only Donald's iframe confirmation closes the
   operator gate.
4. Optional local re-check after docs/client edits (still not Portal proof):

```
python3 deploy/check-activity-url-map.py
python3 deploy/test-check-activity-url-map.py
```

   PASS lines only prove the PREFIX/TARGET contract text, tracked Pages assets,
   and truthful plain-browser / post-Portal verify copy stay in sync.

## Related

- Voice permission gate: `src/commands_voice/discord.rs`
- Entry Point preserve: `src/main.rs` → `register_globally_keeping_entry_point`
- Live acceptance: `docs/MLAI-LIVE-ACCEPTANCE.md`
- Application API roadmap: [`docs/discord-application-api-roadmap.md`](discord-application-api-roadmap.md)

## Engagement invitation readiness

Current source supports `/engage invite kind:activity`; invitations remain disabled
without an operator-maintained acceptance record and matching current public
host identity. The current local shared host and historical GitHub Pages solo
rehearsal do not qualify. Actual Discord iframe and two-participant acceptance of
the deployed shared version remain Blocked pending operator hosting/Portal work
and willing participants. No publication or Portal changes are authorized here.

The optional `ABBEY_ACTIVITY_READINESS_FILE` is an operator JSON file with exactly
`https_origin`, `deployed_digest`, `iframe_receipt`, `shared_receipt`, `verified_at`.
The origin is HTTPS root only, without credentials, path, query or fragment;
literal IP/local names are refused. The digest is 64 hexadecimal characters.
Each receipt string contains strict JSON with `digest`, `origin`, `record`,
`verified_at`, `participants`. Both receipts match the same origin, digest and
Unix verification timestamp. `record` names an actual durable `docs/...`
acceptance JSON record relative to the readiness file’s parent directory, without traversal; iframe acceptance needs at least one
participant and shared acceptance needs at least two. The operator record is a
trusted attestation, never generated by the bot or filled from a public probe.
Record the real Discord iframe launch, same room/case/votes for both participants,
disconnection/recovery and no solo-vote upload, with actual version identity.
Synthetic offline fixtures are not valid operator acceptance.

Acceptance expires after 30 days; future timestamps fail closed. Before proposing,
generating and sending an Activity invitation, the read-only adapter probes the
public HTTPS origin with no redirects or environment proxy, pins resolved public
IPv4 addresses and requires a successful response with
`x-abbey-deployed-digest` exactly matching the accepted digest. Hosts resolving to
private/special addresses or IPv6 are conservatively refused in this first
implementation. The current host lacks this identity header, so it cannot qualify
through an HTTP 200 alone. A health probe never creates iframe/shared receipts.
The command receipt binds the originally accepted origin and digest; a version
change rejects the pending candidate instead of advertising different software.
No actual environment settings were changed for source verification.

Voice invitation source verification uses existing configuration/provider guards
and synthetic transport tests only. Live consented audible reply, barge-in,
participant-change pause/resume, withdrawal and leave remain Blocked until a
willing participant is available. An invitation never counts as consent or starts
capture. No unsolicited witness outreach or live Discord test was performed.

The loader also opens each referenced acceptance JSON (maximum 16 KiB each) and
requires exact `digest`, `origin`, `verified_at`, `participants`, `discord_iframe`,
`shared_room_case_votes`, `disconnection_recovery`, `no_solo_vote_upload` fields.
Version/time/origin must match the readiness file; `discord_iframe` must be true.
For shared evidence, at least two participants and all three shared/recovery/solo
checks must be true. Missing files, malformed/incomplete records or mismatched
versions disable invitations. These bounded content-free operator attestations
must be backed by actual human acceptance, not synthetic fixtures or bot inference.

Root read-only observation on 2026-10-01 at 16:28:58 UTC: the historical GitHub
Pages Activity URL returned HTTP 200, without `x-abbey-deployed-digest`. This proves
static reachability only; shared API and genuine iframe/two-participant proof
remain unqualified. No probe or metadata here grants publication authority.


## 2026-10-02 Court v2 source candidate

The dependency-free Node host now exposes a content-bound release digest and
`GET /health`; it snapshots frontend assets at boot. Epoch/revision checks reject
stale writes and responses after room expiry/restart. Embedded Court requests
wait for a source/origin-checked Discord READY bridge; browser previews and
Discord instances have distinct room prefixes. Anonymous browser ballots remain
unauthenticated. No OAuth, bot credentials, chat or audio enters Court.

Source checks and local two-browser interaction establish local game behavior.
Public HTTPS hosting, Portal mapping, real iframe READY and two-participant
Discord acceptance remain open. The existing `com.donaldfilimon.abbey-court`
launch agent is a separate installed process; it was not restarted or republished
by these source tests. Publish only an immutable reviewed Node release after the
operator selects the HTTPS host/routing and authorizes that publication.


## 2026-10-03 Task 4 source preparation

Immutable four-asset packages, observed Node identity, finite proposal-only
ingress and an exact dry-run rollback receipt are now implemented. The commands
and actual package paths are in [Activity source preparation](../activity/README.md#immutable-source-package-preparation).
The candidate manifest is `7c5464c089240ac2be6fcb3a409540b266c392bb47db3ad8d2fd7467b7ab368b`;
its Court digest is `b81786c9d780ba01a7d1c757825ae4b898998c44358fc60f5e4a8c9dc1f93530`.
A separately copied prior installed-assets package is source inventory, not proof
of the running process or operational rollback readiness.

Current focused checks: 17 package tests, four DNS tests, six actual direct-child
probe tests and 20 Court Node tests passed. The packaged Court test used an
ephemeral loopback port, matched health/static/POST digests and observed process
termination. The source helper's oversized-label refusal and valid internal
hyphen/punycode support have attributable RED/GREEN evidence. The complete
combined source qualification is pending the stable strict gate; historical
source or local-host receipts do not qualify these additions.

Task 4 remains Partial: public HTTPS host selection, installed finite ingress,
public API, operator activation, Portal mapping, iframe READY, participant,
separate-instance, disconnect/restart and human witnesses remain open. This
preparation does not alter the existing invitation-readiness contract.


Task4 source qualification correction: the independently reviewed complete878-input
snapshot passed required-WDBX gate2 with actualexit0,2338Rust/366Python/Swift12+16.
[Source receipt](verification/2026-10-03-community-activity-source.md) preserves
actual package/Node tests and all public/operator/human gaps. This bounded source
result does not close the overall Task4 publication/iframe acceptance.
