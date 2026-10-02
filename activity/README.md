# Abbey Activity (Embedded App)

Static Discord Activity client. Hosted after merge at:

https://donaldfilimon.github.io/abbey-bot/activity/

## What this folder ships

| File | Role |
|---|---|
| `index.html` + `app.js` | Same-origin Pages client (ready + channel/guild/participants UI) |
| `src/main.js` | Preferred `@discord/embedded-app-sdk` rebuild source |
| `server/token-exchange.example.mjs` | Env-only OAuth code exchange stub (not on Pages) |

Brand: **Abbey / Intelligence Without Limits** only — never Quesar on this UI.
No bot Go Live. No Client Secret in git.

Pages serves `app.js` directly (no build step required on deploy). To rebuild
from the official SDK package: install deps in this folder and run the `build`
script (esbuild bundles `src/main.js` → `app.js`). Keep behavior in sync.

See `docs/activities.md` for rocket launch, Portal URL mapping, and P2 OAuth.

## Post-Portal verify (operator-gated)

Local checker PASS is not Portal proof. After Donald saves the URL mapping,
confirm Abbey `ready()` inside the **discordsays iframe** (rocket launch), not
only a plain browser tab. Full checklist: `docs/activities.md` § Remaining
Developer Portal clicks / After Donald clicks Portal.

## Plain-browser vs iframe

Opening the Pages URL above is a static smoke check. `app.js` labels that path
**Pages shell only** / **No Discord parent (plain browser)** and never claims
Portal URL mapping is done. Real `ready()` only happens inside the Discord
Activity iframe after rocket launch (and after Donald maps Portal).


## Bad Idea Court

The game is an anonymous shared vote on eight absurd proposals. Everyone in
one Discord Activity instance sees the same case and tally. Each browser has
a random juror identifier, can replace its vote, and can advance the case.
This is a casual party game, not authenticated voting; reloading can create
another juror. No bot token, OAuth secret, chat, audio, or member profile enters
the court server. Rooms expire after 30 minutes of inactivity, with at most
256 rooms and 100 votes per case. All state is in memory and resets on restart.

Run the dependency-free loopback host and its multiplayer regression check:

```sh
node activity/server/court.mjs
npm --prefix activity test
```

Open `http://127.0.0.1:8790/` and copy its room link into a second browser.
The full host serves only index.html, app.js, court.js and POST /court.
For a public host, put HTTPS in front of this loopback service; map the Activity
root to that host in the Developer Portal. The embedded client posts through
`/.proxy/court`. Existing GitHub Pages can serve a solo rehearsal, but cannot
host the shared in-memory court API. Connection failure visibly switches to
solo rehearsal and never claims synchronized votes. Portal changes and an
actual Discord iframe launch still need separate verification.

When a shared connection fails, solo rehearsal keeps the current case and clears
shared vote totals. Failed polls preserve your solo vote. Reconnection replaces
that rehearsal with the server's current case and votes; solo votes are never
submitted automatically. The first explicit action after a disconnection refreshes
shared state and asks you to choose again, so a solo case cannot receive a ballot
in a different shared case. Malformed responses also enter solo rehearsal.
The test command above runs both real HTTP multiplayer checks and deterministic
client disconnect, recovery, and malformed-response checks.


## Protocol v2 release candidate

Court now binds every room incarnation to a fresh epoch and orders snapshots by
revision. A stale vote or next-case request is refused, followed by a read;
mutations and solo rehearsal votes are never automatically replayed. Room expiry
and backend restart invalidate outstanding responses. Routine polls run every
2.5 seconds while visible and pause in hidden tabs. Vote and next controls retain
independent pending state.

Embedded clients wait for the existing Discord READY bridge, which verifies the
RPC parent and allowed origin. Discord-instance and browser-preview room names
use separate prefixes. These names isolate ordinary clients, not authenticated
members; the public API intentionally accepts anonymous client-chosen ballot keys.

The host snapshots its four release assets at startup and returns
`x-abbey-deployed-digest` on all responses. `GET /health` reports service
`abbey-court`, protocol `2`, ready status and the same digest. The SHA-256 input
is the ordered files `app.js`, `court.js`, `index.html`, `server/court.mjs`;
each contributes its relative filename, NUL, decimal byte length, NUL, then its
bytes. Publish immutable release directories and restart the process to change
versions; replacing live files does not replace the host's in-memory assets.

The host limits JSON requests to 1,024 bytes, rooms to 256, browser ballots to
100 per room, active connections to 256, and request/header/socket inactivity to
five seconds. Public ingress must preserve these limits and avoid access logs
containing room/player query values. Local health and two-browser tests do not
qualify public HTTPS, Discord iframe operation, or invitations.
