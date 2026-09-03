# MLAI Community — live acceptance checklist

Guild: **MLAI Community**. Voice bind: **Office Hours** (`ABBEY_VOICE_MODE=local`).
Host: official launchd `com.donaldfilimon.abbey-bot`.

Do **not** paste tokens here. Retain only pass/fail + timestamps.

## A — Bot / guild (already green if launchd up)

- [ ] `pgrep -lf abbey-bot` shows the launchd binary
- [ ] `/stats` answers in a text channel
- [ ] Guild settings: act on, learning/vision/voice on (stores)
- [ ] Slash commands resolve for this guild

## B — Discord structure (onboarding-safe)

- [x] `@everyone` view/read-history at role level
- [x] `Member` hoisted; Residents holders also have Member
- [x] `#welcome` remains everyone read+send (Discord Onboarding requirement)
- [x] `#rules-guidelines` / announcements view-oriented; STAFF locked from @everyone
- [x] `#help` forum under COMMUNITY; AI LAB topics set

## C — Consented foreground voice (`docs/live-test-protocol.md` §4)

Requires **everyone present** in Office Hours + owner/admin.

1. Owner/admin: `/voice verify start`
2. Manager in channel: `/voice join consent:true` (after public notice + unanimous consent)
3. Observe coarse voice states via `inspect_status` / `/voice status`: presence → awaiting-consent → active
4. Audible wake/reply (human confirms)
5. Barge-in observed
6. New participant or unattested → pause; `/voice resume consent:true` after renewed consent
7. `/voice leave` → presence cleared
8. `/voice verify report` → `observed: 8/8`

**Automate:** env bind, process, channel topics, bot-ops notes.  
**Humans only:** consent attestation, audible confirm, being in VC.

## D — MLX (fail closed)

- [ ] `com.donaldfilimon.abbey-mlx-audio` launchd loaded (currently **not**)
- [ ] `com.donaldfilimon.abbey-mlx-vlm` launchd loaded (currently **not**)
- [ ] Reasoning / tool-calling / vision interfaces verified for the exact revision
- [ ] No claim of MLX Gemma multimodal/tools until the above is evidenced

Until D passes, keep OpenAI-compatible / local loopback endpoints as the documented path.

## E — Residual learning-loop

- [ ] Observe one `OverBudget` refusal after exhausting hourly unsolicited budget (default 6/h)
