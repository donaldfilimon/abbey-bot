//! Shared text helpers — the single source for “blank?” and “normalized”.

/// Trimmed, non-empty view of `s`.
///
/// Returns `None` when `s` is empty or whitespace only, mirroring the
/// `value.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())`
/// pattern that previously appeared in `llm`, `offline_voice`, and
/// `vision::provider`.
#[must_use]
pub fn non_blank(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Unified normalization: lowercase, drop ASCII and curly apostrophes,
/// map every other non-alphanumeric run to a single space, collapse
/// repeats, and trim leading/trailing whitespace.
///
/// This is the single implementation that previously lived as
/// `routing_signals::normalize` (padded) and
/// `voice_session::control::normalized_voice_text` (trimmed).  The trimmed
/// form is canonical; callers that need word-boundary checks add their own
/// padding (e.g. `contains_phrase` in both modules already does so).
#[must_use]
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_space = true;
    for ch in s.chars() {
        if ch == '\'' || ch == '\u{2019}' {
            continue;
        }
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            at_space = false;
        } else if !at_space {
            out.push(' ');
            at_space = true;
        }
    }
    if at_space && out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_blank_trims_and_rejects_empty() {
        assert_eq!(non_blank(""), None);
        assert_eq!(non_blank("   "), None);
        assert_eq!(non_blank("\t\n "), None);
        assert_eq!(non_blank(" hello "), Some("hello"));
        assert_eq!(non_blank("  hello world  "), Some("hello world"));
        assert_eq!(non_blank("a"), Some("a"));
    }

    #[test]
    fn normalize_lowercases_and_collapses() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
        assert_eq!(normalize("Can't—stop!"), "cant stop");
        assert_eq!(normalize("  Hello   WORLD  "), "hello world");
        assert_eq!(
            normalize("I don't understand this"),
            "i dont understand this"
        );
        assert_eq!(normalize("It\u{2019}s fine"), "its fine");
        assert_eq!(normalize("café naïve"), "café naïve");
        assert_eq!(normalize("a  b\tc\n\nd"), "a b c d");
        assert_eq!(normalize("nowhere"), "nowhere");
        assert_eq!(normalize("!!!"), "");
        assert_eq!(
            normalize("  leading and trailing  "),
            "leading and trailing"
        );
    }

    #[test]
    fn normalize_handles_apostrophes_consistently() {
        assert_eq!(normalize("can't"), normalize("cant"));
        assert_eq!(normalize("don\u{2019}t"), normalize("dont"));
        assert_eq!(normalize("'hello'"), "hello");
        assert_eq!(normalize("''"), "");
    }

    #[test]
    fn normalize_deterministic() {
        for input in [
            "",
            "   ",
            "Hello",
            "Can't",
            "line one\nline two",
            "🙂🙃",
            "———",
        ] {
            let a = normalize(input);
            let b = normalize(input);
            assert_eq!(a, b, "not deterministic for {input:?}");
        }
    }
}
