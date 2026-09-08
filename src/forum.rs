//! Pure #help forum helpers: tag suggest, first-post templates, overwrite gap-fill.
//!
//! Discord translation lives in `commands_forum`. Decisions here take plain
//! values so tests do not need a live guild.

use serenity::all::Permissions;

/// Default `#help` tags from `blueprints/mlai-community.toml` (creation-time set).
pub const HELP_TAG_NAMES: &[&str] = &[
    "wdbx",
    "abi",
    "abbey",
    "site-builder",
    "mobile",
    "apple-silicon",
    "build",
];

/// Permissions Abbey needs on a forum channel to create posts via the API.
pub fn required_forum_bot_permissions() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::READ_MESSAGE_HISTORY
}

/// First-post template kinds for `#help`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Question,
    Bug,
    Build,
    General,
}

impl Template {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Bug => "bug",
            Self::Build => "build",
            Self::General => "general",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Question => "Question",
            Self::Bug => "Bug",
            Self::Build => "Build",
            Self::General => "General",
        }
    }
}

/// Render the starter body for a help forum post.
pub fn first_post_body(template: Template, details: &str) -> String {
    let trimmed = details.trim();
    let details_block = if trimmed.is_empty() {
        "_Add your details here._".to_string()
    } else {
        trimmed.to_string()
    };
    match template {
        Template::Question => format!(
            "**What are you trying to do?**\n{details_block}\n\n\
             **What did you try already?**\n_\n\n\
             **What happened instead?**\n_\n\n\
             — posted with Abbey `/forum` (Intelligence Without Limits)"
        ),
        Template::Bug => format!(
            "**Expected**\n_\n\n\
             **Actual**\n{details_block}\n\n\
             **Repro steps**\n1. \n2. \n3. \n\n\
             **Environment** (OS / toolchain / Abbey SHA if known)\n_\n\n\
             — posted with Abbey `/forum` (Intelligence Without Limits)"
        ),
        Template::Build => format!(
            "**Project / surface**\n{details_block}\n\n\
             **Goal**\n_\n\n\
             **Blocked on**\n_\n\n\
             **Links** (repo, logs, screenshots)\n_\n\n\
             — posted with Abbey `/forum` (Intelligence Without Limits)"
        ),
        Template::General => format!(
            "{details_block}\n\n\
             — posted with Abbey `/forum` (Intelligence Without Limits)"
        ),
    }
}

/// Suggest forum tags from title + details against the channel's available names.
///
/// Matching is case-insensitive substring / keyword based. Returns at most five
/// names that exist in `available`, preserving `available` order.
pub fn suggest_tags(title: &str, details: &str, available: &[&str]) -> Vec<String> {
    let haystack = format!("{title}\n{details}").to_ascii_lowercase();
    let mut out = Vec::new();
    for name in available {
        if out.len() >= 5 {
            break;
        }
        let needle = name.to_ascii_lowercase();
        if needle.is_empty() {
            continue;
        }
        let hit = haystack.contains(&needle)
            || keywords_for(name)
                .iter()
                .any(|kw| haystack.split_whitespace().any(|w| w == *kw));
        if hit {
            out.push((*name).to_string());
        }
    }
    out
}

fn keywords_for(tag: &str) -> &'static [&'static str] {
    match tag.to_ascii_lowercase().as_str() {
        "wdbx" => &["wdbx", "hnsw", "vector", "mvcc", "retrieval"],
        "abi" => &["abi", "foundation", "fm26", "apple-intelligence"],
        "abbey" => &["abbey", "bot", "slash", "voice", "persona"],
        "site-builder" => &["site", "builder", "pages", "frontend", "web"],
        "mobile" => &["mobile", "ios", "ipad", "iphone", "android"],
        "apple-silicon" => &["silicon", "m1", "m2", "m3", "m4", "mlx", "metal"],
        "build" => &["build", "compile", "cargo", "ci", "gate", "clippy"],
        _ => &[],
    }
}

/// Clamp a forum post title to Discord's 2..=100 character window.
pub fn clamp_title(raw: &str) -> Result<String, &'static str> {
    let title = raw.trim();
    let chars = title.chars().count();
    if chars < 2 {
        return Err("Forum post titles need at least 2 characters.");
    }
    if chars > 100 {
        return Err("Forum post titles must be 100 characters or fewer.");
    }
    Ok(title.to_string())
}

/// Result of computing a gap-fill for one overwrite target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapFill {
    pub allow: Permissions,
    pub deny: Permissions,
    /// Bits that were added to allow (and cleared from deny when present).
    pub added: Permissions,
}

/// Gap-fill one overwrite: union required into allow, clear those bits from deny.
///
/// Never clears unrelated allow/deny bits. Returns `None` when required is
/// already fully present in allow (deny may still hold other bits).
pub fn gap_fill_overwrite(
    existing_allow: Permissions,
    existing_deny: Permissions,
    required: Permissions,
) -> Option<GapFill> {
    let missing = required - existing_allow;
    if missing.is_empty() {
        return None;
    }
    Some(GapFill {
        allow: existing_allow | required,
        deny: existing_deny - required,
        added: missing,
    })
}

/// Format permission bit names for operator-facing replies.
pub fn permission_labels(bits: Permissions) -> Vec<&'static str> {
    bits.get_permission_names()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_render_iwl_brand_and_sections() {
        for template in [
            Template::Question,
            Template::Bug,
            Template::Build,
            Template::General,
        ] {
            let body = first_post_body(template, "need help with cargo");
            assert!(body.contains("Intelligence Without Limits"), "{body}");
            assert!(body.contains("need help with cargo"), "{body}");
            assert!(!body.to_ascii_lowercase().contains("quesar"), "{body}");
            assert!(!template.slug().is_empty());
            assert!(!template.label().is_empty());
        }
        assert!(first_post_body(Template::General, "   ").contains("_Add your details here._"));
    }

    #[test]
    fn suggest_tags_matches_keywords_and_preserves_available_order() {
        let available = HELP_TAG_NAMES;
        let suggested = suggest_tags("WDBX retrieval slow", "hnsw query latency", available);
        assert_eq!(suggested, vec!["wdbx".to_string()]);
        let multi = suggest_tags(
            "Abbey voice on Apple Silicon",
            "mlx build failed in CI",
            available,
        );
        assert_eq!(
            multi,
            vec![
                "abbey".to_string(),
                "apple-silicon".to_string(),
                "build".to_string()
            ]
        );
        assert!(suggest_tags("hello", "world", available).is_empty());
        assert_eq!(
            suggest_tags(
                "wdbx abi abbey site mobile silicon build extra",
                "",
                available
            )
            .len(),
            5
        );
    }

    #[test]
    fn clamp_title_enforces_discord_bounds() {
        assert!(clamp_title("a").is_err());
        assert_eq!(clamp_title("  ok  ").unwrap(), "ok");
        assert!(clamp_title(&"x".repeat(101)).is_err());
        assert_eq!(clamp_title(&"y".repeat(100)).unwrap().chars().count(), 100);
    }

    #[test]
    fn gap_fill_only_adds_missing_required_bits() {
        let required = required_forum_bot_permissions();
        assert!(gap_fill_overwrite(required, Permissions::empty(), required).is_none());

        let existing_allow = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
        let existing_deny = Permissions::MANAGE_THREADS | Permissions::CREATE_PUBLIC_THREADS;
        let fill = gap_fill_overwrite(existing_allow, existing_deny, required).unwrap();
        assert!(fill.allow.contains(required));
        assert!(fill.allow.contains(Permissions::VIEW_CHANNEL));
        assert!(fill.deny.contains(Permissions::MANAGE_THREADS));
        assert!(!fill.deny.contains(Permissions::CREATE_PUBLIC_THREADS));
        assert!(fill.added.contains(Permissions::CREATE_PUBLIC_THREADS));
        assert!(!fill.added.contains(Permissions::VIEW_CHANNEL));
    }

    #[test]
    fn gap_fill_from_empty_overwrite_is_required_only() {
        let required = required_forum_bot_permissions();
        let fill =
            gap_fill_overwrite(Permissions::empty(), Permissions::empty(), required).unwrap();
        assert_eq!(fill.allow, required);
        assert!(fill.deny.is_empty());
        assert_eq!(fill.added, required);
    }
}
