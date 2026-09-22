//! Effective-mode switching: join snapshots, switch refusals, reservation pinning.
use super::*;

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
