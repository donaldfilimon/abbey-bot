//! Pure Discord surface policy. No transport, clock, environment, or live identities.

mod availability;
mod data;
mod decisions;
pub use availability::{Availability, Blocker, availability};
#[cfg(test)]
use data::BOTH;
use data::{PLANNED, REGISTERED};
pub use decisions::{access_decision, condition_decision};

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
    ForumDraft,
    ForumPost,
    ForumPerms,
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
    VoicePlay,
    VoicePause,
    VoiceResumeMusic,
    VoiceStopMusic,
    VoiceVolume,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    Generation,
    ToolGeneration,
    Vision,
    Ocr,
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
    GuildVisionAllowed,
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
    C8,
    C9,
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
            Self::C2 => All(&[GuildVisionAllowed, Available(Vision)]),
            Self::C3 => All(&[
                GuildVisionAllowed,
                Available(Vision),
                Any(&[Input(InputPredicate::FollowUpAbsent), Available(Generation)]),
            ]),
            Self::C4 => Available(VoiceConfigured),
            Self::C5 => All(&[Available(VoiceConfigured), SelectedVoiceModeReady]),
            Self::C6 => All(&[Available(VoiceConfigured), Available(VoiceLocal)]),
            Self::C7 => HierarchyAllowsAction,
            Self::C8 => Available(ToolGeneration),
            Self::C9 => All(&[GuildVisionAllowed, Available(Ocr)]),
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
/// Exactly one current observation per capability; absence means unconfigured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityReadiness {
    Ready,
    Blocked(Blocker),
}
#[derive(Debug, Clone)]
pub struct EligibilityInput {
    pub context: InteractionContext,
    pub permissions: Vec<DiscordPermission>,
    pub self_subject: Option<bool>,
    pub application_owner: bool,
    pub caller_present_in_voice: Option<bool>,
    pub selected_voice_mode: SelectedVoiceMode,
    pub readiness: std::collections::BTreeMap<Capability, CapabilityReadiness>,
    pub vision_allowed: bool,
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
            readiness: std::collections::BTreeMap::new(),
            vision_allowed: true,
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
    access_decision(rule, input).is_ok()
}
#[cfg(test)]
fn condition_allows(rule: ConditionRule, input: &EligibilityInput, mode: EvaluationMode) -> bool {
    condition_decision(rule, input, mode).is_ok()
}
pub fn eligible(spec: &CommandSpec, input: &EligibilityInput, mode: EvaluationMode) -> bool {
    let result = availability(spec, input);
    match mode {
        EvaluationMode::Invocation => result == Availability::Ready,
        EvaluationMode::Discoverability => !matches!(result, Availability::AccessBlocked(_)),
    }
}
pub fn render_help(section: HelpSection, input: &EligibilityInput) -> String {
    let mut text = format!(
        "**Abbey · {}**\nChoose a section. Commands below are permitted here; execution rechecks readiness.\n\n",
        section.label()
    );
    let mut count = 0;
    let mut member_menu = false;
    let mut message_menu = false;
    for spec in REGISTERED.iter().filter(|spec| {
        spec.section == section && eligible(spec, input, EvaluationMode::Discoverability)
    }) {
        let prefix = if spec.kind == CommandKind::Slash {
            "/"
        } else {
            ""
        };
        text.push_str(&format!(
            "`{prefix}{}` ({}; {}) · {}\n",
            spec.name,
            crate::help_center::invocation_hint(spec.kind),
            crate::help_center::visibility_hint(spec.private, input.context),
            match availability(spec, input) {
                Availability::Blocked(reason) => reason.label(),
                _ => spec.description,
            }
        ));
        member_menu |= spec.kind == CommandKind::UserContext;
        message_menu |= spec.kind == CommandKind::MessageContext;
        count += 1;
    }
    if count == 0 {
        text.push_str("No commands in this section are currently available to you.\n");
    }
    if section == HelpSection::Start {
        text.push_str("\nTask buttons open private workflows. Use the section menu for the command reference.\n");
    }
    match (member_menu, message_menu) {
        (true, true) => {
            text.push_str("\nOpen the member or message menu, then Apps, for its listed actions.\n")
        }
        (true, false) => {
            text.push_str("\nOpen the member menu, then Apps, for its listed actions.\n")
        }
        (false, true) => {
            text.push_str("\nOpen the message menu, then Apps, for its listed actions.\n")
        }
        (false, false) => {}
    }
    text.push_str("\nProvider health is checked when you run a command. Controls expire 15 minutes after opening; `/help` starts a new private session.");
    text
}
#[cfg(test)]
pub fn render_readme() -> String {
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
    out.push_str("\nThe member voice status, typed voice-mode choices, manager diagnostics, and classic administration dashboard are registered surfaces.\n<!-- END GENERATED COMMAND CATALOG -->");
    out
}

#[cfg(test)]
mod tests;
