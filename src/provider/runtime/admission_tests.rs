//! Finite-poll regressions for queued capacity grants and fresh admission.
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

struct CountingAdapter {
    id: ProviderId,
    calls: AtomicUsize,
}
impl TurnAdapter for CountingAdapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(std::future::ready(Ok(ModelTurn {
            text: "synthetic answer".into(),
            calls: Vec::new(),
        })))
    }
}
fn adapter(runtime: &mut ProviderRuntime, name: &str) -> Arc<CountingAdapter> {
    let adapter = Arc::new(CountingAdapter {
        id: ProviderId::parse(name).unwrap(),
        calls: AtomicUsize::new(0),
    });
    runtime.register_test_adapter(adapter.clone());
    adapter
}
fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test(start_paused = true)]
async fn queued_reservations_retain_the_fifo_grant_legacy() {
    queued_reservations_retain_the_fifo_grant(false).await;
}

#[tokio::test(start_paused = true)]
async fn queued_reservations_retain_the_fifo_grant_adaptive() {
    queued_reservations_retain_the_fifo_grant(true).await;
}

async fn queued_reservations_retain_the_fifo_grant(adaptive: bool) {
    for pinned in [true, false] {
        let mut runtime = ProviderRuntime::empty();
        let adapter = adapter(&mut runtime, "primary");
        if adaptive {
            runtime.apply_configuration(
                ProviderConfig::from_iter([("ABBEY_PROVIDER_ORDER", "primary")]).unwrap(),
            );
        }
        let slots = &runtime.entries[&adapter.id].slots;
        let mut blocker = runtime.begin(false, false);
        blocker.reserve().await.unwrap();
        let mut first = runtime.conversation(
            RequestClass::TextReadOnly,
            false,
            false,
            pinned.then(|| adapter.id.clone()),
        );
        let mut second = runtime.conversation(
            RequestClass::TextReadOnly,
            false,
            false,
            pinned.then(|| adapter.id.clone()),
        );
        let mut first_reservation = Box::pin(first.reserve());
        let mut second_reservation = Box::pin(second.reserve());
        assert!(poll_once(first_reservation.as_mut()).is_pending());
        assert!(poll_once(second_reservation.as_mut()).is_pending());
        drop(blocker);

        // A must consume its FIFO grant before B is polled again. Dropping and
        // reacquiring the grant instead returns Pending and hands it to B.
        assert!(
            matches!(poll_once(first_reservation.as_mut()), Poll::Ready(Ok(()))),
            "first FIFO waiter did not admit: adaptive={adaptive}, pinned={pinned}"
        );
        drop(first_reservation);
        assert!(first.lease.as_ref().unwrap().permit.is_some());
        assert!(poll_once(second_reservation.as_mut()).is_pending());
        assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
        assert_eq!(slots.available_permits(), 0);

        first
            .execute("synthetic", &[], &[], ResponseStyle::Default, None)
            .await
            .unwrap();
        assert_eq!(adapter.calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            poll_once(second_reservation.as_mut()),
            Poll::Ready(Ok(()))
        ));
        drop(second_reservation);
        assert!(second.lease.as_ref().unwrap().permit.is_some());
        second
            .execute("synthetic", &[], &[], ResponseStyle::Default, None)
            .await
            .unwrap();
        assert_eq!(adapter.calls.load(Ordering::Relaxed), 2);
        assert!(first.lease.is_none() && second.lease.is_none());
        assert_eq!(slots.available_permits(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn queued_grant_rechecks_a_new_circuit_block() {
    let mut runtime = ProviderRuntime::empty();
    let adapter = adapter(&mut runtime, "primary");
    let entry = &runtime.entries[&adapter.id];
    let mut blocker = runtime.begin(false, false);
    blocker.reserve().await.unwrap();
    let mut waiter = runtime.begin(false, false);
    let mut reservation = Box::pin(waiter.reserve());
    assert!(poll_once(reservation.as_mut()).is_pending());
    assert!(lock(&runtime.state).router.restore_blocked(
        &adapter.id,
        &entry.identity,
        ProviderFailureKind::Authentication,
    ));
    drop(blocker);
    let Poll::Ready(Err(error)) = poll_once(reservation.as_mut()) else {
        panic!("queued grant bypassed the new circuit block");
    };
    assert_eq!(
        error.unavailable(),
        Some(RouteUnavailableReason::BlockedPendingRequalification)
    );
    drop(reservation);
    assert!(waiter.lease.is_none());
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
    assert_eq!(entry.slots.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn queued_grant_rechecks_a_new_conversation_exclusion() {
    let mut runtime = ProviderRuntime::empty();
    let adapter = adapter(&mut runtime, "primary");
    let mut waiter = runtime.begin(false, false);
    waiter.reserve().await.unwrap();
    // Release the prior attempt while preserving its conversation selection.
    drop(waiter.lease.take());
    let effects = waiter.effects();
    let mut blocker = runtime.begin(false, false);
    blocker.reserve().await.unwrap();
    let mut reservation = Box::pin(waiter.reserve());
    assert!(poll_once(reservation.as_mut()).is_pending());
    assert!(lock(&effects.0).begin_fallback(ProviderFailureKind::Timeout));
    drop(blocker);
    let Poll::Ready(Err(error)) = poll_once(reservation.as_mut()) else {
        panic!("queued grant bypassed the new conversation exclusion");
    };
    assert_eq!(
        error.unavailable(),
        Some(RouteUnavailableReason::NoConfiguredProvider)
    );
    drop(reservation);
    assert!(waiter.lease.is_none());
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
    assert_eq!(runtime.entries[&adapter.id].slots.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn cancelled_fifo_waiter_returns_its_assigned_grant() {
    let mut runtime = ProviderRuntime::empty();
    let adapter = adapter(&mut runtime, "primary");
    let mut blocker = runtime.begin(false, false);
    blocker.reserve().await.unwrap();
    let mut first = runtime.begin(false, false);
    let mut second = runtime.begin(false, false);
    let mut first_reservation = Box::pin(first.reserve());
    let mut second_reservation = Box::pin(second.reserve());
    assert!(poll_once(first_reservation.as_mut()).is_pending());
    assert!(poll_once(second_reservation.as_mut()).is_pending());
    drop(blocker);
    // Tokio assigned the grant to A; cancelling A must transfer it to B.
    drop(first_reservation);
    assert!(first.lease.is_none());
    assert!(matches!(
        poll_once(second_reservation.as_mut()),
        Poll::Ready(Ok(()))
    ));
    drop(second_reservation);
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 0);
    second
        .execute("synthetic", &[], &[], ResponseStyle::Default, None)
        .await
        .unwrap();
    assert_eq!(adapter.calls.load(Ordering::Relaxed), 1);
    assert!(second.lease.is_none());
    assert_eq!(runtime.entries[&adapter.id].slots.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn queued_grant_keeps_adaptive_ranking_and_releases_unused_capacity() {
    let mut runtime = ProviderRuntime::empty();
    let primary = adapter(&mut runtime, "primary");
    let secondary = adapter(&mut runtime, "secondary");
    runtime.apply_configuration(
        ProviderConfig::from_iter([("ABBEY_PROVIDER_ORDER", "primary,secondary")]).unwrap(),
    );
    // Lower primary's score without executing either synthetic adapter.
    {
        let mut state = lock(&runtime.state);
        let (_, attempt) = state
            .router
            .select(
                RequestClass::TextReadOnly,
                runtime.clock.now_ms(),
                Some(&primary.id),
                &Default::default(),
            )
            .unwrap();
        state.router.complete(
            attempt,
            ProviderFailureKind::Timeout,
            RetryAfter::Absent,
            None,
            runtime.clock.now_ms(),
        );
    }
    let mut primary_blocker = runtime.voice(&primary.id);
    primary_blocker.reserve().await.unwrap();
    let mut secondary_blocker = runtime.voice(&secondary.id);
    secondary_blocker.reserve().await.unwrap();
    let mut waiter = runtime.begin(false, false);
    let mut reservation = Box::pin(waiter.reserve());
    assert!(poll_once(reservation.as_mut()).is_pending());
    // The route-order wait queues primary. Both slots become available to
    // selection, but secondary now ranks higher than the retained grant.
    drop(primary_blocker);
    drop(secondary_blocker);
    assert!(matches!(
        poll_once(reservation.as_mut()),
        Poll::Ready(Ok(()))
    ));
    drop(reservation);
    assert_eq!(waiter.lease.as_ref().unwrap().id, secondary.id);
    assert_eq!(runtime.entries[&primary.id].slots.available_permits(), 1);
    assert_eq!(runtime.entries[&secondary.id].slots.available_permits(), 0);
    waiter
        .execute("synthetic", &[], &[], ResponseStyle::Default, None)
        .await
        .unwrap();
    assert_eq!(primary.calls.load(Ordering::Relaxed), 0);
    assert_eq!(secondary.calls.load(Ordering::Relaxed), 1);
    assert!(waiter.lease.is_none());
    assert_eq!(runtime.entries[&primary.id].slots.available_permits(), 1);
    assert_eq!(runtime.entries[&secondary.id].slots.available_permits(), 1);
}
