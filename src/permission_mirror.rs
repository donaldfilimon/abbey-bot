//! Permission-mirror gate for guild mutations Abbey performs on request.
//!
//! Abbey may take a server action only when **both**:
//! 1. the requesting member currently holds the Discord permission for that action, and
//! 2. the bot currently holds the same capability in the guild.
//!
//! Fail closed on any missing bit. Administrator on either side satisfies every
//! permission check. Destructive actions require an explicit confirm flag.
//! Roles with any holders can never be deleted through this gate.

use serenity::all::Permissions;

/// Guild mutations Abbey can perform when permission-mirror allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerAction {
    ChannelCreate,
    ChannelEdit,
    ChannelDelete,
    RoleAssign,
    RoleRemove,
    /// Never succeeds when [`ActionContext::role_holders`] is `Some(n)` with `n > 0`.
    RoleDelete,
    Slowmode,
    MoveMember,
    PurgeMessages,
}

impl ServerAction {
    /// Discord permission bit both requester and bot must hold.
    pub const fn required_permission(self) -> Permissions {
        match self {
            Self::ChannelCreate | Self::ChannelEdit | Self::ChannelDelete | Self::Slowmode => {
                Permissions::MANAGE_CHANNELS
            }
            Self::RoleAssign | Self::RoleRemove | Self::RoleDelete => Permissions::MANAGE_ROLES,
            Self::MoveMember => Permissions::MOVE_MEMBERS,
            Self::PurgeMessages => Permissions::MANAGE_MESSAGES,
        }
    }

    /// Human label for refusals and audit copy.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ChannelCreate => "create a channel",
            Self::ChannelEdit => "edit a channel",
            Self::ChannelDelete => "delete a channel",
            Self::RoleAssign => "assign a role",
            Self::RoleRemove => "remove a role",
            Self::RoleDelete => "delete a role",
            Self::Slowmode => "set slowmode",
            Self::MoveMember => "move a member in voice",
            Self::PurgeMessages => "purge messages",
        }
    }

    /// Destructive by default — needs [`ActionContext::confirm`].
    pub const fn requires_confirm(self) -> bool {
        matches!(
            self,
            Self::ChannelDelete | Self::RoleDelete | Self::PurgeMessages
        )
    }
}

/// Extra facts the gate needs beyond the two permission bitfields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionContext {
    /// Explicit operator confirm for destructive actions.
    pub confirm: bool,
    /// Known holders of a role targeted for deletion. `None` means unknown —
    /// unknown is treated as unsafe and denied.
    pub role_holders: Option<u64>,
}

/// Why the gate refused. Prefer these over unstructured strings in adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denial {
    RequesterMissing {
        action: ServerAction,
        required: Permissions,
    },
    BotMissing {
        action: ServerAction,
        required: Permissions,
    },
    ConfirmRequired {
        action: ServerAction,
    },
    RoleHasHolders {
        holders: u64,
    },
    RoleHolderCountUnknown,
}

impl Denial {
    pub fn message(&self) -> String {
        match self {
            Self::RequesterMissing { action, required } => format!(
                "You need **{}** to {} — Abbey will not do it for you without that permission.",
                permission_label(*required),
                action.label()
            ),
            Self::BotMissing { action, required } => format!(
                "Abbey is missing **{}**, so she cannot {} even though you can.",
                permission_label(*required),
                action.label()
            ),
            Self::ConfirmRequired { action } => format!(
                "Refusing to {} without `confirm:true` — destructive by default stays off.",
                action.label()
            ),
            Self::RoleHasHolders { holders } => format!(
                "Refusing to delete a role that still has **{holders}** member(s). Remove the role from everyone first."
            ),
            Self::RoleHolderCountUnknown => {
                "Refusing to delete a role whose member count could not be verified.".into()
            }
        }
    }
}

fn permission_label(required: Permissions) -> &'static str {
    if required.contains(Permissions::MANAGE_CHANNELS) {
        "Manage Channels"
    } else if required.contains(Permissions::MANAGE_ROLES) {
        "Manage Roles"
    } else if required.contains(Permissions::MOVE_MEMBERS) {
        "Move Members"
    } else if required.contains(Permissions::MANAGE_MESSAGES) {
        "Manage Messages"
    } else {
        "the required permission"
    }
}

fn holds(bits: Permissions, required: Permissions) -> bool {
    bits.contains(Permissions::ADMINISTRATOR) || bits.contains(required)
}

/// Fail-closed authorization: requester and bot must both hold the action's
/// permission; destructive actions need confirm; role delete never proceeds
/// with holders > 0 (or unknown holder count).
pub fn authorize(
    action: ServerAction,
    requester: Permissions,
    bot: Permissions,
    context: ActionContext,
) -> Result<(), Denial> {
    let required = action.required_permission();

    if !holds(requester, required) {
        return Err(Denial::RequesterMissing { action, required });
    }
    if !holds(bot, required) {
        return Err(Denial::BotMissing { action, required });
    }
    if action.requires_confirm() && !context.confirm {
        return Err(Denial::ConfirmRequired { action });
    }
    if action == ServerAction::RoleDelete {
        match context.role_holders {
            None => return Err(Denial::RoleHolderCountUnknown),
            Some(0) => {}
            Some(holders) => return Err(Denial::RoleHasHolders { holders }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Permissions {
        Permissions::empty()
    }

    #[test]
    fn allow_when_both_hold_manage_channels() {
        let bits = Permissions::MANAGE_CHANNELS;
        assert!(
            authorize(
                ServerAction::ChannelCreate,
                bits,
                bits,
                ActionContext::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn deny_when_requester_missing_permission() {
        let err = authorize(
            ServerAction::ChannelCreate,
            empty(),
            Permissions::MANAGE_CHANNELS,
            ActionContext::default(),
        )
        .expect_err("requester missing");
        assert!(matches!(err, Denial::RequesterMissing { .. }));
        assert!(err.message().contains("Manage Channels"));
    }

    #[test]
    fn deny_when_bot_missing_permission() {
        let err = authorize(
            ServerAction::PurgeMessages,
            Permissions::MANAGE_MESSAGES,
            empty(),
            ActionContext {
                confirm: true,
                ..ActionContext::default()
            },
        )
        .expect_err("bot missing");
        assert!(matches!(err, Denial::BotMissing { .. }));
        assert!(err.message().contains("Abbey is missing"));
    }

    #[test]
    fn administrator_on_requester_satisfies_permission() {
        assert!(
            authorize(
                ServerAction::MoveMember,
                Permissions::ADMINISTRATOR,
                Permissions::MOVE_MEMBERS,
                ActionContext::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn administrator_on_bot_satisfies_permission() {
        assert!(
            authorize(
                ServerAction::RoleAssign,
                Permissions::MANAGE_ROLES,
                Permissions::ADMINISTRATOR,
                ActionContext::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn channel_delete_requires_confirm() {
        let bits = Permissions::MANAGE_CHANNELS;
        let err = authorize(
            ServerAction::ChannelDelete,
            bits,
            bits,
            ActionContext::default(),
        )
        .expect_err("confirm required");
        assert_eq!(
            err,
            Denial::ConfirmRequired {
                action: ServerAction::ChannelDelete
            }
        );

        assert!(
            authorize(
                ServerAction::ChannelDelete,
                bits,
                bits,
                ActionContext {
                    confirm: true,
                    ..ActionContext::default()
                },
            )
            .is_ok()
        );
    }

    #[test]
    fn purge_requires_confirm_and_manage_messages() {
        let bits = Permissions::MANAGE_MESSAGES;
        assert!(matches!(
            authorize(
                ServerAction::PurgeMessages,
                bits,
                bits,
                ActionContext::default()
            ),
            Err(Denial::ConfirmRequired { .. })
        ));
        assert!(
            authorize(
                ServerAction::PurgeMessages,
                bits,
                bits,
                ActionContext {
                    confirm: true,
                    ..ActionContext::default()
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn role_delete_never_with_holders() {
        let bits = Permissions::MANAGE_ROLES;
        let err = authorize(
            ServerAction::RoleDelete,
            bits,
            bits,
            ActionContext {
                confirm: true,
                role_holders: Some(3),
            },
        )
        .expect_err("holders");
        assert_eq!(err, Denial::RoleHasHolders { holders: 3 });
    }

    #[test]
    fn role_delete_unknown_holders_fail_closed() {
        let bits = Permissions::MANAGE_ROLES;
        let err = authorize(
            ServerAction::RoleDelete,
            bits,
            bits,
            ActionContext {
                confirm: true,
                role_holders: None,
            },
        )
        .expect_err("unknown");
        assert_eq!(err, Denial::RoleHolderCountUnknown);
    }

    #[test]
    fn role_delete_zero_holders_with_confirm_ok() {
        let bits = Permissions::MANAGE_ROLES;
        assert!(
            authorize(
                ServerAction::RoleDelete,
                bits,
                bits,
                ActionContext {
                    confirm: true,
                    role_holders: Some(0),
                },
            )
            .is_ok()
        );
    }

    #[test]
    fn success_path_move_member_mocked() {
        // Pure success path used by adapters before any Discord REST call.
        let plan = authorize(
            ServerAction::MoveMember,
            Permissions::MOVE_MEMBERS,
            Permissions::MOVE_MEMBERS,
            ActionContext::default(),
        );
        assert_eq!(plan, Ok(()));
    }


    #[test]
    fn non_destructive_channel_edit_needs_no_confirm() {
        let bits = Permissions::MANAGE_CHANNELS;
        assert!(
            authorize(
                ServerAction::ChannelEdit,
                bits,
                bits,
                ActionContext::default()
            )
            .is_ok()
        );
        assert!(authorize(ServerAction::Slowmode, bits, bits, ActionContext::default()).is_ok());
    }
}
