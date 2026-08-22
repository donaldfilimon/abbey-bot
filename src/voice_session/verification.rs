//! Content-free, process-memory evidence for one consented live voice run.
//!
//! This module deliberately stores only fixed counters, consent epochs, and
//! aggregate participant counts. Audio, identities, transcripts, responses,
//! message content, and timestamps never cross this boundary.

use std::sync::atomic::Ordering;

use super::VoiceRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationRunStatus {
    Armed,
    Complete,
}

impl VerificationRunStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Armed => "armed",
            Self::Complete => "complete",
        }
    }
}

/// Content-free evidence that a Discord activation crossed both command-side
/// authorization gates. The runtime supplies the consent epoch atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationActivation {
    pub manager_authorized: bool,
    pub caller_present: bool,
    pub participant_count: usize,
    pub resumed: bool,
}

/// Ephemeral counters for one human-observed live acceptance run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceVerificationSnapshot {
    pub run: u64,
    pub status: VerificationRunStatus,
    pub activations: u64,
    pub manager_authorized_activations: u64,
    pub caller_present_activations: u64,
    pub first_consent_epoch: Option<u64>,
    pub latest_consent_epoch: Option<u64>,
    pub first_participant_count: Option<usize>,
    pub latest_participant_count: Option<usize>,
    pub decoded_receives: u64,
    pub stt_completions: u64,
    pub playback_completions: u64,
    pub barge_in_cancellations: u64,
    pub participant_change_pauses: u64,
    pub resumed_after_participant_change: bool,
    pub final_leave: bool,
}

impl VoiceVerificationSnapshot {
    fn new(run: u64) -> Self {
        Self {
            run,
            status: VerificationRunStatus::Armed,
            activations: 0,
            manager_authorized_activations: 0,
            caller_present_activations: 0,
            first_consent_epoch: None,
            latest_consent_epoch: None,
            first_participant_count: None,
            latest_participant_count: None,
            decoded_receives: 0,
            stt_completions: 0,
            playback_completions: 0,
            barge_in_cancellations: 0,
            participant_change_pauses: 0,
            resumed_after_participant_change: false,
            final_leave: false,
        }
    }

    #[must_use]
    pub fn observed_checks(&self) -> usize {
        let authorization = self.activations > 0
            && self.manager_authorized_activations == self.activations
            && self.caller_present_activations == self.activations;
        let consent_changed =
            self.activations >= 2 && self.first_consent_epoch != self.latest_consent_epoch;
        [
            authorization,
            consent_changed,
            self.decoded_receives > 0,
            self.stt_completions > 0,
            self.playback_completions > 0,
            self.barge_in_cancellations > 0,
            self.participant_change_pauses > 0 && self.resumed_after_participant_change,
            self.final_leave,
        ]
        .into_iter()
        .filter(|observed| *observed)
        .count()
    }
}

#[derive(Default)]
pub(super) struct VerificationState {
    next_run: u64,
    current: Option<VoiceVerificationSnapshot>,
}

impl VoiceRuntime {
    /// Arm a new process-memory-only live acceptance run. A completed report
    /// may be replaced; an active run must first reach a final `/voice leave`.
    pub fn begin_verification(&self) -> Result<VoiceVerificationSnapshot, &'static str> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.pending_start_generation.load(Ordering::SeqCst) != 0
            || self.media_epoch.load(Ordering::SeqCst) != 0
        {
            return Err(
                "Start verification before the consented join; a voice start or media epoch is already active.",
            );
        }
        let mut verification = self
            .verification
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if verification
            .current
            .as_ref()
            .is_some_and(|run| run.status == VerificationRunStatus::Armed)
        {
            return Err(
                "A live voice verification run is already armed. Finish it with `/voice leave`, then read `/voice verify report`.",
            );
        }
        verification.next_run = verification.next_run.saturating_add(1);
        let run = VoiceVerificationSnapshot::new(verification.next_run);
        verification.current = Some(run.clone());
        Ok(run)
    }

    #[must_use]
    pub fn verification_active(&self) -> bool {
        self.verification
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .as_ref()
            .is_some_and(|run| run.status == VerificationRunStatus::Armed)
    }

    #[must_use]
    pub fn verification_snapshot(&self) -> Option<VoiceVerificationSnapshot> {
        self.verification
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .clone()
    }

    /// Opaque correlation token for the run armed when a final leave begins.
    /// A leave that predates a later run must never complete that later run.
    #[must_use]
    pub fn verification_run_token(&self) -> Option<u64> {
        self.verification
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .as_ref()
            .filter(|run| run.status == VerificationRunStatus::Armed)
            .map(|run| run.run)
    }

    /// Called while `activation_gate` is held so a participant pause cannot
    /// interleave between opening media and recording its activation evidence.
    pub(super) fn record_verification_activation(
        &self,
        evidence: VerificationActivation,
        consent_epoch: u64,
    ) {
        self.with_active_verification(|run| {
            run.activations = run.activations.saturating_add(1);
            if evidence.manager_authorized {
                run.manager_authorized_activations =
                    run.manager_authorized_activations.saturating_add(1);
            }
            if evidence.caller_present {
                run.caller_present_activations = run.caller_present_activations.saturating_add(1);
            }
            run.first_consent_epoch.get_or_insert(consent_epoch);
            run.latest_consent_epoch = Some(consent_epoch);
            run.first_participant_count
                .get_or_insert(evidence.participant_count);
            run.latest_participant_count = Some(evidence.participant_count);
            if evidence.resumed
                && run.participant_change_pauses > 0
                && run.first_consent_epoch != run.latest_consent_epoch
            {
                run.resumed_after_participant_change = true;
            }
        });
    }

    /// Called while `activation_gate` is held after an exact participant pause
    /// wins, keeping the pause paired with the activation it interrupted.
    pub(super) fn record_verification_participant_pause(&self, participant_count: usize) {
        self.with_active_verification(|run| {
            if run.activations == 0 {
                return;
            }
            run.participant_change_pauses = run.participant_change_pauses.saturating_add(1);
            run.latest_participant_count = Some(participant_count);
        });
    }

    /// Record only a successfully queued, voiced, attributed decoded frame.
    pub fn note_verification_decoded_receive(&self) {
        self.with_active_verification(|run| {
            run.decoded_receives = run.decoded_receives.saturating_add(1);
        });
    }

    /// Record successful local recognition only while its exact media epoch is
    /// still open. The recognized text is never passed to this tracker.
    pub fn note_verification_stt_completion(&self, epoch: u64) {
        let _ = self.with_media_enabled(epoch, || {
            self.with_active_verification(|run| {
                run.stt_completions = run.stt_completions.saturating_add(1);
            });
        });
    }

    /// The ordinary barge-in counter includes cancellation of in-flight
    /// recognition/reasoning. This records only an actual playback stop.
    pub fn note_verification_barge_in_cancellation(&self) {
        self.with_active_verification(|run| {
            run.barge_in_cancellations = run.barge_in_cancellations.saturating_add(1);
        });
    }

    pub fn note_completed_turn(&self) {
        self.completed_turns.fetch_add(1, Ordering::Relaxed);
        self.with_active_verification(|run| {
            run.playback_completions = run.playback_completions.saturating_add(1);
        });
    }

    /// Complete the run only after Songbird reports that the final leave
    /// removed the configured call.
    pub fn note_verification_final_leave(&self, expected_run: u64) -> bool {
        let mut verification = self
            .verification
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(run) = verification
            .current
            .as_mut()
            .filter(|run| run.status == VerificationRunStatus::Armed && run.run == expected_run)
        else {
            return false;
        };
        run.final_leave = true;
        run.status = VerificationRunStatus::Complete;
        true
    }

    #[must_use]
    pub fn verification_report(&self) -> String {
        let Some(run) = self.verification_snapshot() else {
            return "Abbey live voice verification: no run is armed. An owner or administrator can use `/voice verify start` before the consented session.".into();
        };
        let observed = |condition: bool| if condition { "yes" } else { "pending" };
        let authorization = run.activations > 0
            && run.manager_authorized_activations == run.activations
            && run.caller_present_activations == run.activations;
        let consent_changed =
            run.activations >= 2 && run.first_consent_epoch != run.latest_consent_epoch;
        let participant_flow =
            run.participant_change_pauses > 0 && run.resumed_after_participant_change;
        let consent_epochs = run
            .first_consent_epoch
            .zip(run.latest_consent_epoch)
            .map_or_else(
                || "none".into(),
                |(first, latest)| format!("{first}->{latest}"),
            );
        let participant_counts = run
            .first_participant_count
            .zip(run.latest_participant_count)
            .map_or_else(
                || "none".into(),
                |(first, latest)| format!("{first}->{latest}"),
            );
        format!(
            "Abbey live voice verification (redacted)\nRun {}: {} · mode: {} · observed: {}/8\nAuthorization: {} (manager {}/{} · in-channel caller {}/{} · attested-human counts {})\nConsent epochs: {} · changed: {}\nMedia: decoded receive {} · local STT {} · synthesized playback end {} · barge-in cancellation {}\nParticipant change: pause/resume {} · final leave {}\nRetention: counters only; no audio, user or message IDs, transcripts, responses, or message content are retained by this run, and conversation commits stay disabled while it is armed.\nManual witness still required: every current human explicitly consented and a human heard the reply. This live report is separate from local/source test evidence.",
            run.run,
            run.status.label(),
            self.config.mode().label(),
            run.observed_checks(),
            observed(authorization),
            run.manager_authorized_activations,
            run.activations,
            run.caller_present_activations,
            run.activations,
            participant_counts,
            consent_epochs,
            observed(consent_changed),
            observed(run.decoded_receives > 0),
            observed(run.stt_completions > 0),
            observed(run.playback_completions > 0),
            observed(run.barge_in_cancellations > 0),
            observed(participant_flow),
            observed(run.final_leave),
        )
    }

    fn with_active_verification(&self, action: impl FnOnce(&mut VoiceVerificationSnapshot)) {
        let mut verification = self
            .verification
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(run) = verification
            .current
            .as_mut()
            .filter(|run| run.status == VerificationRunStatus::Armed)
        {
            action(run);
        }
    }
}
