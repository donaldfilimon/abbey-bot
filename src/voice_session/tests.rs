use super::*;
use crate::voice::{VoiceBackendConfig, VoiceConfig};

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

fn runtime() -> VoiceRuntime {
    VoiceRuntime::new(VoiceConfig {
        guild_id: 1,
        channel_id: 2,
        backend: VoiceBackendConfig::Disabled,
        wake_word_required: true,
    })
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

#[test]
fn text_boundary_matches_only_explicit_consent_and_control_language() {
    let snapshot = voice_snapshot(VoicePhase::PresenceOnly);
    for text in [
        "I consent to local speech recognition for this voice session.",
        "I do not consent to voice recording.",
        "You may listen now.",
        "Can you resume voice now?",
        "Please stop audio processing.",
        "The local speech components are back on.",
        "Are you listening?",
        "@Abbey, /voice status",
    ] {
        assert!(
            authoritative_text_reply(text, &snapshot).is_some(),
            "expected a match: {text}"
        );
    }

    for text in [
        "I consent to the updated code of conduct.",
        "I agree the voice sounds warmer.",
        "I agree to use a different voice.",
        "Please resume the download.",
        "We should discuss audio codecs.",
        "Start by explaining how voice synthesis works.",
        "The microphone stand is on the table.",
        "The mic is on the desk.",
        "yes",
    ] {
        assert!(
            authoritative_text_reply(text, &snapshot).is_none(),
            "unexpected match: {text}"
        );
    }
}

#[test]
fn awaiting_consent_accepts_only_standalone_explicit_responses() {
    let awaiting = voice_snapshot(VoicePhase::AwaitingConsent);
    for text in [
        "I agree.",
        "I consent",
        "Abbey, we all consent!",
        "everyone consents",
        "I opt in",
        "I do not consent",
        "we don't consent",
    ] {
        assert!(
            authoritative_text_reply(text, &awaiting).is_some(),
            "expected awaiting-consent match: {text}"
        );
    }
    for text in [
        "yes",
        "sure",
        "I agree the voice sounds warmer",
        "I consent to the code of conduct",
    ] {
        assert!(
            authoritative_text_reply(text, &awaiting).is_none(),
            "unexpected awaiting-consent match: {text}"
        );
    }

    let active = voice_snapshot(VoicePhase::Listening);
    assert!(authoritative_text_reply("I agree", &active).is_none());
    assert!(authoritative_text_reply("I consent", &active).is_none());
}

#[test]
fn only_explicit_negative_language_requests_active_epoch_revocation() {
    let active = voice_snapshot(VoicePhase::Listening);
    for text in [
        "I do not consent",
        "Abbey, we don't consent",
        "I do not consent to voice recording",
        "I withdraw my consent",
        "I revoke my consent",
        "I no longer consent",
        "please stop listening now",
        "stop listening please",
        "stop recording",
        "stop recording me please",
        "do not record me",
        "don't transcribe me",
        "stop transcription now",
        "stop transcribing me",
        "Abbey turn voice off",
    ] {
        assert!(
            requests_consent_withdrawal(text, &active),
            "expected withdrawal: {text}"
        );
        assert!(authoritative_text_reply(text, &active).is_some());
    }
    for text in [
        "yes",
        "I consent",
        "resume voice",
        "how do I stop voice",
        "I do not consent to the code of conduct",
        "how do I stop recording?",
        "can you stop transcribing someone else?",
        "I agree the voice sounds warmer",
    ] {
        assert!(
            !requests_consent_withdrawal(text, &active),
            "unexpected withdrawal: {text}"
        );
    }

    let inactive = voice_snapshot(VoicePhase::AwaitingConsent);
    assert!(!requests_consent_withdrawal("I do not consent", &inactive));
}

#[test]
fn awaiting_consent_copy_cannot_claim_that_listening_resumed() {
    let snapshot = voice_snapshot(VoicePhase::AwaitingConsent);
    let copy = authoritative_text_reply("I consent", &snapshot).expect("fixed reply");
    assert_eq!(
        copy,
        "Voice has not resumed. Abbey is paused with its media gate closed, so no participant audio is being processed; the pause procedure also tears down any existing conversational connection. Renewed consent from everyone currently present plus a manager's `/voice resume consent:true` are required before voice can restart. Consent epoch: 7 · participants recorded: 4."
    );
    assert!(!copy.contains("untrusted prose"));
    assert!(!copy.contains("processing is enabled"));
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

#[tokio::test]
async fn stale_epoch_cannot_overwrite_disconnect() {
    let runtime = runtime();
    let epoch = runtime.begin(HashSet::new()).await;
    runtime.disconnect("left").await;
    runtime
        .set_status(epoch, VoicePhase::Speaking, "stale")
        .await;
    let status = runtime.snapshot().await;
    assert_eq!(status.phase, VoicePhase::Disconnected);
    assert_eq!(status.status, "left");
    assert!(!runtime.media_enabled(epoch));
}

#[tokio::test]
async fn epoch_allocation_waits_for_serialized_state_publication() {
    let runtime = Arc::new(runtime());
    let initial_epoch = runtime.current_epoch();
    let state_guard = runtime.inner.lock().await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let stopping = {
        let runtime = Arc::clone(&runtime);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            runtime.disconnect("serialized stop").await;
        })
    };
    barrier.wait().await;
    tokio::task::yield_now().await;

    // The former atomic-then-await order advanced this counter while the
    // matching RuntimeState publication was still blocked, allowing another
    // stop to publish a newer epoch first and then be overwritten by this one.
    assert_eq!(runtime.current_epoch(), initial_epoch);
    drop(state_guard);
    stopping.await.expect("stop task");
    let snapshot = runtime.snapshot().await;
    assert!(runtime.is_current(snapshot.epoch));
    assert_eq!(snapshot.status, "serialized stop");
}

#[tokio::test]
async fn adverse_event_revokes_only_the_exact_bound_discord_session() {
    let runtime = runtime();
    let first_start = runtime.reserve_start();
    let first_epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .bind_discord_session(first_epoch, "session-one".into())
            .await
    );
    assert!(runtime.activate(first_epoch, first_start, "active").await);

    assert_eq!(
        runtime.revoke_for_discord_session("session-one"),
        DiscordSessionEvent::Current {
            epoch: first_epoch,
            media_was_enabled: true,
        }
    );
    assert!(!runtime.media_enabled(first_epoch));
}

#[tokio::test]
async fn delayed_retired_session_event_cannot_revoke_replacement() {
    let runtime = runtime();
    let first_start = runtime.reserve_start();
    let first_epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .bind_discord_session(first_epoch, "session-one".into())
            .await
    );
    assert!(runtime.activate(first_epoch, first_start, "first").await);

    let replacement_start = runtime.reserve_start();
    let replacement_epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .bind_discord_session(replacement_epoch, "session-two".into())
            .await
    );
    assert!(
        runtime
            .activate(replacement_epoch, replacement_start, "replacement")
            .await
    );

    assert_eq!(
        runtime.revoke_for_discord_session("session-one"),
        DiscordSessionEvent::Retired
    );
    assert!(runtime.media_enabled(replacement_epoch));
}

#[tokio::test]
async fn unknown_bot_session_event_fails_closed() {
    let runtime = runtime();
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::new()).await;
    assert!(
        runtime
            .bind_discord_session(epoch, "known-session".into())
            .await
    );
    assert!(runtime.activate(epoch, start, "active").await);

    assert_eq!(
        runtime.revoke_for_discord_session("unclassified-session"),
        DiscordSessionEvent::Unknown {
            epoch,
            media_was_enabled: true,
        }
    );
    assert!(!runtime.media_enabled(epoch));
}

#[tokio::test]
async fn delayed_attested_participant_join_cannot_revoke_replacement() {
    let runtime = runtime();
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(epoch, start, "replacement").await);

    assert_eq!(runtime.revoke_for_unattested_participant(7), None);
    assert!(runtime.media_enabled(epoch));
    assert_eq!(runtime.revoke_for_unattested_participant(8), Some(epoch));
    assert!(!runtime.media_enabled(epoch));
}

#[tokio::test]
async fn slow_start_cancellation_notifies_preflight_immediately() {
    let runtime = Arc::new(runtime());
    let generation = runtime.reserve_start();
    let waiting = {
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            runtime.wait_for_start_cancellation(generation).await;
        })
    };
    tokio::task::yield_now().await;
    runtime.cancel_pending_start();
    tokio::time::timeout(Duration::from_millis(100), waiting)
        .await
        .expect("cancellation notification timed out")
        .expect("wait task");
}

#[tokio::test]
async fn media_gate_requires_activation_for_the_current_epoch() {
    let runtime = runtime();
    let start_generation = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::new()).await;
    assert!(runtime.snapshot().await.start_pending);
    assert!(!runtime.media_enabled(epoch));
    assert!(runtime.activate(epoch, start_generation, "ready").await);
    assert!(!runtime.snapshot().await.start_pending);
    assert!(runtime.media_enabled(epoch));
    runtime.pause_for_consent(HashSet::new()).await;
    assert!(!runtime.media_enabled(epoch));
}

#[tokio::test]
async fn pending_preflight_is_visible_and_explicit_withdrawal_cancels_it() {
    let runtime = runtime();
    let generation = runtime.reserve_start();
    let pending = runtime.snapshot().await;
    assert!(pending.start_pending);
    assert!(runtime.start_is_current(generation));
    assert!(requests_consent_withdrawal("stop listening", &pending));
    let copy = authoritative_text_reply("stop listening", &pending).expect("fixed reply");
    assert!(copy.contains("voice start is pending"));
    assert!(copy.contains("media gate is closed"));

    runtime.cancel_pending_start();
    let cancelled = runtime.snapshot().await;
    assert!(!cancelled.start_pending);
    assert!(!runtime.start_is_current(generation));
    assert!(!cancelled.media_enabled);
}

#[test]
fn finishing_an_old_start_cannot_clear_its_replacement() {
    let runtime = runtime();
    let old = runtime.reserve_start();
    let replacement = runtime.reserve_start();
    runtime.finish_start_attempt(old);
    assert!(runtime.start_is_current(replacement));
    runtime.finish_start_attempt(replacement);
    assert!(!runtime.start_is_current(replacement));
}

#[tokio::test]
async fn a_stop_between_start_entry_and_reservation_invalidates_the_older_operation() {
    let runtime = runtime();
    let issued_start = runtime.start_operation_token();

    // Models `/voice leave` or a text withdrawal completing while the join
    // command is suspended in its defer/channel REST awaits.
    runtime.cancel_pending_start();

    assert_eq!(runtime.reserve_start_if_unchanged(issued_start), None);
    assert!(!runtime.snapshot().await.start_pending);
}

#[tokio::test]
async fn provider_readiness_cannot_publish_listening_before_activation() {
    let runtime = runtime();
    let start_generation = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::new()).await;
    runtime
        .set_prepared_status(epoch, "provider ready but media closed")
        .await;
    let prepared = runtime.snapshot().await;
    assert_eq!(prepared.phase, VoicePhase::Connecting);
    assert!(!prepared.media_enabled);
    assert_eq!(prepared.status, "provider ready but media closed");

    assert!(runtime.activate(epoch, start_generation, "active").await);
    runtime
        .set_prepared_status(epoch, "late duplicate readiness")
        .await;
    let active = runtime.snapshot().await;
    assert_eq!(active.phase, VoicePhase::Listening);
    assert!(active.media_enabled);
    assert_eq!(active.status, "active");
}

#[tokio::test]
async fn cancellation_and_activation_cannot_leave_media_open() {
    for _ in 0..64 {
        let runtime = Arc::new(runtime());
        let start_generation = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::new()).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));

        let activating = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                runtime
                    .activate(epoch, start_generation, "must not survive cancellation")
                    .await
            })
        };
        let cancelling = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                runtime.cancel_pending_start();
            })
        };

        barrier.wait().await;
        let _ = activating.await.expect("activation task");
        cancelling.await.expect("cancellation task");
        assert!(!runtime.media_enabled(epoch));
        assert!(!runtime.start_is_current(start_generation));
    }
}

#[tokio::test]
async fn side_effect_gate_rejects_work_after_revocation() {
    let runtime = runtime();
    let start_generation = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::new()).await;
    assert!(runtime.activate(epoch, start_generation, "active").await);

    let mut effects = 0_u8;
    assert_eq!(
        runtime.with_media_enabled(epoch, || {
            effects += 1;
        }),
        Some(())
    );
    assert!(runtime.revoke_media(epoch));
    assert_eq!(
        runtime.with_media_enabled(epoch, || {
            effects += 1;
        }),
        None
    );
    assert_eq!(effects, 1);
}

#[tokio::test]
async fn connecting_consent_pause_cannot_be_lost_to_activation() {
    for _ in 0..64 {
        let runtime = Arc::new(runtime());
        let start_generation = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::from([7])).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));

        let activating = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                runtime.activate(epoch, start_generation, "active").await
            })
        };
        let pausing = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                let pause = runtime
                    .begin_pause_epoch_for_consent(epoch, HashSet::from([7, 8]), "new participant")
                    .await
                    .expect("exact connecting epoch pauses");
                pause.finish().await;
            })
        };

        barrier.wait().await;
        let _ = activating.await.expect("activation task");
        pausing.await.expect("pause task");
        let snapshot = runtime.snapshot().await;
        assert_eq!(snapshot.phase, VoicePhase::AwaitingConsent);
        assert!(!snapshot.media_enabled);
        assert!(!snapshot.start_pending);
        assert!(!runtime.start_is_current(start_generation));
    }
}

#[test]
fn status_is_flattened_and_bounded() {
    let status = bounded_status(format!("line one\n{}", "x".repeat(500)));
    assert!(!status.contains('\n'));
    assert!(status.chars().count() <= 241);
}
