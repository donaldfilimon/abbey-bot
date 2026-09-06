use super::*;

fn default_wake_words() -> Vec<String> {
    crate::voice::VoiceConfig::default_wake_words()
}

fn snapshot(phase: VoicePhase) -> crate::voice_session::VoiceSnapshot {
    crate::voice_session::VoiceSnapshot {
        epoch: 9,
        phase,
        media_enabled: phase.processes_audio(),
        start_pending: false,
        status: "internal status must not become spoken copy".into(),
        consent_epoch: 3,
        participant_count: 2,
        dropped_input: 0,
        aborted_overruns: 0,
        barge_ins: 0,
        completed_turns: 0,
    }
}

#[tokio::test]
async fn continuation_is_scoped_to_the_same_speaker() {
    let wake = Mutex::new(WakeState::default());
    let wake_words = default_wake_words();
    assert!(
        is_addressed(
            "Abbey hello",
            Some(1),
            true,
            true,
            &wake,
            &wake_words,
            Instant::now()
        )
        .await
    );
    assert!(
        !is_addressed(
            "an aside while waiting",
            Some(1),
            true,
            true,
            &wake,
            &wake_words,
            Instant::now(),
        )
        .await
    );
    begin_reply(&wake, Some(1)).await;
    open_continuation(&wake).await;
    assert!(
        is_addressed(
            "and one more thing",
            Some(1),
            true,
            true,
            &wake,
            &wake_words,
            Instant::now(),
        )
        .await
    );
    assert!(
        !is_addressed(
            "private aside",
            Some(2),
            true,
            true,
            &wake,
            &wake_words,
            Instant::now()
        )
        .await
    );
    assert!(
        !is_addressed(
            "unknown voice",
            None,
            false,
            true,
            &wake,
            &wake_words,
            Instant::now()
        )
        .await
    );
    assert!(
        is_addressed(
            "Abbey explicit",
            Some(2),
            false,
            true,
            &wake,
            &wake_words,
            Instant::now()
        )
        .await
    );
    assert!(
        !is_addressed(
            "unsafe continuation",
            Some(2),
            false,
            true,
            &wake,
            &wake_words,
            Instant::now(),
        )
        .await
    );
}

#[tokio::test]
async fn queued_pre_playback_speech_cannot_become_a_followup() {
    let wake = Mutex::new(WakeState::default());
    let words = default_wake_words();
    assert!(
        is_addressed(
            "Abby hello",
            Some(1),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
    let captured_before_playback = Instant::now() - Duration::from_secs(1);
    begin_reply(&wake, Some(1)).await;
    open_continuation(&wake).await;
    assert!(
        !is_addressed(
            "an earlier aside",
            Some(1),
            true,
            true,
            &wake,
            &words,
            captured_before_playback
        )
        .await
    );
    assert!(
        is_addressed(
            "an actual followup",
            Some(1),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
}

#[tokio::test]
async fn old_playback_completion_cannot_open_a_new_questions_followup_window() {
    let wake = Mutex::new(WakeState::default());
    let words = default_wake_words();
    assert!(
        is_addressed(
            "Abby hello",
            Some(1),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
    begin_reply(&wake, Some(1)).await;
    open_continuation(&wake).await;
    assert!(
        is_addressed(
            "Abby another question",
            Some(1),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
    begin_reply(&wake, Some(1)).await;
    extend_continuation(&wake).await;
    assert!(
        !is_addressed(
            "an aside before the new answer",
            Some(1),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
}

#[tokio::test]
async fn queued_recognition_cannot_change_the_playing_replies_window() {
    let wake = Mutex::new(WakeState::default());
    let words = default_wake_words();
    begin_reply(&wake, Some(1)).await;
    assert!(
        is_addressed(
            "Abby next question",
            Some(2),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
    // Simulate old playback acquiring its call lock only after queued STT
    // completes. Recognition must not hand this opening to the new speaker.
    open_continuation(&wake).await;
    assert_eq!(wake.lock().await.speaker, Some(1));
    begin_reply(&wake, Some(2)).await;
    assert!(
        !is_addressed(
            "an aside before the new reply",
            Some(2),
            true,
            true,
            &wake,
            &words,
            Instant::now()
        )
        .await
    );
}

#[test]
fn safely_attributed_withdrawal_is_classified_before_wake_gating() {
    let active = snapshot(VoicePhase::Thinking);
    assert!(pre_wake_withdrawal("stop listening", &active, true, true));
    assert!(pre_wake_withdrawal(
        "I withdraw my consent",
        &active,
        true,
        true
    ));
    assert!(pre_wake_withdrawal(
        "stop listening please",
        &active,
        true,
        true
    ));
    assert!(pre_wake_withdrawal("I do not consent", &active, true, true));
    assert!(!pre_wake_withdrawal("stop listening", &active, false, true));
    assert!(!pre_wake_withdrawal("stop listening", &active, true, false));
    assert!(!pre_wake_withdrawal("I consent", &active, true, true));
}

#[test]
fn operational_voice_questions_use_runtime_copy_not_model_prose() {
    let active = snapshot(VoicePhase::Listening);
    let Some(OperationalVoiceTurn::Reply(reply)) =
        operational_voice_turn("Abbey, is voice active?", &active)
    else {
        panic!("expected fixed operational reply");
    };
    assert!(reply.contains("Voice is active for the current consent epoch"));
    assert!(!reply.contains("internal status"));
    assert!(operational_voice_turn("Abbey, tell me a joke", &active).is_none());
}

#[test]
fn transcript_scopes_isolate_consent_speakers_and_unattributed_turns() {
    let first = voice_scope(1, 2, 3, Some(4), 5, true);
    let same = voice_scope(1, 2, 3, Some(4), 99, true);
    let other_speaker = voice_scope(1, 2, 3, Some(6), 5, true);
    let later_consent = voice_scope(1, 2, 7, Some(4), 5, true);
    let unknown_a = voice_scope(1, 2, 3, None, 8, false);
    let unknown_b = voice_scope(1, 2, 3, None, 9, false);
    assert_eq!(first, same);
    assert_ne!(first, other_speaker);
    assert_ne!(first, later_consent);
    assert_ne!(unknown_a, unknown_b);
}

#[test]
fn armed_verification_disables_conversation_commits() {
    assert!(should_commit_turn(true, false));
    assert!(!should_commit_turn(true, true));
    assert!(!should_commit_turn(false, false));
    assert!(!should_commit_turn(false, true));
}

#[test]
fn stop_command_result_only_arms_later_confirmation() {
    assert!(playback_stop_requested(Ok(())));
    assert!(!playback_stop_requested(Err(
        songbird::tracks::ControlError::Finished
    )));
}

#[test]
fn playback_lifecycle_distinguishes_every_terminal_outcome() {
    let mut lifecycle = PlaybackLifecycle::default();
    assert_eq!(
        lifecycle.observe(7, 7, PlaybackTermination::Natural),
        PlaybackObservation::NaturalCompletion
    );
    assert_eq!(
        lifecycle.observe(8, 8, PlaybackTermination::Errored),
        PlaybackObservation::Errored
    );
    assert_eq!(
        lifecycle.observe(9, 9, PlaybackTermination::Stopped),
        PlaybackObservation::OrdinaryStop
    );

    lifecycle.note_barge_stop_requested(9);
    assert_eq!(
        lifecycle.observe(10, 9, PlaybackTermination::Natural),
        PlaybackObservation::Stale,
        "a natural-end race must not be reported as barge cancellation"
    );

    lifecycle.note_barge_stop_requested(10);
    assert_eq!(
        lifecycle.observe(11, 10, PlaybackTermination::Errored),
        PlaybackObservation::Stale,
        "a playback error must not be reported as barge cancellation"
    );

    lifecycle.note_barge_stop_requested(11);
    assert_eq!(
        lifecycle.observe(12, 11, PlaybackTermination::Stopped),
        PlaybackObservation::ConfirmedBargeInCancellation
    );
}
