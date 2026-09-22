//! Start and activation: preflight, cancellation, media gate, side-effect gate.
use super::*;

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
