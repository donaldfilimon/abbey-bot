use super::*;

#[test]
fn guided_sections_preserve_every_eligible_entry_and_visibility_before_clamping() {
    for context in [InteractionContext::Guild, InteractionContext::BotDm] {
        for permission in [
            None,
            Some(DiscordPermission::ManageMessages),
            Some(DiscordPermission::ModerateMembers),
            Some(DiscordPermission::ManageServer),
            Some(DiscordPermission::Administrator),
        ] {
            for bits in 0..32 {
                let mut input = member();
                input.context = context;
                input.permissions = permission.into_iter().collect();
                input.caller_present_in_voice = Some(true);
                input.selected_voice_mode = SelectedVoiceMode::Local;
                input.readiness = [
                    Capability::Generation,
                    Capability::Vision,
                    Capability::VoiceConfigured,
                    Capability::VoiceLocal,
                    Capability::VoiceOpenAi,
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(index, cap)| {
                    (bits & (1 << index) != 0).then_some((cap, CapabilityReadiness::Ready))
                })
                .collect();
                for section in HelpSection::ALL {
                    let rendered = render_help(section, &input);
                    assert!(
                        rendered.chars().count() <= 2000,
                        "{section:?}: {}",
                        rendered.chars().count()
                    );
                    assert_eq!(crate::commands::clamp_message(rendered.clone()), rendered);
                    for spec in registered_commands() {
                        let name = format!(
                            "`{}{}` (",
                            if spec.kind == CommandKind::Slash {
                                "/"
                            } else {
                                ""
                            },
                            spec.name
                        );
                        let expected = spec.section == section
                            && eligible(spec, &input, EvaluationMode::Discoverability);
                        assert_eq!(
                            rendered.contains(&name),
                            expected,
                            "{section:?}: {} {input:?}",
                            spec.name
                        );
                        if expected {
                            let surface = match spec.kind {
                                CommandKind::Slash => "slash command",
                                CommandKind::UserContext => "member menu",
                                CommandKind::MessageContext => "message menu",
                            };
                            let visibility = if spec.private {
                                "private"
                            } else if context == InteractionContext::BotDm {
                                "reply in this DM"
                            } else {
                                "channel-visible"
                            };
                            assert!(rendered.contains(&format!("{name}{surface}; {visibility})")));
                        }
                    }
                    if context == InteractionContext::BotDm {
                        assert!(
                            !rendered.contains("channel-visible")
                                && !rendered.contains("member menu")
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn representative_guided_output_is_readable() {
    let mut input = member();
    input.readiness = [Capability::Generation, Capability::Vision]
        .into_iter()
        .map(|capability| (capability, CapabilityReadiness::Ready))
        .collect();
    for section in [HelpSection::Start, HelpSection::Memory, HelpSection::Images] {
        let rendered = render_help(section, &input);
        match section {
            HelpSection::Start => assert!(rendered.contains("Task buttons open private workflows")),
            HelpSection::Memory => {
                assert!(rendered.contains("Open the member menu, then Apps"));
                assert!(!rendered.contains("message menu"));
            }
            HelpSection::Images => {
                assert!(rendered.contains("Open the message menu, then Apps"));
                assert!(!rendered.contains("member menu"));
            }
            _ => unreachable!(),
        }
        println!("{rendered}\n");
    }
    input.context = InteractionContext::BotDm;
    println!("{}", render_help(HelpSection::Conversation, &input));
}

fn member() -> EligibilityInput {
    let mut input = EligibilityInput::new(InteractionContext::Guild);
    input.self_subject = Some(true);
    input.follow_up_absent = Some(true);
    input.caller_present_in_voice = Some(false);
    input
}
#[test]
fn access_rule_truth_tables_cover_roles_subject_and_voice_presence() {
    use DiscordPermission::*;
    for permission in [
        None,
        Some(ManageMessages),
        Some(ModerateMembers),
        Some(ManageWebhooks),
        Some(ManageServer),
        Some(Administrator),
    ] {
        for self_subject in [false, true] {
            for present in [false, true] {
                for owner in [false, true] {
                    let mut input = member();
                    input.permissions = permission.into_iter().collect();
                    input.self_subject = Some(self_subject);
                    input.caller_present_in_voice = Some(present);
                    input.application_owner = owner;
                    let admin = permission == Some(Administrator);
                    let manager = permission == Some(ManageServer) || admin;
                    let expected = [
                        true,
                        self_subject
                            || matches!(
                                permission,
                                Some(ManageMessages | ManageServer | Administrator)
                            ),
                        permission == Some(ModerateMembers) || admin,
                        permission == Some(ManageWebhooks) || admin,
                        manager,
                        manager && present,
                        manager || present,
                        owner || admin,
                    ];
                    for (rule, expected) in [
                        AccessId::A0,
                        AccessId::A1,
                        AccessId::A2,
                        AccessId::A3,
                        AccessId::A4,
                        AccessId::A5,
                        AccessId::A6,
                        AccessId::A7,
                    ]
                    .into_iter()
                    .zip(expected)
                    {
                        assert_eq!(
                            access_allows(rule.rule(), &input),
                            expected,
                            "{rule:?}: {input:?}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn capability_input_and_selected_mode_matrices_are_exact() {
    for bits in 0..32 {
        for selected in [
            SelectedVoiceMode::Off,
            SelectedVoiceMode::Local,
            SelectedVoiceMode::OpenAi,
        ] {
            for absent in [None, Some(false), Some(true)] {
                let mut input = member();
                input.readiness = [
                    Capability::Generation,
                    Capability::Vision,
                    Capability::VoiceConfigured,
                    Capability::VoiceLocal,
                    Capability::VoiceOpenAi,
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(index, capability)| {
                    (bits & (1 << index) != 0).then_some((capability, CapabilityReadiness::Ready))
                })
                .collect();
                input.selected_voice_mode = selected;
                input.follow_up_absent = absent;
                let generation = bits & 1 != 0;
                let vision = bits & 2 != 0;
                let voice = bits & 4 != 0;
                let local = bits & 8 != 0;
                let openai = bits & 16 != 0;
                let mode_ready = match selected {
                    SelectedVoiceMode::Off => false,
                    SelectedVoiceMode::Local => local,
                    SelectedVoiceMode::OpenAi => openai,
                };
                for (rule, expected) in [
                    ConditionId::C0,
                    ConditionId::C1,
                    ConditionId::C2,
                    ConditionId::C3,
                    ConditionId::C4,
                    ConditionId::C5,
                    ConditionId::C6,
                ]
                .into_iter()
                .zip([
                    true,
                    generation,
                    vision,
                    vision && (absent == Some(true) || generation),
                    voice,
                    voice && mode_ready,
                    voice && local,
                ]) {
                    assert_eq!(
                        condition_allows(rule.rule(), &input, EvaluationMode::Invocation),
                        expected,
                        "{rule:?}: {input:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn target_hierarchy_is_potential_in_help_and_mandatory_at_invocation() {
    let mut input = member();
    input.permissions = vec![DiscordPermission::ModerateMembers];
    assert!(eligible(
        command(CommandKey::Modcall),
        &input,
        EvaluationMode::Discoverability
    ));
    assert!(!eligible(
        command(CommandKey::Modcall),
        &input,
        EvaluationMode::Invocation
    ));
    for resolved in [false, true] {
        for hierarchy in [None, Some(false), Some(true)] {
            input.action_target_resolved = resolved;
            input.hierarchy_allows_action = hierarchy;
            assert_eq!(
                condition_allows(ConditionId::C7.rule(), &input, EvaluationMode::Invocation),
                resolved && hierarchy == Some(true)
            );
        }
    }
}
#[test]
fn dm_is_self_only_and_operator_commands_do_not_leak() {
    let mut input = member();
    input.context = InteractionContext::BotDm;
    input.permissions = vec![DiscordPermission::Administrator];
    for key in [
        CommandKey::Remember,
        CommandKey::Forget,
        CommandKey::PendingList,
        CommandKey::PendingConfirm,
        CommandKey::PendingDismiss,
        CommandKey::Recall,
        CommandKey::Reputation,
    ] {
        input.self_subject = Some(true);
        assert!(eligible(command(key), &input, EvaluationMode::Invocation));
        for subject in [None, Some(false)] {
            input.self_subject = subject;
            assert!(!eligible(command(key), &input, EvaluationMode::Invocation));
        }
    }
    for section in [
        HelpSection::Voice,
        HelpSection::Administration,
        HelpSection::Moderation,
    ] {
        let rendered = render_help(section, &input);
        assert!(rendered.contains("No commands in this section"));
        assert!(!rendered.contains("Manage Server"));
    }
    assert!(render_help(HelpSection::Start, &input).contains("`/help`"));
}
#[test]
fn catalog_identity_policy_and_description_data_are_valid() {
    let mut keys = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for spec in registered_commands().iter().chain(planned_commands()) {
        assert!(keys.insert(spec.key));
        assert!(names.insert(spec.name));
        assert!(!spec.registration.contexts.is_empty());
        assert!(
            !spec.description.is_empty() && spec.description.chars().count() <= 100,
            "{}",
            spec.name
        );
        assert!(!spec.name.contains("launch"));
        fn access_valid(rule: AccessRule, depth: usize) -> bool {
            depth < 32
                && match rule {
                    AccessRule::All(rules) | AccessRule::Any(rules) => {
                        !rules.is_empty() && rules.iter().all(|rule| access_valid(*rule, depth + 1))
                    }
                    _ => true,
                }
        }
        fn condition_valid(rule: ConditionRule, depth: usize) -> bool {
            depth < 32
                && match rule {
                    ConditionRule::All(rules) | ConditionRule::Any(rules) => {
                        !rules.is_empty()
                            && rules.iter().all(|rule| condition_valid(*rule, depth + 1))
                    }
                    _ => true,
                }
        }
        assert!(access_valid(spec.eligibility.access.rule(), 0));
        assert!(condition_valid(spec.eligibility.condition.rule(), 0));
    }
    assert_eq!(keys.len(), 51);
    assert_eq!(registered_commands().len(), 51);
    assert!(planned_commands().is_empty());
    let input = member();
    for rule in [AccessRule::All(&[]), AccessRule::Any(&[])] {
        assert!(!access_allows(rule, &input));
    }
    for rule in [ConditionRule::All(&[]), ConditionRule::Any(&[])] {
        assert!(!condition_allows(rule, &input, EvaluationMode::Invocation));
    }
}
#[test]
fn completed_voice_and_admin_features_are_advertised_by_current_policy() {
    let mut input = member();
    input.readiness = [Capability::VoiceConfigured, Capability::VoiceLocal]
        .into_iter()
        .map(|capability| (capability, CapabilityReadiness::Ready))
        .collect();
    input.selected_voice_mode = SelectedVoiceMode::Local;
    assert!(planned_commands().is_empty());
    assert!(eligible(
        command(CommandKey::VoiceStatus),
        &input,
        EvaluationMode::Discoverability
    ));
    assert!(!eligible(
        command(CommandKey::VoiceDiagnostics),
        &input,
        EvaluationMode::Discoverability
    ));
    assert_eq!(
        command(CommandKey::VoiceStatus).eligibility.access,
        AccessId::A0
    );
    assert!(render_help(HelpSection::Voice, &input).contains("`/voice status`"));
    assert!(render_help(HelpSection::Voice, &input).contains("`/voice consent`"));
    assert!(render_help(HelpSection::Images, &input).contains("`/ocr`"));
    assert!(render_help(HelpSection::Conversation, &input).contains("`/persona ask`"));
    assert!(render_help(HelpSection::Conversation, &input).contains("`/persona route`"));
}
#[test]
fn readme_generated_region_matches_catalog_exactly() {
    let readme = include_str!("../../README.md");
    let begin = "<!-- BEGIN GENERATED COMMAND CATALOG -->";
    let end = "<!-- END GENERATED COMMAND CATALOG -->";
    assert_eq!(readme.matches(begin).count(), 1);
    assert_eq!(readme.matches(end).count(), 1);
    let start = readme.find(begin).unwrap();
    let stop = readme.find(end).unwrap() + end.len();
    assert_eq!(&readme[start..stop], render_readme());
}

#[test]
fn music_commands_require_management_and_presence_without_inference_capability() {
    for key in [
        CommandKey::VoicePlay,
        CommandKey::VoicePause,
        CommandKey::VoiceResumeMusic,
        CommandKey::VoiceStopMusic,
        CommandKey::VoiceVolume,
    ] {
        let spec = command(key);
        for manager in [false, true] {
            for present in [false, true] {
                let mut input = member();
                input.permissions = if manager {
                    vec![DiscordPermission::ManageServer]
                } else {
                    vec![]
                };
                input.caller_present_in_voice = Some(present);
                input.readiness = [Capability::VoiceConfigured]
                    .into_iter()
                    .map(|capability| (capability, CapabilityReadiness::Ready))
                    .collect();
                assert_eq!(
                    eligible(spec, &input, EvaluationMode::Invocation),
                    manager && present
                );
                assert!(spec.private);
            }
        }
    }
}

#[test]
fn unconfigured_status_remains_executable_and_unavailable_help_remains_visible() {
    let input = member();
    assert!(eligible(
        command(CommandKey::VoiceStatus),
        &input,
        EvaluationMode::Invocation
    ));
    assert!(render_help(HelpSection::Conversation, &input).contains("`/persona ask`"));
    assert!(render_help(HelpSection::Images, &input).contains("`/ocr`"));
}

#[test]
fn availability_distinguishes_exact_operations_and_policy() {
    let mut input = member();
    input.readiness = [Capability::Generation, Capability::Ocr]
        .into_iter()
        .map(|capability| (capability, CapabilityReadiness::Ready))
        .collect();
    assert_eq!(
        availability(command(CommandKey::Summarize), &input),
        Availability::Ready
    );
    assert_eq!(
        availability(command(CommandKey::PersonaAsk), &input),
        Availability::Blocked(Blocker::Generation)
    );
    assert_eq!(
        availability(command(CommandKey::Ocr), &input),
        Availability::Ready
    );
    assert_eq!(
        availability(command(CommandKey::DescribeImage), &input),
        Availability::Blocked(Blocker::Vision)
    );
    input.vision_allowed = false;
    assert_eq!(
        availability(command(CommandKey::Ocr), &input),
        Availability::Blocked(Blocker::VisionPolicy)
    );
    input.readiness.insert(
        Capability::ToolGeneration,
        CapabilityReadiness::Blocked(Blocker::Busy),
    );
    assert_eq!(
        availability(command(CommandKey::PersonaAsk), &input).message(),
        "The provider is busy. Wait briefly and try again."
    );
    input.readiness.insert(
        Capability::ToolGeneration,
        CapabilityReadiness::Blocked(Blocker::Unknown),
    );
    assert!(
        availability(command(CommandKey::PersonaAsk), &input)
            .message()
            .contains("unknown")
    );
    input.permissions.push(DiscordPermission::ManageServer);
    assert_eq!(
        availability(command(CommandKey::VoiceJoin), &input),
        Availability::AccessBlocked(Blocker::VoicePresence)
    );
    input.caller_present_in_voice = Some(true);
    assert_eq!(
        availability(command(CommandKey::VoiceJoin), &input),
        Availability::Blocked(Blocker::VoiceSetup)
    );
    assert_eq!(
        availability(command(CommandKey::AdminDashboard), &member()),
        Availability::AccessBlocked(Blocker::Permission)
    );
}

#[test]
fn image_followup_requires_read_only_generation_and_names_that_blocker() {
    let mut input = member();
    input.readiness = [Capability::Vision]
        .into_iter()
        .map(|capability| (capability, CapabilityReadiness::Ready))
        .collect();
    input.follow_up_absent = Some(false);
    assert_eq!(
        availability(command(CommandKey::See), &input),
        Availability::Blocked(Blocker::Generation)
    );
    input
        .readiness
        .insert(Capability::Generation, CapabilityReadiness::Ready);
    assert_eq!(
        availability(command(CommandKey::See), &input),
        Availability::Ready
    );
}

#[test]
fn display_section_cannot_change_guild_vision_policy() {
    let mut input = member();
    input.readiness = [Capability::Vision, Capability::Ocr]
        .into_iter()
        .map(|capability| (capability, CapabilityReadiness::Ready))
        .collect();
    input.vision_allowed = false;
    for key in [
        CommandKey::See,
        CommandKey::DescribeImage,
        CommandKey::Ocr,
        CommandKey::ReadImage,
    ] {
        for section in HelpSection::ALL {
            let spec = CommandSpec {
                section,
                ..*command(key)
            };
            assert_eq!(
                availability(&spec, &input),
                Availability::Blocked(Blocker::VisionPolicy)
            );
        }
    }
    let unrelated = CommandSpec {
        section: HelpSection::Images,
        ..*command(CommandKey::Help)
    };
    assert_eq!(availability(&unrelated, &input), Availability::Ready);
}

#[test]
fn each_capability_has_one_replaceable_readiness_observation() {
    let mut input = member();
    input
        .readiness
        .insert(Capability::ToolGeneration, CapabilityReadiness::Ready);
    assert_eq!(
        availability(command(CommandKey::PersonaAsk), &input),
        Availability::Ready
    );
    input.readiness.insert(
        Capability::ToolGeneration,
        CapabilityReadiness::Blocked(Blocker::Busy),
    );
    assert_eq!(input.readiness.len(), 1);
    assert_eq!(
        availability(command(CommandKey::PersonaAsk), &input),
        Availability::Blocked(Blocker::Busy)
    );
    input
        .readiness
        .insert(Capability::ToolGeneration, CapabilityReadiness::Ready);
    assert_eq!(input.readiness.len(), 1);
    assert_eq!(
        availability(command(CommandKey::PersonaAsk), &input),
        Availability::Ready
    );
}

#[test]
fn typed_decisions_retain_empty_and_depth_guards() {
    let input = member();
    for rule in [ConditionRule::All(&[]), ConditionRule::Any(&[])] {
        assert!(condition_decision(rule, &input, EvaluationMode::Invocation).is_err());
    }
    for rule in [AccessRule::All(&[]), AccessRule::Any(&[])] {
        assert!(access_decision(rule, &input).is_err());
    }
    let mut access = AccessRule::Allow;
    let mut condition = ConditionRule::Always;
    for _ in 0..32 {
        access = AccessRule::All(Box::leak(Box::new([access])));
        condition = ConditionRule::All(Box::leak(Box::new([condition])));
    }
    assert!(access_decision(access, &input).is_ok());
    assert!(condition_decision(condition, &input, EvaluationMode::Invocation).is_ok());
    access = AccessRule::Any(Box::leak(Box::new([access])));
    condition = ConditionRule::Any(Box::leak(Box::new([condition])));
    assert!(access_decision(access, &input).is_err());
    assert!(condition_decision(condition, &input, EvaluationMode::Invocation).is_err());
}

#[test]
fn registered_invocation_and_discovery_match_independent_requirement_tables() {
    let capabilities = [
        Capability::Generation,
        Capability::ToolGeneration,
        Capability::Vision,
        Capability::Ocr,
        Capability::VoiceConfigured,
        Capability::VoiceLocal,
        Capability::VoiceOpenAi,
    ];
    for bits in 0..128 {
        for scenario in 0..24 {
            let mut input = member();
            input.context = if scenario % 2 == 0 {
                InteractionContext::Guild
            } else {
                InteractionContext::BotDm
            };
            let grant = [
                None,
                Some(DiscordPermission::ManageMessages),
                Some(DiscordPermission::ManageServer),
                Some(DiscordPermission::Administrator),
                Some(DiscordPermission::ModerateMembers),
                Some(DiscordPermission::ManageWebhooks),
            ][scenario % 6];
            input.permissions = grant.into_iter().collect();
            input.self_subject = Some(scenario % 3 == 0);
            input.caller_present_in_voice = Some(scenario % 4 < 2);
            input.application_owner = scenario % 5 == 0;
            input.follow_up_absent = Some(scenario % 3 == 1);
            input.vision_allowed = scenario % 4 != 0;
            input.action_target_resolved = scenario % 3 != 0;
            input.hierarchy_allows_action = Some(scenario % 3 == 1);
            input.selected_voice_mode = [
                SelectedVoiceMode::Off,
                SelectedVoiceMode::Local,
                SelectedVoiceMode::OpenAi,
            ][scenario % 3];
            input.readiness = capabilities
                .into_iter()
                .enumerate()
                .map(|(index, capability)| {
                    (
                        capability,
                        if bits & (1 << index) != 0 {
                            CapabilityReadiness::Ready
                        } else {
                            CapabilityReadiness::Blocked(Blocker::Busy)
                        },
                    )
                })
                .collect();
            let has = |index| bits & (1 << index) != 0;
            let permission =
                |p| grant == Some(p) || grant == Some(DiscordPermission::Administrator);
            let own = input.self_subject == Some(true);
            let present = input.caller_present_in_voice == Some(true);
            for spec in registered_commands() {
                let access = match spec.eligibility.access {
                    AccessId::A0 => true,
                    AccessId::A1 => {
                        own || permission(DiscordPermission::ManageMessages)
                            || permission(DiscordPermission::ManageServer)
                    }
                    AccessId::A2 => permission(DiscordPermission::ModerateMembers),
                    AccessId::A3 => permission(DiscordPermission::ManageWebhooks),
                    AccessId::A4 => permission(DiscordPermission::ManageServer),
                    AccessId::A5 => permission(DiscordPermission::ManageServer) && present,
                    AccessId::A6 => permission(DiscordPermission::ManageServer) || present,
                    AccessId::A7 => {
                        input.application_owner || permission(DiscordPermission::Administrator)
                    }
                } && spec.registration.contexts.contains(&input.context)
                    && !(input.context == InteractionContext::BotDm
                        && spec.eligibility.access == AccessId::A1
                        && !own);
                let condition = match spec.eligibility.condition {
                    ConditionId::C0 => true,
                    ConditionId::C1 => has(0),
                    ConditionId::C2 => input.vision_allowed && has(2),
                    ConditionId::C3 => {
                        input.vision_allowed
                            && has(2)
                            && (input.follow_up_absent == Some(true) || has(0))
                    }
                    ConditionId::C4 => has(4),
                    ConditionId::C5 => {
                        has(4)
                            && match input.selected_voice_mode {
                                SelectedVoiceMode::Off => false,
                                SelectedVoiceMode::Local => has(5),
                                SelectedVoiceMode::OpenAi => has(6),
                            }
                    }
                    ConditionId::C6 => has(4) && has(5),
                    ConditionId::C7 => {
                        input.action_target_resolved && input.hierarchy_allows_action == Some(true)
                    }
                    ConditionId::C8 => has(1),
                    ConditionId::C9 => input.vision_allowed && has(3),
                };
                assert_eq!(
                    eligible(spec, &input, EvaluationMode::Invocation),
                    access && condition,
                    "{:?} {bits} {scenario}",
                    spec.key
                );
                assert_eq!(
                    eligible(spec, &input, EvaluationMode::Discoverability),
                    access,
                    "{:?} {bits} {scenario}",
                    spec.key
                );
            }
        }
    }
}
