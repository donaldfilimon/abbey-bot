//! Pure `/roleplay` admission: Aviva only in bot DMs or NSFW guild channels,
//! and only while the durable NSFW/roleplay gate is enabled. No Discord types.

use crate::persona::Persona;

/// Where `/roleplay` was invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleplayContext {
    BotDm,
    Guild { channel_nsfw: bool },
}

/// Outcome of the roleplay gate. Callers render [`RoleplayDecision::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleplayDecision {
    /// Proceed with Aviva (product Discord NSFW mode — not an HQ pack rewrite).
    AllowAviva,
    /// Gate is off (DM or NSFW guild).
    RefuseDisabled,
    /// Guild SFW channel — never Aviva via `/roleplay`, and never Abbey SFW roleplay.
    RefuseGuildSfw,
}

impl RoleplayDecision {
    pub const fn allow(self) -> bool {
        matches!(self, Self::AllowAviva)
    }

    pub const fn persona(self) -> Option<Persona> {
        match self {
            Self::AllowAviva => Some(Persona::Aviva),
            Self::RefuseDisabled | Self::RefuseGuildSfw => None,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::AllowAviva => "Aviva roleplay is available here. I'll answer as Aviva.",
            Self::RefuseDisabled => {
                "Roleplay is disabled here. An operator can enable it with `/admin nsfw on` in a server, or `/nsfw on` in a DM with me."
            }
            Self::RefuseGuildSfw => {
                "Roleplay (Aviva) is only available in NSFW channels or in a DM with me — not in SFW server channels. Abbey stays SFW here."
            }
        }
    }
}

/// Decide whether `/roleplay` may force Aviva.
///
/// Matrix (fail-closed: `enabled` defaults off):
/// - BotDm × enabled → Aviva
/// - BotDm × disabled → refuse disabled
/// - Guild NSFW × enabled → Aviva
/// - Guild NSFW × disabled → refuse disabled
/// - Guild SFW × anything → refuse SFW (never Abbey SFW roleplay)
pub fn decide(context: RoleplayContext, enabled: bool) -> RoleplayDecision {
    match context {
        RoleplayContext::BotDm => {
            if enabled {
                RoleplayDecision::AllowAviva
            } else {
                RoleplayDecision::RefuseDisabled
            }
        }
        RoleplayContext::Guild {
            channel_nsfw: false,
        } => RoleplayDecision::RefuseGuildSfw,
        RoleplayContext::Guild { channel_nsfw: true } => {
            if enabled {
                RoleplayDecision::AllowAviva
            } else {
                RoleplayDecision::RefuseDisabled
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_matrix_matches_product_gate() {
        // BotDm × enabled / disabled
        assert_eq!(
            decide(RoleplayContext::BotDm, true),
            RoleplayDecision::AllowAviva
        );
        assert_eq!(
            decide(RoleplayContext::BotDm, false),
            RoleplayDecision::RefuseDisabled
        );

        // Guild NSFW × enabled / disabled
        assert_eq!(
            decide(RoleplayContext::Guild { channel_nsfw: true }, true),
            RoleplayDecision::AllowAviva
        );
        assert_eq!(
            decide(RoleplayContext::Guild { channel_nsfw: true }, false),
            RoleplayDecision::RefuseDisabled
        );

        // Guild SFW × anything → refuse (not Abbey SFW roleplay)
        assert_eq!(
            decide(
                RoleplayContext::Guild {
                    channel_nsfw: false
                },
                true
            ),
            RoleplayDecision::RefuseGuildSfw
        );
        assert_eq!(
            decide(
                RoleplayContext::Guild {
                    channel_nsfw: false
                },
                false
            ),
            RoleplayDecision::RefuseGuildSfw
        );
    }

    #[test]
    fn allow_forces_aviva_only() {
        let allow = RoleplayDecision::AllowAviva;
        assert!(allow.allow());
        assert_eq!(allow.persona(), Some(Persona::Aviva));
        assert!(allow.message().contains("Aviva"));

        for refuse in [
            RoleplayDecision::RefuseDisabled,
            RoleplayDecision::RefuseGuildSfw,
        ] {
            assert!(!refuse.allow());
            assert_eq!(refuse.persona(), None);
        }
        assert!(
            RoleplayDecision::RefuseGuildSfw
                .message()
                .contains("Abbey stays SFW")
        );
    }

    #[test]
    fn gate_defaults_fail_closed_in_docs_of_decide() {
        // Documented contract: disabled is the safe default for callers.
        assert_eq!(
            decide(RoleplayContext::BotDm, false),
            RoleplayDecision::RefuseDisabled
        );
    }
}
