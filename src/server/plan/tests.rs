use super::*;
use crate::server::{Archetype, blueprint};

fn mlai() -> Plan {
    Plan::from_toml(MLAI_COMMUNITY).expect("shipped plan validates")
}

fn minimal(extra: &str) -> String {
    format!(
        r#"
name = "t"
[[roles]]
name = "Member"
[[categories]]
name = "Main"
[[categories.channels]]
name = "general"
kind = "text"
{extra}
"#
    )
}

#[test]
fn every_archetype_converts_and_validates() {
    for archetype in Archetype::ALL {
        let plan = Plan::from(&blueprint(archetype));
        plan.validate()
            .unwrap_or_else(|problems| panic!("{archetype:?}: {problems:?}"));
    }
}

#[test]
fn archetype_gating_becomes_the_overwrite_pair_the_steps_describe() {
    let plan = Plan::from(&blueprint(Archetype::Community));
    let (category, channel) = plan
        .channels()
        .find(|(_, ch)| ch.name == "mod-log")
        .expect("community has mod-log");
    let overwrites = plan.effective_overwrites(category, channel);
    assert!(denies_everyone_view(overwrites));
    assert!(
        overwrites
            .iter()
            .any(|o| o.role == "Moderator" && o.allow == [VIEW_CHANNEL])
    );
    let (category, rules) = plan.channels().find(|(_, ch)| ch.name == "rules").unwrap();
    let rules = plan.effective_overwrites(category, rules);
    assert!(!denies_everyone_view(rules), "newcomers must see rules");
    assert!(
        rules
            .iter()
            .any(|o| o.role == EVERYONE && o.deny.iter().any(|p| p == "Send Messages"))
    );
}

#[test]
fn the_shipped_mlai_plan_parses_and_validates() {
    let plan = mlai();
    assert_eq!(plan.name, "MLAI");
    assert_eq!(
        plan.categories.len(),
        8,
        "START HERE, COMMONS, THE STACK, BUILD LOG, PRODUCTS, VOICE, STAFF, ARCHIVE"
    );
    assert_eq!(plan.roles.len(), 12);
    assert!(
        plan.needs_community(),
        "forums, announcements, and a stage need COMMUNITY"
    );
}

#[test]
fn mlai_hides_exactly_the_channels_the_proposal_hides() {
    let plan = mlai();
    let hidden: Vec<&str> = plan
        .channels()
        .filter(|(c, ch)| denies_everyone_view(plan.effective_overwrites(c, ch)))
        .map(|(_, ch)| ch.name.as_str())
        .collect();
    assert_eq!(
        hidden,
        [
            "ci-and-deploys",
            "ops-console",
            "mod-chat",
            "mod-log",
            "staging",
            "server-updates"
        ]
    );
    let staff = plan.category("STAFF").unwrap();
    assert!(
        staff.channels.iter().all(|ch| ch.overwrites.is_empty()),
        "STAFF children inherit (sync left on)"
    );
    assert!(denies_everyone_view(&staff.overwrites));
}

#[test]
fn mlai_read_only_channels_block_every_posting_path() {
    let plan = mlai();
    for name in ["welcome", "rules", "announcements", "releases"] {
        let (category, channel) = plan.channels().find(|(_, ch)| ch.name == name).unwrap();
        let everyone = plan
            .effective_overwrites(category, channel)
            .iter()
            .find(|o| o.role == EVERYONE)
            .unwrap_or_else(|| panic!("{name} has no @everyone overwrite"));
        for permission in [
            "Send Messages",
            "Create Public Threads",
            "Create Private Threads",
            "Send Messages in Threads",
        ] {
            assert!(
                everyone.deny.iter().any(|p| p == permission),
                "{name} leaves {permission} open"
            );
        }
        assert!(!everyone.denies_view(), "{name} must stay visible");
    }
}

#[test]
fn mlai_console_and_stage_overwrites_match_the_proposal() {
    let plan = mlai();
    let (category, console) = plan
        .channels()
        .find(|(_, ch)| ch.name == "ops-console")
        .unwrap();
    let overwrites = plan.effective_overwrites(category, console);
    assert!(
        overwrites
            .iter()
            .any(|o| o.role == "Console" && o.allow.contains(&"Send Messages".to_string()))
    );
    assert!(
        overwrites
            .iter()
            .any(|o| o.role == "Team" && o.allow == [VIEW_CHANNEL])
    );

    let (category, stage) = plan
        .channels()
        .find(|(_, ch)| ch.name == "Office Hours")
        .unwrap();
    assert_eq!(stage.kind, ChannelKind::Stage);
    let overwrites = plan.effective_overwrites(category, stage);
    let everyone = overwrites.iter().find(|o| o.role == EVERYONE).unwrap();
    assert!(everyone.allow.contains(&"Request to Speak".to_string()));
    assert_eq!(everyone.deny, ["Speak"]);

    let (_, help) = plan.channels().find(|(_, ch)| ch.name == "help").unwrap();
    assert_eq!(help.kind, ChannelKind::Forum);
    assert_eq!(help.tags.len(), 7);
}

#[test]
fn mlai_interest_roles_carry_no_permissions_and_are_not_hoisted() {
    let plan = mlai();
    for name in [
        "WDBX",
        "ABI",
        "Personas",
        "Apple Silicon",
        "Site Builder",
        "Announcements",
    ] {
        let role = plan.role(name).unwrap_or_else(|| panic!("missing {name}"));
        assert!(role.permissions.is_empty(), "{name}");
        assert!(!role.hoist && !role.mentionable, "{name}");
    }
    assert_eq!(plan.role("Team").unwrap().colour, Some(0x22d3ee));
    assert!(plan.role("Team").unwrap().hoist);
}

#[test]
fn mlai_text_names_are_in_discords_final_form() {
    // The validator already checks this; the test states it as a property
    // the shipped file must keep even if the validator is loosened.
    for (_, channel) in mlai().channels() {
        if channel.kind.normalizes_name() {
            assert_eq!(normalize_text_name(&channel.name), channel.name);
        }
    }
}

#[test]
fn unknown_fields_are_rejected() {
    let text = minimal("colour = \"#ffffff\"");
    let error = Plan::from_toml(&text).unwrap_err();
    assert!(error.contains("unknown field"), "{error}");
    assert!(Plan::from_toml("name = \"x\"\n[[roles]]\nname = \"a\"\nperms = []\n").is_err());
}

#[test]
fn overwrites_must_target_a_declared_role() {
    let text = minimal(r#"overwrites = [{ role = "Ghost", allow = ["View Channel"] }]"#);
    let error = Plan::from_toml(&text).unwrap_err();
    assert!(error.contains("never creates"), "{error}");
}

#[test]
fn a_text_channel_name_discord_would_rewrite_is_rejected() {
    let text = minimal("").replace("name = \"general\"", "name = \"General Chat\"");
    let error = Plan::from_toml(&text).unwrap_err();
    assert!(error.contains("general-chat"), "{error}");
    // Voice names are exempt.
    let voice = minimal("")
        .replace("kind = \"text\"", "kind = \"voice\"")
        .replace("name = \"general\"", "name = \"Squad 1\"");
    Plan::from_toml(&voice).expect("voice names are free-form");
}

#[test]
fn a_plan_that_hides_everything_is_rejected() {
    let text = minimal(
        r#"overwrites = [{ role = "@everyone", deny = ["View Channel"] }, { role = "Member", allow = ["View Channel"] }]"#,
    );
    let error = Plan::from_toml(&text).unwrap_err();
    assert!(error.contains("empty server"), "{error}");
}

#[test]
fn a_hidden_channel_must_let_some_role_in() {
    let text = format!(
        "{}\n[[categories.channels]]\nname = \"secret\"\nkind = \"text\"\noverwrites = [{{ role = \"@everyone\", deny = [\"View Channel\"] }}]\n",
        minimal("")
    );
    let error = Plan::from_toml(&text).unwrap_err();
    assert!(error.contains("allows no role View Channel"), "{error}");
}

#[test]
fn everyone_is_never_allowed_a_dangerous_permission() {
    let text = minimal(r#"overwrites = [{ role = "@everyone", allow = ["Administrator"] }]"#);
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("never be allowed")
    );
    let text = minimal("").replace(
        "name = \"t\"",
        "name = \"t\"\n[everyone]\npermissions = [\"Manage Guild\"]",
    );
    assert!(Plan::from_toml(&text).unwrap_err().contains("never hold"));
}

#[test]
fn contradictory_duplicate_and_empty_overwrites_are_rejected() {
    let text =
        minimal(r#"overwrites = [{ role = "Member", allow = ["Speak"], deny = ["Speak"] }]"#);
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("both allowed and denied")
    );
    let text = minimal(
        r#"overwrites = [{ role = "Member", allow = ["Speak"] }, { role = "Member", deny = ["Connect"] }]"#,
    );
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("two overwrites")
    );
    let text = minimal(r#"overwrites = [{ role = "Member" }]"#);
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("allows and denies nothing")
    );
}

#[test]
fn tags_slowmode_and_topics_are_kind_checked() {
    assert!(
        Plan::from_toml(&minimal("tags = [\"a\"]"))
            .unwrap_err()
            .contains("only forum")
    );
    assert!(
        Plan::from_toml(&minimal("slowmode_secs = 30000"))
            .unwrap_err()
            .contains("exceeds")
    );
    let voice_topic = minimal("topic = \"x\"")
        .replace("kind = \"text\"", "kind = \"voice\"")
        .replace("name = \"general\"", "name = \"Lounge\"");
    assert!(
        Plan::from_toml(&voice_topic)
            .unwrap_err()
            .contains("carry no topic")
    );
}

#[test]
fn duplicate_roles_categories_and_channels_are_rejected() {
    let text = format!("{}\n[[roles]]\nname = \"Member\"\n", minimal(""));
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("declared twice")
    );
    let text = format!("{}\n[[categories]]\nname = \"main\"\n", minimal(""));
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("declared twice")
    );
    let text = format!(
        "{}\n[[categories.channels]]\nname = \"general\"\nkind = \"text\"\n",
        minimal("")
    );
    assert!(
        Plan::from_toml(&text)
            .unwrap_err()
            .contains("already exists")
    );
}

#[test]
fn colours_parse_and_render_round_trip() {
    assert_eq!(parse_colour("#22D3EE").unwrap(), Some(0x22d3ee));
    assert_eq!(parse_colour("").unwrap(), None);
    assert!(parse_colour("22d3ee").is_err());
    assert!(parse_colour("#22d3e").is_err());
    assert!(parse_colour("#zzzzzz").is_err());
    assert_eq!(colour_hex(0x22d3ee), "#22d3ee");
    assert_eq!(colour_hex(0x000005), "#000005");
    let bad = minimal("").replace("name = \"Member\"", "name = \"Member\"\ncolour = \"red\"");
    assert!(Plan::from_toml(&bad).unwrap_err().contains("#rrggbb"));
}

#[test]
fn hide_marker_is_recognised_by_content_not_order() {
    let marker = Overwrite {
        role: EVERYONE.into(),
        allow: vec![],
        deny: vec![VIEW_CHANNEL.into(), VIEW_CHANNEL.into()],
    };
    assert!(marker.is_hide_marker());
    let mut wider = marker.clone();
    wider.deny.push("Send Messages".into());
    assert!(!wider.is_hide_marker());
    assert!(wider.denies_view());
    let role_marker = Overwrite {
        role: "Member".into(),
        ..marker
    };
    assert!(!role_marker.is_hide_marker());
}

#[test]
fn permission_names_collect_every_mention() {
    let plan = mlai();
    let names = plan.permission_names();
    for expected in [
        "Administrator",
        "Request to Speak",
        "Manage Threads",
        "Create Invites",
    ] {
        assert!(
            names.contains(expected),
            "{expected} missing from {names:?}"
        );
    }
}
