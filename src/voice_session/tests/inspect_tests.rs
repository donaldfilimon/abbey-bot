//! Inspect publication: coarse, guild-scoped voice lifecycle states.
use super::*;

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

fn inspect_state(inspect: &VoiceInspectRegistry, guild_id: u64) -> VoiceInspectState {
    inspect.state_for(&format!("discord:{guild_id}"))
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
    assert!(
        runtime
            .actor_failed(
                first_epoch,
                "provider failed",
                crate::observability::OperationalErrorCategory::Unavailable,
            )
            .await
    );
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
