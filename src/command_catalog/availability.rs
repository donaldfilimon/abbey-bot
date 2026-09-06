//! Pure authorization and operational blockers shared by discovery and invocation.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Ready,
    AccessBlocked(Blocker),
    Blocked(Blocker),
}
impl Availability {
    pub fn message(self) -> &'static str {
        match self {
            Self::Ready => "Available to attempt; execution checks current readiness.",
            Self::AccessBlocked(reason) | Self::Blocked(reason) => reason.message(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    Context,
    Permission,
    VoicePresence,
    Subject,
    Generation,
    Vision,
    Ocr,
    VisionPolicy,
    VoiceSetup,
    VoiceMode,
    Target,
    Hierarchy,
    Unknown,
    Busy,
    Unavailable,
}
impl Blocker {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Generation => "Needs generation setup",
            Self::Vision => "Needs image-description setup",
            Self::Ocr => "Needs text-extraction setup",
            Self::VisionPolicy => "Images disabled by server policy",
            Self::VoiceSetup => "Needs voice setup",
            Self::VoiceMode => "Selected voice mode unavailable",
            Self::Target => "Needs a member and action",
            Self::Hierarchy => "Blocked by role hierarchy",
            Self::Unknown => "Readiness unknown; ask a manager to check",
            Self::Busy => "Provider busy; retry shortly",
            Self::Unavailable => "Provider unavailable; retry later",
            Self::Context => "Requires a server",
            Self::Permission => "Requires permission",
            Self::VoicePresence => "Requires presence in the voice channel",
            Self::Subject => "Requires your own subject",
        }
    }
    pub const fn message(self) -> &'static str {
        match self {
            Self::Context => "Use this command in a server where Abbey is installed.",
            Self::Permission => {
                "You do not have the required permission for this command. Ask a server manager or open /help for your permitted actions."
            }
            Self::VoicePresence => {
                "Join Abbey's voice channel before using this control. Use /voice status to check the current call."
            }
            Self::Subject => "In a DM, use this command for your own memory only.",
            Self::Generation => {
                "No eligible generation route is configured for this operation. Ask a manager to check provider setup, then retry."
            }
            Self::Vision => {
                "Image description is unavailable. Ask a manager to check image-description provider setup, then retry."
            }
            Self::Ocr => {
                "Text extraction is unavailable. Ask a manager to check OCR provider setup, then retry."
            }
            Self::VisionPolicy => {
                "Images are disabled for this server. Ask a manager to review /admin vision."
            }
            Self::VoiceSetup => {
                "Voice is not configured here. Open /voice status for the current state and ask a manager to check setup."
            }
            Self::VoiceMode => {
                "The selected voice mode is unavailable. Open /voice status and ask a manager to check /voice mode."
            }
            Self::Target => "Choose a member and an action, then run /modcall again.",
            Self::Hierarchy => {
                "Discord's role hierarchy prevents that action. Choose a permitted target or ask a manager to review the roles."
            }
            Self::Unknown => {
                "Current provider readiness is unknown or needs requalification. Ask a manager to check provider diagnostics, then retry."
            }
            Self::Busy => "The provider is busy. Wait briefly and try again.",
            Self::Unavailable => {
                "The provider is temporarily unavailable. Try again later; if it persists, ask a manager to check provider diagnostics."
            }
        }
    }
}
fn missing(capability: Capability, input: &EligibilityInput) -> Blocker {
    input
        .provider_blockers
        .iter()
        .find_map(|(cap, reason)| (*cap == capability).then_some(*reason))
        .unwrap_or(match capability {
            Capability::Generation | Capability::ToolGeneration => Blocker::Generation,
            Capability::Vision => Blocker::Vision,
            Capability::Ocr => Blocker::Ocr,
            Capability::VoiceConfigured => Blocker::VoiceSetup,
            Capability::VoiceLocal | Capability::VoiceOpenAi => Blocker::VoiceMode,
        })
}
fn condition_blocker(rule: ConditionRule, input: &EligibilityInput) -> Option<Blocker> {
    if condition_allows(rule, input, EvaluationMode::Invocation) {
        return None;
    }
    Some(match rule {
        ConditionRule::Available(capability) => missing(capability, input),
        ConditionRule::SelectedVoiceModeReady => Blocker::VoiceMode,
        ConditionRule::HierarchyAllowsAction if input.action_target_resolved => Blocker::Hierarchy,
        ConditionRule::HierarchyAllowsAction | ConditionRule::Input(_) => Blocker::Target,
        ConditionRule::All(rules) => rules
            .iter()
            .find_map(|rule| condition_blocker(*rule, input))
            .unwrap_or(Blocker::Unavailable),
        ConditionRule::Any(rules) => rules
            .iter()
            .filter(|rule| !matches!(rule, ConditionRule::Input(_)))
            .chain(
                rules
                    .iter()
                    .filter(|rule| matches!(rule, ConditionRule::Input(_))),
            )
            .find_map(|rule| condition_blocker(*rule, input))
            .unwrap_or(Blocker::Unavailable),
        ConditionRule::Always => return None,
    })
}
pub fn availability(spec: &CommandSpec, input: &EligibilityInput) -> Availability {
    if !spec.registration.contexts.contains(&input.context) {
        return Availability::AccessBlocked(Blocker::Context);
    }
    if input.context == InteractionContext::BotDm
        && spec.eligibility.access == AccessId::A1
        && input.self_subject != Some(true)
    {
        return Availability::AccessBlocked(Blocker::Subject);
    }
    if !access_allows(spec.eligibility.access.rule(), input) {
        // Distinguish missing presence only when permissions would otherwise allow access.
        let mut present = input.clone();
        present.caller_present_in_voice = Some(true);
        return Availability::AccessBlocked(
            if access_allows(spec.eligibility.access.rule(), &present) {
                Blocker::VoicePresence
            } else {
                Blocker::Permission
            },
        );
    }
    if spec.section == HelpSection::Images && !input.vision_allowed {
        return Availability::Blocked(Blocker::VisionPolicy);
    }
    condition_blocker(spec.eligibility.condition.rule(), input)
        .map_or(Availability::Ready, Availability::Blocked)
}
