use super::*;
use crate::server::diff::{Scope, Stage, diff};
use crate::server::observe::{
    BotState, COMMUNITY_FEATURE, ChannelState, OverwriteState, OverwriteTarget, RoleState,
};
use crate::server::plan::{MLAI_COMMUNITY, Plan, VIEW_CHANNEL, denies_everyone_view};

const GUILD: u64 = 1_000;
const BOT_ROLE: u64 = 1_001;

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

fn empty_guild() -> GuildSnapshot {
    GuildSnapshot {
        guild_id: GUILD,
        features: vec![COMMUNITY_FEATURE.into()],
        roles: vec![
            role(GUILD, "@everyone", 0, &["View Channel", "Send Messages"]),
            RoleState {
                managed: true,
                ..role(BOT_ROLE, "Abbey Bot", 5, &["Administrator"])
            },
        ],
        channels: Vec::new(),
        bot: BotState {
            user_id: 7,
            role_ids: vec![BOT_ROLE],
        },
    }
}

fn mlai() -> Plan {
    Plan::from_toml(MLAI_COMMUNITY).unwrap()
}

async fn run_stage(plan: &Plan, guild: &mut FakeGuild, scope: Scope) -> Outcome {
    let report = diff(plan, &guild.snapshot, &scope);
    assert!(report.is_clear(), "{scope:?}: {:?}", report.blockers);
    let changes = report.changes.clone();
    let outcome = apply(&changes, &guild.snapshot.clone(), guild).await;
    assert!(outcome.failed.is_none(), "{scope:?}: {:?}", outcome.failed);
    let again = diff(plan, &guild.snapshot, &scope);
    assert!(
        again.changes.is_empty(),
        "{scope:?} is not idempotent: {:?}",
        again.changes
    );
    outcome
}

#[tokio::test]
async fn the_full_rollout_is_idempotent_and_reaches_the_plan() {
    let plan = mlai();
    let mut guild = FakeGuild::new(empty_guild());

    let additive = run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Additive,
            category: None,
        },
    )
    .await;
    assert_eq!(
        additive.applied.len(),
        plan.roles.len() + plan.categories.len() + plan.channels().count()
    );
    for role in &plan.roles {
        let created = &guild.snapshot.roles_named(&role.name)[0];
        assert!(
            created.permissions.is_empty() && !created.hoist,
            "{} created hot",
            role.name
        );
    }
    for (_, channel) in plan.channels() {
        let state = guild
            .snapshot
            .channels_matching(&channel.name, channel.kind)[0];
        assert_eq!(
            state.overwrites.len(),
            1,
            "{}: hidden and nothing else",
            channel.name
        );
        assert_eq!(state.overwrites[0].deny, [VIEW_CHANNEL]);
    }

    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Reveal,
            category: None,
        },
    )
    .await;
    assert!(guild.snapshot.roles_named("Team")[0].hoist);
    assert_eq!(guild.snapshot.roles_named("Team")[0].colour, 0x22d3ee);
    assert!(
        guild.snapshot.roles_named("Team")[0].permissions.is_empty(),
        "reveal never grants"
    );
    let rules = guild.snapshot.channels_matching("rules", ChannelKind::Text)[0];
    let everyone = rules.overwrite_for_role(GUILD).unwrap();
    assert!(
        !everyone.deny.iter().any(|p| p == VIEW_CHANNEL),
        "rules is public after reveal"
    );
    assert!(
        everyone.deny.iter().any(|p| p == "Send Messages"),
        "and read-only from the same moment"
    );
    let staff = guild.snapshot.categories_named("STAFF")[0];
    assert_eq!(
        staff.overwrite_for_role(GUILD).unwrap().deny,
        [VIEW_CHANNEL],
        "STAFF stays hidden"
    );
    let general = guild
        .snapshot
        .channels_matching("general", ChannelKind::Text)[0];
    assert!(general.overwrite_for_role(GUILD).unwrap().allow.is_empty());
    assert!(
        general.overwrite_for_role(GUILD).unwrap().deny.is_empty(),
        "the marker is cleared, not deleted"
    );
    assert_eq!(
        general.parent,
        Some(guild.snapshot.categories_named("COMMONS")[0].id)
    );
    assert!(general.topic.as_deref().unwrap().contains("home"));

    for category in &plan.categories {
        run_stage(
            &plan,
            &mut guild,
            Scope {
                stage: Stage::Overwrites,
                category: Some(category.name.clone()),
            },
        )
        .await;
    }
    for (category, channel) in plan.channels() {
        let state = guild
            .snapshot
            .channels_matching(&channel.name, channel.kind)[0];
        for planned in plan.effective_overwrites(category, channel) {
            let role_id = if planned.role == EVERYONE {
                GUILD
            } else {
                guild.snapshot.roles_named(&planned.role)[0].id
            };
            let live = state
                .overwrite_for_role(role_id)
                .unwrap_or_else(|| panic!("{}: no {} overwrite", channel.name, planned.role));
            assert_eq!(
                super::super::plan::sorted_unique(&live.allow),
                super::super::plan::sorted_unique(&planned.allow),
                "{}",
                channel.name
            );
            assert_eq!(
                super::super::plan::sorted_unique(&live.deny),
                super::super::plan::sorted_unique(&planned.deny),
                "{}",
                channel.name
            );
        }
        let hidden_live = state
            .overwrite_for_role(GUILD)
            .is_some_and(|o| o.deny.iter().any(|p| p == VIEW_CHANNEL));
        assert_eq!(
            hidden_live,
            denies_everyone_view(plan.effective_overwrites(category, channel)),
            "{}",
            channel.name
        );
    }
    let console = guild
        .snapshot
        .channels_matching("ops-console", ChannelKind::Text)[0];
    let console_role = guild.snapshot.roles_named("Console")[0].id;
    assert!(
        console
            .overwrite_for_role(console_role)
            .unwrap()
            .allow
            .contains(&"Send Messages".to_string())
    );
}

#[tokio::test]
async fn reveal_leaves_a_pre_existing_view_gate_alone_and_overwrites_warns() {
    let plan = mlai();
    let mut snapshot = empty_guild();
    snapshot
        .roles
        .push(role(2_000, "Member", 2, &["Send Messages"]));
    // A hand-gated #rules from before the plan: hidden, Member may view.
    snapshot.channels.push(ChannelState {
        id: 3_000,
        name: "rules".into(),
        class: ChannelClass::Kind(ChannelKind::Text),
        parent: None,
        topic: Some("old".into()),
        overwrites: vec![
            OverwriteState {
                target: OverwriteTarget::Role(GUILD),
                allow: vec![],
                deny: vec![VIEW_CHANNEL.into()],
            },
            OverwriteState {
                target: OverwriteTarget::Role(2_000),
                allow: vec![VIEW_CHANNEL.into()],
                deny: vec![],
            },
        ],
    });
    let mut guild = FakeGuild::new(snapshot);
    let additive = run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Additive,
            category: None,
        },
    )
    .await;
    assert!(
        !additive
            .applied
            .iter()
            .any(|c| matches!(c, Change::CreateChannel { name, .. } if name == "rules"))
    );
    assert!(
        !additive
            .applied
            .iter()
            .any(|c| matches!(c, Change::CreateRole { name, .. } if name == "Member"))
    );

    let reveal = diff(
        &plan,
        &guild.snapshot,
        &Scope {
            stage: Stage::Reveal,
            category: None,
        },
    );
    assert!(reveal.is_clear(), "{:?}", reveal.blockers);
    assert!(
            reveal.changes.iter().any(|c| matches!(c, Change::EditChannel { name, category, topic: TopicEdit::Set(_), .. } if name == "rules" && category == "START HERE")),
            "{:?}",
            reveal.changes
        );
    assert!(
            !reveal.changes.iter().any(|c| matches!(c, Change::SetOverwrite { target: Target::Channel { name, .. }, overwrite } if name == "rules" && overwrite.role == EVERYONE)),
            "reveal must not open a hand-gated channel: {:?}",
            reveal.changes
        );
    assert!(
        reveal
            .warnings
            .iter()
            .any(|w| w.contains("#rules") && w.contains("3.B.9")),
        "{:?}",
        reveal.warnings
    );
    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Reveal,
            category: None,
        },
    )
    .await;
    let rules = guild
        .snapshot
        .channels
        .iter()
        .find(|c| c.id == 3_000)
        .unwrap();
    assert_eq!(
        rules.overwrite_for_role(GUILD).unwrap().deny,
        [VIEW_CHANNEL],
        "still hidden"
    );
    assert!(
        rules.overwrite_for_role(2_000).is_some(),
        "the Member entry survives untouched"
    );

    let scope = Scope {
        stage: Stage::Overwrites,
        category: Some("START HERE".into()),
    };
    let report = diff(&plan, &guild.snapshot, &scope);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("#rules") && w.contains("did not place")),
        "{:?}",
        report.warnings
    );
    run_stage(&plan, &mut guild, scope).await;
    let rules = guild
        .snapshot
        .channels
        .iter()
        .find(|c| c.id == 3_000)
        .unwrap();
    assert!(
        !rules
            .overwrite_for_role(GUILD)
            .unwrap()
            .deny
            .iter()
            .any(|p| p == VIEW_CHANNEL)
    );
    assert!(
        rules.overwrite_for_role(2_000).is_some(),
        "SetOverwrite never removes another entry"
    );
}

#[tokio::test]
async fn reveal_preserves_absent_topics_and_parent_only_moves_preserve_topics() {
    let mut plan = mlai();
    let mut guild = FakeGuild::new(empty_guild());
    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Additive,
            category: None,
        },
    )
    .await;

    let clear_plan = plan
        .categories
        .iter_mut()
        .flat_map(|category| category.channels.iter_mut())
        .find(|channel| channel.topic.is_some())
        .expect("the shipped plan has a channel with a topic");
    let clear_name = clear_plan.name.clone();
    let clear_kind = clear_plan.kind;
    clear_plan.topic = None;
    let clear_state = guild
        .snapshot
        .channels
        .iter_mut()
        .find(|channel| {
            channel.class == ChannelClass::Kind(clear_kind)
                && channel.name == plan_channel_key(&clear_name, clear_kind)
        })
        .unwrap();
    clear_state.topic = Some("remove me".into());
    clear_state.parent = None;

    let (_, keep_plan) = plan
        .channels()
        .find(|(_, channel)| channel.topic.is_some())
        .expect("the shipped plan has another channel with a topic");
    let keep_name = keep_plan.name.clone();
    let keep_kind = keep_plan.kind;
    let intended_topic = keep_plan.topic.clone().unwrap();
    let keep_state = guild
        .snapshot
        .channels
        .iter_mut()
        .find(|channel| {
            channel.class == ChannelClass::Kind(keep_kind)
                && channel.name == plan_channel_key(&keep_name, keep_kind)
        })
        .unwrap();
    keep_state.topic = Some(intended_topic.clone());
    keep_state.parent = None;

    let scope = Scope {
        stage: Stage::Reveal,
        category: None,
    };
    let report = diff(&plan, &guild.snapshot, &scope);
    assert!(report.changes.iter().any(|change| matches!(
        change,
        Change::EditChannel { name, topic: TopicEdit::Unchanged, .. } if name == &clear_name
    )));
    assert!(report.changes.iter().any(|change| matches!(
        change,
        Change::EditChannel { name, topic: TopicEdit::Unchanged, .. } if name == &keep_name
    )));

    let outcome = apply(&report.changes, &guild.snapshot.clone(), &mut guild).await;
    assert!(outcome.failed.is_none(), "{:?}", outcome.failed);
    let preserved_omitted = guild.snapshot.channels_matching(&clear_name, clear_kind)[0];
    assert_eq!(preserved_omitted.topic.as_deref(), Some("remove me"));
    let preserved = guild.snapshot.channels_matching(&keep_name, keep_kind)[0];
    assert_eq!(preserved.topic.as_deref(), Some(intended_topic.as_str()));

    let again = diff(&plan, &guild.snapshot, &scope);
    assert!(again.changes.is_empty(), "{:?}", again.changes);
}

#[tokio::test]
async fn gating_a_visible_channel_warns_about_the_lockout() {
    let plan = mlai();
    let mut snapshot = empty_guild();
    snapshot.channels.push(ChannelState {
        id: 3_100,
        name: "ci-and-deploys".into(),
        class: ChannelClass::Kind(ChannelKind::Text),
        parent: None,
        topic: None,
        overwrites: vec![],
    });
    let mut guild = FakeGuild::new(snapshot);
    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Additive,
            category: None,
        },
    )
    .await;
    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Reveal,
            category: None,
        },
    )
    .await;
    let ci = guild
        .snapshot
        .channels
        .iter()
        .find(|c| c.id == 3_100)
        .unwrap();
    assert!(
        ci.overwrite_for_role(GUILD).is_none(),
        "reveal never gates a visible channel"
    );
    let scope = Scope {
        stage: Stage::Overwrites,
        category: Some("BUILD LOG".into()),
    };
    let report = diff(&plan, &guild.snapshot, &scope);
    let warning = report
        .warnings
        .iter()
        .find(|w| w.contains("#ci-and-deploys"))
        .expect("lockout warning");
    assert!(
        warning.contains("3.B.9") && warning.contains("Contributor") && warning.contains("Team"),
        "{warning}"
    );
}

#[tokio::test]
async fn a_plan_without_a_topic_leaves_a_hand_written_topic_alone() {
    // The archetype plans carry no topics. Reveal must not keep emitting
    // an empty edit for a channel that already has one.
    let plan = Plan::from(&crate::server::blueprint(
        crate::server::Archetype::Community,
    ));
    let mut snapshot = empty_guild();
    snapshot.channels.push(ChannelState {
        id: 3_200,
        name: "general".into(),
        class: ChannelClass::Kind(ChannelKind::Text),
        parent: None,
        topic: Some("hand-written".into()),
        overwrites: vec![],
    });
    let mut guild = FakeGuild::new(snapshot);
    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Additive,
            category: None,
        },
    )
    .await;
    run_stage(
        &plan,
        &mut guild,
        Scope {
            stage: Stage::Reveal,
            category: None,
        },
    )
    .await;
    let general = guild
        .snapshot
        .channels
        .iter()
        .find(|c| c.id == 3_200)
        .unwrap();
    assert_eq!(general.topic.as_deref(), Some("hand-written"));
    assert!(
        general.parent.is_some(),
        "it was still moved into its category"
    );
}

#[tokio::test]
async fn apply_stops_at_the_first_failure_and_reports_what_landed() {
    let plan = mlai();
    let mut guild = FakeGuild::new(empty_guild());
    guild.fail_at = Some(2);
    let report = diff(
        &plan,
        &guild.snapshot,
        &Scope {
            stage: Stage::Additive,
            category: None,
        },
    );
    let outcome = apply(&report.changes, &guild.snapshot.clone(), &mut guild).await;
    assert_eq!(outcome.applied.len(), 2);
    let (failed, reason) = outcome.failed.as_ref().unwrap();
    assert_eq!(failed, &report.changes[2]);
    assert_eq!(reason, "injected failure");
    assert_eq!(outcome.remaining.len(), report.changes.len() - 3);
    let rendered = outcome.render();
    assert!(
        rendered.contains("FAILED") && rendered.contains("not attempted"),
        "{rendered}"
    );
    // A second dry run picks up exactly where it stopped.
    let again = diff(
        &plan,
        &guild.snapshot,
        &Scope {
            stage: Stage::Additive,
            category: None,
        },
    );
    assert_eq!(again.changes.len(), report.changes.len() - 2);
}

#[tokio::test]
async fn a_channel_whose_category_is_missing_fails_resolution_not_discord() {
    let mut guild = FakeGuild::new(empty_guild());
    let change = Change::CreateChannel {
        name: "orphan".into(),
        kind: ChannelKind::Text,
        category: "Nowhere".into(),
        topic: None,
        slowmode_secs: None,
        tags: vec![],
        overwrites: vec![],
    };
    let outcome = apply(
        std::slice::from_ref(&change),
        &guild.snapshot.clone(),
        &mut guild,
    )
    .await;
    assert_eq!(outcome.failed.as_ref().map(|(c, _)| c), Some(&change));
    assert!(outcome.failed.unwrap().1.contains("no id yet"));
    assert!(guild.snapshot.channels.is_empty());
}

/// A guild that lives in a snapshot. Every op mutates it the way Discord
/// would, so `diff` can be re-run against the result.
pub struct FakeGuild {
    pub snapshot: GuildSnapshot,
    next_id: u64,
    /// Fail the op with this index (0-based), for the stop-on-failure test.
    pub fail_at: Option<usize>,
    performed: usize,
}

impl FakeGuild {
    pub fn new(snapshot: GuildSnapshot) -> Self {
        let next_id = snapshot
            .roles
            .iter()
            .map(|r| r.id)
            .chain(snapshot.channels.iter().map(|c| c.id))
            .max()
            .unwrap_or(snapshot.guild_id)
            + 1;
        Self {
            snapshot,
            next_id,
            fail_at: None,
            performed: 0,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl GuildWriter for FakeGuild {
    async fn perform(&mut self, op: &Op) -> Result<Option<u64>, String> {
        use crate::server::observe::{ChannelState, OverwriteState, OverwriteTarget, RoleState};
        if self.fail_at == Some(self.performed) {
            return Err("injected failure".into());
        }
        self.performed += 1;
        match op {
            Op::CreateRole { name, colour } => {
                // Discord inserts new roles just above @everyone and shifts
                // everything else up.
                for role in &mut self.snapshot.roles {
                    if role.position >= 1 {
                        role.position += 1;
                    }
                }
                let id = self.fresh_id();
                self.snapshot.roles.push(RoleState {
                    id,
                    name: name.clone(),
                    position: 1,
                    managed: false,
                    hoist: false,
                    mentionable: false,
                    colour: colour.unwrap_or(0),
                    permissions: Vec::new(),
                });
                Ok(Some(id))
            }
            Op::EditRole {
                id,
                hoist,
                mentionable,
                colour,
            } => {
                let role = self
                    .snapshot
                    .roles
                    .iter_mut()
                    .find(|r| r.id == *id)
                    .ok_or_else(|| format!("no role {id}"))?;
                role.hoist = *hoist;
                role.mentionable = *mentionable;
                role.colour = colour.unwrap_or(0);
                Ok(None)
            }
            Op::CreateChannel {
                name,
                class,
                parent,
                topic,
                overwrites,
                ..
            } => {
                if let Some(parent) = parent
                    && !self
                        .snapshot
                        .channels
                        .iter()
                        .any(|c| c.id == *parent && c.class == ChannelClass::Category)
                {
                    return Err(format!("no category {parent}"));
                }
                let id = self.fresh_id();
                let stored_name = match class {
                    ChannelClass::Kind(kind) => plan_channel_key(name, *kind),
                    ChannelClass::Category | ChannelClass::Other => name.clone(),
                };
                self.snapshot.channels.push(ChannelState {
                    id,
                    name: stored_name,
                    class: *class,
                    parent: *parent,
                    topic: topic.clone(),
                    overwrites: overwrites
                        .iter()
                        .map(|o| OverwriteState {
                            target: OverwriteTarget::Role(o.role_id),
                            allow: o.allow.clone(),
                            deny: o.deny.clone(),
                        })
                        .collect(),
                });
                Ok(Some(id))
            }
            Op::EditChannel { id, parent, topic } => {
                let channel = self
                    .snapshot
                    .channels
                    .iter_mut()
                    .find(|c| c.id == *id)
                    .ok_or_else(|| format!("no channel {id}"))?;
                if let Some(parent) = parent {
                    channel.parent = Some(*parent);
                }
                match topic {
                    TopicEdit::Unchanged => {}
                    TopicEdit::Set(topic) => channel.topic = Some(topic.clone()),
                }
                Ok(None)
            }
            Op::SetOverwrite {
                channel_id,
                role_id,
                allow,
                deny,
            } => {
                let channel = self
                    .snapshot
                    .channels
                    .iter_mut()
                    .find(|c| c.id == *channel_id)
                    .ok_or_else(|| format!("no channel {channel_id}"))?;
                let target = OverwriteTarget::Role(*role_id);
                match channel.overwrites.iter_mut().find(|o| o.target == target) {
                    Some(existing) => {
                        existing.allow.clone_from(allow);
                        existing.deny.clone_from(deny);
                    }
                    None => channel.overwrites.push(OverwriteState {
                        target,
                        allow: allow.clone(),
                        deny: deny.clone(),
                    }),
                }
                Ok(None)
            }
        }
    }
}
