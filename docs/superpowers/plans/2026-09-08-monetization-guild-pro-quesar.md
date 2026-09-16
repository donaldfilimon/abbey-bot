# Monetization Guild Pro + Quesar phase-1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land phase-1 monetization docs and operator checklist that reuse existing Discord type-5 SKU `1293228939929452574` (rename Portal → Abbey Guild Pro; no second SKU), map entitlements `voice_ux_pro` / `activity_access` / `admin_workflow` onto already-shipped capabilities only, keep `storefront_available=false` with **no live charges**, document future fail-closed Rust `premium_entitlements` behind `ABBEY_ENTITLEMENT_ENFORCE` default-off, and schedule a separate Quesar pilot CTA in `donaldfilimon/mlai-website-app` (never IWL).

**Architecture:** Docs-first dual GTM. Abbey Premium Apps path lives in this repo (SKU reuse + entitlement map + Portal checklist + pure Rust entitlement core later). Quesar pilot CTA lives in `mlai-website-app` only. Ops gates (billing unlock, Activity URL map, SKU rename, entitlement attach, storefront publish) stay human Donald Portal steps — bot token cannot set URL mappings or publish the storefront. Phase 1 ships documentation and checklist only; Rust enforcement and storefront publish are explicitly later.

**Tech Stack:** Rust 1.98 (future `src/premium_entitlements.rs`), Serenity 0.12.5 / Poise 0.6.2 (future adapters only), Discord Premium Apps / Entitlement API (operator Portal + future HTTP), TypeScript/React in `mlai-website-app` for Quesar copy, GitHub Pages inventory gate (`scripts/test-check-pages-liquid.py`).

**Spec:** `docs/superpowers/specs/2026-09-08-monetization-guild-pro-quesar-design.md` (locked draft on PR #127; included on this branch).

## Global Constraints

- Reuse SKU id `1293228939929452574` (type-5); rename Portal → **Abbey Guild Pro**; **no second SKU**.
- App id `1147940171099152464` only for Abbey Premium Apps surfaces.
- Entitlements exactly: `voice_ux_pro`, `activity_access`, `admin_workflow` — map to existing shipped capabilities only; invent no new product features for monetization.
- `storefront_available=false`; **NO live charges** in phase 1; do not publish storefront.
- Brand: IWL only Abbey/ABI/Abbey Bot; Quesar never IWL; no invented metrics (QPS, latency SLAs, TAM, GPU burn).
- Activity map TARGET: `donaldfilimon.github.io/abbey-bot/activity` PREFIX `/` (no `https://`, directory not `index.html`). Checker PASS ≠ Portal configured.
- Future enforce env: `ABBEY_ENTITLEMENT_ENFORCE` **default off** (unset / empty / `0` / `false` / `off` → allow; only explicit on-values enforce).
- Fail-closed like `/admin` when enforce is on: missing/expired/wrong-sku entitlement → ephemeral deny; zero capability mutations.
- Never commit Client Secret, bot token, Stripe secrets, monetization keys, or payment material.
- Never bot Go Live; never Components V2 on pinned crates; never force-push `main`.
- Keep production modules under 1,000 lines when Rust lands later; leave unrelated dirty files alone.
- Do **not** merge while tip Gate may be billing-locked.
- No TBD/TODO placeholders in this plan.

## File map

### abbey-bot (this repository)

- `docs/superpowers/specs/2026-09-08-monetization-guild-pro-quesar-design.md`: locked design (included from #127).
- `docs/superpowers/plans/2026-09-08-monetization-guild-pro-quesar.md`: this plan.
- `docs/ops/monetization-portal-checklist.md`: operator checklist (API-confirmed vs Donald Portal steps).
- `docs/superpowers/README.md`: monetization spec + plan pointers.
- `docs/discord-application-api-roadmap.md`: phase-1 monetization status block.
- `scripts/test-check-pages-liquid.py`: Pages markdown inventory entries for new docs.
- `src/premium_entitlements.rs`: **future** pure entitlement model + unit tests (Task 3 describes; do not implement in the docs PR).
- `.env.example`: **future** `ABBEY_ENTITLEMENT_ENFORCE` comment block (Task 4).
- Thin future adapters (not this PR): voice UX premium gate, Activity access gate, `/admin` workflow gate — call pure helpers only when enforce is on.

### mlai-website-app (other repository — Task 5)

- `src/content/quesar.ts`: pilot CTA copy object (private AI ops; request pilot; no IWL; no invented metrics).
- `src/components/quesar-pages.tsx`: landing CTA row wiring for pilot request (separate PR in that repo).

---

### Task 1: Land design + plan + README pointer

**Files:** design spec (already on branch from #127), this plan, `docs/superpowers/README.md`, Pages inventory.

- [x] Branch `docs/monetization-implementation-plan-20260908` from `origin/docs/monetization-guild-pro-quesar-design-20260908` so design remains included while #127 is open (prefer including design over branching from `main` alone).
- [x] Write this plan with locked SKU / entitlement / brand / Activity map / enforce-default-off facts copied exactly.
- [x] Add monetization plan link under Plans in `docs/superpowers/README.md` (spec link already present from #127).
- [x] Extend `scripts/test-check-pages-liquid.py` expected inventory for every new tracked Markdown this PR adds.
- [ ] Confirm `python3 -m unittest scripts.test-check-pages-liquid.MarkdownFilesTests.test_live_repository_selection_matches_current_pages_rendering` passes on the branch tip.

### Task 2: Create ops checklist + roadmap note (lands in this PR)

**Files:** Create `docs/ops/monetization-portal-checklist.md`; update `docs/discord-application-api-roadmap.md`.

- [x] Write `docs/ops/monetization-portal-checklist.md` with:
  - API-confirmed items checked (`monetization_state=ENABLED`, `is_monetized=true`, team-owned, verified, SKU id `1293228939929452574`, `storefront_available=false`).
  - Donald Portal steps unchecked (payout/region, owner onboarding, Stripe Connect link, Monetization Terms, SKU rename → Abbey Guild Pro, attach three entitlements, **do not publish** storefront in phase 1).
  - Activity map steps (PREFIX `/`, TARGET `donaldfilimon.github.io/abbey-bot/activity`) unchecked until Donald confirms iframe.
  - Billing / tip-Gate note: do not merge while tip Gate may be billing-locked.
  - Non-goals: live charges, second SKU, Stripe SDK in abbey-bot, Quesar on IWL surfaces, invented metrics.
- [x] Add short phase-1 status block to `docs/discord-application-api-roadmap.md` under Later — Monetization with links to design, plan, and checklist; restate **no live charges**.
- [x] Add Pages inventory entries for the checklist + this plan.

### Task 3: TDD plan for `src/premium_entitlements.rs` (implement later — describe only)

**Files (future implementation PR, not this docs PR):** Create `src/premium_entitlements.rs`; register `mod premium_entitlements;` in `src/main.rs`; optional thin call sites later.

**Do not write Rust source in the docs PR.** The following is the binding TDD contract for the future implementation PR.

**Interfaces (exact):**

```rust
pub const SKU_GUILD_PRO: u64 = 1_293_228_939_929_452_574;
pub const APP_ID: u64 = 1_147_940_171_099_152_464;

pub const ENTITLEMENT_VOICE_UX_PRO: &str = "voice_ux_pro";
pub const ENTITLEMENT_ACTIVITY_ACCESS: &str = "activity_access";
pub const ENTITLEMENT_ADMIN_WORKFLOW: &str = "admin_workflow";

pub const ENV_ENTITLEMENT_ENFORCE: &str = "ABBEY_ENTITLEMENT_ENFORCE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitlementKey {
    VoiceUxPro,
    ActivityAccess,
    AdminWorkflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementGrant {
    pub sku_id: u64,
    pub key: EntitlementKey,
    pub guild_id: u64,
    /// Unix seconds; `None` means non-expiring while Discord reports active.
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    EnforceOffSkipped,
    MissingGrant,
    WrongSku,
    Expired,
    WrongGuild,
    UnknownKey,
}

/// Parse enforce flag. Default off: unset, empty, `0`, `false`, `off`, `no` (ASCII case-insensitive) → false.
/// On: `1`, `true`, `on`, `yes` → true. Any other value → false (fail-soft config; never panic).
pub fn enforce_enabled(raw: Option<&str>) -> bool;

pub fn key_str(key: EntitlementKey) -> &'static str;
pub fn parse_key(raw: &str) -> Result<EntitlementKey, Denial>;

/// Pure gate: when enforce is false, return Ok(()) without consulting grants.
/// When enforce is true, require an active grant for (guild, key) on SKU_GUILD_PRO.
pub fn authorize(
    enforce: bool,
    guild_id: u64,
    key: EntitlementKey,
    now: u64,
    grants: &[EntitlementGrant],
) -> Result<(), Denial>;

pub fn grant_active(grant: &EntitlementGrant, guild_id: u64, key: EntitlementKey, now: u64) -> Result<(), Denial>;
```

**Step 1 — Write failing unit tests first** (future PR; paste into `src/premium_entitlements.rs` `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_defaults_off() {
        assert!(!enforce_enabled(None));
        assert!(!enforce_enabled(Some("")));
        assert!(!enforce_enabled(Some("0")));
        assert!(!enforce_enabled(Some("false")));
        assert!(!enforce_enabled(Some("OFF")));
        assert!(!enforce_enabled(Some("no")));
        assert!(!enforce_enabled(Some("maybe")));
    }

    #[test]
    fn enforce_explicit_on() {
        assert!(enforce_enabled(Some("1")));
        assert!(enforce_enabled(Some("true")));
        assert!(enforce_enabled(Some("ON")));
        assert!(enforce_enabled(Some("yes")));
    }

    #[test]
    fn key_round_trip() {
        assert_eq!(key_str(EntitlementKey::VoiceUxPro), ENTITLEMENT_VOICE_UX_PRO);
        assert_eq!(key_str(EntitlementKey::ActivityAccess), ENTITLEMENT_ACTIVITY_ACCESS);
        assert_eq!(key_str(EntitlementKey::AdminWorkflow), ENTITLEMENT_ADMIN_WORKFLOW);
        assert_eq!(parse_key("voice_ux_pro"), Ok(EntitlementKey::VoiceUxPro));
        assert_eq!(parse_key("activity_access"), Ok(EntitlementKey::ActivityAccess));
        assert_eq!(parse_key("admin_workflow"), Ok(EntitlementKey::AdminWorkflow));
        assert_eq!(parse_key("mystery"), Err(Denial::UnknownKey));
    }

    #[test]
    fn authorize_skips_when_enforce_off() {
        let out = authorize(false, 42, EntitlementKey::VoiceUxPro, 1_000, &[]);
        assert_eq!(out, Ok(()));
    }

    #[test]
    fn authorize_deny_missing_when_enforce_on() {
        let out = authorize(true, 42, EntitlementKey::VoiceUxPro, 1_000, &[]);
        assert_eq!(out, Err(Denial::MissingGrant));
    }

    #[test]
    fn authorize_accepts_active_guild_pro_grant() {
        let grants = [EntitlementGrant {
            sku_id: SKU_GUILD_PRO,
            key: EntitlementKey::ActivityAccess,
            guild_id: 99,
            expires_at: Some(2_000),
        }];
        assert_eq!(
            authorize(true, 99, EntitlementKey::ActivityAccess, 1_500, &grants),
            Ok(())
        );
    }

    #[test]
    fn authorize_deny_wrong_sku_expired_wrong_guild() {
        let wrong_sku = [EntitlementGrant {
            sku_id: 1,
            key: EntitlementKey::AdminWorkflow,
            guild_id: 7,
            expires_at: None,
        }];
        assert_eq!(
            authorize(true, 7, EntitlementKey::AdminWorkflow, 10, &wrong_sku),
            Err(Denial::WrongSku)
        );

        let expired = [EntitlementGrant {
            sku_id: SKU_GUILD_PRO,
            key: EntitlementKey::AdminWorkflow,
            guild_id: 7,
            expires_at: Some(5),
        }];
        assert_eq!(
            authorize(true, 7, EntitlementKey::AdminWorkflow, 10, &expired),
            Err(Denial::Expired)
        );

        let foreign = [EntitlementGrant {
            sku_id: SKU_GUILD_PRO,
            key: EntitlementKey::AdminWorkflow,
            guild_id: 8,
            expires_at: None,
        }];
        assert_eq!(
            authorize(true, 7, EntitlementKey::AdminWorkflow, 10, &foreign),
            Err(Denial::WrongGuild)
        );
    }

    #[test]
    fn sku_and_app_constants_match_locked_design() {
        assert_eq!(SKU_GUILD_PRO, 1_293_228_939_929_452_574);
        assert_eq!(APP_ID, 1_147_940_171_099_152_464);
    }
}
```

- [ ] **Future PR Step 1:** Add the test module above; `cargo test --locked premium_entitlements::` fails (module missing / stubs incomplete).
- [ ] **Future PR Step 2:** Implement constants, enums, `enforce_enabled`, `key_str`, `parse_key`, `grant_active`, `authorize` until tests pass.
- [ ] **Future PR Step 3:** Document mapping only — do **not** wire Discord HTTP Entitlement fetch or slash-command gates until storefront publish is an explicit phase-2 decision. Keep adapters behind `enforce_enabled(std::env::var_os(...))` so production remains open while flag is off.
- [ ] **Future PR Step 4:** `cargo fmt`; `cargo clippy -D warnings` on the new module; focused `cargo test --locked premium_entitlements::`.

**Capability mapping (documentation for adapters, not new features):**

| Entitlement key | Existing capability only |
|---|---|
| `voice_ux_pro` | Classic Action Row voice controls (play/stop/skip/refresh polish) per voice-classic UX design |
| `activity_access` | Activity URL map path (PREFIX `/` → `donaldfilimon.github.io/abbey-bot/activity`) |
| `admin_workflow` | `/admin` Action Row / pending helpers already shipped |

### Task 4: Docs note — enforce default-off

**Files:** this plan (done here); future `.env.example` comment in the Rust PR; roadmap pointer already states phase-1 docs-only.

- [x] State in this plan and in `docs/ops/monetization-portal-checklist.md` that `ABBEY_ENTITLEMENT_ENFORCE` defaults **off** and must stay off through phase 1 (no live charges; no production deny path).
- [ ] **Future Rust PR:** add to `.env.example`:

```
# Premium Apps entitlement enforce (phase 2+). Default off.
# Unset / empty / 0 / false / off / no → do not enforce (phase-1 safe).
# 1 / true / on / yes → fail-closed authorize() against Guild Pro grants.
# ABBEY_ENTITLEMENT_ENFORCE=
```

- [ ] Never enable enforce in launchd / production env until Donald publishes storefront and accepts live-charge risk.

### Task 5: Quesar pilot CTA in `donaldfilimon/mlai-website-app` (other repo)

**Files (other repo, separate PR):** `src/content/quesar.ts`, `src/components/quesar-pages.tsx`.

- [ ] Add a `pilot` (or equivalent) content object on `quesar` with: private AI operations framing; request-pilot CTA label; consent/audit/invite/org/gateway/KMS bullets already implied by existing pillars; **no** IWL / Abbey / ABI naming; **no** QPS / latency / TAM / GPU claims.
- [ ] On `QuesarLanding`, keep primary path to consent; ensure secondary CTA is explicitly “Request pilot” (mailto or `/contact` with pilot intent query) — do not imply Discord Premium Apps checkout.
- [ ] Claims check: grep the Quesar surface for `Intelligence Without Limits`, `Abbey Bot`, invented metrics; zero hits on the pilot CTA copy.
- [ ] Open PR in `mlai-website-app` only; do not fold that code into abbey-bot.

### Task 6: Donald ops (billing, Portal map, SKU rename, entitlements attach, no publish)

**Operator-only (Donald).** Agents prepare checklist state; Donald clicks.

- [ ] Clear GitHub Actions billing lock so tip Gate can run green before any merge of monetization docs/code stacks.
- [ ] Portal → Activities → URL Mappings: PREFIX `/`, TARGET `donaldfilimon.github.io/abbey-bot/activity`; Desktop + Web; confirm Entry Point `launch` still present; rocket iframe verify in Office Hours.
- [ ] Portal → monetization: rename SKU `1293228939929452574` joke name → **Abbey Guild Pro**.
- [ ] Attach entitlements `voice_ux_pro`, `activity_access`, `admin_workflow` to that SKU only.
- [ ] Confirm US/UK/EU payout / Stripe Connect / Monetization Terms as Portal requires — never paste secrets into git or chat logs that land in the repo.
- [ ] **Do not publish** storefront in phase 1 (`storefront_available` remains false).
- [ ] Do not enable `ABBEY_ENTITLEMENT_ENFORCE` in production.

---

## Out of scope (phase 2+)

- Storefront publish / `storefront_available=true` / live Discord charges or checkout against users.
- Inventing a second guild SKU alongside `1293228939929452574`.
- Stripe SDK, payment webhooks, or payment secrets inside abbey-bot.
- Wiring Discord Entitlement HTTP fetch + slash/component gates while enforce is on.
- Quesar branding on Abbey Premium Apps, Activity UI, or guild-subscription surfaces.
- Components V2, bot Go Live, second bot, invented metrics.
- Merging while tip Gate is billing-locked.

---

## Gate checklist

- Design included on branch (or already on `main` via #127) with locked SKU / entitlements / brand / Activity map facts.
- Plan + ops checklist + README + roadmap pointers present; no TBD/TODO placeholders.
- Pages liquid inventory lists every new Markdown path; unittest green.
- Docs PR opened; **not** merged while tip Gate may be billing-locked.
- No Rust `premium_entitlements` implementation in the docs PR (Task 3 is describe-only).
- No storefront publish; no live charges; `ABBEY_ENTITLEMENT_ENFORCE` remains default-off.
- Quesar work tracked as other-repo Task 5; never IWL on Quesar CTA.
- Donald Portal steps remain unchecked until human confirmation.

## Self-Review

1. Spec coverage: Task 1–2 land design/plan/checklist/roadmap/README/Pages; Task 3 binds future pure Rust TDD; Task 4 locks enforce default-off; Task 5 isolates Quesar to `mlai-website-app`; Task 6 lists Donald-only Portal/billing gates.
2. No placeholders: SKU id, app id, entitlement keys, Activity PREFIX/TARGET, enforce env name, and test bodies are fully specified.
3. Brand freeze held: Abbey/IWL only on Abbey surfaces; Quesar never IWL; no invented metrics.
4. Docs-only PR: zero Rust source changes; implementation deferred to a later PR that follows Task 3 tests-first.
5. Merge policy: open PR, do not merge while tip Gate may be billing-locked; keep paired with #127 design until that lands or this branch already contains it.
