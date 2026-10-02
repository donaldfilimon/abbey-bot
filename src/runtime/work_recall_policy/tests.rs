use super::*;
use crate::{
    persist::{PersistErrorCategory, tests::RuntimeRecordingSink},
    service::ServiceSupervisor,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "abbey-recall-policy-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create recall-policy fixture directory: {error}"),
            }
        }
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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

fn state(dir: &Directory, sink: RuntimeRecordingSink, enabled: bool) -> Arc<AppState> {
    let mut state = AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(sink));
    if enabled {
        Arc::get_mut(&mut state).unwrap().work_recall_rollout =
            RecallRollout::parse(Some(r#"[{"Personal":{"owner":1}}]"#)).unwrap();
    }
    AppState::lock(&state.stores)
        .work
        .create_project(access(), "Private", "one")
        .unwrap();
    state
}

#[tokio::test]
async fn policy_runtime_observes_canonical_failure_and_keeps_native_success_on_disk_failure() {
    for canonical_failure in [false, true] {
        let dir = Directory::new();
        let sink = if canonical_failure {
            RuntimeRecordingSink::fail_canonical(PersistErrorCategory::SyncTemporary)
        } else {
            RuntimeRecordingSink::fail_projection(PersistErrorCategory::SyncTemporary)
        };
        let state = state(&dir, sink.clone(), true);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let mut writer = state.attach_service(supervisor.operations());
        let result = state.configure_work_recall(access(), true, 0).await;
        assert_eq!(result.is_err(), canonical_failure);
        if canonical_failure {
            assert_eq!(result, Err(WorkError::Persistence));
        }
        supervisor.next_completion().await;
        let status = state.work_recall_policy(access()).unwrap();
        assert_eq!(status.policy.enabled, !canonical_failure);
        assert!(status.operator_available);
        assert!(AppState::lock(&state.stores).work.recall.records.is_empty());
        assert!(
            AppState::lock(&state.stores)
                .work
                .recall
                .attempts
                .is_empty()
        );
        assert_eq!(
            sink.attempts(),
            if canonical_failure {
                vec!["canonical"]
            } else {
                vec!["canonical", "wdbx"]
            }
        );
        writer.close_admission();
        writer.stop();
        writer.joined().await.unwrap();
    }
}

#[tokio::test]
async fn policy_operator_removal_preserves_inspection_disable_and_no_history_reindex() {
    let dir = Directory::new();
    let sink = RuntimeRecordingSink::success();
    let state = state(&dir, sink.clone(), false);
    AppState::lock(&state.stores)
        .work
        .configure_recall(access(), true, 0)
        .unwrap();
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let snapshot = state.work_recall_policy(access()).unwrap();
    assert!(snapshot.policy.enabled);
    assert!(!snapshot.operator_available);
    assert_eq!(
        state.configure_work_recall(access(), true, 1).await,
        Err(WorkError::Denied)
    );
    supervisor.next_completion().await;
    assert!(sink.attempts().is_empty());
    let disabled = state
        .configure_work_recall(access(), false, 1)
        .await
        .unwrap();
    supervisor.next_completion().await;
    assert!(!disabled.policy.enabled);
    assert_eq!(disabled.policy.revision, 2);
    assert_eq!(disabled.policy.configured_by, 1);
    assert!(AppState::lock(&state.stores).work.recall.records.is_empty());
    assert!(
        AppState::lock(&state.stores)
            .work
            .recall
            .attempts
            .is_empty()
    );
    let restored: crate::persist::Stores =
        serde_json::from_slice(&serde_json::to_vec(&state.final_snapshot().stores).unwrap())
            .unwrap();
    assert_eq!(
        restored.work.recall_policy(access()).unwrap(),
        disabled.policy
    );
    assert_eq!(
        state.work_recall_policy(WorkAccess {
            actor: 2,
            ..access()
        }),
        Err(WorkError::Missing)
    );
    writer.close_admission();
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn policy_duplicate_and_stale_commands_do_not_rewrite_revision() {
    let dir = Directory::new();
    let state = state(&dir, RuntimeRecordingSink::success(), true);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let first = state
        .configure_work_recall(access(), true, 0)
        .await
        .unwrap();
    supervisor.next_completion().await;
    assert_eq!(
        state.configure_work_recall(access(), false, 0).await,
        Err(WorkError::Stale)
    );
    supervisor.next_completion().await;
    assert_eq!(
        state
            .configure_work_recall(access(), true, 1)
            .await
            .unwrap(),
        first
    );
    supervisor.next_completion().await;
    writer.close_admission();
    writer.stop();
    writer.joined().await.unwrap();
}
