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
                input.capabilities = [
                    Capability::Generation,
                    Capability::Vision,
                    Capability::VoiceConfigured,
                    Capability::VoiceLocal,
                    Capability::VoiceOpenAi,
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(index, cap)| (bits & (1 << index) != 0).then_some(cap))
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
    input.capabilities = vec![Capability::Generation, Capability::Vision];
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
                input.capabilities = [
                    Capability::Generation,
                    Capability::Vision,
                    Capability::VoiceConfigured,
                    Capability::VoiceLocal,
                    Capability::VoiceOpenAi,
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(index, capability)| (bits & (1 << index) != 0).then_some(capability))
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
    input.capabilities = vec![Capability::VoiceConfigured, Capability::VoiceLocal];
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
                input.capabilities = vec![Capability::VoiceConfigured];
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
    input.capabilities = vec![Capability::Generation, Capability::Ocr];
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
    input
        .provider_blockers
        .push((Capability::ToolGeneration, Blocker::Busy));
    assert_eq!(
        availability(command(CommandKey::PersonaAsk), &input).message(),
        "The provider is busy. Wait briefly and try again."
    );
    input.provider_blockers[0].1 = Blocker::Unknown;
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
