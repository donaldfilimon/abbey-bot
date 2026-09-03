//! Shared gateway helpers — single-responsibility extraction.
//!
//! Holds every helper the three adapters previously duplicated or re-defined:
//! the Discord/Telegram caps, the generic `clamp` and the Discord-specific
//! `clamp_message` (kept in sync with `commands::clamp_message`), the
//! `Snowflake` newtype that replaces stringly `parse_id`, the
//! `SecretString` wrapper for token redaction, the small `PollLoop`
//! abstraction, and the bounded `fetch_capped`.

use std::fmt;
use std::time::Duration;

/// Discord's message cap, in codepoints.
pub const DISCORD_MESSAGE_CAP: usize = 2_000;
/// Telegram's `sendMessage` cap.
pub const TELEGRAM_MESSAGE_CAP: usize = 4_096;

// ---------------------------------------------------------------------------
// clamp helpers
// ---------------------------------------------------------------------------

/// Generic codepoint clamp with an ellipsis. Telegram and the pipeline's
/// fallback use this directly; Discord's user-facing path goes through
/// `clamp_message`.
pub fn clamp(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut out: String = text.chars().take(cap - 1).collect();
    out.push('…');
    out
}

/// Discord-specific clamp that stays in sync with `commands::clamp_message`.
///
/// Every rendered answer passes through this. Without it, a long-but-valid
/// answer fails the followup with "Message too large" after the defer has
/// already succeeded. Uses the same marker as the command layer so the two
/// surfaces never drift.
pub fn clamp_message(text: String) -> String {
    const MARKER: &str = "\n… (truncated to fit Discord's 2,000-character limit)";
    if text.chars().count() <= DISCORD_MESSAGE_CAP {
        return text;
    }
    let keep = DISCORD_MESSAGE_CAP - MARKER.chars().count();
    let mut clamped: String = text.chars().take(keep).collect();
    clamped.push_str(MARKER);
    clamped
}

// ---------------------------------------------------------------------------
// Snowflake newtype
// ---------------------------------------------------------------------------

/// Typed Discord/Telegram snowflake. Replaces the stringly `parse_id` helper
/// with a distinct type so a channel id cannot be passed where a message id is
/// expected without an explicit `.get()`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(u64);

impl Snowflake {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        s.parse::<u64>()
            .map(Snowflake)
            .map_err(|_| format!("{s:?} is not a snowflake"))
    }
}

impl fmt::Debug for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Snowflake").field(&self.0).finish()
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u64> for Snowflake {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<Snowflake> for u64 {
    fn from(s: Snowflake) -> Self {
        s.0
    }
}

/// Legacy helper kept for callers that still expect a raw `u64` — delegates to
/// `Snowflake::parse` so the error copy stays single-sourced.
#[allow(dead_code)]
pub fn parse_id(s: &str) -> Result<u64, String> {
    Snowflake::parse(s).map(|s| s.get())
}

// ---------------------------------------------------------------------------
// SecretString — Debug redaction for tokens
// ---------------------------------------------------------------------------

/// Holds a secret (bot token) and redacts it on `Debug`. Use `.expose()` to
/// get the raw value where it must be sent on the wire.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

// ---------------------------------------------------------------------------
// PollLoop abstraction — deduplicates the two polling loops
// ---------------------------------------------------------------------------

/// Deduplicates the "back off and retry" loops shared by Telegram's
/// `getUpdates` poll and Slack's Socket Mode reconnect. Each adapter was
/// re-typing the same `tokio::time::sleep(Duration::from_secs(N))` with a
/// different `N`; this centralises the durations and the `wait` so the caps
/// are not re-typed and the retry shape is one place.
#[derive(Debug, Clone, Copy)]
pub struct PollLoop {
    backoff: Duration,
}

impl PollLoop {
    pub const fn new(backoff: Duration) -> Self {
        Self { backoff }
    }

    pub const fn telegram() -> Self {
        Self::new(Duration::from_secs(5))
    }

    pub const fn slack_open() -> Self {
        Self::new(Duration::from_secs(10))
    }

    pub const fn slack_reconnect() -> Self {
        Self::new(Duration::from_secs(2))
    }

    #[allow(dead_code)]
    pub fn backoff_duration(self) -> Duration {
        self.backoff
    }

    pub async fn wait(self) {
        tokio::time::sleep(self.backoff).await;
    }
}

// ---------------------------------------------------------------------------
// Bounded fetch
// ---------------------------------------------------------------------------

/// GET `url` and refuse bodies over `max` bytes — attachments are
/// attacker-controlled (`docs/spec/vision.md`).
pub async fn fetch_capped(
    client: &reqwest::Client,
    url: &str,
    max: usize,
    bearer: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut req = client.get(url);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    let response = req.send().await.map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("attachment fetch failed: HTTP {status}"));
    }
    crate::http_body::read_capped(response, max)
        .await
        .map_err(|error| {
            if error.is_too_large() {
                format!("attachment exceeds the {max}-byte cap")
            } else {
                "attachment download failed while reading the response".to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_respects_the_cap_in_codepoints() {
        let long = "é".repeat(2_500);
        let clamped = clamp(&long, DISCORD_MESSAGE_CAP);
        assert_eq!(clamped.chars().count(), DISCORD_MESSAGE_CAP);
        assert!(clamped.ends_with('…'));
        assert_eq!(clamp("short", DISCORD_MESSAGE_CAP), "short");
    }

    #[test]
    fn clamp_message_syncs_with_commands() {
        let short = "fits".to_string();
        assert_eq!(clamp_message(short.clone()), short);
        let long: String = "é".repeat(2500);
        let out = clamp_message(long);
        assert!(out.chars().count() <= DISCORD_MESSAGE_CAP);
        assert!(out.ends_with("limit)"));
    }

    #[test]
    fn snowflakes_parse_and_garbage_does_not() {
        assert_eq!(Snowflake::parse("123").unwrap().get(), 123);
        assert!(Snowflake::parse("abc").is_err());
        assert_eq!(parse_id("123"), Ok(123));
        assert!(parse_id("abc").is_err());
    }

    #[test]
    fn secret_string_redacts_debug() {
        let s = SecretString::new("secret-token-123");
        assert_eq!(format!("{s:?}"), "<redacted>");
        assert_eq!(s.expose(), "secret-token-123");
    }

    #[test]
    fn poll_loop_holds_backoff() {
        let l = PollLoop::new(Duration::from_secs(5));
        assert_eq!(l.backoff_duration(), Duration::from_secs(5));
    }
}
