//! Pure task-control envelope. Identity, scope, action and fixed lifetime travel together.
pub const STALE: &str =
    "That task control is stale or invalid. Open `/help` for fresh private controls.";
const LIFETIME: u64 = 900;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Conversation,
    Memory,
    Images,
    Voice,
    Administration,
}
impl Action {
    pub const ALL: [Self; 5] = [
        Self::Conversation,
        Self::Memory,
        Self::Images,
        Self::Voice,
        Self::Administration,
    ];
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Conversation => "ask",
            Self::Memory => "memory",
            Self::Images => "images",
            Self::Voice => "voice",
            Self::Administration => "admin",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Conversation => "Talk with Abbey",
            Self::Memory => "Review memory",
            Self::Images => "Use an image",
            Self::Voice => "Voice & Music",
            Self::Administration => "Manage Abbey",
        }
    }
}
// No Debug: controls contain private caller and context identifiers.
#[derive(Clone, Copy)]
pub struct Session {
    pub owner: u64,
    pub guild: Option<u64>,
    pub channel: u64,
    pub expiry: u64,
    pub action: Action,
}
impl Session {
    pub fn custom_id(self) -> String {
        format!(
            "abbey:task:v1:{}:{}:{}:{}:{}",
            self.owner,
            self.guild
                .map_or_else(|| "d".to_string(), |g| g.to_string()),
            self.channel,
            self.expiry,
            self.action.slug()
        )
    }
}
fn number(value: &str) -> Option<u64> {
    let n = value.parse::<u64>().ok()?;
    (n > 0 && n.to_string() == value).then_some(n)
}
pub fn validate(
    id: &str,
    owner: u64,
    guild: Option<u64>,
    channel: u64,
    now: u64,
) -> Result<Session, &'static str> {
    let parts: Vec<_> = id.split(':').collect();
    if parts.len() != 8 || parts[..3] != ["abbey", "task", "v1"] || id.len() > 100 {
        return Err(STALE);
    }
    let session = Session {
        owner: number(parts[3]).ok_or(STALE)?,
        guild: if parts[4] == "d" {
            None
        } else {
            Some(number(parts[4]).ok_or(STALE)?)
        },
        channel: number(parts[5]).ok_or(STALE)?,
        expiry: number(parts[6]).ok_or(STALE)?,
        action: Action::ALL
            .into_iter()
            .find(|a| a.slug() == parts[7])
            .ok_or(STALE)?,
    };
    if session.owner != owner {
        return Err(
            "These private task controls belong to someone else. Open `/help` for your own.",
        );
    }
    if session.guild != guild || session.channel != channel {
        return Err(STALE);
    }
    if session.expiry <= now || session.expiry.saturating_sub(now) > LIFETIME {
        return Err("This task session has expired. Open `/help` for fresh private controls.");
    }
    Ok(session)
}
pub fn question(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= 2000).then_some(value)
}
#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
