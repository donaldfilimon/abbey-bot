//! Phase outcomes and copy, status bounds, and music never touching listening consent.
use super::*;

#[test]
fn only_active_conversation_phases_process_audio() {
    // `processes_audio` is the gate that decides whether a frame may reach
    // STT, so the paused states are the safety-critical half of this
    // assertion. `AwaitingConsent` in particular exists precisely because
    // consent was revoked or membership changed: if it ever starts
    // reporting true, revocation silently stops muting the microphone.
    for active in [
        VoicePhase::Listening,
        VoicePhase::Thinking,
        VoicePhase::Speaking,
    ] {
        assert!(
            active.processes_audio(),
            "{} must process audio",
            active.label()
        );
    }
    for paused in [
        VoicePhase::Disconnected,
        VoicePhase::PresenceOnly,
        VoicePhase::Connecting,
        VoicePhase::AwaitingConsent,
        VoicePhase::Failed,
    ] {
        assert!(
            !paused.processes_audio(),
            "{} must never process audio",
            paused.label()
        );
    }
}

#[test]
fn voice_phase_copy_distinguishes_active_and_inactive_snapshots() {
    for phase in [
        VoicePhase::Disconnected,
        VoicePhase::PresenceOnly,
        VoicePhase::Connecting,
        VoicePhase::AwaitingConsent,
        VoicePhase::Failed,
    ] {
        let snapshot = voice_snapshot(phase);
        let copy = authoritative_text_reply("voice status", &snapshot).expect("fixed reply");
        assert!(
            copy.contains("no participant audio is being processed")
                || copy.contains(
                    "Capture, recognition, reasoning, synthesis, and playback are disabled"
                )
        );
        assert!(!copy.contains("Participant audio processing is enabled"));
    }

    for phase in [
        VoicePhase::Listening,
        VoicePhase::Thinking,
        VoicePhase::Speaking,
    ] {
        let snapshot = voice_snapshot(phase);
        let copy = authoritative_text_reply("voice status", &snapshot).expect("fixed reply");
        assert!(copy.contains("Voice is active for the current consent epoch"));
        assert!(copy.contains("Participant audio processing is enabled"));
        assert!(copy.contains(phase.label()));
        assert!(!copy.contains("untrusted prose"));
    }

    let mut closing = voice_snapshot(VoicePhase::Speaking);
    closing.media_enabled = false;
    let copy = authoritative_text_reply("voice status", &closing).expect("fixed reply");
    assert!(copy.contains("media gate is closed"));
    assert!(copy.contains("no participant audio is being processed"));
    assert!(!copy.contains("Participant audio processing is enabled"));

    let mut active_replacement = voice_snapshot(VoicePhase::Listening);
    active_replacement.start_pending = true;
    let copy = authoritative_text_reply("voice status", &active_replacement)
        .expect("fixed active replacement reply");
    assert!(copy.contains("Participant audio processing is enabled"));
    assert!(copy.contains("replacement start is pending"));
    assert!(!copy.contains("media gate is closed"));
}

#[test]
fn status_is_flattened_and_bounded() {
    let status = bounded_status(format!("line one\n{}", "x".repeat(500)));
    assert!(!status.contains('\n'));
    assert!(status.chars().count() <= 241);
}

#[tokio::test]
async fn music_never_opens_or_renews_listening_consent() {
    let runtime = runtime();
    let before = runtime.snapshot().await;
    let generation = runtime.music.begin(crate::player_control::Player::Spotify);
    runtime.music.set_volume(35);
    let after = runtime.snapshot().await;
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.consent_epoch, before.consent_epoch);
    assert!(!after.media_enabled);
    let epoch = runtime.begin(HashSet::new()).await;
    runtime
        .pause_epoch_for_consent(epoch, HashSet::new(), "withdrawn")
        .await;
    assert!(
        runtime.music.current(generation),
        "listening withdrawal cannot cancel music"
    );
    assert!(!runtime.music_may_restore_output(runtime.current_epoch()));
    runtime.music_consent_teardown_complete(epoch).await;
    assert!(
        !runtime.music_may_restore_output(runtime.current_epoch()),
        "stale teardown cannot restore output"
    );
    runtime
        .music_consent_teardown_complete(runtime.current_epoch())
        .await;
    assert!(runtime.music_may_restore_output(runtime.current_epoch()));
    assert!(!runtime.snapshot().await.media_enabled);
    runtime.disconnect("leave").await;
    assert!(
        !runtime.music.current(generation),
        "leave cancels both paths"
    );
}

#[tokio::test]
async fn stale_music_completion_cannot_stop_a_replacement() {
    let runtime = runtime();
    let old = runtime.music.begin(crate::player_control::Player::Spotify);
    let new = runtime.music.begin(crate::player_control::Player::Music);
    runtime
        .music
        .finish(old, "old failed", PlaybackTermination::Errored);
    assert!(runtime.music.current(new));
    runtime.music.stop("paused", PlaybackTermination::Stopped);
    assert!(!runtime.music.current(new));
    assert_eq!(
        runtime.music.player(),
        Some(crate::player_control::Player::Music)
    );
    assert!(!runtime.snapshot().await.media_enabled);
}

#[test]
fn presence_only_is_degraded_and_never_reads_as_listening() {
    // Managed runs disable tracing (`EnvFilter::new("off")`), so this closed
    // event is the only operational record of voice state. Presence-only is
    // connected but deaf and mute; Listening is the only phase that can hear a
    // participant. Collapsing both into `Ready` made "is voice actually on?"
    // unanswerable from the event log, which is how a real failure went
    // undiagnosed.
    use crate::observability::EventOutcome;
    assert_eq!(
        phase_outcome(VoicePhase::PresenceOnly),
        EventOutcome::Degraded
    );
    assert_eq!(phase_outcome(VoicePhase::Listening), EventOutcome::Ready);
    assert_ne!(
        phase_outcome(VoicePhase::PresenceOnly),
        phase_outcome(VoicePhase::Listening)
    );
}

#[test]
fn every_voice_phase_maps_to_its_own_closed_outcome() {
    use crate::observability::EventOutcome;
    for (phase, expected) in [
        (VoicePhase::Disconnected, EventOutcome::Stopped),
        (VoicePhase::PresenceOnly, EventOutcome::Degraded),
        (VoicePhase::Listening, EventOutcome::Ready),
        (VoicePhase::Connecting, EventOutcome::Started),
        (VoicePhase::Thinking, EventOutcome::Started),
        (VoicePhase::Speaking, EventOutcome::Started),
        (VoicePhase::AwaitingConsent, EventOutcome::Skipped),
        (VoicePhase::Failed, EventOutcome::Failed),
    ] {
        assert_eq!(phase_outcome(phase), expected, "phase {phase:?}");
    }
}

#[test]
fn only_a_failure_carries_an_error_category() {
    // A bare `failed` with no category is what left a live voice failure
    // undiagnosable: the reason existed at every call site and was dropped
    // before telemetry. Equally, a non-failure phase must never imply a fault.
    use crate::observability::OperationalErrorCategory;
    assert_eq!(
        phase_error(VoicePhase::Failed, Some(OperationalErrorCategory::Timeout)),
        Some(OperationalErrorCategory::Timeout)
    );
    for phase in [
        VoicePhase::Disconnected,
        VoicePhase::PresenceOnly,
        VoicePhase::Listening,
        VoicePhase::Connecting,
        VoicePhase::Thinking,
        VoicePhase::Speaking,
        VoicePhase::AwaitingConsent,
    ] {
        assert_eq!(
            phase_error(phase, Some(OperationalErrorCategory::Timeout)),
            None,
            "phase {phase:?} must not report a fault"
        );
    }
    assert_eq!(phase_error(VoicePhase::Failed, None), None);
}
