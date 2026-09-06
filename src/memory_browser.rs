//! Pure private fact browsing: strict owner/scope envelopes and complete bounded pages.
//! Callers provide time and identities; this module reads no stores or clocks.

use crate::memory::{MAX_FACT_CHARS, MAX_FACTS};

pub const FACTS_PER_PAGE: usize = 4;
pub const MAX_PAGE_INDEX: u8 = ((MAX_FACTS - 1) / FACTS_PER_PAGE) as u8;
pub const LIFETIME_SECS: u64 = 15 * 60;
pub const UNAVAILABLE: &str =
    "These facts are unavailable in this view. Open `/recall` to view the summary.";

// Identity-bearing scopes and sessions deliberately have no Debug implementation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemoryScope {
    Guild(u64),
    BotDm,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MemorySession {
    pub owner: u64,
    pub subject: u64,
    pub scope: MemoryScope,
    pub expiry: u64,
    pub page: u8,
}

impl MemorySession {
    pub fn new(owner: u64, subject: u64, scope: MemoryScope, now: u64) -> Option<Self> {
        if owner == 0 || subject == 0 || !scope_valid(scope, owner, subject) {
            return None;
        }
        Some(Self {
            owner,
            subject,
            scope,
            expiry: now.checked_add(LIFETIME_SECS)?,
            page: 0,
        })
    }

    pub fn navigate(self, page: u8) -> Option<Self> {
        (page <= MAX_PAGE_INDEX).then_some(Self { page, ..self })
    }

    pub fn custom_id(self) -> String {
        let scope = match self.scope {
            MemoryScope::Guild(guild) => guild.to_string(),
            MemoryScope::BotDm => "d".to_string(),
        };
        format!(
            "abbey:mem:v1:{}:{}:{scope}:{}:{}",
            self.owner, self.subject, self.expiry, self.page
        )
    }
}

fn scope_valid(scope: MemoryScope, owner: u64, subject: u64) -> bool {
    match scope {
        MemoryScope::Guild(guild) => guild != 0,
        MemoryScope::BotDm => owner == subject,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRejection {
    Stale,
    NotOwner,
    WrongScope,
    Expired,
}

impl BrowserRejection {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Stale => {
                "That fact browser control is stale or invalid. Open `/recall` for fresh private controls."
            }
            Self::NotOwner => {
                "These private fact controls belong to someone else. Open `/recall` for your own."
            }
            Self::WrongScope => {
                "This fact browser belongs to another conversation. Open `/recall` here for fresh private controls."
            }
            Self::Expired => {
                "This fact browser has expired. Open `/recall` for fresh private controls."
            }
        }
    }
}

fn decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

pub fn validate(
    id: &str,
    actor: u64,
    scope: &MemoryScope,
    now: u64,
) -> Result<MemorySession, BrowserRejection> {
    if !id.is_ascii() || id.len() > 100 {
        return Err(BrowserRejection::Stale);
    }
    let mut fields = id.split(':');
    if fields.next() != Some("abbey") || fields.next() != Some("mem") || fields.next() != Some("v1")
    {
        return Err(BrowserRejection::Stale);
    }
    let owner = fields
        .next()
        .and_then(decimal)
        .filter(|id| *id != 0)
        .ok_or(BrowserRejection::Stale)?;
    let subject = fields
        .next()
        .and_then(decimal)
        .filter(|id| *id != 0)
        .ok_or(BrowserRejection::Stale)?;
    let parsed_scope = match fields.next() {
        Some("d") => MemoryScope::BotDm,
        Some(guild) => MemoryScope::Guild(
            decimal(guild)
                .filter(|id| *id != 0)
                .ok_or(BrowserRejection::Stale)?,
        ),
        None => return Err(BrowserRejection::Stale),
    };
    let expiry = fields
        .next()
        .and_then(decimal)
        .ok_or(BrowserRejection::Stale)?;
    let page = fields
        .next()
        .and_then(decimal)
        .filter(|page| *page <= u64::from(MAX_PAGE_INDEX))
        .ok_or(BrowserRejection::Stale)? as u8;
    if fields.next().is_some() || !scope_valid(parsed_scope, owner, subject) {
        return Err(BrowserRejection::Stale);
    }
    if owner != actor {
        return Err(BrowserRejection::NotOwner);
    }
    if parsed_scope != *scope {
        return Err(BrowserRejection::WrongScope);
    }
    if now >= expiry {
        return Err(BrowserRejection::Expired);
    }
    if expiry - now > LIFETIME_SECS {
        return Err(BrowserRejection::Stale);
    }
    Ok(MemorySession {
        owner,
        subject,
        scope: parsed_scope,
        expiry,
        page,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotRejection {
    TooManyFacts,
    FactTooLong,
}

/// Validate the complete canonical snapshot, including facts outside this page.
pub fn validate_snapshot(facts: &[String]) -> Result<(), SnapshotRejection> {
    if facts.len() > MAX_FACTS {
        return Err(SnapshotRejection::TooManyFacts);
    }
    if facts
        .iter()
        .any(|fact| fact.chars().count() > MAX_FACT_CHARS)
    {
        return Err(SnapshotRejection::FactTooLong);
    }
    Ok(())
}

pub struct FactPage<'a> {
    pub index: u8,
    pub total_pages: u8,
    pub total_facts: usize,
    pub facts: &'a [String],
    /// Invalid legacy snapshots expose no partial facts or successful page claim.
    /// The shell must omit navigation whenever this outcome is an error.
    pub validity: Result<(), SnapshotRejection>,
}

pub fn page(facts: &[String], requested: u8) -> FactPage<'_> {
    let validity = validate_snapshot(facts);
    if validity.is_err() {
        return FactPage {
            index: 0,
            total_pages: 1,
            total_facts: facts.len(),
            facts: &facts[..0],
            validity,
        };
    }
    let total_pages = facts.len().div_ceil(FACTS_PER_PAGE).max(1) as u8;
    let index = requested.min(total_pages - 1);
    let first = usize::from(index) * FACTS_PER_PAGE;
    FactPage {
        index,
        total_pages,
        total_facts: facts.len(),
        facts: &facts[first..(first + FACTS_PER_PAGE).min(facts.len())],
        validity,
    }
}

pub fn render(subject: u64, page: &FactPage<'_>) -> String {
    if page.validity.is_err() {
        return UNAVAILABLE.to_string();
    }
    let mut text = format!(
        "**Stored facts · <@{subject}>**\nPage {} of {} · {} facts\n\n",
        usize::from(page.index) + 1,
        page.total_pages,
        page.total_facts
    );
    if page.facts.is_empty() {
        text.push_str("No facts on record.\n");
    } else {
        for (offset, fact) in page.facts.iter().enumerate() {
            let number = usize::from(page.index) * FACTS_PER_PAGE + offset + 1;
            text.push_str(&format!("{number}. {fact}\n"));
        }
    }
    text.push_str("\nRead-only. Facts refresh when you change pages. Controls expire 15 minutes after opening.");
    text
}

#[cfg(test)]
mod tests;
