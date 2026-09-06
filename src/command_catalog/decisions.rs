//! Canonical rule decisions. Every visited predicate is evaluated once;
//! invocation booleans and explanatory availability derive from these results.
use super::*;
type Decision = Result<(), Blocker>;
const MAX_DEPTH: usize = 32;

/// All requires every child and reports the first failed prerequisite.
fn all(mut decisions: impl Iterator<Item = Decision>) -> Decision {
    let first = decisions.next().ok_or(Blocker::Unavailable)?;
    first?;
    decisions.try_for_each(|decision| decision)
}

/// Any succeeds at its first satisfied alternative. Otherwise prefer a concrete
/// operational/presence repair over missing input, then permission. Equal-rank
/// failures retain declaration order, including failures from nested rules.
fn any(decisions: impl Iterator<Item = Decision>) -> Decision {
    fn rank(reason: Blocker) -> u8 {
        match reason {
            Blocker::Target => 1,
            Blocker::Permission | Blocker::Subject => 2,
            _ => 0,
        }
    }
    let mut failure = None;
    for decision in decisions {
        match decision {
            Ok(()) => return Ok(()),
            Err(reason) if failure.is_none_or(|previous| rank(reason) < rank(previous)) => {
                failure = Some(reason);
            }
            Err(_) => {}
        }
    }
    Err(failure.unwrap_or(Blocker::Unavailable))
}

pub fn access_decision(rule: AccessRule, input: &EligibilityInput) -> Decision {
    fn evaluate(rule: AccessRule, input: &EligibilityInput, depth: usize) -> Decision {
        if depth > MAX_DEPTH {
            return Err(Blocker::Unavailable);
        }
        match rule {
            AccessRule::Allow => Ok(()),
            AccessRule::Permission(permission) => (input.permissions.contains(&permission)
                || input
                    .permissions
                    .contains(&DiscordPermission::Administrator))
            .then_some(())
            .ok_or(Blocker::Permission),
            AccessRule::SelfSubject => (input.self_subject == Some(true))
                .then_some(())
                .ok_or(Blocker::Permission),
            AccessRule::ApplicationOwner => input
                .application_owner
                .then_some(())
                .ok_or(Blocker::Permission),
            AccessRule::CallerPresentInVoice => (input.caller_present_in_voice == Some(true))
                .then_some(())
                .ok_or(Blocker::VoicePresence),
            AccessRule::All(rules) => {
                all(rules.iter().map(|rule| evaluate(*rule, input, depth + 1)))
            }
            AccessRule::Any(rules) => {
                any(rules.iter().map(|rule| evaluate(*rule, input, depth + 1)))
            }
        }
    }
    evaluate(rule, input, 0)
}

fn capability_decision(capability: Capability, input: &EligibilityInput) -> Decision {
    let observation = input.readiness.get(&capability).copied().unwrap_or({
        CapabilityReadiness::Blocked(match capability {
            Capability::Generation | Capability::ToolGeneration => Blocker::Generation,
            Capability::Vision => Blocker::Vision,
            Capability::Ocr => Blocker::Ocr,
            Capability::VoiceConfigured => Blocker::VoiceSetup,
            Capability::VoiceLocal | Capability::VoiceOpenAi => Blocker::VoiceMode,
        })
    });
    match observation {
        CapabilityReadiness::Ready => Ok(()),
        CapabilityReadiness::Blocked(reason) => Err(reason),
    }
}

pub fn condition_decision(
    rule: ConditionRule,
    input: &EligibilityInput,
    mode: EvaluationMode,
) -> Decision {
    fn evaluate(
        rule: ConditionRule,
        input: &EligibilityInput,
        mode: EvaluationMode,
        depth: usize,
    ) -> Decision {
        if depth > MAX_DEPTH {
            return Err(Blocker::Unavailable);
        }
        match rule {
            ConditionRule::Always => Ok(()),
            ConditionRule::GuildVisionAllowed => input
                .vision_allowed
                .then_some(())
                .ok_or(Blocker::VisionPolicy),
            ConditionRule::Available(capability) => capability_decision(capability, input),
            ConditionRule::Input(InputPredicate::FollowUpAbsent) => (input.follow_up_absent
                == Some(true))
            .then_some(())
            .ok_or(Blocker::Target),
            ConditionRule::Input(InputPredicate::ActionTargetResolved) => input
                .action_target_resolved
                .then_some(())
                .ok_or(Blocker::Target),
            ConditionRule::SelectedVoiceModeReady => match input.selected_voice_mode {
                SelectedVoiceMode::Off => Err(Blocker::VoiceMode),
                SelectedVoiceMode::Local => capability_decision(Capability::VoiceLocal, input),
                SelectedVoiceMode::OpenAi => capability_decision(Capability::VoiceOpenAi, input),
            },
            ConditionRule::HierarchyAllowsAction => {
                if mode == EvaluationMode::Discoverability && !input.action_target_resolved {
                    return Ok(());
                }
                evaluate(
                    ConditionRule::Input(InputPredicate::ActionTargetResolved),
                    input,
                    mode,
                    depth + 1,
                )?;
                (input.hierarchy_allows_action == Some(true))
                    .then_some(())
                    .ok_or(Blocker::Hierarchy)
            }
            ConditionRule::All(rules) => all(rules
                .iter()
                .map(|rule| evaluate(*rule, input, mode, depth + 1))),
            ConditionRule::Any(rules) => any(rules
                .iter()
                .map(|rule| evaluate(*rule, input, mode, depth + 1))),
        }
    }
    evaluate(rule, input, mode, 0)
}
