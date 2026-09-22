//! Epoch and Discord session events: stale epochs and adverse events revoke only their own session.
use super::*;

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
