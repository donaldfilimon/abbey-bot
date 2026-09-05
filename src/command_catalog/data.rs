//! Ordered registered and planned command specifications and their static defaults.

use super::{
    AccessId, CommandKey, CommandKind, CommandSpec, ConditionId, DiscordPermission,
    EligibilityRule, HelpSection, ImplementationStatus, InteractionContext, RegistrationPolicy,
};

pub(super) const BOTH: &[InteractionContext] =
    &[InteractionContext::Guild, InteractionContext::BotDm];
const GUILD: &[InteractionContext] = &[InteractionContext::Guild];
macro_rules! spec {
    ($key:ident, $kind:ident, $name:literal, $contexts:ident, $access:ident, $condition:ident, $section:ident, $private:literal, $description:literal) => {
        CommandSpec {
            key: CommandKey::$key,
            kind: CommandKind::$kind,
            name: $name,
            registration: RegistrationPolicy {
                contexts: $contexts,
                default_member_permissions: match AccessId::$access {
                    AccessId::A2 => Some(DiscordPermission::ModerateMembers),
                    AccessId::A3 => Some(DiscordPermission::ManageWebhooks),
                    AccessId::A4 | AccessId::A5 => Some(DiscordPermission::ManageServer),
                    _ => None,
                },
            },
            eligibility: EligibilityRule {
                access: AccessId::$access,
                condition: ConditionId::$condition,
            },
            section: HelpSection::$section,
            description: $description,
            private: $private,
            status: ImplementationStatus::Registered,
        }
    };
}
pub(super) const REGISTERED: &[CommandSpec] = &[
    spec!(
        Help,
        Slash,
        "help",
        BOTH,
        A0,
        C0,
        Start,
        true,
        "Browse commands available to you privately."
    ),
    spec!(
        PersonaRoute,
        Slash,
        "persona route",
        BOTH,
        A0,
        C0,
        Conversation,
        false,
        "Choose a persona and explain the routing."
    ),
    spec!(
        PersonaAsk,
        Slash,
        "persona ask",
        BOTH,
        A0,
        C1,
        Conversation,
        false,
        "Ask a question through the configured generation backend."
    ),
    spec!(
        Whois,
        Slash,
        "whois",
        GUILD,
        A0,
        C0,
        Server,
        false,
        "Read a member's profile and roles."
    ),
    spec!(
        Profile,
        UserContext,
        "Abbey: profile",
        GUILD,
        A0,
        C0,
        Server,
        true,
        "Read the selected member's profile privately."
    ),
    spec!(
        AskMessage,
        MessageContext,
        "Ask Abbey",
        BOTH,
        A0,
        C1,
        Conversation,
        true,
        "Ask about a selected message privately."
    ),
    spec!(
        Perms,
        Slash,
        "perms",
        GUILD,
        A0,
        C0,
        Server,
        false,
        "Explain a member's channel permissions."
    ),
    spec!(
        Modcall,
        Slash,
        "modcall",
        GUILD,
        A2,
        C7,
        Moderation,
        true,
        "Recommend a moderation action after permission and hierarchy checks."
    ),
    spec!(
        Server,
        Slash,
        "server",
        BOTH,
        A0,
        C0,
        Server,
        true,
        "Create a server blueprint without changing the server."
    ),
    spec!(
        Webhook,
        Slash,
        "webhook",
        GUILD,
        A3,
        C0,
        Server,
        true,
        "Show a safe incoming-webhook setup guide."
    ),
    spec!(
        Remember,
        Slash,
        "remember",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Store a fact about yourself; moderators may choose a member."
    ),
    spec!(
        Forget,
        Slash,
        "forget",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Remove a stored fact about yourself or an authorized member."
    ),
    spec!(
        PendingList,
        Slash,
        "pending list",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Review proposed fact replacements."
    ),
    spec!(
        PendingConfirm,
        Slash,
        "pending confirm",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Apply an explicitly chosen fact replacement."
    ),
    spec!(
        PendingDismiss,
        Slash,
        "pending dismiss",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Dismiss a proposed replacement and keep both facts."
    ),
    spec!(
        Recall,
        Slash,
        "recall",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Read your facts and standing, or an authorized member's."
    ),
    spec!(
        Reputation,
        Slash,
        "reputation",
        BOTH,
        A1,
        C0,
        Memory,
        true,
        "Read your standing privately; moderators may choose a member."
    ),
    spec!(
        Summarize,
        Slash,
        "summarize",
        BOTH,
        A0,
        C1,
        Conversation,
        false,
        "Summarize the recent conversation through the backend."
    ),
    spec!(
        See,
        Slash,
        "see",
        BOTH,
        A0,
        C3,
        Images,
        false,
        "Describe an image; an optional question also needs generation."
    ),
    spec!(
        Ocr,
        Slash,
        "ocr",
        BOTH,
        A0,
        C2,
        Images,
        false,
        "Read the text in an image."
    ),
    spec!(
        Stats,
        Slash,
        "stats",
        BOTH,
        A0,
        C0,
        Start,
        true,
        "Read command usage and learning statistics."
    ),
    spec!(
        AdminShow,
        Slash,
        "admin show",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Read this server's settings."
    ),
    spec!(
        AdminPersona,
        Slash,
        "admin persona",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Set the server's default persona."
    ),
    spec!(
        AdminLearning,
        Slash,
        "admin learning",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Control learning for this server."
    ),
    spec!(
        AdminVision,
        Slash,
        "admin vision",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Control image understanding for this server."
    ),
    spec!(
        AdminCooldown,
        Slash,
        "admin cooldown",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Set the unsolicited reply cooldown."
    ),
    spec!(
        AdminAct,
        Slash,
        "admin act",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Opt this server into unsolicited replies or turn them off."
    ),
    spec!(
        AdminBudget,
        Slash,
        "admin budget",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Set the hourly unsolicited reply budget."
    ),
    spec!(
        AdminBrain,
        Slash,
        "admin brain",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Inspect the learning policy and exploration setting."
    ),
    spec!(
        AdminFlush,
        Slash,
        "admin flush",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Persist current state and report each result."
    ),
    spec!(
        AdminExport,
        Slash,
        "admin export",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Export the server's brain snapshot privately."
    ),
    spec!(
        AdminReset,
        Slash,
        "admin reset",
        GUILD,
        A4,
        C0,
        Administration,
        true,
        "Clear only this channel's transcript."
    ),
    spec!(
        VoiceConsent,
        Slash,
        "voice consent",
        GUILD,
        A0,
        C4,
        Voice,
        true,
        "Review, agree to, or withdraw your voice choice."
    ),
    spec!(
        VoiceNotice,
        Slash,
        "voice notice",
        GUILD,
        A4,
        C4,
        Voice,
        true,
        "Publish the member voice consent controls."
    ),
    spec!(
        VoicePlay,
        Slash,
        "voice play",
        GUILD,
        A5,
        C4,
        Voice,
        true,
        "Play native music and mirror eligible host audio; requires macOS."
    ),
    spec!(
        VoicePause,
        Slash,
        "voice pause",
        GUILD,
        A5,
        C4,
        Voice,
        true,
        "Pause music and close host capture without changing consent."
    ),
    spec!(
        VoiceResumeMusic,
        Slash,
        "voice resume-music",
        GUILD,
        A5,
        C4,
        Voice,
        true,
        "Resume music only; never renew listening consent."
    ),
    spec!(
        VoiceStopMusic,
        Slash,
        "voice stop-music",
        GUILD,
        A5,
        C4,
        Voice,
        true,
        "Stop host audio capture and mirrored music."
    ),
    spec!(
        VoiceVolume,
        Slash,
        "voice volume",
        GUILD,
        A5,
        C4,
        Voice,
        true,
        "Set music volume; duck to one quarter while Abbey speaks."
    ),
    spec!(
        VoiceJoin,
        Slash,
        "voice join",
        GUILD,
        A5,
        C5,
        Voice,
        true,
        "Start voice after every participant's saved agreement."
    ),
    spec!(
        VoiceResume,
        Slash,
        "voice resume",
        GUILD,
        A5,
        C5,
        Voice,
        true,
        "Resume voice after current participant consent checks."
    ),
    spec!(
        VoiceLeave,
        Slash,
        "voice leave",
        GUILD,
        A6,
        C4,
        Voice,
        true,
        "Stop the current call immediately without deleting consent."
    ),
    // Task 6 must supply a member-safe projection before changing this A4 to A0.
    spec!(
        VoiceStatus,
        Slash,
        "voice status",
        GUILD,
        A4,
        C4,
        Voice,
        true,
        "Read private operator voice diagnostics (member view planned)."
    ),
    spec!(
        VoiceMode,
        Slash,
        "voice mode",
        GUILD,
        A4,
        C4,
        Voice,
        true,
        "Read or select a fully configured voice mode."
    ),
    spec!(
        VoiceVerifyStart,
        Slash,
        "voice verify start",
        GUILD,
        A7,
        C6,
        Voice,
        true,
        "Arm a local content-free voice acceptance run."
    ),
    spec!(
        VoiceVerifyReport,
        Slash,
        "voice verify report",
        GUILD,
        A7,
        C6,
        Voice,
        true,
        "Read the private local voice acceptance report."
    ),
];
pub(super) const PLANNED: &[CommandSpec] = &[
    CommandSpec {
        status: ImplementationStatus::Planned,
        ..spec!(
            MemoryMenu,
            UserContext,
            "Abbey: memory",
            GUILD,
            A1,
            C0,
            Memory,
            true,
            "Planned: a private member memory card."
        )
    },
    CommandSpec {
        status: ImplementationStatus::Planned,
        ..spec!(
            DescribeImage,
            MessageContext,
            "Abbey: describe image",
            BOTH,
            A0,
            C2,
            Images,
            true,
            "Planned: describe a selected image attachment privately."
        )
    },
    CommandSpec {
        status: ImplementationStatus::Planned,
        ..spec!(
            ReadImage,
            MessageContext,
            "Abbey: read image text",
            BOTH,
            A0,
            C2,
            Images,
            true,
            "Planned: read a selected image attachment privately."
        )
    },
    CommandSpec {
        status: ImplementationStatus::Planned,
        ..spec!(
            AdminDashboard,
            Slash,
            "admin dashboard",
            GUILD,
            A4,
            C0,
            Administration,
            true,
            "Planned: private administration controls."
        )
    },
    CommandSpec {
        status: ImplementationStatus::Planned,
        ..spec!(
            VoiceDiagnostics,
            Slash,
            "voice diagnostics",
            GUILD,
            A4,
            C4,
            Voice,
            true,
            "Planned: separate private operator voice diagnostics."
        )
    },
];
