//! Owner-bound private help protocol; callers supply time and identities.
use crate::command_catalog::HelpSection;

pub const LIFETIME_SECS: u64 = 15 * 60;
pub const STALE: &str = "That control is stale or invalid. Open /help for fresh private controls.";
pub const NOT_OWNER: &str =
    "These private help controls belong to someone else. Open /help for your own.";
pub const EXPIRED: &str = "This help session has expired. Open /help for fresh private controls.";

// Deliberately no Debug: the encoded ID carries a member snowflake.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct HelpSession {
    pub owner: u64,
    pub expiry: u64,
    pub section: HelpSection,
}
impl HelpSession {
    pub fn new(owner: u64, now: u64, section: HelpSection) -> Option<Self> {
        (owner != 0).then_some(Self {
            owner,
            expiry: now.checked_add(LIFETIME_SECS)?,
            section,
        })
    }
    pub fn custom_id(self) -> String {
        format!(
            "abbey:help:v1:{}:{}:{}",
            self.owner,
            self.expiry,
            self.section.slug()
        )
    }
    pub fn navigate(self, section: HelpSection) -> Self {
        Self { section, ..self }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    Stale,
    NotOwner,
    Expired,
}
impl Rejection {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Stale => STALE,
            Self::NotOwner => NOT_OWNER,
            Self::Expired => EXPIRED,
        }
    }
}
fn decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}
pub fn validate(custom_id: &str, actor: u64, now: u64) -> Result<HelpSession, Rejection> {
    if !custom_id.is_ascii() || custom_id.len() > 100 {
        return Err(Rejection::Stale);
    }
    let mut fields = custom_id.split(':');
    if fields.next() != Some("abbey")
        || fields.next() != Some("help")
        || fields.next() != Some("v1")
    {
        return Err(Rejection::Stale);
    }
    let owner = fields
        .next()
        .and_then(decimal)
        .filter(|id| *id != 0)
        .ok_or(Rejection::Stale)?;
    let expiry = fields.next().and_then(decimal).ok_or(Rejection::Stale)?;
    let section = fields
        .next()
        .and_then(HelpSection::parse)
        .ok_or(Rejection::Stale)?;
    if fields.next().is_some() {
        return Err(Rejection::Stale);
    }
    if owner != actor {
        return Err(Rejection::NotOwner);
    }
    if now >= expiry {
        return Err(Rejection::Expired);
    }
    // A valid control cannot have a lifetime longer than its issuing session.
    if expiry - now > LIFETIME_SECS {
        return Err(Rejection::Stale);
    }
    Ok(HelpSession {
        owner,
        expiry,
        section,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_protocol_and_fixed_expiry() {
        let session = HelpSession::new(u64::MAX, 1000, HelpSection::Start).unwrap();
        assert_eq!(session.expiry, 1900);
        for section in HelpSection::ALL {
            let next = session.navigate(section);
            assert_eq!(next.expiry, 1900);
            assert!(next.custom_id().len() <= 100);
            assert!(next.custom_id().is_ascii());
            let parsed = validate(&next.custom_id(), u64::MAX, 1899).unwrap();
            assert_eq!(parsed.section, section);
            assert_eq!(
                validate(&next.custom_id(), u64::MAX, 1900).err(),
                Some(Rejection::Expired)
            );
        }
        assert_eq!(
            validate(&session.custom_id(), 2, 1000).err(),
            Some(Rejection::NotOwner)
        );
        assert!(HelpSession::new(0, 1000, HelpSection::Start).is_none());
        assert!(HelpSession::new(1, u64::MAX, HelpSection::Start).is_none());
    }
    #[test]
    fn malformed_unknown_overlong_and_future_controls_fail_closed() {
        for id in [
            "abbey:help:v2:1:1000:start",
            "abbey:admin:v1:1:1000:start",
            "abbey:help:v1:0:1000:start",
            "abbey:help:v1:+1:1000:start",
            "abbey:help:v1:01:1000:start",
            "abbey:help:v1:1:01000:start",
            "abbey:help:v1:1:1000:Start",
            "abbey:help:v1:1:1000:start:extra",
            "abbey:help:v1:1:1000:🦀",
            "abbey:help:v1:18446744073709551616:1000:start",
            "abbey:help:v1:1:1001:start",
        ] {
            assert_eq!(validate(id, 1, 100).err(), Some(Rejection::Stale), "{id}");
        }
        assert_eq!(
            validate(&"a".repeat(101), 1, 100).err(),
            Some(Rejection::Stale)
        );
    }
}
