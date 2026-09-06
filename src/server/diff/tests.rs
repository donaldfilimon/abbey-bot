use super::context::{is_engine_hidden, reveal_permits};
use super::*;
use crate::server::observe::{BotState, ChannelClass, RoleState};
use crate::server::observe::{
    COMMUNITY_FEATURE, ChannelState, GuildSnapshot, OverwriteState, OverwriteTarget,
};
use crate::server::plan::{EVERYONE, Plan};
use crate::server::plan::{MLAI_COMMUNITY, VIEW_CHANNEL};

const GUILD: u64 = 500;

fn role(id: u64, name: &str, position: u16, permissions: &[&str]) -> RoleState {
    RoleState {
        id,
        name: name.into(),
        position,
        managed: false,
        hoist: false,
        mentionable: false,
        colour: 0,
        permissions: permissions.iter().map(|p| (*p).into()).collect(),
    }
}

fn guild(bot_permissions: &[&str]) -> GuildSnapshot {
    GuildSnapshot {
        guild_id: GUILD,
        features: vec![COMMUNITY_FEATURE.into()],
        roles: vec![
            role(GUILD, "@everyone", 0, &["View Channel"]),
            RoleState {
                managed: true,
                ..role(501, "Abbey Bot", 4, bot_permissions)
            },
        ],
        channels: Vec::new(),
        bot: BotState {
            user_id: 1,
            role_ids: vec![501],
        },
    }
}

fn mlai() -> Plan {
    Plan::from_toml(MLAI_COMMUNITY).unwrap()
}

fn sample_changes() -> Vec<Change> {
    vec![
        Change::CreateRole {
            name: "r".into(),
            colour: None,
        },
        Change::EditRole {
            name: "r".into(),
            hoist: true,
            mentionable: false,
            colour: Some(1),
        },
        Change::CreateCategory {
            name: "c".into(),
            overwrites: vec![Overwrite::hide_marker()],
        },
        Change::CreateChannel {
            name: "t".into(),
            kind: ChannelKind::Text,
            category: "c".into(),
            topic: Some("x".into()),
            slowmode_secs: Some(5),
            tags: vec![],
            overwrites: vec![Overwrite::hide_marker()],
        },
        Change::EditChannel {
            name: "t".into(),
            kind: ChannelKind::Text,
            category: "c".into(),
            topic: TopicEdit::Unchanged,
        },
        Change::SetOverwrite {
            target: Target::Channel {
                name: "t".into(),
                kind: ChannelKind::Text,
            },
            overwrite: Overwrite {
                role: EVERYONE.into(),
                allow: vec![],
                deny: vec![],
            },
        },
    ]
}

#[test]
fn every_change_variant_is_additive_and_none_can_delete() {
    // If a variant is added, `KIND_NAMES` and this sample must both grow,
    // and the name check keeps "Delete"/"Remove" out of the vocabulary.
    let sample = sample_changes();
    assert_eq!(sample.len(), Change::KIND_NAMES.len());
    for (change, expected) in sample.iter().zip(Change::KIND_NAMES) {
        assert_eq!(change.kind_name(), expected);
    }
    for name in Change::KIND_NAMES {
        let lower = name.to_ascii_lowercase();
        assert!(
            !lower.contains("delete") && !lower.contains("remove") && !lower.contains("permission"),
            "{name}"
        );
    }
    for change in &sample {
        let text = change.describe();
        assert!(!text.is_empty() && !text.contains("Permissions("), "{text}");
    }
}

#[test]
fn reveal_permits_exactly_the_marker_to_visible_transition_for_everyone() {
    let marker = Overwrite::hide_marker();
    let public = Overwrite {
        role: EVERYONE.into(),
        allow: vec![],
        deny: vec!["Send Messages".into()],
    };
    let gated = Overwrite {
        role: EVERYONE.into(),
        allow: vec![],
        deny: vec![VIEW_CHANNEL.into(), "Send Messages".into()],
    };
    let hand_hidden = Overwrite {
        role: EVERYONE.into(),
        allow: vec!["Connect".into()],
        deny: vec![VIEW_CHANNEL.into()],
    };
    assert!(
        reveal_permits(Some(&marker), &public, true),
        "engine-hidden → visible"
    );
    assert!(
        !reveal_permits(Some(&marker), &public, false),
        "the same entry beside a role allow is a hand gate"
    );
    assert!(reveal_permits(None, &public, false), "no view change");
    assert!(
        reveal_permits(Some(&gated), &gated, false),
        "hidden stays hidden"
    );
    assert!(
        !reveal_permits(None, &gated, false),
        "gating a visible channel is 3.B.9"
    );
    assert!(
        !reveal_permits(Some(&hand_hidden), &public, false),
        "opening a hand-gated channel waits for --stage overwrites"
    );
    let role = Overwrite {
        role: "Team".into(),
        allow: vec![VIEW_CHANNEL.into()],
        deny: vec![],
    };
    assert!(
        reveal_permits(None, &role, false),
        "roles are never a lockout"
    );

    let marker_only = ChannelState {
        id: 1,
        name: "x".into(),
        class: ChannelClass::Kind(ChannelKind::Text),
        parent: None,
        topic: None,
        overwrites: vec![OverwriteState {
            target: OverwriteTarget::Role(GUILD),
            allow: vec![],
            deny: vec![VIEW_CHANNEL.into()],
        }],
    };
    assert!(is_engine_hidden(&marker_only, GUILD));
    let mut with_role = marker_only.clone();
    with_role.overwrites.push(OverwriteState {
        target: OverwriteTarget::Role(9),
        allow: vec![VIEW_CHANNEL.into()],
        deny: vec![],
    });
    assert!(!is_engine_hidden(&with_role, GUILD));
    let mut wider = marker_only;
    wider.overwrites[0].deny.push("Send Messages".into());
    assert!(!is_engine_hidden(&wider, GUILD));
}

#[test]
fn additive_on_an_empty_guild_creates_everything_hidden_and_powerless() {
    let plan = mlai();
    let report = diff(
        &plan,
        &guild(&["Administrator"]),
        &Scope {
            stage: Stage::Additive,
            category: None,
        },
    );
    assert!(report.is_clear(), "{:?}", report.blockers);
    let roles = report
        .changes
        .iter()
        .filter(|c| matches!(c, Change::CreateRole { .. }))
        .count();
    assert_eq!(roles, plan.roles.len());
    for change in &report.changes {
        match change {
            Change::CreateCategory { overwrites, .. }
            | Change::CreateChannel { overwrites, .. } => {
                assert_eq!(overwrites, &[Overwrite::hide_marker()]);
            }
            Change::CreateRole { .. } => {}
            other => panic!("additive emitted {}", other.kind_name()),
        }
    }
    assert!(
        report
            .manual
            .iter()
            .any(|m| m.contains("after --stage additive, grant role \"Team\"")),
        "{:?}",
        report.manual
    );
    assert!(
        report.manual.iter().any(|m| m.starts_with("order roles")),
        "{:?}",
        report.manual
    );
    assert!(
        report
            .manual
            .iter()
            .any(|m| m.starts_with("@everyone guild permissions")),
        "{:?}",
        report.manual
    );
    let rendered = report.render("heading");
    assert!(
        rendered.starts_with("heading\n") && rendered.contains("manual steps"),
        "{rendered}"
    );
}

#[test]
fn the_bot_needs_manage_channels_and_manage_roles_unless_administrator() {
    let plan = mlai();
    let scope = Scope {
        stage: Stage::Additive,
        category: None,
    };
    let report = diff(&plan, &guild(&["Manage Channels"]), &scope);
    assert!(
        report.blockers.iter().any(|b| b.contains("Manage Roles")),
        "{:?}",
        report.blockers
    );
    let report = diff(&plan, &guild(&["Manage Roles"]), &scope);
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("Manage Channels")),
        "{:?}",
        report.blockers
    );
    // Holding both but not View Channel: the hide marker denies a
    // permission the bot does not hold, which Discord refuses.
    let mut snapshot = guild(&["Manage Channels", "Manage Roles"]);
    snapshot.roles[0].permissions.clear();
    let report = diff(&plan, &snapshot, &scope);
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("does not hold itself") && b.contains(VIEW_CHANNEL)),
        "{:?}",
        report.blockers
    );
    let report = diff(&plan, &guild(&["Manage Channels", "Manage Roles"]), &scope);
    assert!(report.is_clear(), "{:?}", report.blockers);
}

#[test]
fn community_kinds_are_blocked_without_the_feature() {
    let plan = mlai();
    let mut snapshot = guild(&["Administrator"]);
    snapshot.features.clear();
    let report = diff(
        &plan,
        &snapshot,
        &Scope {
            stage: Stage::Additive,
            category: None,
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("forum #help") && b.contains("Community mode")),
        "{:?}",
        report.blockers
    );
    assert!(
        report
            .manual
            .iter()
            .any(|m| m.contains("enable Community mode")),
        "{:?}",
        report.manual
    );
}

#[test]
fn ambiguous_names_block_rather_than_guess() {
    let plan = mlai();
    let mut snapshot = guild(&["Administrator"]);
    snapshot.roles.push(role(600, "Team", 2, &[]));
    snapshot.roles.push(role(601, "Team", 1, &[]));
    let report = diff(
        &plan,
        &snapshot,
        &Scope {
            stage: Stage::Additive,
            category: None,
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("\"Team\" exists 2 times")),
        "{:?}",
        report.blockers
    );
    assert!(
        !report
            .changes
            .iter()
            .any(|c| matches!(c, Change::CreateRole { name, .. } if name == "Team"))
    );
}

#[test]
fn a_plan_role_that_matches_a_bots_managed_role_is_a_blocker() {
    // The live MLAI guild's bot role is named Abbey and the proposal had
    // an interest role of that name; the first live dry run refused it.
    // Guessing would edit the bot's role, so any such match blocks.
    let plan = mlai();
    let mut snapshot = guild(&["Administrator"]);
    snapshot.roles.push(RoleState {
        managed: true,
        ..role(650, "Team", 3, &[])
    });
    // The overwrites stage checks roles only inside an existing category,
    // so it is covered by the rollout tests in `apply`; the two stages
    // that walk every plan role are checked here.
    for stage in [Stage::Additive, Stage::Reveal] {
        let report = diff(
            &plan,
            &snapshot,
            &Scope {
                stage,
                category: None,
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("\"Team\" is an integration-managed role")),
            "{stage:?}: {:?}",
            report.blockers
        );
    }
}

#[test]
fn reveal_and_overwrites_require_additive_to_have_run() {
    let plan = mlai();
    let report = diff(
        &plan,
        &guild(&["Administrator"]),
        &Scope {
            stage: Stage::Reveal,
            category: None,
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("run --stage additive first")),
        "{:?}",
        report.blockers
    );
    let report = diff(
        &plan,
        &guild(&["Administrator"]),
        &Scope {
            stage: Stage::Overwrites,
            category: Some("STAFF".into()),
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("run --stage additive first")),
        "{:?}",
        report.blockers
    );
    let report = diff(
        &plan,
        &guild(&["Administrator"]),
        &Scope {
            stage: Stage::Overwrites,
            category: None,
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("needs --category")),
        "{:?}",
        report.blockers
    );
    let report = diff(
        &plan,
        &guild(&["Administrator"]),
        &Scope {
            stage: Stage::Overwrites,
            category: Some("NOPE".into()),
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("no category \"NOPE\"") && b.contains("STAFF")),
        "{:?}",
        report.blockers
    );
}

#[test]
fn reveal_refuses_to_edit_a_role_at_or_above_the_bot_and_skips_managed_ones() {
    let plan = mlai();
    let mut snapshot = guild(&["Administrator"]);
    for (index, role_spec) in plan.roles.iter().enumerate() {
        let id = 700 + index as u64;
        snapshot.roles.push(role(id, &role_spec.name, 1, &[]));
    }
    for category in &plan.categories {
        let id = 800 + snapshot.channels.len() as u64;
        snapshot.channels.push(ChannelState {
            id,
            name: category.name.clone(),
            class: ChannelClass::Category,
            parent: None,
            topic: None,
            overwrites: vec![],
        });
        for channel in &category.channels {
            let cid = 900 + snapshot.channels.len() as u64;
            snapshot.channels.push(ChannelState {
                id: cid,
                name: channel.name.clone(),
                class: ChannelClass::Kind(channel.kind),
                parent: Some(id),
                topic: channel.topic.clone(),
                overwrites: vec![],
            });
        }
    }
    // Team sits above the bot; Moderator is integration-managed.
    snapshot
        .roles
        .iter_mut()
        .find(|r| r.name == "Team")
        .unwrap()
        .position = 9;
    snapshot
        .roles
        .iter_mut()
        .find(|r| r.name == "Moderator")
        .unwrap()
        .managed = true;
    let report = diff(
        &plan,
        &snapshot,
        &Scope {
            stage: Stage::Reveal,
            category: None,
        },
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("\"Team\" sits at position 9")),
        "{:?}",
        report.blockers
    );
    assert!(
        report
            .blockers
            .iter()
            .any(|b| b.contains("\"Moderator\" is an integration-managed role")),
        "{:?}",
        report.blockers
    );
    assert!(!report.changes.iter().any(
        |c| matches!(c, Change::EditRole { name, .. } if name == "Team" || name == "Moderator")
    ));
    assert!(
        report.changes.iter().any(
            |c| matches!(c, Change::EditRole { name, hoist: true, .. } if name == "Contributor")
        )
    );
    // Roles that exist with matching permissions produce no manual grant,
    // and the order step names the plan order.
    assert!(
        report
            .manual
            .iter()
            .any(|m| m.starts_with("order roles") && m.contains("Owner > Team > Moderator")),
        "{:?}",
        report.manual
    );
}

#[test]
fn every_stage_reports_the_same_preflight_facts() {
    let plan = mlai();
    for stage in Stage::ALL {
        let scope = Scope {
            stage,
            category: (stage == Stage::Overwrites).then(|| "STAFF".to_string()),
        };
        let report = diff(&plan, &guild(&["Administrator"]), &scope);
        assert_eq!(report.stage, stage);
        assert!(
            report.preflight[0].contains("Administrator"),
            "{:?}",
            report.preflight
        );
        assert!(
            report.preflight[1].contains("COMMUNITY feature: present"),
            "{:?}",
            report.preflight
        );
        assert_eq!(Stage::parse(stage.label()), Some(stage));
    }
    assert_eq!(Stage::parse("delete"), None);
}
