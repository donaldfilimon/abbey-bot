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
///
/// Roles and members carry their snowflake id alongside the display name:
/// matching happens on the id, rendering on the name. Discord permits two
/// roles with the same name in one guild (divider roles routinely are), so
/// name-based matching would pull a stranger's overwrite into the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Everyone,
    Role {
        id: u64,
        name: String,
    },
    Member {
        id: u64,
        name: String,
    },
    /// An overwrite whose target kind this build does not recognise.
    /// `PermissionOverwriteType` is `#[non_exhaustive]`, so Discord can add
    /// kinds we have never seen. Carrying them explicitly means an unknown
    /// overwrite is reported as unknown, rather than being folded into
    /// `Everyone` and silently claiming the highest-precedence slot.
    Unrecognized,
}

/// One channel permission overwrite, reduced to what the walkthrough needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overwrite {
    pub scope: Scope,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// The member we are evaluating for. Roles are ids for the same reason
/// [`Scope`] carries them: names are not unique.
#[derive(Debug, Clone, Default)]
pub struct Subject {
    pub name: String,
    pub user_id: u64,
    pub role_ids: Vec<u64>,
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
        Scope::Role { id, .. } => subject.role_ids.contains(id),
        _ => false,
    }));

    chain.extend(overwrites.iter().filter(|o| match &o.scope {
        Scope::Member { id, .. } => *id == subject.user_id,
        _ => false,
    }));

    // Last, and never ahead of a real overwrite: we cannot know where Discord
    // would evaluate a kind we do not model, so the one thing we must not do is
    // imply a position we have not established.
    chain.extend(overwrites.iter().filter(|o| o.scope == Scope::Unrecognized));

    chain
}

fn label(scope: &Scope) -> String {
    match scope {
        Scope::Everyone => "@everyone".to_string(),
        Scope::Role { name, .. } => format!("role @{name}"),
        Scope::Member { name, .. } => format!("member {name}"),
        Scope::Unrecognized => "unrecognized overwrite type (not modelled here)".to_string(),
    }
}

/// Render the walkthrough for a channel.
/// `channel_label` arrives pre-formatted ("#general", "🔊 Lobby", …) because
/// only the caller knows the channel kind; a hardcoded `#` here misrendered
/// voice channels and categories.
pub fn explain(channel_label: &str, overwrites: &[Overwrite], subject: &Subject) -> String {
    if subject.is_owner {
        return format!(
            "**{}** in {channel_label}: owner. Overwrites do not apply — ownership bypasses every check.",
            subject.name
        );
    }
    if subject.is_admin {
        return format!(
            "**{}** in {channel_label}: has Administrator. That bypasses all channel overwrites, so nothing below would change the outcome.",
            subject.name
        );
    }

    let chain = applicable(overwrites, subject);
    if chain.is_empty() {
        return format!(
            "**{}** in {channel_label}: no overwrite touches them. Whatever they can or cannot do here comes from guild-level role permissions, not from this channel.",
            subject.name
        );
    }

    let mut out = format!(
        "**{}** in {channel_label} — evaluation order:\n",
        subject.name
    );
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

    // Say only what the chain actually contains. The role rule was previously
    // appended whenever no member overwrite applied — including chains holding
    // nothing but @everyone, where there is no role step to have a rule about.
    let has_member = chain
        .iter()
        .any(|o| matches!(o.scope, Scope::Member { .. }));
    let has_role = chain.iter().any(|o| matches!(o.scope, Scope::Role { .. }));
    if has_member {
        out.push_str("\nA member overwrite is present, and it is applied last — it overrides every role result above it.");
    } else if has_role {
        out.push_str("\nNo member overwrite, so the role denies union before the role allows do. Role position is irrelevant at this step.");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMBER_ROLE: u64 = 1;
    const FRANKIE: u64 = 42;

    fn subject() -> Subject {
        Subject {
            name: "frankie".to_string(),
            user_id: FRANKIE,
            role_ids: vec![MEMBER_ROLE],
            ..Default::default()
        }
    }

    fn role(id: u64, name: &str, allow: &[&str], deny: &[&str]) -> Overwrite {
        Overwrite {
            scope: Scope::Role {
                id,
                name: name.to_string(),
            },
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
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
        assert!(explain("#general", &ows, &owner).contains("bypasses every check"));

        let admin = Subject {
            is_admin: true,
            ..subject()
        };
        let out = explain("#general", &ows, &admin);
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
            role(9, "Moderator", &["View Channel"], &[]),
        ];
        let chain = applicable(&ows, &subject());
        assert_eq!(chain.len(), 1, "only @everyone applies to a plain Member");
        assert_eq!(
            chain[0].scope,
            Scope::Everyone,
            "and it is the @everyone one"
        );
    }

    #[test]
    fn evaluation_order_is_imposed_not_inherited_from_storage() {
        // Stored member-first, everyone-last — the reverse of evaluation order.
        let ows = [
            Overwrite {
                scope: Scope::Member {
                    id: FRANKIE,
                    name: "frankie".to_string(),
                },
                allow: vec!["View Channel".to_string()],
                deny: vec![],
            },
            role(MEMBER_ROLE, "Member", &[], &["Send Messages"]),
            everyone_denies_view(),
        ];
        let chain = applicable(&ows, &subject());
        assert_eq!(chain[0].scope, Scope::Everyone);
        assert!(matches!(
            chain[1].scope,
            Scope::Role {
                id: MEMBER_ROLE,
                ..
            }
        ));
        assert!(matches!(chain[2].scope, Scope::Member { id: FRANKIE, .. }));
    }

    #[test]
    fn a_member_overwrite_is_called_out_as_final() {
        let ows = [
            everyone_denies_view(),
            Overwrite {
                scope: Scope::Member {
                    id: FRANKIE,
                    name: "frankie".to_string(),
                },
                allow: vec!["View Channel".to_string()],
                deny: vec![],
            },
        ];
        let out = explain("secret", &ows, &subject());
        assert!(out.contains("applied last"), "{out}");
    }

    #[test]
    fn without_a_member_overwrite_the_role_rule_is_explained() {
        // Must actually contain a role overwrite: the previous version of this
        // test passed a lone @everyone deny, so it asserted a rule about a step
        // the chain never reached.
        let ows = [
            everyone_denies_view(),
            role(MEMBER_ROLE, "Member", &["View Channel"], &[]),
        ];
        let out = explain("#general", &ows, &subject());
        assert!(out.contains("Role position is irrelevant"), "{out}");
    }

    #[test]
    fn the_role_rule_is_omitted_when_no_role_overwrite_applies() {
        let out = explain("#general", &[everyone_denies_view()], &subject());
        assert!(
            !out.contains("Role position is irrelevant"),
            "no role overwrite is in this chain: {out}"
        );
    }

    #[test]
    fn a_duplicate_role_name_cannot_smuggle_in_a_strangers_overwrite() {
        // Discord permits two roles named identically. The subject holds role
        // id 2; this overwrite belongs to role id 7 with the same display name.
        // Matching by name — the previous implementation — pulled it in.
        let ows = [role(7, "Muted", &[], &["Send Messages"])];
        let holder_of_other_muted = Subject {
            name: "frankie".to_string(),
            user_id: FRANKIE,
            role_ids: vec![2],
            ..Default::default()
        };
        let chain = applicable(&ows, &holder_of_other_muted);
        assert!(chain.is_empty(), "id 7 does not apply to a holder of id 2");
    }

    #[test]
    fn no_applicable_overwrite_points_at_guild_level_instead() {
        let ows = [role(9, "Moderator", &["View Channel"], &[])];
        let out = explain("#general", &ows, &subject());
        assert!(out.contains("guild-level role permissions"), "{out}");
    }

    #[test]
    fn an_unrecognized_overwrite_never_impersonates_everyone() {
        // Regression: the command layer used to map serenity's non_exhaustive
        // wildcard onto Scope::Everyone, which put an unknown overwrite at
        // position 1 and printed it as literally "@everyone" — misattributed and
        // promoted above the overwrites that actually decide the outcome.
        let ows = [
            Overwrite {
                scope: Scope::Unrecognized,
                allow: vec!["View Channel".to_string()],
                deny: vec![],
            },
            everyone_denies_view(),
        ];
        let chain = applicable(&ows, &subject());
        assert_eq!(
            chain[0].scope,
            Scope::Everyone,
            "a real overwrite goes first"
        );
        assert_eq!(chain[1].scope, Scope::Unrecognized);

        let out = explain("#general", &ows, &subject());
        assert!(out.contains("unrecognized overwrite type"), "{out}");
    }

    #[test]
    fn an_empty_overwrite_is_labelled_rather_than_rendered_blank() {
        let ows = [Overwrite {
            scope: Scope::Everyone,
            allow: vec![],
            deny: vec![],
        }];
        assert!(explain("#general", &ows, &subject()).contains("cosmetic"));
    }
}
