//! Pure Premium Apps entitlement model (phase-1 safe, enforce default-off).
//!
//! No Discord HTTP, no slash-command gates. Call sites may later consult
//! [`authorize`] behind [`enforce_enabled`] so production stays open while
//! `ABBEY_ENTITLEMENT_ENFORCE` is unset/off.

// Public API is intentionally unused until phase-2 adapters wire call sites.
#![allow(dead_code)]

/// Discord type-5 Guild Pro SKU (existing Portal SKU; rename only — no second SKU).
pub const SKU_GUILD_PRO: u64 = 1_293_228_939_929_452_574;
/// Discord application id for Abbey.
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
    /// Reserved; [`authorize`] returns `Ok(())` when enforce is false.
    EnforceOffSkipped,
    MissingGrant,
    WrongSku,
    Expired,
    WrongGuild,
    UnknownKey,
}

/// Parse enforce flag. Default off: unset, empty, `0`, `false`, `off`, `no`
/// (ASCII case-insensitive) → false. On: `1`, `true`, `on`, `yes` → true.
/// Any other value → false (fail-soft config; never panic).
pub fn enforce_enabled(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    if raw.is_empty() {
        return false;
    }
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

pub fn key_str(key: EntitlementKey) -> &'static str {
    match key {
        EntitlementKey::VoiceUxPro => ENTITLEMENT_VOICE_UX_PRO,
        EntitlementKey::ActivityAccess => ENTITLEMENT_ACTIVITY_ACCESS,
        EntitlementKey::AdminWorkflow => ENTITLEMENT_ADMIN_WORKFLOW,
    }
}

pub fn parse_key(raw: &str) -> Result<EntitlementKey, Denial> {
    match raw {
        ENTITLEMENT_VOICE_UX_PRO => Ok(EntitlementKey::VoiceUxPro),
        ENTITLEMENT_ACTIVITY_ACCESS => Ok(EntitlementKey::ActivityAccess),
        ENTITLEMENT_ADMIN_WORKFLOW => Ok(EntitlementKey::AdminWorkflow),
        _ => Err(Denial::UnknownKey),
    }
}

/// Pure gate: when enforce is false, return `Ok(())` without consulting grants.
/// When enforce is true, require an active grant for `(guild, key)` on
/// [`SKU_GUILD_PRO`]. Matching key is preferred first, then diagnose.
pub fn authorize(
    enforce: bool,
    guild_id: u64,
    key: EntitlementKey,
    now: u64,
    grants: &[EntitlementGrant],
) -> Result<(), Denial> {
    if !enforce {
        return Ok(());
    }

    let mut saw_key = false;
    let mut last_denial = Denial::MissingGrant;
    for grant in grants {
        if grant.key != key {
            continue;
        }
        saw_key = true;
        match grant_active(grant, guild_id, key, now) {
            Ok(()) => return Ok(()),
            Err(denial) => last_denial = denial,
        }
    }

    if saw_key {
        Err(last_denial)
    } else {
        Err(Denial::MissingGrant)
    }
}

/// Diagnose whether a single grant authorizes `(guild_id, key)` at `now`.
/// Prefer key match, then guild, then SKU, then expiry (`None` = never expires;
/// `Some(exp)` active while `now < exp`).
pub fn grant_active(
    grant: &EntitlementGrant,
    guild_id: u64,
    key: EntitlementKey,
    now: u64,
) -> Result<(), Denial> {
    if grant.key != key {
        return Err(Denial::MissingGrant);
    }
    if grant.guild_id != guild_id {
        return Err(Denial::WrongGuild);
    }
    if grant.sku_id != SKU_GUILD_PRO {
        return Err(Denial::WrongSku);
    }
    if grant.expires_at.is_some_and(|exp| now >= exp) {
        return Err(Denial::Expired);
    }
    Ok(())
}

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
        assert_eq!(
            key_str(EntitlementKey::VoiceUxPro),
            ENTITLEMENT_VOICE_UX_PRO
        );
        assert_eq!(
            key_str(EntitlementKey::ActivityAccess),
            ENTITLEMENT_ACTIVITY_ACCESS
        );
        assert_eq!(
            key_str(EntitlementKey::AdminWorkflow),
            ENTITLEMENT_ADMIN_WORKFLOW
        );
        assert_eq!(parse_key("voice_ux_pro"), Ok(EntitlementKey::VoiceUxPro));
        assert_eq!(
            parse_key("activity_access"),
            Ok(EntitlementKey::ActivityAccess)
        );
        assert_eq!(
            parse_key("admin_workflow"),
            Ok(EntitlementKey::AdminWorkflow)
        );
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
        assert_eq!(ENV_ENTITLEMENT_ENFORCE, "ABBEY_ENTITLEMENT_ENFORCE");
        // Reserved variant: authorize must not return it when enforce is off.
        let _ = Denial::EnforceOffSkipped;
        assert_ne!(Denial::EnforceOffSkipped, Denial::MissingGrant);
    }
}
