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

