use super::*;

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
    assert_eq!(registered_commands().len(), 46);
    assert_eq!(planned_commands().len(), 5);
    let input = member();
    for rule in [AccessRule::All(&[]), AccessRule::Any(&[])] {
        assert!(!access_allows(rule, &input));
    }
    for rule in [ConditionRule::All(&[]), ConditionRule::Any(&[])] {
        assert!(!condition_allows(rule, &input, EvaluationMode::Invocation));
    }
}
#[test]
fn planned_features_and_operator_voice_status_cannot_be_advertised_to_members() {
    let mut input = member();
    input.capabilities = vec![Capability::VoiceConfigured, Capability::VoiceLocal];
    input.selected_voice_mode = SelectedVoiceMode::Local;
    for spec in planned_commands() {
        assert!(!eligible(spec, &input, EvaluationMode::Discoverability));
    }
    assert_eq!(
        command(CommandKey::VoiceStatus).eligibility.access,
        AccessId::A4
    );
    assert_eq!(
        command(CommandKey::VoiceStatus).target_eligibility().access,
        AccessId::A0
    );
    assert!(!render_help(HelpSection::Voice, &input).contains("`/voice status`"));
    assert!(render_help(HelpSection::Voice, &input).contains("`/voice consent`"));
    assert!(!render_help(HelpSection::Images, &input).contains("`/ocr`"));
    assert!(!render_help(HelpSection::Conversation, &input).contains("`/persona ask`"));
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
