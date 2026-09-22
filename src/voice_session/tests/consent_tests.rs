//! Consent phase: saved-consent activation, withdrawal, and the text consent boundary.
use super::*;

#[tokio::test]
async fn saved_consent_is_required_at_both_activation_paths_for_the_exact_roster() {
    for verified in [false, true] {
        let mut runtime = runtime();
        runtime.set_effective_mode(VoiceMode::Local);
        let start = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::from([10, 20])).await;
        assert!(!runtime.activate(epoch, start, "missing storage").await);
        runtime.consent = Arc::new(
            crate::voice_consent_store::ConsentStore::acknowledged_fixture(
                1,
                &[10],
                VoiceMode::Local,
            ),
        );
        let evidence = VerificationActivation {
            manager_authorized: true,
            caller_present: true,
            participant_count: 2,
            resumed: false,
        };
        // Test the two production entry points, not just the policy helper.
        let accepted = if verified {
            runtime
                .activate_verified(epoch, start, "active", evidence)
                .await
        } else {
            runtime.activate(epoch, start, "active").await
        };
        assert!(!accepted);
        assert!(!runtime.media_enabled(epoch));
        runtime.consent = Arc::new(
            crate::voice_consent_store::ConsentStore::acknowledged_fixture(
                1,
                &[10, 20],
                VoiceMode::OpenAi,
            ),
        );
        assert!(
            !runtime
                .activate(epoch, start, "wrong processing scope")
                .await
        );
        runtime.consent = Arc::new(
            crate::voice_consent_store::ConsentStore::acknowledged_fixture(
                1,
                &[10, 20],
                VoiceMode::Local,
            ),
        );
        let accepted = if verified {
            runtime
                .activate_verified(epoch, start, "active", evidence)
                .await
        } else {
            runtime.activate(epoch, start, "active").await
        };
        assert!(accepted);
    }
}

#[tokio::test]
async fn withdrawal_closes_media_and_invalidates_a_reserved_start_before_disk_wait() {
    let mut runtime = runtime();
    runtime.set_effective_mode(VoiceMode::Local);
    runtime.consent = Arc::new(
        crate::voice_consent_store::ConsentStore::acknowledged_fixture(1, &[10], VoiceMode::Local),
    );
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([10])).await;
    assert!(runtime.activate(epoch, start, "active").await);
    let pending = runtime.reserve_start();
    let save = runtime.change_consent(10, 2, crate::voice_consent::Choice::Withdraw, 2, true);
    assert!(!runtime.media_enabled(epoch));
    assert!(!runtime.start_is_current(pending));
    assert!(!runtime.consent.agrees(10, VoiceMode::Local));
    // This unit fixture intentionally has no disk. Failure may never undo
    // synchronous revocation or make an old start eligible again.
    assert!(save.saved.await.unwrap().is_err());
    let epoch = runtime.begin(HashSet::from([10])).await;
    let start = runtime.reserve_start();
    assert!(!runtime.activate(epoch, start, "cannot revive").await);
}

#[tokio::test]
async fn absent_attested_withdrawal_closes_the_epoch_but_stale_stop_does_not() {
    let mut runtime = runtime();
    runtime.set_effective_mode(VoiceMode::Local);
    runtime.consent = Arc::new(
        crate::voice_consent_store::ConsentStore::acknowledged_fixture(1, &[10], VoiceMode::Local),
    );
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([10])).await;
    assert!(runtime.activate(epoch, start, "active").await);
    let stale = runtime.change_consent(10, 1, crate::voice_consent::Choice::Withdraw, 2, true);
    assert_eq!(stale.epoch_to_stop, None);
    assert!(runtime.media_enabled(epoch));
    assert!(!stale.saved.await.unwrap().unwrap());
    // The caller is absent now, but remains in the immutable receive epoch.
    let withdrawn = runtime.change_consent(10, 2, crate::voice_consent::Choice::Withdraw, 2, false);
    assert_eq!(withdrawn.epoch_to_stop, Some(epoch));
    assert!(!runtime.media_enabled(epoch));
    assert_eq!(runtime.revoke_for_unattested_participant(10), None);
    assert!(
        !runtime.media_enabled(epoch),
        "rejoin must not revive old attestation"
    );
    assert!(withdrawn.saved.await.unwrap().is_err());
}

#[tokio::test]
async fn unattested_outsider_withdrawal_does_not_stop_other_participants_call() {
    let mut runtime = runtime();
    runtime.set_effective_mode(VoiceMode::Local);
    runtime.consent = Arc::new(
        crate::voice_consent_store::ConsentStore::acknowledged_fixture(
            1,
            &[10, 20],
            VoiceMode::Local,
        ),
    );
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([10])).await;
    assert!(runtime.activate(epoch, start, "active").await);

    let withdrawn = runtime.change_consent(20, 2, crate::voice_consent::Choice::Withdraw, 2, false);

    assert_eq!(withdrawn.epoch_to_stop, None);
    assert!(runtime.media_enabled(epoch));
    assert!(!runtime.consent.agrees(20, VoiceMode::Local));
    assert!(withdrawn.saved.await.unwrap().is_err());
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
    assert!(requests_consent_withdrawal("I do not consent", &inactive));
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

#[tokio::test]
async fn exact_consent_epoch_closes_and_cancels_before_slow_actor_cleanup() {
    let runtime = Arc::new(runtime());
    let start_generation = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(epoch, start_generation, "active").await);

    let (cancel, mut cancellation) = watch::channel(false);
    let cancelled = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let actor = {
        let cancelled = Arc::clone(&cancelled);
        let release = Arc::clone(&release);
        tokio::spawn(async move {
            cancellation.changed().await.expect("runtime owns sender");
            assert!(*cancellation.borrow(), "actor receives cancellation");
            cancelled.store(true, Ordering::SeqCst);
            // Model slow provider/playback cleanup after cancellation. The
            // public epoch and media gate must already be closed while this
            // actor remains alive.
            release.notified().await;
        })
    };
    assert!(
        runtime
            .install_control(
                epoch,
                SessionControl {
                    cancel,
                    task: actor.into(),
                    playback: Arc::new(Mutex::new(None)),
                },
            )
            .await
    );

    let pause = runtime
        .begin_pause_epoch_for_consent(epoch, HashSet::from([7, 8]), "consent withdrawn")
        .await
        .expect("exact live epoch pauses");
    let snapshot = runtime.snapshot().await;
    assert_eq!(snapshot.epoch, epoch + 1);
    assert_eq!(snapshot.phase, VoicePhase::AwaitingConsent);
    assert!(!snapshot.media_enabled);
    assert!(!snapshot.start_pending);
    assert!(!runtime.is_current(epoch));

    tokio::time::timeout(Duration::from_millis(100), async {
        while !cancelled.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actor cancellation is immediate");

    let finishing = tokio::spawn(pause.finish());
    tokio::task::yield_now().await;
    assert!(!finishing.is_finished(), "cleanup fixture is still slow");
    release.notify_one();
    finishing.await.expect("cleanup task");
}
