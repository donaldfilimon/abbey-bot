//! The `spec!` constructor for registered command specifications; item paths
//! resolve at the call site in `data.rs`.

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
                    AccessId::A8 => Some(DiscordPermission::ManageChannels),
                    AccessId::A9 => Some(DiscordPermission::ManageRoles),
                    AccessId::A10 => Some(DiscordPermission::MoveMembers),
                    AccessId::A11 => Some(DiscordPermission::ManageMessages),
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
