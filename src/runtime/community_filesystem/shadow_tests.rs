use super::*;
use crate::moderation::shadow::tests::capture;
use crate::persist::moderation_shadow::tests::fixture;
use crate::service::{
    FreezeError, OperationKind, OwnedTaskKind, ServiceSupervisor, ShutdownReason, TaskExit,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

#[tokio::test(flavor = "current_thread")]
async fn actual_shadow_publication_owner_survives_waiter_cancel_and_is_observed_joined() {
    let f = fixture();
    let state = AppState::in_memory_with_persistence(
        Some(f.data.clone()),
        Arc::new(crate::persist::FsPersistenceSink),
    );
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    assert!(state.service.set(supervisor.operations()).is_ok());
    let (entered, started) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let first = std::cell::Cell::new(true);
    let (release, blocked) = std::sync::mpsc::channel();
    let waiting = state.clone();
    let policy = f.policy.clone();
    let digest = f.digest.clone();
    let shadow_lock = f.data.join("community-operations/shadow.lock");
    let waiter = tokio::spawn(async move {
        waiting
            .publish_shadow_using_clock(policy, digest, capture(100), move || {
                if shadow_lock.exists() && first.replace(false) {
                    let _ = entered.lock().unwrap().take().unwrap().send(());
                    blocked.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                100
            })
            .await
    });
    started.await.unwrap();
    // The actual transaction now owns both policy/shadow locks on the retained
    // blocking worker, before its authorized private reload and publication.
    // Earlier nonmutating preflight clocks return 100 without blocking.
    assert!(f.policy.with_extension("mode-lock").exists());
    assert!(f.data.join("community-operations/shadow.lock").exists());
    assert!(
        !f.data
            .join("community-operations/contextual-shadow.json")
            .exists()
    );
    tokio::time::timeout(
        Duration::from_millis(100),
        tokio::time::sleep(Duration::from_millis(5)),
    )
    .await
    .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    supervisor.request_cancellation();
    supervisor.request_abort();
    assert_eq!(supervisor.outstanding().len(), 1);
    assert_eq!(
        supervisor.outstanding()[0].kind,
        OwnedTaskKind::Operation(OperationKind::CommunityFilesystem)
    );
    assert_eq!(
        supervisor.try_freeze(true),
        Err(FreezeError::TasksNotJoined)
    );
    release.send(()).unwrap();
    let joined = supervisor.next_completion().await;
    assert_eq!(
        joined.kind,
        OwnedTaskKind::Operation(OperationKind::CommunityFilesystem)
    );
    assert_eq!(joined.exit, TaskExit::Returned);
    assert!(!joined.abort_requested);
    assert!(supervisor.outstanding().is_empty());
    assert_eq!(supervisor.try_freeze(true), Ok(()));
    let observed = crate::persist::moderation_shadow::load(&f.data).unwrap();
    assert_eq!((observed.revision, observed.cases.len()), (1, 1));
    assert!(!f.policy.with_extension("mode-lock").exists());
    assert!(!f.data.join("community-operations/shadow.lock").exists());
    assert_eq!(
        state
            .publish_shadow_using_clock(f.policy.clone(), f.digest.clone(), capture(101), || 101)
            .await,
        Err("filesystem admission closed")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn no_retained_admission_never_publishes_or_claims_saved() {
    let f = fixture();
    let state = AppState::in_memory_with_persistence(
        Some(f.data.clone()),
        Arc::new(crate::persist::FsPersistenceSink),
    );
    assert_eq!(
        state
            .publish_shadow_using_clock(f.policy.clone(), f.digest.clone(), capture(100), || 100)
            .await,
        Err("filesystem ownership unavailable")
    );
    assert!(
        !f.data
            .join("community-operations/contextual-shadow.json")
            .exists()
    );
}
