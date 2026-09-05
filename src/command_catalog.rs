//! Pure Discord surface policy. No transport, clock, environment, or live identities.

mod data;
#[cfg(test)]
use data::BOTH;
use data::{PLANNED, REGISTERED};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKey {
    Help,
    PersonaRoute,
    PersonaAsk,
    Whois,
    Profile,
    AskMessage,
    Perms,
    Modcall,
    Server,
    Webhook,
    Remember,
    Forget,
    PendingList,
    PendingConfirm,
    PendingDismiss,
    Recall,
    Reputation,
    MemoryMenu,
    Summarize,
    See,
    Ocr,
    DescribeImage,
    ReadImage,
    Stats,
    AdminShow,
    AdminPersona,
    AdminLearning,
    AdminVision,
    AdminCooldown,
    AdminAct,
    AdminBudget,
    AdminBrain,
    AdminFlush,
    AdminExport,
    AdminReset,
    AdminDashboard,
    VoiceConsent,
    VoiceNotice,
    VoiceJoin,
    VoiceResume,
    VoiceLeave,
    VoiceStatus,
    VoiceDiagnostics,
    VoiceMode,
    VoiceVerifyStart,
    VoiceVerifyReport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    Slash,
    UserContext,
    MessageContext,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionContext {
    Guild,
    BotDm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordPermission {
    ManageMessages,
    ModerateMembers,
    ManageWebhooks,
    ManageServer,
    Administrator,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessRule {
    Allow,
    Permission(DiscordPermission),
    CallerPresentInVoice,
    SelfSubject,
    ApplicationOwner,
    All(&'static [Self]),
    Any(&'static [Self]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Generation,
    Vision,
    VoiceConfigured,
    VoiceLocal,
    VoiceOpenAi,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPredicate {
    FollowUpAbsent,
    ActionTargetResolved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionRule {
    Always,
    Available(Capability),
    Input(InputPredicate),
    SelectedVoiceModeReady,
    HierarchyAllowsAction,
    All(&'static [Self]),
    Any(&'static [Self]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessId {
    A0,
    A1,
    A2,
    A3,
    A4,
    A5,
    A6,
    A7,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionId {
    C0,
    C1,
    C2,
    C3,
    C4,
    C5,
    C6,
    C7,
}
impl AccessId {
    pub const fn rule(self) -> AccessRule {
        use AccessRule::*;
        use DiscordPermission::*;
        match self {
            Self::A0 => Allow,
            Self::A1 => Any(&[
                SelfSubject,
                Permission(ManageMessages),
                Permission(ManageServer),
                Permission(Administrator),
            ]),
            Self::A2 => Permission(ModerateMembers),
            Self::A3 => Permission(ManageWebhooks),
            Self::A4 => Permission(ManageServer),
            Self::A5 => All(&[Permission(ManageServer), CallerPresentInVoice]),
            Self::A6 => Any(&[CallerPresentInVoice, Permission(ManageServer)]),
            Self::A7 => Any(&[ApplicationOwner, Permission(Administrator)]),
        }
    }
}
impl ConditionId {
    pub const fn rule(self) -> ConditionRule {
        use Capability::*;
        use ConditionRule::*;
        match self {
            Self::C0 => Always,
            Self::C1 => Available(Generation),
            Self::C2 => Available(Vision),
            Self::C3 => All(&[
                Available(Vision),
                Any(&[Input(InputPredicate::FollowUpAbsent), Available(Generation)]),
            ]),
            Self::C4 => Available(VoiceConfigured),
            Self::C5 => All(&[Available(VoiceConfigured), SelectedVoiceModeReady]),
            Self::C6 => All(&[Available(VoiceConfigured), Available(VoiceLocal)]),
            Self::C7 => HierarchyAllowsAction,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EligibilityRule {
    pub access: AccessId,
    pub condition: ConditionId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationPolicy {
    pub contexts: &'static [InteractionContext],
    pub default_member_permissions: Option<DiscordPermission>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpSection {
    Start,
    Conversation,
    Memory,
    Images,
    Moderation,
    Server,
    Voice,
    Administration,
}
impl HelpSection {
    pub const ALL: [Self; 8] = [
        Self::Start,
        Self::Conversation,
        Self::Memory,
        Self::Images,
        Self::Moderation,
        Self::Server,
        Self::Voice,
        Self::Administration,
    ];
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Conversation => "conversation",
            Self::Memory => "memory",
            Self::Images => "images",
            Self::Moderation => "moderation",
            Self::Server => "server",
            Self::Voice => "voice",
            Self::Administration => "administration",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Conversation => "Conversation",
            Self::Memory => "Memory",
            Self::Images => "Images",
            Self::Moderation => "Moderation",
            Self::Server => "Server",
            Self::Voice => "Voice",
            Self::Administration => "Administration",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.slug() == value)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationStatus {
    Registered,
    Planned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub key: CommandKey,
    pub kind: CommandKind,
    pub name: &'static str,
    pub registration: RegistrationPolicy,
    pub eligibility: EligibilityRule,
    pub section: HelpSection,
    pub description: &'static str,
    pub private: bool,
    pub status: ImplementationStatus,
}
impl CommandSpec {
    /// Approved target policy, kept distinct from the temporary operator-only
    /// status adapter until Task 6 supplies a safe member projection.
    #[cfg(test)]
    pub const fn target_eligibility(&self) -> EligibilityRule {
        match self.key {
            CommandKey::VoiceStatus => EligibilityRule {
                access: AccessId::A0,
                condition: ConditionId::C4,
            },
            _ => self.eligibility,
        }
    }
}
pub const fn registered_commands() -> &'static [CommandSpec] {
    REGISTERED
}
pub const fn planned_commands() -> &'static [CommandSpec] {
    PLANNED
}
pub fn command(key: CommandKey) -> &'static CommandSpec {
    REGISTERED
        .iter()
        .chain(planned_commands())
        .find(|spec| spec.key == key)
        .expect("exhaustive command catalog")
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedVoiceMode {
    Off,
    Local,
    OpenAi,
}
#[derive(Debug, Clone)]
pub struct EligibilityInput {
    pub context: InteractionContext,
    pub permissions: Vec<DiscordPermission>,
    pub self_subject: Option<bool>,
    pub application_owner: bool,
    pub caller_present_in_voice: Option<bool>,
    pub selected_voice_mode: SelectedVoiceMode,
    pub capabilities: Vec<Capability>,
    pub follow_up_absent: Option<bool>,
    pub action_target_resolved: bool,
    pub hierarchy_allows_action: Option<bool>,
}
impl EligibilityInput {
    pub fn new(context: InteractionContext) -> Self {
        Self {
            context,
            permissions: Vec::new(),
            self_subject: None,
            application_owner: false,
            caller_present_in_voice: None,
            selected_voice_mode: SelectedVoiceMode::Off,
            capabilities: Vec::new(),
            follow_up_absent: None,
            action_target_resolved: false,
            hierarchy_allows_action: None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationMode {
    Invocation,
    Discoverability,
}
pub fn access_allows(rule: AccessRule, input: &EligibilityInput) -> bool {
    fn evaluate(rule: AccessRule, input: &EligibilityInput, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        match rule {
            AccessRule::Allow => true,
            AccessRule::Permission(permission) => {
                input.permissions.contains(&permission)
                    || input
                        .permissions
                        .contains(&DiscordPermission::Administrator)
            }
            AccessRule::SelfSubject => input.self_subject == Some(true),
            AccessRule::ApplicationOwner => input.application_owner,
            AccessRule::CallerPresentInVoice => input.caller_present_in_voice == Some(true),
            AccessRule::All(rules) => {
                !rules.is_empty() && rules.iter().all(|rule| evaluate(*rule, input, depth + 1))
            }
            AccessRule::Any(rules) => {
                !rules.is_empty() && rules.iter().any(|rule| evaluate(*rule, input, depth + 1))
            }
        }
    }
    evaluate(rule, input, 0)
}
pub fn condition_allows(
    rule: ConditionRule,
    input: &EligibilityInput,
    mode: EvaluationMode,
) -> bool {
    fn evaluate(
        rule: ConditionRule,
        input: &EligibilityInput,
        mode: EvaluationMode,
        depth: usize,
    ) -> bool {
        if depth > 32 {
            return false;
        }
        match rule {
            ConditionRule::Always => true,
            ConditionRule::Available(capability) => input.capabilities.contains(&capability),
            ConditionRule::Input(InputPredicate::FollowUpAbsent) => {
                input.follow_up_absent == Some(true)
            }
            ConditionRule::Input(InputPredicate::ActionTargetResolved) => {
                input.action_target_resolved
            }
            ConditionRule::SelectedVoiceModeReady => match input.selected_voice_mode {
                SelectedVoiceMode::Off => false,
                SelectedVoiceMode::Local => input.capabilities.contains(&Capability::VoiceLocal),
                SelectedVoiceMode::OpenAi => input.capabilities.contains(&Capability::VoiceOpenAi),
            },
            ConditionRule::HierarchyAllowsAction => {
                if mode == EvaluationMode::Discoverability && !input.action_target_resolved {
                    true
                } else {
                    evaluate(
                        ConditionRule::Input(InputPredicate::ActionTargetResolved),
                        input,
                        mode,
                        depth + 1,
                    ) && input.hierarchy_allows_action == Some(true)
                }
            }
            ConditionRule::All(rules) => {
                !rules.is_empty()
                    && rules
                        .iter()
                        .all(|rule| evaluate(*rule, input, mode, depth + 1))
            }
            ConditionRule::Any(rules) => {
                !rules.is_empty()
                    && rules
                        .iter()
                        .any(|rule| evaluate(*rule, input, mode, depth + 1))
            }
        }
    }
    evaluate(rule, input, mode, 0)
}
pub fn eligible(spec: &CommandSpec, input: &EligibilityInput, mode: EvaluationMode) -> bool {
    spec.status == ImplementationStatus::Registered
        && spec.registration.contexts.contains(&input.context)
        && !(input.context == InteractionContext::BotDm
            && spec.eligibility.access == AccessId::A1
            && input.self_subject != Some(true))
        && access_allows(spec.eligibility.access.rule(), input)
        && condition_allows(spec.eligibility.condition.rule(), input, mode)
}
pub fn render_help(section: HelpSection, input: &EligibilityInput) -> String {
    let mut text = format!(
        "**Abbey · {}**\nChoose a section below. Only commands usable here are listed.\n\n",
        section.label()
    );
    let mut count = 0;
    for spec in REGISTERED.iter().filter(|spec| {
        spec.section == section && eligible(spec, input, EvaluationMode::Discoverability)
    }) {
        let prefix = if spec.kind == CommandKind::Slash {
            "/"
        } else {
            ""
        };
        text.push_str(&format!("`{prefix}{}` — {}\n", spec.name, spec.description));
        count += 1;
    }
    if count == 0 {
        text.push_str("No commands in this section are currently available to you.\n");
    }
    text.push_str("\nSome commands may be hidden because a permission or deployment capability is unavailable. Use `/help` to start a new private session; controls expire after 15 minutes.");
    text
}
#[cfg(test)]
pub fn render_readme() -> String {
    debug_assert_eq!(
        command(CommandKey::VoiceStatus).target_eligibility().access,
        AccessId::A0
    );
    let mut out = "<!-- BEGIN GENERATED COMMAND CATALOG -->\n| Command | Context | Response | What it does |\n|---|---|---|---|\n".to_string();
    for spec in REGISTERED {
        let prefix = if spec.kind == CommandKind::Slash {
            "/"
        } else {
            ""
        };
        let context = if spec.registration.contexts == BOTH {
            "guild, bot DM"
        } else {
            "guild"
        };
        let visibility = if spec.private { "private" } else { "public" };
        out.push_str(&format!(
            "| `{prefix}{}` | {context} | {visibility} | {} |\n",
            spec.name, spec.description
        ));
    }
    out.push_str("\nPlanned (not registered or shown as usable in help): ");
    out.push_str(
        &planned_commands()
            .iter()
            .map(|spec| format!("`{}`", spec.name))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(". Member-safe `/voice status` and typed voice-mode choices also remain Task 6 work; the current status is private and manager-only.\n<!-- END GENERATED COMMAND CATALOG -->");
    out
}

#[cfg(test)]
mod tests;
