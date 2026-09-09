# Monetization Portal checklist — Abbey Guild Pro (phase 1)

**App:** [1147940171099152464](https://discord.com/developers/applications/1147940171099152464)  
**SKU (reuse, do not recreate):** `1293228939929452574` (type-5) — rename Portal → **Abbey Guild Pro**  
**Spec:** [`docs/superpowers/specs/2026-09-08-monetization-guild-pro-quesar-design.md`](../superpowers/specs/2026-09-08-monetization-guild-pro-quesar-design.md)  
**Plan:** [`docs/superpowers/plans/2026-09-08-monetization-guild-pro-quesar.md`](../superpowers/plans/2026-09-08-monetization-guild-pro-quesar.md)

Phase 1 success = eligibility documented + SKU clarified + entitlements drafted + Quesar pilot tracked separately. **No live Discord charges. Do not publish the storefront.**

---

## API-confirmed (2026-09-08) — checked

- [x] `monetization_state=ENABLED`
- [x] `is_monetized=true`
- [x] Team-owned application
- [x] Verified application
- [x] Existing Subscription Group + type-5 subscription SKU id `1293228939929452574` present (current joke name is not the customer-facing lock)
- [x] `storefront_available=false` (must remain false through phase 1)

---

## Donald Portal — Premium Apps / SKU (unchecked)

- [ ] US/UK/EU payout / region requirements confirmed in Portal as needed for a future publish
- [ ] Owner onboarding complete for storefront publish (still do **not** publish in phase 1)
- [ ] Stripe Connect payout linked in Discord Premium Apps flow (Discord-side only; never Stripe secrets in abbey-bot)
- [ ] Monetization Terms accepted
- [ ] Rename/clarify SKU `1293228939929452574` → **Abbey Guild Pro** (or shorter **Abbey Pro** if Portal length prefers) — **no second SKU**
- [ ] Attach entitlements to that SKU only:
  - [ ] `voice_ux_pro` — classic Action Row voice controls already shipped
  - [ ] `activity_access` — Activity URL map path
  - [ ] `admin_workflow` — `/admin` Action Row / pending helpers already shipped
- [ ] **Do not publish** storefront (`storefront_available` stays false)
- [ ] Leave production `ABBEY_ENTITLEMENT_ENFORCE` unset / off (default-off; phase 1 must not deny users)

---

## Donald Portal — Activity URL map (unchecked; PASS≠Portal)

Canonical contract (also in [`docs/activities.md`](../activities.md) and P0 roadmap):

1. **Activities → URL Mappings**
   - PREFIX: `/`
   - TARGET: `donaldfilimon.github.io/abbey-bot/activity` (no `https://`, directory not `index.html`)
2. **Activities → Settings / Supported Platforms:** Desktop + Web (Mobile optional)
3. Confirm Entry Point `launch` still present
4. Join Office Hours → rocket → Abbey; confirm discordsays iframe (first load may cache ~1 min)

Checklist:

- [ ] PREFIX `/` mapped
- [ ] TARGET `donaldfilimon.github.io/abbey-bot/activity` mapped
- [ ] Desktop + Web enabled
- [ ] Entry Point `launch` still present
- [ ] Human rocket / iframe verify in Office Hours
- [ ] Treat `deploy/check-activity-url-map.py` PASS as local contract only — it does **not** prove Portal is configured

Do not sell or imply `activity_access` until the iframe path is confirmed.

---

## Billing / merge gate

- [ ] GitHub Actions billing unlocked so tip Gate can finish green
- [ ] Do **not** merge monetization docs/code stacks while tip Gate may be billing-locked
- [ ] Prefer pairing with design PR [#127](https://github.com/donaldfilimon/abbey-bot/pull/127) (or land design first)

---

## Billing / payout links (operator)

- Discord Developer Portal → application `1147940171099152464` → monetization / Premium Apps surfaces (rename SKU, entitlements, publish controls)
- Discord Premium Apps payout / Stripe Connect onboarding (Discord-hosted; no secrets into this repo)
- GitHub org/user billing for Actions (tip Gate) — separate from Discord payout

---

## Non-goals (phase 1)

- Live SKUs published / `storefront_available` flipped to true
- Live Discord charges or checkout flows exercised against users
- Inventing a second guild SKU alongside `1293228939929452574`
- Stripe SDK, payment webhooks, or payment secrets in abbey-bot
- Enabling `ABBEY_ENTITLEMENT_ENFORCE` in production
- Quesar (or any non-IWL brand) on Abbey Premium Apps / Activity / guild-subscription surfaces
- Invented metrics (QPS, latency SLAs, TAM, GPU burn claims)
- Bot Go Live / screenshare

Quesar pilot CTA is tracked in `donaldfilimon/mlai-website-app` (`src/content/quesar.ts`, `src/components/quesar-pages.tsx`) — never IWL.
