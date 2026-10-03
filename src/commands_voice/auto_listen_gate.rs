//! Pure auto-listen decision helpers (no Discord I/O).

use std::collections::HashSet;

use crate::voice::VoiceMode;
use crate::voice_session::VoicePhase;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoListenDecision {
    Disabled,
    WrongMode(VoiceMode),
    EmptyChannel,
    ConsentUnavailable(&'static str),
    ConsentIncomplete(Vec<u64>),
    Ready,
}

pub(crate) fn decide_auto_listen(
    enabled: bool,
    mode: VoiceMode,
    participants: &HashSet<u64>,
    coverage: Result<Vec<u64>, &'static str>,
) -> AutoListenDecision {
    if !enabled {
        return AutoListenDecision::Disabled;
    }
    if mode != VoiceMode::Local {
        return AutoListenDecision::WrongMode(mode);
    }
    if participants.is_empty() {
        return AutoListenDecision::EmptyChannel;
    }
    match coverage {
        Err(message) => AutoListenDecision::ConsentUnavailable(message),
        Ok(missing) if missing.is_empty() => AutoListenDecision::Ready,
        Ok(missing) => AutoListenDecision::ConsentIncomplete(missing),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpgradeDecision {
    Noop,
    Eligible,
}

/// The existing auto-listen decision owns opt-in, Local mode, roster and receipt
/// coverage. Upgrade adds the current phase and independently refreshed permissions.
pub(crate) struct UpgradeFacts<'a> {
    pub phase: VoicePhase,
    pub auto_listen: &'a AutoListenDecision,
    pub current_permissions: bool,
}

pub(crate) fn decide_upgrade(facts: UpgradeFacts<'_>) -> UpgradeDecision {
    if facts.phase == VoicePhase::PresenceOnly
        && facts.current_permissions
        && *facts.auto_listen == AutoListenDecision::Ready
    {
        UpgradeDecision::Eligible
    } else {
        UpgradeDecision::Noop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_listen_requires_env_local_mode_members_and_complete_consent() {
        let members = HashSet::from([1122140354737623110u64]);
        assert_eq!(
            decide_auto_listen(false, VoiceMode::Local, &members, Ok(vec![])),
            AutoListenDecision::Disabled
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::OpenAi, &members, Ok(vec![])),
            AutoListenDecision::WrongMode(VoiceMode::OpenAi)
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &HashSet::new(), Ok(vec![])),
            AutoListenDecision::EmptyChannel
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &members, Err("unavailable")),
            AutoListenDecision::ConsentUnavailable("unavailable")
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &members, Ok(vec![99])),
            AutoListenDecision::ConsentIncomplete(vec![99])
        );
        assert_eq!(
            decide_auto_listen(true, VoiceMode::Local, &members, Ok(vec![])),
            AutoListenDecision::Ready
        );
    }

    fn upgrade(
        phase: VoicePhase,
        decision: &AutoListenDecision,
        permissions: bool,
    ) -> UpgradeDecision {
        decide_upgrade(UpgradeFacts {
            phase,
            auto_listen: decision,
            current_permissions: permissions,
        })
    }

    #[test]
    fn auto_listen_current_permissions_are_required_for_presence_upgrade() {
        assert_eq!(
            upgrade(VoicePhase::PresenceOnly, &AutoListenDecision::Ready, false),
            UpgradeDecision::Noop
        );
        assert_eq!(
            upgrade(VoicePhase::PresenceOnly, &AutoListenDecision::Ready, true),
            UpgradeDecision::Eligible
        );
    }

    #[test]
    fn auto_listen_phase_matrix_only_upgrades_presence() {
        for phase in [
            VoicePhase::Disconnected,
            VoicePhase::Connecting,
            VoicePhase::Listening,
            VoicePhase::Thinking,
            VoicePhase::Speaking,
            VoicePhase::AwaitingConsent,
            VoicePhase::Failed,
        ] {
            assert_eq!(
                upgrade(phase, &AutoListenDecision::Ready, true),
                UpgradeDecision::Noop,
                "{phase:?}"
            );
        }
    }

    #[test]
    fn auto_listen_upgrade_requires_every_existing_admission_fact() {
        for decision in [
            AutoListenDecision::Disabled,
            AutoListenDecision::WrongMode(VoiceMode::Disabled),
            AutoListenDecision::WrongMode(VoiceMode::OpenAi),
            AutoListenDecision::EmptyChannel,
            AutoListenDecision::ConsentUnavailable("unavailable"),
            AutoListenDecision::ConsentIncomplete(vec![1]),
        ] {
            assert_eq!(
                upgrade(VoicePhase::PresenceOnly, &decision, true),
                UpgradeDecision::Noop
            );
        }
    }
}
