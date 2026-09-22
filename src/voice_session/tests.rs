use super::verification::VerificationRunStatus;
use super::*;
use crate::inspect::{VoiceInspectRegistry, VoiceInspectState};
use crate::voice::{VoiceBackendConfig, VoiceConfig};
use std::sync::{Arc, atomic::AtomicBool};

fn runtime() -> VoiceRuntime {
    VoiceRuntime::new(VoiceConfig::selected_only(
        1,
        2,
        VoiceBackendConfig::Disabled,
        true,
    ))
}

fn voice_snapshot(phase: VoicePhase) -> VoiceSnapshot {
    VoiceSnapshot {
        epoch: 12,
        phase,
        media_enabled: phase.processes_audio(),
        start_pending: false,
        status: "untrusted prose saying speech is back on".into(),
        consent_epoch: 7,
        participant_count: 4,
        dropped_input: 1,
        aborted_overruns: 2,
        barge_ins: 3,
        completed_turns: 4,
    }
}

mod activation_tests;
mod backend_switch_tests;
mod consent_tests;
mod inspect_tests;
mod session_event_tests;
mod status_tests;
mod verification_tests;
