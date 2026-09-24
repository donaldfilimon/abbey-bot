use super::*;
use crate::service::{OperationKind, ServiceSupervisor, ShutdownReason};
use crate::work::{WorkAccess, WorkError};
use std::sync::{Condvar, Mutex};

struct HeldWorkSink {
    writes: Mutex<Vec<Vec<u8>>>,
    entered: tokio::sync::Notify,
    release: (Mutex<bool>, Condvar),
}
impl PersistenceSink for HeldWorkSink {
    fn publish(
        &self,
        _: &std::path::Path,
        _: &std::path::Path,
        bytes: &[u8],
    ) -> Result<(), PersistErrorCategory> {
        let first = {
            let mut writes = self.writes.lock().unwrap();
            writes.push(bytes.to_vec());
            writes.len() == 1
        };
        if first {
            self.entered.notify_one();
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
        }
        Ok(())
    }
}
struct Release(Arc<HeldWorkSink>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
fn access() -> WorkAccess {
    WorkAccess {
        actor: 1,
        guild: None,
        channel: 2,
        can_view: true,
        can_manage: false,
    }
}

#[tokio::test]
async fn framework_abort_retains_publication_and_final_snapshot() {
    let sink = Arc::new(HeldWorkSink {
        writes: Mutex::new(Vec::new()),
        entered: tokio::sync::Notify::new(),
        release: (Mutex::new(false), Condvar::new()),
    });
    let release = Release(sink.clone());
    let state = Arc::new(AppState::in_memory_with_persistence(
        Some(std::env::temp_dir().join("injected-work")),
        sink.clone(),
    ));
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let registry = supervisor.operations();
    let mut writer = state.attach_service(registry.clone());
    let owned = state.clone();
    let result = registry
        .spawn_result(OperationKind::FrameworkDispatch, async move {
            owned
                .commit_work(|store| store.create_project(access(), "Retained", "one"))
                .await
        })
        .unwrap();
    sink.entered.notified().await;
    supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    supervisor.request_abort();
    supervisor.next_completion().await;
    assert!(result.await.is_err());
    assert!(supervisor.try_freeze(writer.idle()).is_err());
    assert_eq!(
        state.commit_work(|_| Ok(())).await,
        Err(WorkError::Persistence)
    );
    drop(release);
    supervisor.next_completion().await;
    assert_eq!(AppState::lock(&state.stores).work.projects.len(), 1);
    assert_eq!(state.final_snapshot().stores.work.projects.len(), 1);
    assert!(writer.idle());
    supervisor.try_freeze(writer.idle()).unwrap();
    writer.close_admission();
    writer
        .final_snapshot(
            state.final_snapshot(),
            tokio::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    writer.stop();
    writer.joined().await.unwrap();
    let writes = sink.writes.lock().unwrap();
    let final_stores: Stores = serde_json::from_slice(&writes[2]).unwrap();
    assert_eq!(final_stores.work.projects.len(), 1);
}

#[tokio::test]
async fn canonical_failure_does_not_publish_but_projection_failure_does() {
    for canonical_failure in [true, false] {
        let sink = if canonical_failure {
            crate::persist::tests::RuntimeRecordingSink::fail_canonical(
                PersistErrorCategory::SyncTemporary,
            )
        } else {
            crate::persist::tests::RuntimeRecordingSink::fail_projection(
                PersistErrorCategory::SyncTemporary,
            )
        };
        let state = Arc::new(AppState::in_memory_with_persistence(
            Some(std::env::temp_dir().join("injected-work")),
            Arc::new(sink),
        ));
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let mut writer = state.attach_service(supervisor.operations());
        let result = state
            .commit_work(|store| store.create_project(access(), "Candidate", "one"))
            .await;
        assert_eq!(result.is_err(), canonical_failure);
        assert_eq!(
            AppState::lock(&state.stores).work.projects.len(),
            usize::from(!canonical_failure)
        );
        supervisor.next_completion().await;
        writer.close_admission();
        writer.stop();
        writer.joined().await.unwrap();
    }
}

#[tokio::test]
async fn dropped_waiter_keeps_serialization_through_publication() {
    let sink = Arc::new(HeldWorkSink {
        writes: Mutex::new(Vec::new()),
        entered: tokio::sync::Notify::new(),
        release: (Mutex::new(false), Condvar::new()),
    });
    let release = Release(sink.clone());
    let state = Arc::new(AppState::in_memory_with_persistence(
        Some(std::env::temp_dir().join("injected-work")),
        sink.clone(),
    ));
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let owned = state.clone();
    let waiter = tokio::spawn(async move {
        owned
            .commit_work(|store| store.create_project(access(), "First", "one"))
            .await
    });
    sink.entered.notified().await;
    waiter.abort();
    assert!(waiter.await.is_err());
    let owned = state.clone();
    let second = tokio::spawn(async move {
        owned
            .commit_work(|store| store.create_project(access(), "Second", "two"))
            .await
    });
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    assert_eq!(sink.writes.lock().unwrap().len(), 1);
    drop(release);
    second.await.unwrap().unwrap();
    supervisor.next_completion().await;
    supervisor.next_completion().await;
    assert_eq!(state.final_snapshot().stores.work.projects.len(), 2);
    writer.close_admission();
    writer.stop();
    writer.joined().await.unwrap();
    let writes = sink.writes.lock().unwrap();
    let second_snapshot: Stores = serde_json::from_slice(&writes[2]).unwrap();
    assert_eq!(second_snapshot.work.projects.len(), 2);
}
