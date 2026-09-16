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
pub(crate) enum WhilePresentGate {
    Skip,
    RemainPresent,
    Activate,
}

pub(crate) fn while_present_gate(
    phase: VoicePhase,
    decision: &AutoListenDecision,
) -> WhilePresentGate {
    if phase != VoicePhase::PresenceOnly {
        return WhilePresentGate::Skip;
    }
    match decision {
        AutoListenDecision::Ready => WhilePresentGate::Activate,
        _ => WhilePresentGate::RemainPresent,
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

    #[test]
    fn presence_only_ready_decision_is_the_only_activate_gate() {
        let ready = AutoListenDecision::Ready;
        let incomplete = AutoListenDecision::ConsentIncomplete(vec![1]);
        assert_eq!(
            while_present_gate(VoicePhase::PresenceOnly, &ready),
            WhilePresentGate::Activate
        );
        assert_eq!(
            while_present_gate(VoicePhase::PresenceOnly, &incomplete),
            WhilePresentGate::RemainPresent
        );
        assert_eq!(
            while_present_gate(VoicePhase::Listening, &ready),
            WhilePresentGate::Skip
        );
        assert_eq!(
            while_present_gate(VoicePhase::Connecting, &ready),
            WhilePresentGate::Skip
        );
        assert_eq!(
            while_present_gate(VoicePhase::Disconnected, &ready),
            WhilePresentGate::Skip
        );
    }
}
