//! Profile reads.
//!
//! The skill's "Reading a Person" order is identity → presence → relationship →
//! standing → vibe, and its output rule is a tight paragraph rather than a field
//! dump. Both are implemented here over a plain struct, so the wording is
//! testable without a gateway connection.
//!
//! One deliberate omission: **presence is not reported.** Status dot and activity
//! require the `GUILD_PRESENCES` privileged intent, which this bot does not
//! request. Saying "offline" when we simply cannot see is worse than saying
//! nothing, so the summary states the limit instead of inventing a value.

/// What a profile read can actually establish from non-privileged data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileFacts {
    /// Server nickname if set, otherwise the global display name.
    pub display_name: String,
    /// The `@handle`, without the leading `@`.
    pub handle: String,
    /// Discord's own BOT tag.
    pub is_bot: bool,
    /// Server nickname, when it differs from the global name.
    pub nickname: Option<String>,
    /// Role names, excluding `@everyone`, highest first.
    pub roles: Vec<String>,
    /// Join date, already formatted for display.
    pub joined: Option<String>,
    /// Whether this member owns the guild.
    pub is_owner: bool,
}

/// Standing markers worth calling out, in the skill's priority order.
fn standing(facts: &ProfileFacts) -> Option<String> {
    if facts.is_owner {
        return Some("owns this server".to_string());
    }
    // `roles` is highest-first, so the first entry is the top role.
    facts.roles.first().map(|top| format!("top role {top}"))
}

/// Render a profile read as one tight paragraph.
///
/// A bot account short-circuits: the skill is explicit that a `BOT` tag means
/// software, not a social actor, so reading it for vibe and standing would be
/// theatre.
pub fn summarize(facts: &ProfileFacts) -> String {
    if facts.is_bot {
        return format!(
            "**{}** (`@{}`) — BOT. Software, not a social actor; no profile read.",
            facts.display_name, facts.handle
        );
    }

    let mut out = format!("**{}** (`@{}`)", facts.display_name, facts.handle);

    // Only worth saying when it differs — otherwise it repeats the name we just
    // printed.
    if let Some(nick) = &facts.nickname
        && nick != &facts.display_name
    {
        out.push_str(&format!(", server nick *{nick}*"));
    }

    if let Some(s) = standing(facts) {
        out.push_str(&format!(" — {s}"));
    }

    // "of N roles" reads as a qualifier on "top role X" but is ungrammatical
    // after "owns this server", which is a different kind of clause.
    match (facts.is_owner, facts.roles.len()) {
        (_, 0) => out.push_str(", no roles"),
        (_, 1) => {}
        (true, n) => out.push_str(&format!(", {n} roles")),
        (false, n) => out.push_str(&format!(" of {n} roles")),
    }

    if let Some(joined) = &facts.joined {
        out.push_str(&format!(". Joined {joined}"));
    }

    out.push('.');
    // Stated, not silently omitted: the read is bounded by the intents we hold.
    out.push_str(" Presence and activity unavailable — the presences intent is not enabled.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ProfileFacts {
        ProfileFacts {
            display_name: "frankie".to_string(),
            handle: "frankie87".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn bot_accounts_short_circuit() {
        let facts = ProfileFacts {
            is_bot: true,
            roles: vec!["Admin".to_string()],
            is_owner: true,
            ..base()
        };
        let out = summarize(&facts);
        assert!(out.contains("BOT"), "{out}");
        // Standing must not leak into a bot read.
        assert!(!out.contains("owns this server"), "{out}");
    }

    #[test]
    fn owner_outranks_top_role() {
        let facts = ProfileFacts {
            is_owner: true,
            roles: vec!["Moderator".to_string()],
            ..base()
        };
        assert!(summarize(&facts).contains("owns this server"));
    }

    #[test]
    fn reports_top_role_when_not_owner() {
        let facts = ProfileFacts {
            roles: vec!["Moderator".to_string(), "Booster".to_string()],
            ..base()
        };
        let out = summarize(&facts);
        assert!(out.contains("top role Moderator"), "{out}");
        assert!(out.contains("of 2 roles"), "{out}");
    }

    #[test]
    fn never_claims_a_presence_it_cannot_see() {
        let out = summarize(&base());
        assert!(out.contains("Presence and activity unavailable"), "{out}");
        // Lowercased first: Discord's own status names are capitalised (Online,
        // Idle, Do Not Disturb), so a case-sensitive check tests the spellings
        // a realistic bug would NOT use. Mutation-confirmed: injecting
        // " Status: Online." passed the old version of this assertion.
        let lowered = out.to_lowercase();
        for invented in ["online", "idle", "offline", "do not disturb"] {
            assert!(
                !lowered.contains(invented),
                "invented presence {invented}: {out}"
            );
        }
    }

    #[test]
    fn a_nickname_equal_to_the_display_name_is_not_repeated() {
        let facts = ProfileFacts {
            nickname: Some("frankie".to_string()),
            ..base()
        };
        assert!(!summarize(&facts).contains("server nick"));
    }

    #[test]
    fn a_nickname_differing_from_the_display_name_is_shown() {
        // The positive branch had no test at all: deleting the whole `if let`
        // block left the suite green.
        let facts = ProfileFacts {
            nickname: Some("frank".to_string()),
            ..base()
        };
        let out = summarize(&facts);
        assert!(out.contains("server nick *frank*"), "{out}");
    }

    #[test]
    fn an_owner_with_several_roles_reads_grammatically() {
        let facts = ProfileFacts {
            is_owner: true,
            roles: vec!["Admin".to_string(), "Booster".to_string()],
            ..base()
        };
        let out = summarize(&facts);
        assert!(!out.contains("server of 2 roles"), "ungrammatical: {out}");
        assert!(out.contains("owns this server, 2 roles"), "{out}");
    }

    #[test]
    fn roleless_member_is_stated_rather_than_left_blank() {
        assert!(summarize(&base()).contains("no roles"));
    }
}
