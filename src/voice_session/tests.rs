use super::verification::VerificationRunStatus;
use super::*;
use crate::inspect::{VoiceInspectRegistry, VoiceInspectState};
use crate::voice::{VoiceBackendConfig, VoiceConfig};
use std::sync::{Arc, atomic::AtomicBool};

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
    VoiceRuntime::new(VoiceConfig::selected_only(
        1,
        2,
        VoiceBackendConfig::Disabled,
        true,
    ))
}

fn runtime_with_inspect(guild_id: u64) -> (VoiceRuntime, Arc<VoiceInspectRegistry>) {
    let inspect = Arc::new(VoiceInspectRegistry::default());
    let runtime = VoiceRuntime::new_with_inspect(
        VoiceConfig::selected_only(guild_id, 2, VoiceBackendConfig::Disabled, true),
        Arc::clone(&inspect),
        Arc::new(crate::voice_consent_store::ConsentStore::load(
            None, guild_id,
        )),
    );
    (runtime, inspect)
}

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
fn the_effective_mode_starts_as_the_startup_selection() {
    let runtime = runtime();
    assert_eq!(runtime.effective_mode(), runtime.config.mode());
}

#[test]
fn switching_the_effective_mode_round_trips_through_the_atomic() {
    // The mode lives in an atomic rather than a lock so it can never join the
    // documented `inner -> activation_gate -> discord_sessions` order. That
    // only holds if the byte encoding round-trips exactly.
    let runtime = runtime();
    for mode in [VoiceMode::Local, VoiceMode::OpenAi, VoiceMode::Disabled] {
        runtime.set_effective_mode(mode);
        assert_eq!(runtime.effective_mode(), mode, "{mode:?}");
    }
}

#[tokio::test]
async fn a_join_snapshot_is_unaffected_by_a_later_mode_change() {
    // This is the consent-integrity property in miniature. `start_voice`
    // snapshots the backend once and threads it through the Songbird decode
    // mode, the public consent notice, the actor it spawns, and the reply. A
    // switch after that snapshot must not retroactively change what those
    // describe, or participants could be told "local, stays on this Mac" while
    // a cloud actor connects.
    //
    // It does not exercise `start_voice` itself, which needs a live Discord
    // context; it pins the ownership property the snapshot relies on.
    let runtime = runtime();
    let snapshot = runtime
        .effective_backend()
        .expect("the startup backend is always available");
    let snapshot_mode = snapshot.mode();

    runtime.set_effective_mode(VoiceMode::Local);

    assert_eq!(
        snapshot.mode(),
        snapshot_mode,
        "an owned snapshot must not follow later switches"
    );
    assert_ne!(
        runtime.effective_mode(),
        snapshot_mode,
        "the runtime itself did move, so the snapshot is what protects the join"
    );
}

#[test]
fn a_mode_with_no_retained_backend_has_nothing_to_snapshot() {
    // `/voice mode` refuses to select an unconfigured backend; if it ever let
    // one through, this is where the join would fail closed instead of
    // connecting something unintended.
    let runtime = runtime();
    runtime.set_effective_mode(VoiceMode::OpenAi);
    assert!(runtime.effective_backend().is_none());
}

#[tokio::test]
async fn a_backend_change_is_accepted_only_where_no_media_can_be_open() {
    // The same phase set gates `/voice verify start`, so the two surfaces
    // cannot drift: a phase that may arm a run may also change the backend.
    for phase in [
        VoicePhase::Disconnected,
        VoicePhase::PresenceOnly,
        VoicePhase::Failed,
    ] {
        assert!(phase.accepts_backend_change(), "{phase:?}");
        assert_eq!(
            runtime().switch_effective_mode(VoiceMode::OpenAi, phase),
            Ok(()),
            "{phase:?}"
        );
    }
    for phase in [
        VoicePhase::Connecting,
        VoicePhase::Listening,
        VoicePhase::Thinking,
        VoicePhase::Speaking,
        VoicePhase::AwaitingConsent,
    ] {
        assert!(!phase.accepts_backend_change(), "{phase:?}");
        assert_eq!(
            runtime().switch_effective_mode(VoiceMode::OpenAi, phase),
            Err(ModeSwitchRefusal::Active(phase)),
            "{phase:?}"
        );
    }
}

#[tokio::test]
async fn a_pending_start_blocks_a_switch_even_from_an_idle_phase() {
    // The local prepare has a long window in which the phase is still
    // Disconnected, so a phase-only check would be a TOCTOU. The reservation
    // is read under the same gate that writes the mode, not from a snapshot.
    let runtime = runtime();
    let _generation = runtime.reserve_start();
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::OpenAi, VoicePhase::Disconnected),
        Err(ModeSwitchRefusal::Starting)
    );
    assert_eq!(runtime.effective_mode(), VoiceMode::Disabled, "no write");
}

#[tokio::test]
async fn an_open_media_epoch_blocks_a_switch_whatever_the_phase_says() {
    // Activation clears the pending-start token, so this is the state a
    // phase-only or reservation-only check would miss.
    let runtime = runtime();
    let generation = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::new()).await;
    assert!(runtime.activate(epoch, generation, "active").await);
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::OpenAi, VoicePhase::Disconnected),
        Err(ModeSwitchRefusal::MediaOpen)
    );
}

#[tokio::test]
async fn an_armed_verification_run_pins_the_backend_to_local() {
    // Arm-then-switch was the one window left after #76: the run observes
    // local inference only, so a later join under OpenAI would record a cloud
    // activation as local evidence. Refuse at the switch instead.
    let runtime = runtime();
    let _ = runtime.begin_verification().expect("an idle runtime arms");
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::OpenAi, VoicePhase::Disconnected),
        Err(ModeSwitchRefusal::VerificationArmed)
    );
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::Disabled, VoicePhase::Disconnected),
        Err(ModeSwitchRefusal::VerificationArmed),
        "disabled is also a move away from local"
    );
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::Local, VoicePhase::Disconnected),
        Ok(()),
        "returning to local cannot invalidate a local-only run"
    );

    // Completing the run through the idle-leave path releases the pin.
    let token = runtime.verification_run_token().expect("armed");
    assert!(runtime.note_verification_final_leave(token));
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::OpenAi, VoicePhase::Disconnected),
        Ok(())
    );
}

#[test]
fn every_refusal_names_a_way_out_and_the_phase_it_refused() {
    // These sentences reach a Discord channel, and each is written once.
    for refusal in [
        ModeSwitchRefusal::Starting,
        ModeSwitchRefusal::MediaOpen,
        ModeSwitchRefusal::Active(VoicePhase::Listening),
        ModeSwitchRefusal::VerificationArmed,
    ] {
        assert!(refusal.message().contains("/voice leave"), "{refusal:?}");
    }
    assert!(
        ModeSwitchRefusal::Active(VoicePhase::Speaking)
            .message()
            .contains(VoicePhase::Speaking.label())
    );
}

#[test]
fn a_reservation_captures_the_backend_it_will_use_and_pins_the_switch() {
    // Codex review on #82: a join reserved and snapshotted between the mode
    // switch's check and its write. Now both sides go through the activation
    // gate: a pending reservation refuses the switch, and a reservation made
    // after a switch captures the switched backend.
    let runtime = runtime();
    let token = runtime.start_operation_token();
    let (generation, backend) = runtime
        .reserve_start_with_backend(token)
        .expect("fresh token reserves");
    assert!(runtime.start_is_current(generation));
    assert!(matches!(backend, Some(VoiceBackendConfig::Disabled)));

    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::Local, VoicePhase::Disconnected),
        Err(ModeSwitchRefusal::Starting),
        "a pending start pins the mode"
    );
    assert_eq!(runtime.effective_mode(), VoiceMode::Disabled);

    runtime.cancel_pending_start();
    assert_eq!(
        runtime.switch_effective_mode(VoiceMode::Local, VoicePhase::Disconnected),
        Ok(())
    );
    assert_eq!(runtime.effective_mode(), VoiceMode::Local);

    // The next reservation sees the switched mode, which this fixture has no
    // retained backend for: the join fails closed rather than using the old.
    let token = runtime.start_operation_token();
    let (_, backend) = runtime
        .reserve_start_with_backend(token)
        .expect("fresh token reserves");
    assert!(backend.is_none());
}

#[test]
fn a_stale_operation_token_reserves_nothing_and_captures_nothing() {
    let runtime = runtime();
    let token = runtime.start_operation_token();
    runtime.cancel_pending_start();
    assert!(runtime.reserve_start_with_backend(token).is_none());
}

fn inspect_state(inspect: &VoiceInspectRegistry, guild_id: u64) -> VoiceInspectState {
    inspect.state_for(&format!("discord:{guild_id}"))
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
async fn inspect_voice_lifecycle_is_coarse_guild_scoped_and_offline() {
    let (runtime, inspect) = runtime_with_inspect(41);
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Off);
    assert_eq!(inspect_state(&inspect, 42), VoiceInspectState::Off);

    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Paused);
    assert!(runtime.activate(epoch, start, "active").await);
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Active);

    for phase in [
        VoicePhase::Listening,
        VoicePhase::Thinking,
        VoicePhase::Speaking,
    ] {
        runtime.set_status(epoch, phase, "safe coarse state").await;
        assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Active);
    }
    assert_eq!(inspect_state(&inspect, 42), VoiceInspectState::Off);

    assert!(
        runtime
            .pause_epoch_for_consent(epoch, HashSet::from([7, 8]), "membership changed")
            .await
    );
    assert_eq!(
        inspect_state(&inspect, 41),
        VoiceInspectState::AwaitingConsent
    );

    runtime
        .set_presence_with_discord_session("presence-session".into(), "presence only")
        .await;
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Presence);
    runtime.disconnect("shutdown").await;
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Off);
    assert_eq!(inspect_state(&inspect, 42), VoiceInspectState::Off);
}

#[tokio::test]
async fn inspect_voice_revocation_and_late_status_remain_paused() {
    let (runtime, inspect) = runtime_with_inspect(41);
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(epoch, start, "active").await);
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Active);

    assert!(runtime.revoke_media(epoch));
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Paused);
    runtime
        .set_status(epoch, VoicePhase::Listening, "late actor status")
        .await;
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Paused);
}

#[tokio::test]
async fn inspect_voice_actor_failure_and_consent_withdrawal_publish_safe_states() {
    let (runtime, inspect) = runtime_with_inspect(41);
    let first_start = runtime.reserve_start();
    let first_epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(first_epoch, first_start, "active").await);
    assert!(runtime.actor_failed(first_epoch, "provider failed").await);
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Paused);

    let second_start = runtime.reserve_start();
    let second_epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(second_epoch, second_start, "active").await);
    assert!(
        runtime
            .actor_awaiting_consent(second_epoch, "consent withdrawn")
            .await
    );
    assert_eq!(
        inspect_state(&inspect, 41),
        VoiceInspectState::AwaitingConsent
    );
}

#[tokio::test]
async fn inspect_voice_adverse_session_events_close_only_the_bound_guild() {
    let (runtime, inspect) = runtime_with_inspect(41);
    inspect.publish("discord:42", VoiceInspectState::Presence);
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .bind_discord_session(epoch, "session-one".into())
            .await
    );
    assert!(runtime.activate(epoch, start, "active").await);
    assert_eq!(
        runtime.revoke_for_discord_session("session-one"),
        DiscordSessionEvent::Current {
            epoch,
            media_was_enabled: true,
        }
    );
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Paused);
    assert_eq!(inspect_state(&inspect, 42), VoiceInspectState::Presence);
}

#[tokio::test]
async fn inspect_voice_stale_epoch_and_shutdown_cannot_restore_active() {
    let (runtime, inspect) = runtime_with_inspect(41);
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::new()).await;
    assert!(runtime.activate(epoch, start, "active").await);
    runtime.disconnect("process shutdown stopped voice").await;
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Off);

    runtime
        .set_status(epoch, VoicePhase::Speaking, "stale active status")
        .await;
    assert_eq!(inspect_state(&inspect, 41), VoiceInspectState::Off);
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
                    task: actor,
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

#[test]
fn status_is_flattened_and_bounded() {
    let status = bounded_status(format!("line one\n{}", "x".repeat(500)));
    assert!(!status.contains('\n'));
    assert!(status.chars().count() <= 241);
}

#[tokio::test]
async fn redacted_verification_spans_activation_pause_resume_and_final_leave() {
    let runtime = runtime();
    let run = runtime.begin_verification().expect("arm verification");
    assert_eq!(run.run, 1);
    assert!(runtime.verification_active());

    let first_start = runtime.reserve_start();
    let first_epoch = runtime
        .begin(HashSet::from([
            900_000_000_000_000_001,
            900_000_000_000_000_002,
        ]))
        .await;
    assert!(
        runtime
            .activate_verified(
                first_epoch,
                first_start,
                "active",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 2,
                    resumed: false,
                },
            )
            .await
    );
    runtime.note_verification_decoded_receive();
    runtime.note_verification_stt_completion(first_epoch);
    runtime.note_completed_turn();
    runtime.note_barge_in();
    runtime.note_verification_barge_in_cancellation();

    let pause = runtime
        .begin_participant_pause_epoch_for_consent(
            first_epoch,
            HashSet::from([
                900_000_000_000_000_001,
                900_000_000_000_000_002,
                900_000_000_000_000_003,
            ]),
            "new participant",
        )
        .await
        .expect("active epoch pauses");
    pause.finish().await;

    let resumed_start = runtime.reserve_start();
    let resumed_epoch = runtime
        .begin(HashSet::from([
            900_000_000_000_000_001,
            900_000_000_000_000_002,
            900_000_000_000_000_003,
        ]))
        .await;
    assert!(
        runtime
            .activate_verified(
                resumed_epoch,
                resumed_start,
                "resumed",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 3,
                    resumed: true,
                },
            )
            .await
    );
    runtime.disconnect("left").await;
    assert!(runtime.note_verification_final_leave(run.run));

    let verification = runtime
        .verification_snapshot()
        .expect("completed verification remains reportable");
    assert_eq!(verification.status, VerificationRunStatus::Complete);
    assert_eq!(verification.observed_checks(), 8);
    assert_eq!(verification.activations, 2);
    assert_eq!(verification.first_participant_count, Some(2));
    assert_eq!(verification.latest_participant_count, Some(3));
    assert_ne!(
        verification.first_consent_epoch,
        verification.latest_consent_epoch
    );
    assert!(!runtime.verification_active());

    let report = runtime.verification_report();
    assert!(report.contains("observed: 8/8"));
    assert!(report.contains("counters only; no audio, user or message IDs, transcripts"));
    assert!(report.contains("Manual witness still required"));
    assert!(report.contains("separate from local/source test evidence"));
    assert!(!report.contains("900000000000000001"));
    assert!(!report.contains("new participant"));
    assert!(report.chars().count() < 2_000);
}

#[tokio::test]
async fn verification_ignores_stale_stt_and_requires_final_leave_before_rearming() {
    let runtime = runtime();
    let run = runtime.begin_verification().expect("arm verification");
    assert!(runtime.begin_verification().is_err());

    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .activate_verified(
                epoch,
                start,
                "active",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 1,
                    resumed: false,
                },
            )
            .await
    );
    runtime.disconnect("closed gate").await;
    runtime.note_verification_stt_completion(epoch);
    assert_eq!(
        runtime
            .verification_snapshot()
            .expect("run")
            .stt_completions,
        0
    );

    assert!(runtime.note_verification_final_leave(run.run));
    let replacement = runtime.begin_verification().expect("replace completed run");
    assert_eq!(replacement.run, 2);
    assert_eq!(replacement.observed_checks(), 0);
}

#[test]
fn a_stale_leave_token_cannot_complete_a_replacement_run() {
    let runtime = runtime();
    let first = runtime.begin_verification().expect("first run");
    assert_eq!(runtime.verification_run_token(), Some(first.run));
    assert!(runtime.note_verification_final_leave(first.run));

    let replacement = runtime.begin_verification().expect("replacement run");
    assert_ne!(first.run, replacement.run);
    assert!(
        !runtime.note_verification_final_leave(first.run),
        "an older leave must not complete a later verifier"
    );
    let snapshot = runtime
        .verification_snapshot()
        .expect("replacement remains");
    assert_eq!(snapshot.run, replacement.run);
    assert_eq!(snapshot.status, VerificationRunStatus::Armed);
    assert!(!snapshot.final_leave);
}

#[tokio::test]
async fn verification_cannot_arm_during_a_pending_or_open_media_epoch() {
    let runtime = runtime();
    let pending = runtime.reserve_start();
    assert!(runtime.begin_verification().is_err());
    runtime.finish_start_attempt(pending);

    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(epoch, start, "active").await);
    assert!(runtime.begin_verification().is_err());
    runtime.disconnect("left").await;
    assert!(runtime.begin_verification().is_ok());
}

#[tokio::test]
async fn verified_activation_and_immediate_participant_pause_are_linearized() {
    let runtime = runtime();
    runtime.begin_verification().expect("arm verification");
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .activate_verified(
                epoch,
                start,
                "active",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 1,
                    resumed: false,
                },
            )
            .await
    );
    let pause = runtime
        .begin_participant_pause_epoch_for_consent(
            epoch,
            HashSet::from([7, 8]),
            "participant changed",
        )
        .await
        .expect("the exact active epoch pauses");
    pause.finish().await;

    let run = runtime.verification_snapshot().expect("run");
    assert_eq!(run.activations, 1);
    assert_eq!(run.participant_change_pauses, 1);
    assert_eq!(run.latest_participant_count, Some(2));
}

#[tokio::test]
async fn incomplete_authorization_cannot_satisfy_the_acceptance_check() {
    let runtime = runtime();
    runtime.begin_verification().expect("arm verification");
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7, 8])).await;
    assert!(
        runtime
            .activate_verified(
                epoch,
                start,
                "active",
                VerificationActivation {
                    manager_authorized: false,
                    caller_present: true,
                    participant_count: 2,
                    resumed: false,
                },
            )
            .await
    );
    let run = runtime.verification_snapshot().expect("run");
    assert_eq!(run.observed_checks(), 0);
    assert!(
        runtime
            .verification_report()
            .contains("Authorization: pending")
    );
}
