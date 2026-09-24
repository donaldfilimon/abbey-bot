use super::*;
use crate::{
    persist::{PersistErrorCategory, PersistenceSink},
    service::{ServiceSupervisor, ShutdownReason},
    work::*,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn access() -> WorkAccess {
    WorkAccess {
        actor: 1,
        guild: None,
        channel: 2,
        can_view: true,
        can_manage: false,
    }
}
#[derive(Default)]
struct Sink {
    writes: Mutex<Vec<Vec<u8>>>,
    canonical: AtomicUsize,
    fail_at: AtomicUsize,
    projection_fail: AtomicUsize,
    hold_at: AtomicUsize,
    entered: tokio::sync::Notify,
    release: (Mutex<bool>, std::sync::Condvar),
}
impl PersistenceSink for Sink {
    fn publish(
        &self,
        _: &std::path::Path,
        target: &std::path::Path,
        bytes: &[u8],
    ) -> Result<(), PersistErrorCategory> {
        let json = target.extension().is_some_and(|s| s == "json");
        if json {
            let n = self.canonical.fetch_add(1, Ordering::SeqCst) + 1;
            if self.hold_at.load(Ordering::SeqCst) == n {
                self.entered.notify_one();
                let (lock, signal) = &self.release;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = signal.wait(released).unwrap();
                }
            }
            if self.fail_at.load(Ordering::SeqCst) == n {
                return Err(PersistErrorCategory::WriteTemporary);
            }
        } else if self.projection_fail.load(Ordering::SeqCst) != 0 {
            return Err(PersistErrorCategory::WriteTemporary);
        }
        self.writes.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }
}
struct FakeGate {
    calls: Mutex<Vec<crate::episode_gate::MemoryCandidateRequest>>,
    outcome: crate::episode_gate::GateOutcome,
    entered: tokio::sync::Notify,
    hold: bool,
    release: tokio::sync::Notify,
    ungated: AtomicUsize,
}
impl FakeGate {
    fn new(hold: bool, outcome: crate::episode_gate::GateOutcome) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            outcome,
            entered: Default::default(),
            hold,
            release: Default::default(),
            ungated: AtomicUsize::new(0),
        })
    }
    fn appended(hold: bool) -> Arc<Self> {
        Self::new(
            hold,
            crate::episode_gate::GateOutcome::Appended {
                digest_hex: "ab".repeat(32),
                sequence: "1".into(),
            },
        )
    }
}
impl Gate for FakeGate {
    fn fingerprint(&self) -> String {
        "cd".repeat(32)
    }
    fn nonce(&self) -> u64 {
        9
    }
    fn ungated_forget(&self) {
        self.ungated.fetch_add(1, Ordering::SeqCst);
    }
    fn propose(
        &self,
        request: crate::episode_gate::MemoryCandidateRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = crate::episode_gate::GateOutcome> + Send + '_>,
    > {
        Box::pin(async move {
            self.calls.lock().unwrap().push(request);
            self.entered.notify_one();
            if self.hold {
                self.release.notified().await;
            }
            self.outcome.clone()
        })
    }
}
async fn fixture() -> (
    Arc<AppState>,
    ServiceSupervisor,
    crate::service::persistence::PersistenceWriter,
    Arc<Sink>,
    WorkSourceKey,
) {
    let sink = Arc::new(Sink::default());
    let state = AppState::in_memory_with_persistence(
        Some(std::env::temp_dir().join("recall-injected")),
        sink.clone(),
    );
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = state.attach_service(supervisor.operations());
    let key = state
        .commit_work(|s| {
            let project = s.create_project(access(), "Work", "one")?;
            let id = s.record_decision(project, access(), "Use rust", 1, "decision")?;
            Ok(WorkSourceKey::Decision { project, id })
        })
        .await
        .unwrap();
    (state, supervisor, writer, sink, key)
}
async fn stop(
    mut supervisor: ServiceSupervisor,
    mut writer: crate::service::persistence::PersistenceWriter,
) {
    supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    while supervisor.try_freeze(writer.idle()).is_err() {
        supervisor.next_completion().await;
    }
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn admitted_exact_scope_query_and_explicit_forget_preserve_native() {
    let (state, supervisor, writer, _, key) = fixture().await;
    assert_eq!(
        state
            .admit_work_recall(key.clone(), access(), 3, &Default::default())
            .await,
        Err(WorkError::Denied)
    );
    let gate = FakeGate::appended(false);
    assert!(matches!(
        state
            .start_work_recall(key.clone(), access(), 3, Some(gate.clone()), None)
            .await
            .unwrap(),
        AdmissionResult::Committed {
            disk_cleanup_required: false,
            ..
        }
    ));
    {
        let calls = gate.calls.lock().unwrap();
        assert_eq!(calls[0].scoped_guild, "discord:dm:1");
        assert!(calls[0].member_scoped);
        assert_eq!(calls[0].nonce, 9);
    }
    let WorkSourceKey::Decision { project, id } = key else {
        unreachable!()
    };
    let result = state.recall_project(access(), project, 1, "rust").unwrap();
    assert_eq!(result.evidence.len(), 1);
    assert!(result.revision > 0);
    assert!(state.recall_project(access(), project, 2, "rust").is_err());
    let status = state
        .forget_work_recall(WorkSourceKey::Decision { project, id }, access(), 4)
        .await
        .unwrap();
    assert_eq!(
        status,
        RecallStatus {
            retained_rows: 0,
            unresolved: 0,
            disk_cleanup_required: false
        }
    );
    assert!(
        AppState::lock(&state.stores)
            .work
            .decisions
            .contains_key(&id)
    );
    assert!(
        state
            .recall_project(access(), project, 1, "rust")
            .unwrap()
            .evidence
            .is_empty()
    );
    stop(supervisor, writer).await;
}
#[tokio::test]
async fn cancelled_caller_and_shutdown_keep_unknown_owner_until_joined() {
    let (state, mut supervisor, writer, _, key) = fixture().await;
    let gate = FakeGate::appended(true);
    let owned = state.clone();
    let g = gate.clone();
    let k = key.clone();
    let caller =
        tokio::spawn(async move { owned.start_work_recall(k, access(), 3, Some(g), None).await });
    gate.entered.notified().await;
    caller.abort();
    supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    supervisor.request_abort();
    assert!(supervisor.try_freeze(writer.idle()).is_err());
    // Existing retained owner settles Unknown even though fresh admission closed.
    while supervisor.try_freeze(writer.idle()).is_err() {
        supervisor.next_completion().await;
    }
    {
        let stores = AppState::lock(&state.stores);
        assert!(stores.work.recall.records.is_empty());
        assert_eq!(
            stores.work.recall.attempts.values().next().unwrap().state,
            AttemptState::Unknown
        );
        let reload: crate::persist::Stores =
            serde_json::from_slice(&serde_json::to_vec(&*stores).unwrap()).unwrap();
        assert_eq!(reload.work.recall.attempts.len(), 1);
    }
    assert_eq!(gate.calls.lock().unwrap().len(), 1);
    let mut writer = writer;
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn reservation_and_settlement_failures_never_publish_or_retry() {
    for failure in [2, 3] {
        let (state, supervisor, writer, sink, key) = fixture().await;
        sink.fail_at.store(failure, Ordering::SeqCst);
        let gate = FakeGate::appended(false);
        let result = state
            .start_work_recall(key, access(), 3, Some(gate.clone()), None)
            .await;
        assert!(AppState::lock(&state.stores).work.recall.records.is_empty());
        if failure == 2 {
            assert_eq!(result, Err(WorkError::Persistence));
            assert!(gate.calls.lock().unwrap().is_empty());
        } else {
            assert_eq!(result, Ok(AdmissionResult::Unknown));
            let stores = AppState::lock(&state.stores);
            let attempt = stores.work.recall.attempts.values().next().unwrap();
            assert!(
                matches!(&attempt.observed_admission, Some(WorkAdmission::Appended { digest_hex, .. }) if digest_hex == &"ab".repeat(32))
            );
            let loaded: crate::persist::Stores =
                serde_json::from_slice(&serde_json::to_vec(&*stores).unwrap()).unwrap();
            assert_eq!(
                loaded.work.recall.attempts[&attempt.id].observed_admission,
                attempt.observed_admission
            );
            drop(stores);
            assert_eq!(gate.calls.lock().unwrap().len(), 1);
        }
        stop(supervisor, writer).await;
    }
}
#[tokio::test]
async fn projection_failure_keeps_canonical_receipt_and_zero_row_cleanup_repairs_disk() {
    let (state, supervisor, writer, sink, key) = fixture().await;
    sink.projection_fail.store(1, Ordering::SeqCst);
    assert!(matches!(
        state
            .start_work_recall(key.clone(), access(), 3, None, None)
            .await
            .unwrap(),
        AdmissionResult::Committed {
            disk_cleanup_required: true,
            ..
        }
    ));
    let status = state
        .forget_work_recall(key.clone(), access(), 4)
        .await
        .unwrap();
    assert_eq!(status.retained_rows, 0);
    assert!(status.disk_cleanup_required);
    sink.projection_fail.store(0, Ordering::SeqCst);
    assert!(
        !state
            .forget_work_recall(key, access(), 5)
            .await
            .unwrap()
            .disk_cleanup_required
    );
    stop(supervisor, writer).await;
}

#[tokio::test]
async fn source_disable_while_gate_pending_retains_original_as_retired() {
    let (state, supervisor, writer, _, key) = fixture().await;
    let gate = FakeGate::appended(true);
    let owned = state.clone();
    let g = gate.clone();
    let k = key.clone();
    let caller =
        tokio::spawn(async move { owned.start_work_recall(k, access(), 3, Some(g), None).await });
    gate.entered.notified().await;
    let k = key.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        state.commit_work(move |s| s.disable_recall_source(&k, access())),
    )
    .await
    .unwrap()
    .unwrap();
    state
        .memory_service()
        .remember("discord:dm:1", "discord:1", "generic concurrent fact", 4)
        .unwrap();
    gate.release.notify_one();
    assert!(matches!(
        caller.await.unwrap().unwrap(),
        AdmissionResult::Committed { .. }
    ));
    let WorkSourceKey::Decision { project, .. } = key else {
        unreachable!()
    };
    assert!(
        state
            .recall_project(access(), project, 1, "rust")
            .unwrap()
            .evidence
            .is_empty()
    );
    assert_eq!(AppState::lock(&state.stores).work.recall.records.len(), 1);
    assert!(
        !state
            .memory_service()
            .recall("discord:dm:1", "discord:1", "generic", 4)
            .is_empty()
    );
    stop(supervisor, writer).await;
}

#[tokio::test]
async fn rejection_drops_reservation_and_unknown_blocks_repeat() {
    for unknown in [false, true] {
        let (state, supervisor, writer, _, key) = fixture().await;
        let gate = FakeGate::new(
            false,
            if unknown {
                crate::episode_gate::GateOutcome::Unavailable {
                    detail: "private diagnostic".into(),
                }
            } else {
                crate::episode_gate::GateOutcome::Rejected {
                    detail: "budget".into(),
                }
            },
        );
        let result = state
            .start_work_recall(key.clone(), access(), 3, Some(gate.clone()), None)
            .await
            .unwrap();
        assert_eq!(
            result,
            if unknown {
                AdmissionResult::Unknown
            } else {
                AdmissionResult::Rejected
            }
        );
        assert!(AppState::lock(&state.stores).work.recall.records.is_empty());
        assert_eq!(
            AppState::lock(&state.stores).work.recall.attempts.len(),
            usize::from(unknown)
        );
        if unknown {
            assert!(
                state
                    .start_work_recall(key, access(), 4, Some(gate.clone()), None)
                    .await
                    .is_err()
            );
            assert_eq!(gate.calls.lock().unwrap().len(), 1);
        }
        stop(supervisor, writer).await;
    }
}

#[tokio::test]
async fn gate_adapter_scope_receipt_policy_uses_existing_builder() {
    let (state, supervisor, writer, _, key) = fixture().await;
    let mut payload = AppState::lock(&state.stores)
        .work
        .recall_candidate(&key, access())
        .unwrap();
    payload.scope = WorkScope::Team {
        guild: 42,
        channel: 500,
    };
    let gate = FakeGate::appended(false);
    let prior = AdmittedWorkEvidence {
        id: 1,
        payload: payload.clone(),
        payload_digest: "a".repeat(64),
        admission: WorkAdmission::Appended {
            digest_hex: "bc".repeat(32),
            scoped_guild: "discord:42".into(),
        },
    };
    assert!(matches!(
        gate::admit(Some(gate.clone()), &payload, Some(&prior), false, 9, 8)
            .await
            .unwrap(),
        Outcome::Admitted(_)
    ));
    gate::admit(Some(gate.clone()), &payload, Some(&prior), true, 10, 9)
        .await
        .unwrap();
    {
        let requests = gate.calls.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].scoped_guild, "discord:42");
        assert!(!requests[0].member_scoped);
        assert_eq!(requests[0].supersedes, Some([0xbc; 32]));
        assert_eq!(requests[1].forgets, Some([0xbc; 32]));
        assert!(requests[1].payload.is_empty());
    }
    let mut uncovered = prior.clone();
    uncovered.admission = WorkAdmission::Uncovered {
        scoped_guild: "discord:42".into(),
    };
    assert_eq!(
        gate::admit(Some(gate.clone()), &payload, Some(&uncovered), true, 11, 10)
            .await
            .unwrap(),
        Outcome::Admitted(uncovered.admission.clone())
    );
    assert_eq!(gate.ungated.load(Ordering::SeqCst), 1);
    assert_eq!(gate.calls.lock().unwrap().len(), 2);
    uncovered.admission = WorkAdmission::Appended {
        digest_hex: "broken".into(),
        scoped_guild: "discord:42".into(),
    };
    assert_eq!(
        gate::admit(None, &payload, Some(&uncovered), true, 12, 11).await,
        Err(WorkError::Invalid)
    );
    stop(supervisor, writer).await;
}

#[tokio::test]
async fn generic_writer_and_loaded_disk_debt_use_actual_projection_component() {
    let (state, supervisor, writer, sink, key) = fixture().await;
    state
        .start_work_recall(key.clone(), access(), 3, None, None)
        .await
        .unwrap();
    let canonical = AppState::lock(&state.stores).clone();
    let mut missing = crate::wdbx::Recall::new();
    assert_eq!(
        AppState::restore_work_projection(&canonical, &mut missing),
        None
    );
    assert!(
        missing
            .work_projection_current(&canonical.work.recall)
            .unwrap()
    );
    assert_eq!(
        AppState::restore_work_projection(&canonical, &mut missing),
        Some(canonical.work.recall.projection_revision)
    );
    // Ordinary retained generic persistence also writes this canonical namespace.
    sink.projection_fail.store(1, Ordering::SeqCst);
    let report = state.request_persistence().await.unwrap();
    assert_eq!(
        report.canonical_state,
        crate::persist::PersistComponentOutcome::Committed
    );
    assert!(
        state
            .work_recall_status(&key, access())
            .unwrap()
            .disk_cleanup_required
    );
    sink.projection_fail.store(0, Ordering::SeqCst);
    state.request_persistence().await.unwrap();
    assert!(
        !state
            .work_recall_status(&key, access())
            .unwrap()
            .disk_cleanup_required
    );
    // A stale successful report may not clear newer canonical debt.
    state.observe_work_projection(
        0,
        crate::persist::PersistReport::from_components(
            crate::persist::PersistComponentOutcome::Committed,
            crate::persist::PersistComponentOutcome::Committed,
        ),
    );
    assert!(
        state
            .work_recall_status(&key, access())
            .unwrap()
            .disk_cleanup_required
    );
    stop(supervisor, writer).await;
}

#[tokio::test]
async fn bounded_thousand_row_query_measurement() {
    let state = AppState::in_memory_with_persistence(
        Some(std::env::temp_dir().join("measure-recall-injected")),
        Arc::new(Sink::default()),
    );
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = state.attach_service(supervisor.operations());
    let project = {
        let mut stores = AppState::lock(&state.stores);
        let project = stores
            .work
            .create_project(access(), "Measure", "project")
            .unwrap();
        for n in 0..1000 {
            let id = stores
                .work
                .record_decision(
                    project,
                    access(),
                    "Use rust for reliable memory services",
                    1,
                    &format!("d{n}"),
                )
                .unwrap();
            let key = WorkSourceKey::Decision { project, id };
            let (attempt, payload) = stores
                .work
                .prepare_recall(&key, access(), 2, n, "a".repeat(64))
                .unwrap();
            stores
                .work
                .recall
                .settle_add(
                    attempt,
                    payload,
                    WorkAdmission::Uncovered {
                        scoped_guild: "discord:dm:1".into(),
                    },
                    2,
                )
                .unwrap();
        }
        AppState::lock(&state.recall)
            .reconcile_work_evidence(&stores.work.recall)
            .unwrap();
        let start = std::time::Instant::now();
        AppState::lock(&state.recall)
            .reconcile_work_evidence(&stores.work.recall)
            .unwrap();
        println!(
            "1000 populated unchanged reconciliation debug: {:?}",
            start.elapsed()
        );
        let start = std::time::Instant::now();
        assert!(
            AppState::lock(&state.recall)
                .work_projection_current(&stores.work.recall)
                .unwrap()
        );
        println!(
            "1000 populated startup currency debug: {:?}",
            start.elapsed()
        );
        project
    };
    let start = std::time::Instant::now();
    let result = state
        .recall_project(access(), project, 1, "reliable rust")
        .unwrap();
    let elapsed = start.elapsed();
    println!(
        "1000 eligible rows, unoptimized test profile, first query: {elapsed:?}; returned {}",
        result.evidence.len()
    );
    let mut samples = vec![elapsed];
    for _ in 0..4 {
        let start = std::time::Instant::now();
        assert_eq!(
            state
                .recall_project(access(), project, 1, "reliable rust")
                .unwrap()
                .evidence
                .len(),
            8
        );
        samples.push(start.elapsed());
    }
    samples.sort();
    println!(
        "1000-row debug query samples (n=5, sorted): {samples:?}; nearest-rank p95: {:?}",
        samples[4]
    );
    assert_eq!(result.evidence.len(), 8);
    assert!(
        result
            .evidence
            .iter()
            .map(|p| p.text.chars().count())
            .sum::<usize>()
            <= 4000
    );
    let start = std::time::Instant::now();
    state.commit_work(|_| Ok(())).await.unwrap();
    println!(
        "1000 populated unchanged native commit debug (injected sink, includes actor plus live reconciliation): {:?}",
        start.elapsed()
    );
    stop(supervisor, writer).await;
}

#[tokio::test]
async fn settlement_pending_survives_abort_and_prevents_freeze() {
    let (state, mut supervisor, mut writer, sink, key) = fixture().await;
    sink.hold_at.store(3, Ordering::SeqCst);
    let owned = state.clone();
    let caller = tokio::spawn(async move {
        owned
            .start_work_recall(key, access(), 3, Some(FakeGate::appended(false)), None)
            .await
    });
    sink.entered.notified().await;
    supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    supervisor.request_abort();
    caller.abort();
    assert!(supervisor.try_freeze(writer.idle()).is_err());
    assert!(AppState::lock(&state.stores).work.recall.records.is_empty());
    *sink.release.0.lock().unwrap() = true;
    sink.release.1.notify_all();
    while supervisor.try_freeze(writer.idle()).is_err() {
        supervisor.next_completion().await;
    }
    assert_eq!(AppState::lock(&state.stores).work.recall.records.len(), 1);
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn native_filesystem_commit_and_restart_preserve_prepared_without_reproposal() {
    let dir = std::env::temp_dir().join(format!(
        "abbey-work-recall-{}-{}",
        std::process::id(),
        crate::runtime::now()
    ));
    let state = AppState::in_memory_with_persistence(
        Some(dir.clone()),
        Arc::new(crate::persist::FsPersistenceSink),
    );
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = state.attach_service(supervisor.operations());
    let key = state
        .commit_work(|s| {
            let project = s.create_project(access(), "Disk", "disk")?;
            let id = s.record_decision(
                project,
                access(),
                "Retain native authority",
                1,
                "disk-decision",
            )?;
            Ok(WorkSourceKey::Decision { project, id })
        })
        .await
        .unwrap();
    let k = key.clone();
    state
        .commit_work(move |s| s.prepare_recall(&k, access(), 2, 7, "a".repeat(64)))
        .await
        .unwrap();
    let loaded = crate::persist::Stores::load(&dir).unwrap();
    assert_eq!(
        loaded.work.recall.attempts.values().next().unwrap().state,
        AttemptState::Unknown
    );
    assert!(loaded.work.recall.records.is_empty());
    let mut disk = crate::wdbx::Recall::load(&crate::persist::Stores::wdbx_path(&dir)).unwrap();
    assert_eq!(
        AppState::restore_work_projection(&loaded, &mut disk),
        Some(loaded.work.recall.projection_revision)
    );
    assert_eq!(loaded.work.decisions.len(), 1);
    stop(supervisor, writer).await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
mod selected_gate;
