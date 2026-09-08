# Monetization system design — Guild Pro + Quesar pilot + Premium Apps eligibility

Date: 2026-09-08  
Status: draft for Donald review; **NO live charges in this phase**

## Goals

- Dual GTM: Discord server owners (Abbey guild subscription / Premium Apps) **AND** teams wanting private AI ops (Quesar pilot) in parallel
- Success this phase: eligibility checked, existing SKU clarified + entitlement draft, Quesar pilot page designed — **no live Discord charges yet**
- Brand freeze: IWL only Abbey/ABI/Abbey Bot; Quesar = private AI ops never IWL; no invented metrics

## Architecture

Three surfaces (as approved):

1. **Abbey Premium Apps / guild subscription** — Discord storefront + Entitlement API on the Abbey application
2. **Quesar pilot page** — private-ops offer surface (separate brand; never IWL)
3. **Ops gates** — billing unlock, Portal Activity URL map, eligibility/storefront publish steps (human Donald)

---

## Live Discord application facts (API-confirmed 2026-09-08)

Do **not** invent a brand-new Guild Pro SKU from scratch. Reuse and clarify what already exists.

| Field | Value |
|---|---|
| `monetization_state` | `ENABLED` |
| `is_monetized` | `true` |
| Ownership | Team-owned |
| Verification | Verified |
| Existing SKU | Subscription Group + **type-5** subscription |
| SKU name (current) | `Abbey Pro Max Ultimate Plus Ultra Extreme` |
| SKU id | `1293228939929452574` |
| `storefront_available` | `false` |

### SKU reuse / rename / clarify (phase 1 draft)

- **Keep** SKU id `1293228939929452574` (type-5 guild/subscription SKU already on the monetized app).
- **Rename/clarify in Portal (Donald)** to a shippable product name — working product name for docs and UX: **Abbey Guild Pro** (or shorter **Abbey Pro** if Portal length/storefront prefers). The current joke name is not the customer-facing lock; renaming is a Portal edit on the existing SKU, not a new SKU create.
- **Do not** publish the storefront in phase 1 (`storefront_available` stays false until Donald publishes).
- **Do not** attach live prices that charge users in phase 1; entitlement mapping and eligibility docs only.
- Subscription Group membership stays as Discord already configured it; phase 1 only documents entitlements mapped onto this SKU.

### Three entitlements mapped to EXISTING capabilities only

Map these entitlement keys (or Portal-equivalent feature flags) onto the **existing** type-5 SKU — no new product capabilities invented for monetization:

1. **`voice_ux_pro`** — classic Action Row voice controls already shipped (play/stop/skip/refresh polish). Design: `docs/superpowers/specs/2026-09-08-voice-classic-ux-design.md`.
2. **`activity_access`** — Activity URL map path (blocked until Portal PREFIX `/` TARGET `donaldfilimon.github.io/abbey-bot/activity`).
3. **`admin_workflow`** — `/admin` Action Row / pending helpers already shipped.

Enforcement later (phase 2+): Discord Entitlement API; fail-closed like `/admin`. No payment secrets in repo; Discord handles checkout when storefront is published.

---

## Premium Apps eligibility path

Checklist (operator / Portal; phase 1 documents status, does not charge):

- [x] Team-owned app
- [x] Verified
- [x] Monetization enabled (`monetization_state=ENABLED`, `is_monetized=true`)
- [ ] US/UK/EU payout / region requirements confirmed in Portal as needed for publish
- [ ] Owner onboarding complete for storefront publish
- [ ] Stripe Connect payout linked in Discord Premium Apps flow (Discord-side; never Stripe secrets in abbey-bot)
- [ ] Monetization Terms accepted
- [ ] SKU renamed/clarified from joke name → **Abbey Guild Pro** (or chosen short name)
- [ ] Entitlements `voice_ux_pro` / `activity_access` / `admin_workflow` attached to SKU `1293228939929452574`
- [ ] **Storefront publish** — Donald Portal step only; keep unpublished until phase 2 (`storefront_available` remains false in phase 1)
- [ ] Activity URL map PASS≠Portal resolved before selling `activity_access`

---

## Quesar pilot offer page

- Route suggestion: `/quesar` already private-ops mock — extend with pilot CTA only
- Copy: private AI operations; consent/audit; invite/org/gateway/KMS
- CTA: request pilot (form/email) — no QPS/latency/TAM claims
- Explicit non-goals: IWL tagline, dual taglines, free-forever GPU burn
- Quesar never appears on Abbey Premium Apps / Activity / guild-subscription surfaces

---

## Ops blockers (must clear before phase 2 live charges)

- GitHub Actions billing lock (tip Gate may be billing-locked; docs PRs still open, do not merge while tip Gate blocked)
- Portal Activity URL map operator steps (PREFIX `/` → `donaldfilimon.github.io/abbey-bot/activity`)
- Human Play acceptance / Portal rocket verify
- Donald Portal: rename SKU, attach entitlements, then storefront publish (phase 2)

---

## Testing (phase 2)

- Entitlement unit tests fail-closed (missing/expired entitlement → deny like `/admin`)
- Checker for Activity map remains **PASS≠Portal** until Portal map is live
- Claims check for Quesar page (no invented metrics; no IWL bleed)
- SKU id `1293228939929452574` remains the single guild-subscription SKU under test (no duplicate Guild Pro invent)

---

## Non-goals this phase

- Live SKUs published / storefront_available flipped to true
- Live Discord charges or checkout flows exercised against users
- Inventing a second guild SKU alongside `1293228939929452574`
- Stripe SDK or payment secrets in abbey-bot
- Second bot
- Invented metrics (QPS, latency SLAs, TAM, GPU burn claims)

---

## Decision summary

| Decision | Choice |
|---|---|
| New vs existing SKU | **Reuse** id `1293228939929452574`; rename/clarify only |
| Product name | Working: Abbey Guild Pro (Portal rename from joke name) |
| Entitlements | voice_ux_pro, activity_access, admin_workflow — existing caps only |
| Storefront | Donald Portal publish step; **not** in phase 1 |
| Charges | **None** this phase |
| Quesar | Parallel pilot page; never IWL; never on Abbey storefront |
