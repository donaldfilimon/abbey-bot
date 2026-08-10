//! Server blueprints — role hierarchy, channel structure, permission matrix.
//!
//! The skill's output rule for a server setup is "role/channel hierarchy then
//! numbered steps", which is what [`render`] produces.
//!
//! The reason this is a module rather than a block of prose is that a blueprint
//! has *checkable* properties, and the checks catch the mistakes people actually
//! make when hand-writing one:
//!
//! - **Text channel names are not free-form.** Discord lowercases them and
//!   replaces whitespace with hyphens, so a blueprint saying `General Chat`
//!   silently becomes `general-chat` and the setup steps no longer match the
//!   server. Voice channels have no such rule, which is exactly the asymmetry
//!   that trips people. [`normalize_text_name`] encodes it and a test asserts
//!   every text channel here is already in its final form.
//! - **Gating a channel to a role that the blueprint never creates** produces a
//!   setup you cannot follow. A property test walks every archetype and asserts
//!   referential integrity.
//! - **`@everyone` must never carry a dangerous permission.** It is the one role
//!   nobody can be removed from.

/// The kind of server being built. Chosen by a human; this module does not infer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    /// Public, open-join, needs moderation depth and a rules funnel.
    Community,
    /// Gaming group: voice-first, LFG, fewer text channels.
    Gaming,
    /// Work or project server: structured, low noise, no social channels.
    Project,
    /// Small friend group: flat, minimal roles.
    FriendGroup,
}

impl Archetype {
    /// Every archetype, so property tests cannot silently miss one that is added
    /// later. Test-only: nothing in the running bot iterates archetypes.
    #[cfg(test)]
    pub const ALL: [Self; 4] = [
        Self::Community,
        Self::Gaming,
        Self::Project,
        Self::FriendGroup,
    ];
}

/// A role, with the permissions that justify it existing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSpec {
    pub name: &'static str,
    /// Highest first; index 0 sits at the top of the hierarchy.
    pub permissions: &'static [&'static str],
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
    Forum,
}

/// A channel, and the role it is gated to if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSpec {
    pub name: &'static str,
    pub kind: ChannelKind,
    /// When set, `@everyone` is denied View Channel here and this role is
    /// allowed it. Must name a role the same blueprint creates.
    pub gated_to: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub name: &'static str,
    pub channels: &'static [ChannelSpec],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blueprint {
    pub roles: &'static [RoleSpec],
    pub categories: &'static [Category],
    /// What `@everyone` is set to. Data, not a hardcoded render line: the
    /// baseline differs per archetype — a gated community strips it to
    /// view-only, while a flat friend server *is* its `@everyone` grants —
    /// and making it data is what lets the dangerous-permission property test
    /// check something that can actually vary.
    pub everyone: &'static [&'static str],
}

/// Discord caps a category at 50 channels. A specification constant: the
/// property test below is what enforces it, so it is test-only rather than
/// carried as dead weight in the binary.
#[cfg(test)]
pub const MAX_CHANNELS_PER_CATEGORY: usize = 50;

/// Permissions `@everyone` must never hold. Each one lets any member escalate or
/// wreck the server, and `@everyone` is the one role membership is not optional.
#[cfg(test)]
pub const NEVER_FOR_EVERYONE: &[&str] = &[
    "Administrator",
    // serenity emits "Manage Guild"; the Discord client labels it
    // "Manage Server". Match what get_permission_names actually returns,
    // or the entry can never fire.
    "Manage Guild",
    "Manage Roles",
    "Manage Channels",
    "Manage Webhooks",
    "Ban Members",
    "Kick Members",
    "Mention Everyone",
];

/// The gated archetypes strip `@everyone` to read-only; roles grant the rest.
const EVERYONE_READ_ONLY: &[&str] = &["View Channel", "Read Message History"];

/// Apply Discord's text-channel naming rule: lowercase, whitespace to hyphens.
///
/// Discord does this server-side whether you like it or not, so a blueprint that
/// does not already satisfy it describes a server you will not get.
pub fn normalize_text_name(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

const fn text(name: &'static str, gated_to: Option<&'static str>) -> ChannelSpec {
    ChannelSpec {
        name,
        kind: ChannelKind::Text,
        gated_to,
    }
}

const fn voice(name: &'static str, gated_to: Option<&'static str>) -> ChannelSpec {
    ChannelSpec {
        name,
        kind: ChannelKind::Voice,
        gated_to,
    }
}

const fn forum(name: &'static str, gated_to: Option<&'static str>) -> ChannelSpec {
    ChannelSpec {
        name,
        kind: ChannelKind::Forum,
        gated_to,
    }
}

const COMMUNITY_ROLES: &[RoleSpec] = &[
    RoleSpec {
        name: "Admin",
        permissions: &["Administrator"],
        note: "Owner's deputies only. Administrator bypasses every channel overwrite, so keep this list short.",
    },
    RoleSpec {
        name: "Moderator",
        permissions: &["Manage Messages", "Moderate Members", "Kick Members"],
        note: "Deliberately no Ban Members — banning is an Admin call.",
    },
    RoleSpec {
        name: "Member",
        permissions: &["Send Messages", "Connect", "Speak"],
        note: "Granted after the rules gate. This is what makes the gate mean something.",
    },
];

const COMMUNITY_CATEGORIES: &[Category] = &[
    Category {
        name: "Start Here",
        channels: &[text("rules", None), text("announcements", None)],
    },
    Category {
        name: "Community",
        channels: &[
            text("general", Some("Member")),
            text("introductions", Some("Member")),
            forum("help", Some("Member")),
        ],
    },
    Category {
        name: "Staff",
        channels: &[
            text("mod-log", Some("Moderator")),
            text("mod-chat", Some("Moderator")),
        ],
    },
];

const GAMING_ROLES: &[RoleSpec] = &[
    RoleSpec {
        name: "Admin",
        permissions: &["Administrator"],
        note: "Server owner and one backup.",
    },
    RoleSpec {
        name: "Moderator",
        permissions: &["Manage Messages", "Moderate Members", "Move Members"],
        note: "Move Members matters here — most incidents happen in voice.",
    },
    RoleSpec {
        name: "Member",
        permissions: &["Send Messages", "Connect", "Speak", "Stream"],
        note: "Stream is the point of a gaming server; grant it at the base role.",
    },
];

const GAMING_CATEGORIES: &[Category] = &[
    Category {
        name: "Text",
        channels: &[
            // Ungated on purpose: the landing channel. Without one, a new
            // joiner holding only @everyone sees an empty server and has no
            // way to find out how Member is granted.
            text("welcome", None),
            text("general", Some("Member")),
            text("lfg", Some("Member")),
            text("clips", Some("Member")),
        ],
    },
    Category {
        name: "Voice",
        channels: &[
            voice("Lobby", Some("Member")),
            voice("Squad 1", Some("Member")),
            voice("Squad 2", Some("Member")),
            voice("AFK", Some("Member")),
        ],
    },
    Category {
        name: "Staff",
        channels: &[text("mod-chat", Some("Moderator"))],
    },
];

const PROJECT_ROLES: &[RoleSpec] = &[
    RoleSpec {
        name: "Admin",
        permissions: &["Administrator"],
        note: "Project lead.",
    },
    RoleSpec {
        name: "Contributor",
        permissions: &["Send Messages", "Connect", "Speak", "Create Public Threads"],
        note: "Threads keep long discussions out of the main channel.",
    },
];

const PROJECT_CATEGORIES: &[Category] = &[
    Category {
        name: "Project",
        channels: &[
            text("announcements", None),
            text("general", Some("Contributor")),
            forum("decisions", Some("Contributor")),
        ],
    },
    Category {
        name: "Work",
        channels: &[
            text("standup", Some("Contributor")),
            text("ci", Some("Contributor")),
            voice("Pairing", Some("Contributor")),
        ],
    },
];

const FRIEND_ROLES: &[RoleSpec] = &[RoleSpec {
    name: "Admin",
    permissions: &["Administrator"],
    note: "Whoever made the server. A friend group does not need a hierarchy.",
}];

const FRIEND_CATEGORIES: &[Category] = &[Category {
    name: "Hangout",
    channels: &[
        text("general", None),
        text("media", None),
        voice("Voice", None),
    ],
}];

/// The blueprint for an archetype.
pub const fn blueprint(archetype: Archetype) -> Blueprint {
    match archetype {
        Archetype::Community => Blueprint {
            roles: COMMUNITY_ROLES,
            categories: COMMUNITY_CATEGORIES,
            everyone: EVERYONE_READ_ONLY,
        },
        Archetype::Gaming => Blueprint {
            roles: GAMING_ROLES,
            categories: GAMING_CATEGORIES,
            everyone: EVERYONE_READ_ONLY,
        },
        Archetype::Project => Blueprint {
            roles: PROJECT_ROLES,
            categories: PROJECT_CATEGORIES,
            everyone: EVERYONE_READ_ONLY,
        },
        Archetype::FriendGroup => Blueprint {
            roles: FRIEND_ROLES,
            categories: FRIEND_CATEGORIES,
            // Flat is the design: nothing is gated, so members must be able to
            // talk on the base role alone. Stripping @everyone to read-only
            // here — the old hardcoded render line — described a server where
            // every friend could see the channels and nobody could speak.
            everyone: &[
                "View Channel",
                "Read Message History",
                "Send Messages",
                "Connect",
                "Speak",
            ],
        },
    }
}

/// The prefix a channel is shown with. `#` is Discord's own text convention and
/// reads correctly flush against the name; the emoji markers need a space or the
/// name collides with the glyph.
fn kind_marker(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Text => "#",
        ChannelKind::Voice => "🔊 ",
        ChannelKind::Forum => "🗂 ",
    }
}

/// Render a blueprint: hierarchy, structure, then numbered steps.
pub fn render(archetype: Archetype) -> String {
    let bp = blueprint(archetype);
    let mut out = String::new();

    out.push_str("**Role hierarchy** (highest first)\n");
    for role in bp.roles {
        out.push_str(&format!(
            "- **{}** — {}\n  {}\n",
            role.name,
            role.permissions.join(", "),
            role.note
        ));
    }
    out.push_str(&format!(
        "- **@everyone** — {}\n  Everything else is granted by a role, never here.\n",
        bp.everyone.join(", ")
    ));

    out.push_str("\n**Channels**\n");
    for category in bp.categories {
        out.push_str(&format!("- {}\n", category.name));
        for channel in category.channels {
            let gate = match channel.gated_to {
                Some(role) => format!(" — {role}+ only"),
                None => String::new(),
            };
            // Text and forum names go out in the form Discord will actually
            // store, so the steps below always match the resulting server even
            // if a blueprint table drifts.
            let name = match channel.kind {
                ChannelKind::Text | ChannelKind::Forum => normalize_text_name(channel.name),
                ChannelKind::Voice => channel.name.to_string(),
            };
            out.push_str(&format!(
                "  - {}{}{}\n",
                kind_marker(channel.kind),
                name,
                gate
            ));
        }
    }

    out.push_str("\n**Steps**\n");
    out.push_str("1. Create the roles top-down, in the order above — a role can only manage roles below its own position.\n");
    out.push_str(&format!(
        "2. Set `@everyone` to exactly: {}. Everything else comes from a role.\n",
        bp.everyone.join(", ")
    ));
    out.push_str("3. Create the categories, then the channels inside them. Channels inherit category overwrites, so set the gate on the category where every child shares it.\n");
    out.push_str("4. For each gated channel: deny `@everyone` View Channel, allow the named role. Do not grant the role Administrator as a shortcut — it bypasses every overwrite you just set.\n");
    out.push_str("5. Verify with `/perms` on a test account before inviting anyone.\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gated_channel_names_a_role_the_blueprint_creates() {
        for archetype in Archetype::ALL {
            let bp = blueprint(archetype);
            for category in bp.categories {
                for channel in category.channels {
                    if let Some(gate) = channel.gated_to {
                        assert!(
                            bp.roles.iter().any(|r| r.name == gate),
                            "{archetype:?}: {} is gated to {gate:?}, which the blueprint never creates",
                            channel.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn text_channel_names_are_already_in_discords_final_form() {
        // A blueprint saying "General Chat" describes a server you do not get:
        // Discord stores it as "general-chat" and the steps stop matching.
        for archetype in Archetype::ALL {
            for category in blueprint(archetype).categories {
                for channel in category.channels {
                    if matches!(channel.kind, ChannelKind::Text | ChannelKind::Forum) {
                        assert_eq!(
                            normalize_text_name(channel.name),
                            channel.name,
                            "{archetype:?}: Discord would rewrite this name"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn voice_channels_are_exempt_from_the_text_naming_rule() {
        // Asserted through render(), not the blueprint tables. A mutation check
        // showed the old version — which only compared table entries against
        // their own normalized form — stayed green when render's voice arm was
        // changed to normalize, i.e. when the exemption was actually broken.
        let out = render(Archetype::Gaming);
        assert!(out.contains("Squad 1"), "voice name was rewritten: {out}");
        assert!(!out.contains("squad-1"), "voice name was normalized: {out}");
        // The text rule still applies in the same render, so this pins the
        // asymmetry rather than just one half of it.
        assert!(out.contains("#general"), "{out}");
    }

    #[test]
    fn no_blueprint_hands_everyone_a_dangerous_permission() {
        // Checks the blueprint DATA, not a rendered literal. The old version
        // read a hardcoded render line that was byte-identical across all four
        // archetypes, so no blueprint edit could ever trip it.
        for archetype in Archetype::ALL {
            for granted in blueprint(archetype).everyone {
                assert!(
                    !NEVER_FOR_EVERYONE.contains(granted),
                    "{archetype:?} grants @everyone {granted}"
                );
            }
        }
    }

    #[test]
    fn every_blueprint_gives_a_roleless_joiner_somewhere_to_land() {
        // A member holding only @everyone must be able to see at least one
        // channel, or the setup steps produce a server that looks empty to
        // every new joiner. Gaming shipped exactly that until this test.
        for archetype in Archetype::ALL {
            let ungated = blueprint(archetype)
                .categories
                .iter()
                .flat_map(|c| c.channels)
                .any(|ch| ch.gated_to.is_none());
            assert!(ungated, "{archetype:?} gates every channel");
        }
    }

    #[test]
    fn a_flat_blueprint_lets_the_base_role_speak() {
        // FriendGroup gates nothing and creates no granting role, so if
        // @everyone cannot send, nobody but the Admin can talk at all.
        let bp = blueprint(Archetype::FriendGroup);
        assert!(bp.everyone.contains(&"Send Messages"), "{:?}", bp.everyone);
        let no_gates = bp
            .categories
            .iter()
            .flat_map(|c| c.channels)
            .all(|ch| ch.gated_to.is_none());
        assert!(
            no_gates,
            "a gate would need a granting role this blueprint lacks"
        );
    }

    #[test]
    fn role_names_are_unique_within_a_blueprint() {
        for archetype in Archetype::ALL {
            let roles = blueprint(archetype).roles;
            for (i, role) in roles.iter().enumerate() {
                assert!(
                    !roles[..i].iter().any(|r| r.name == role.name),
                    "{archetype:?} defines {} twice",
                    role.name
                );
            }
        }
    }

    #[test]
    fn no_category_exceeds_discords_channel_limit() {
        for archetype in Archetype::ALL {
            for category in blueprint(archetype).categories {
                assert!(
                    category.channels.len() <= MAX_CHANNELS_PER_CATEGORY,
                    "{archetype:?}/{} has {} channels",
                    category.name,
                    category.channels.len()
                );
            }
        }
    }

    #[test]
    fn every_archetype_renders_hierarchy_structure_and_steps() {
        for archetype in Archetype::ALL {
            let out = render(archetype);
            for section in ["Role hierarchy", "Channels", "Steps"] {
                assert!(out.contains(section), "{archetype:?} missing {section}");
            }
        }
    }

    #[test]
    fn normalization_matches_discords_rule() {
        assert_eq!(normalize_text_name("General Chat"), "general-chat");
        assert_eq!(normalize_text_name("  Mod   Log  "), "mod-log");
        assert_eq!(normalize_text_name("already-fine"), "already-fine");
    }

    #[test]
    fn a_friend_group_stays_flat() {
        // The archetype exists to *not* impose a hierarchy; if this grows roles,
        // it has stopped being the thing it is for.
        assert_eq!(blueprint(Archetype::FriendGroup).roles.len(), 1);
    }
}
