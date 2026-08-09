//! Permission override evaluation.
//!
//! Answers the skill's "Why can't I see X" case. Discord resolves channel
//! permissions in a fixed order, and the usual reason someone is confused is that
//! they are reading the overwrites as a flat list instead of as that order:
//!
//! 1. Base permissions from `@everyone` plus the member's roles, at guild level.
//! 2. The channel's `@everyone` overwrite — deny applied, then allow.
//! 3. All applicable *role* overwrites — the denies unioned, then the allows.
//!    Role position does not matter here, which is the step people get wrong.
//! 4. The member-specific overwrite — deny, then allow. This wins outright.
//!
//! Administrator short-circuits the whole thing, which is why it is checked first
//! rather than folded into the walkthrough.

/// Who an overwrite targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Everyone,
    Role(String),
    Member(String),
}

/// One channel permission overwrite, reduced to what the walkthrough needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overwrite {
    pub scope: Scope,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// The member we are evaluating for.
#[derive(Debug, Clone, Default)]
pub struct Subject {
    pub name: String,
    pub role_names: Vec<String>,
    pub is_admin: bool,
    pub is_owner: bool,
}

/// Select the overwrites that actually bear on this member, in evaluation order.
///
/// Note this is order-defining, not merely filtering: role overwrites are
/// returned in Discord's evaluation position regardless of the order the channel
/// happens to store them in.
pub fn applicable<'a>(overwrites: &'a [Overwrite], subject: &Subject) -> Vec<&'a Overwrite> {
    let mut chain: Vec<&Overwrite> = Vec::new();

    chain.extend(overwrites.iter().filter(|o| o.scope == Scope::Everyone));

    chain.extend(overwrites.iter().filter(|o| match &o.scope {
        Scope::Role(name) => subject.role_names.iter().any(|r| r == name),
        _ => false,
    }));

    chain.extend(overwrites.iter().filter(|o| match &o.scope {
        Scope::Member(name) => name == &subject.name,
        _ => false,
    }));

    chain
}

fn label(scope: &Scope) -> String {
    match scope {
        Scope::Everyone => "@everyone".to_string(),
        Scope::Role(name) => format!("role @{name}"),
        Scope::Member(name) => format!("member {name}"),
    }
}

/// Render the walkthrough for a channel.
pub fn explain(channel: &str, overwrites: &[Overwrite], subject: &Subject) -> String {
    if subject.is_owner {
        return format!(
            "**{}** in #{channel}: owner. Overwrites do not apply — ownership bypasses every check.",
            subject.name
        );
    }
    if subject.is_admin {
        return format!(
            "**{}** in #{channel}: has Administrator. That bypasses all channel overwrites, so nothing below would change the outcome.",
            subject.name
        );
    }

    let chain = applicable(overwrites, subject);
    if chain.is_empty() {
        return format!(
            "**{}** in #{channel}: no overwrite touches them. Whatever they can or cannot do here comes from guild-level role permissions, not from this channel.",
            subject.name
        );
    }

    let mut out = format!("**{}** in #{channel} — evaluation order:\n", subject.name);
    for (i, ow) in chain.iter().enumerate() {
        let mut parts = Vec::new();
        if !ow.deny.is_empty() {
            parts.push(format!("denies {}", ow.deny.join(", ")));
        }
        if !ow.allow.is_empty() {
            parts.push(format!("allows {}", ow.allow.join(", ")));
        }
        let body = if parts.is_empty() {
            "no bits set (an empty overwrite — cosmetic)".to_string()
        } else {
            parts.join("; ")
        };
        out.push_str(&format!("{}. {} {}\n", i + 1, label(&ow.scope), body));
    }

    if chain.iter().any(|o| matches!(o.scope, Scope::Member(_))) {
        out.push_str("\nA member overwrite is present, and it is applied last — it overrides every role result above it.");
    } else {
        out.push_str("\nNo member overwrite, so the role denies union before the role allows do. Role position is irrelevant at this step.");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> Subject {
        Subject {
            name: "frankie".to_string(),
            role_names: vec!["Member".to_string()],
            ..Default::default()
        }
    }

    fn everyone_denies_view() -> Overwrite {
        Overwrite {
            scope: Scope::Everyone,
            allow: vec![],
            deny: vec!["View Channel".to_string()],
        }
    }

    #[test]
    fn owner_and_admin_short_circuit_before_any_overwrite() {
        let ows = [everyone_denies_view()];
        let owner = Subject {
            is_owner: true,
            ..subject()
        };
        assert!(explain("general", &ows, &owner).contains("bypasses every check"));

        let admin = Subject {
            is_admin: true,
            ..subject()
        };
        let out = explain("general", &ows, &admin);
        assert!(out.contains("Administrator"), "{out}");
        assert!(
            !out.contains("1."),
            "admin walkthrough should not enumerate: {out}"
        );
    }

    #[test]
    fn unrelated_role_overwrites_are_excluded() {
        let ows = [
            everyone_denies_view(),
            Overwrite {
                scope: Scope::Role("Moderator".to_string()),
                allow: vec!["View Channel".to_string()],
                deny: vec![],
            },
        ];
        let chain = applicable(&ows, &subject());
        assert_eq!(chain.len(), 1, "only @everyone applies to a plain Member");
    }

    #[test]
    fn evaluation_order_is_imposed_not_inherited_from_storage() {
        // Stored member-first, everyone-last — the reverse of evaluation order.
        let ows = [
            Overwrite {
                scope: Scope::Member("frankie".to_string()),
                allow: vec!["View Channel".to_string()],
                deny: vec![],
            },
            Overwrite {
                scope: Scope::Role("Member".to_string()),
                allow: vec![],
                deny: vec!["Send Messages".to_string()],
            },
            everyone_denies_view(),
        ];
        let chain = applicable(&ows, &subject());
        assert_eq!(chain[0].scope, Scope::Everyone);
        assert_eq!(chain[1].scope, Scope::Role("Member".to_string()));
        assert_eq!(chain[2].scope, Scope::Member("frankie".to_string()));
    }

    #[test]
    fn a_member_overwrite_is_called_out_as_final() {
        let ows = [
            everyone_denies_view(),
            Overwrite {
                scope: Scope::Member("frankie".to_string()),
                allow: vec!["View Channel".to_string()],
                deny: vec![],
            },
        ];
        let out = explain("secret", &ows, &subject());
        assert!(out.contains("applied last"), "{out}");
    }

    #[test]
    fn without_a_member_overwrite_the_role_rule_is_explained() {
        let out = explain("general", &[everyone_denies_view()], &subject());
        assert!(out.contains("Role position is irrelevant"), "{out}");
    }

    #[test]
    fn no_applicable_overwrite_points_at_guild_level_instead() {
        let ows = [Overwrite {
            scope: Scope::Role("Moderator".to_string()),
            allow: vec!["View Channel".to_string()],
            deny: vec![],
        }];
        let out = explain("general", &ows, &subject());
        assert!(out.contains("guild-level role permissions"), "{out}");
    }

    #[test]
    fn an_empty_overwrite_is_labelled_rather_than_rendered_blank() {
        let ows = [Overwrite {
            scope: Scope::Everyone,
            allow: vec![],
            deny: vec![],
        }];
        assert!(explain("general", &ows, &subject()).contains("cosmetic"));
    }
}
