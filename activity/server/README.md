# Activity token exchange (example)

See `token-exchange.example.mjs`. This is architecture + a runnable stub for
operators — not a production deploy and not wired into GitHub Pages.

```bash
export DISCORD_CLIENT_SECRET='…'   # Portal → OAuth2; never commit
# optional: export DISCORD_CLIENT_ID=1147940171099152464
node activity/server/token-exchange.example.mjs
```

Then either:

1. Serve Activity static assets from the same host as `/api/token`, **or**
2. Add a Developer Portal URL mapping PREFIX so `/.proxy/api/token` reaches
   this host (Discord CSP blocks unmapped origins).

Client probes `GET /api/token/health` → `{ ok: true }` before calling
`authorize()`. Override with Activity URL `?oauth=1` once mapped.
