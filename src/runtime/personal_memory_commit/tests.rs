use super::*;
use crate::persist::{FsPersistenceSink, PersistErrorCategory, PersistenceSink};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "abbey-personal-tx-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create personal-memory fixture directory: {error}"),
            }
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn action(state: &AppState, id: &str) -> SelfAuthorizedFactAction {
    SelfAuthorizedFactAction::new(
        MemberProof {
            actor: "u".into(),
            subject: "u".into(),
            guild: "g".into(),
            interaction_id: id.into(),
            platform: "discord".into(),
            at: crate::runtime::now(),
            policy_version: 1,
        },
        state.personal_memory_status("g", "u").stamp,
    )
    .unwrap()
}
fn state(
    dir: &Directory,
    sink: Arc<dyn PersistenceSink>,
) -> (
    Arc<AppState>,
    crate::service::persistence::PersistenceWriter,
) {
    let state = AppState::in_memory_with_persistence(Some(dir.0.clone()), sink);
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = state.attach_service(supervisor.operations());
    (state, writer)
}
#[tokio::test]
async fn typed_fact_choice_and_exact_retry() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    let a = action(&state, "remember");
    let first = state
        .remember_personal_memory_fact(a.clone(), "uses rust".into(), "r1".into(), None)
        .await
        .unwrap();
    assert_eq!(first.choice, UseChoice::Off);
    let retry = state
        .remember_personal_memory_fact(a.clone(), "uses rust".into(), "r1".into(), None)
        .await
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(
        state
            .remember_personal_memory_fact(a, "different".into(), "r1".into(), None)
            .await,
        Err(MemoryConsentError::RequestConflict)
    );
    state
        .set_personal_memory_use(action(&state, "on"), UseChoice::On, "r2".into())
        .await
        .unwrap();
    state
        .correct_personal_memory_fact(
            action(&state, "correct"),
            "uses rust".into(),
            "uses zig".into(),
            "r3".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(state.personal_memory_status("g", "u").choice, UseChoice::On);
    state
        .forget_personal_memory_fact(action(&state, "forget"), "uses zig".into(), "r4".into())
        .await
        .unwrap();
    assert_eq!(state.personal_memory_status("g", "u").choice, UseChoice::On);
    assert!(state.memory_service().facts("g", "u").is_empty());
    assert_eq!(
        state
            .forget_personal_memory_fact(action(&state, "missing"), "missing".into(), "r5".into())
            .await,
        Err(MemoryConsentError::NotFound)
    );
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn refused_actions_leave_fact_and_receipt_unchanged() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    let mut expired = action(&state, "expired");
    expired.proof.at -= 301;
    assert_eq!(
        state
            .remember_personal_memory_fact(
                expired,
                "refused".into(),
                "r1".into(),
                Some("a".repeat(64))
            )
            .await,
        Err(MemoryConsentError::InvalidProof)
    );
    assert_eq!(
        state
            .remember_personal_memory_fact(
                action(&state, "bad-id"),
                "refused".into(),
                "".into(),
                None
            )
            .await,
        Err(MemoryConsentError::Bounds)
    );
    assert!(state.memory_service().facts("g", "u").is_empty());
    assert!(AppState::lock(&state.stores).memory_receipts.is_empty());
    writer.stop();
    writer.joined().await.unwrap();
}
struct FailProjection;
impl PersistenceSink for FailProjection {
    fn publish(&self, dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        if path == persist::Stores::wdbx_path(dir) {
            Err(PersistErrorCategory::SyncTemporary)
        } else {
            FsPersistenceSink.publish(dir, path, bytes)
        }
    }
}
#[tokio::test]
async fn failed_on_restart_is_off() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FailProjection));
    assert_eq!(
        state
            .set_personal_memory_use(action(&state, "on"), UseChoice::On, "r1".into())
            .await,
        Err(MemoryConsentError::Persistence)
    );
    writer.stop();
    writer.joined().await.unwrap();
    let mut loaded = persist::Stores::load(&dir.0).unwrap();
    journal::recover(&mut loaded, &dir.0).unwrap();
    let s = &loaded.personal_memory[&subject_key("g", "u")];
    assert_eq!(s.choice, UseChoice::Off);
    assert!(!s.activation_pending);
    assert!(!s.outcomes.contains_key("r1"));
}
struct HeldSink {
    entered: tokio::sync::Notify,
    release: (Mutex<bool>, Condvar),
}
struct Release(Arc<HeldSink>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
impl HeldSink {
    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}
impl PersistenceSink for HeldSink {
    fn publish(&self, dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        if path == persist::Stores::state_path(dir) {
            self.entered.notify_one();
            let mut ready = self.release.0.lock().unwrap();
            while !*ready {
                ready = self.release.1.wait(ready).unwrap();
            }
        }
        FsPersistenceSink.publish(dir, path, bytes)
    }
}
#[tokio::test]
async fn cancelled_waiter_keeps_complete_transaction_and_inspection_responsive() {
    let dir = Directory::new();
    let sink = Arc::new(HeldSink {
        entered: tokio::sync::Notify::new(),
        release: (Mutex::new(false), Condvar::new()),
    });
    let _release = Release(sink.clone());
    let (state, mut writer) = state(&dir, sink.clone());
    let a = action(&state, "remember");
    let retry_action = a.clone();
    let owned = state.clone();
    let waiter = tokio::spawn(async move {
        owned
            .remember_personal_memory_fact(a, "retained".into(), "r1".into(), None)
            .await
    });
    sink.entered.notified().await;
    assert!(state.memory_service().facts("g", "u").is_empty());
    assert_eq!(
        state.personal_memory_status("g", "u").choice,
        UseChoice::Off
    );
    waiter.abort();
    sink.release();
    let completed = state
        .remember_personal_memory_fact(retry_action, "retained".into(), "r1".into(), None)
        .await
        .unwrap();
    assert_eq!(completed.choice, UseChoice::Off);
    assert_eq!(
        persist::Stores::load(&dir.0)
            .unwrap()
            .memory
            .facts("g", "u"),
        ["retained"]
    );
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn stale_correction_preserves_newer_fact() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    state
        .remember_personal_memory_fact(action(&state, "initial"), "old".into(), "r1".into(), None)
        .await
        .unwrap();
    let stale = action(&state, "stale");
    state
        .correct_personal_memory_fact(
            action(&state, "fresh"),
            "old".into(),
            "new".into(),
            "r2".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        state
            .correct_personal_memory_fact(stale, "old".into(), "wrong".into(), "r3".into(), None)
            .await,
        Err(MemoryConsentError::Stale)
    );
    assert_eq!(state.memory_service().facts("g", "u"), ["new"]);
    writer.stop();
    writer.joined().await.unwrap();
}

struct FailCanonical;
impl PersistenceSink for FailCanonical {
    fn publish(
        &self,
        _dir: &Path,
        _path: &Path,
        _bytes: &[u8],
    ) -> Result<(), PersistErrorCategory> {
        Err(PersistErrorCategory::SyncTemporary)
    }
}
#[tokio::test]
async fn canonical_failure_never_publishes_fact_or_proof() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FailCanonical));
    assert_eq!(
        state
            .remember_personal_memory_fact(
                action(&state, "remember"),
                "refused".into(),
                "r1".into(),
                Some("a".repeat(64))
            )
            .await,
        Err(MemoryConsentError::Persistence)
    );
    assert!(state.memory_service().facts("g", "u").is_empty());
    assert!(AppState::lock(&state.stores).personal_memory.is_empty());
    assert!(AppState::lock(&state.stores).memory_receipts.is_empty());
    writer.stop();
    writer.joined().await.unwrap();
}
struct WrongChoice;
impl PersistenceSink for WrongChoice {
    fn publish(&self, dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        if path == persist::Stores::state_path(dir) {
            let mut decoded: persist::Stores = serde_json::from_slice(bytes).unwrap();
            if let Some(s) = decoded.personal_memory.get_mut(&subject_key("g", "u")) {
                s.choice = UseChoice::Off;
            }
            FsPersistenceSink.publish(dir, path, &serde_json::to_vec(&decoded).unwrap())
        } else {
            FsPersistenceSink.publish(dir, path, bytes)
        }
    }
}
#[tokio::test]
async fn equal_stamp_wrong_choice_fails_readback_and_restart_closes() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(WrongChoice));
    assert_eq!(
        state
            .set_personal_memory_use(action(&state, "on"), UseChoice::On, "r1".into())
            .await,
        Err(MemoryConsentError::Persistence)
    );
    assert!(!journal::load(&dir.0).unwrap().activations.is_empty());
    let mut restored = persist::Stores::load(&dir.0).unwrap();
    journal::recover(&mut restored, &dir.0).unwrap();
    assert_eq!(
        restored.personal_memory[&subject_key("g", "u")].choice,
        UseChoice::Off
    );
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn immediate_withdrawal_rejects_an_inflight_old_on() {
    let dir = Directory::new();
    let sink = Arc::new(HeldSink {
        entered: tokio::sync::Notify::new(),
        release: (Mutex::new(false), Condvar::new()),
    });
    let _release = Release(sink.clone());
    let (state, mut writer) = state(&dir, sink.clone());
    let old = action(&state, "on");
    let owned = state.clone();
    let on = tokio::spawn(async move {
        owned
            .set_personal_memory_use(old, UseChoice::On, "r1".into())
            .await
    });
    sink.entered.notified().await;
    let off_action = action(&state, "off");
    let owned = state.clone();
    let off = tokio::spawn(async move {
        owned
            .set_personal_memory_use(off_action, UseChoice::Off, "r2".into())
            .await
    });
    for _ in 0..1000 {
        if state.personal_memory_exposure_epoch() > 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(state.personal_memory_exposure_epoch() > 0);
    assert_eq!(
        state.personal_memory_status("g", "u").choice,
        UseChoice::Off
    );
    sink.release();
    assert_eq!(on.await.unwrap(), Err(MemoryConsentError::Stale));
    let result = off.await.unwrap().unwrap();
    assert_eq!(result.choice, UseChoice::Off);
    assert_eq!(
        persist::Stores::load(&dir.0).unwrap().personal_memory[&subject_key("g", "u")].choice,
        UseChoice::Off
    );
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn retry_after_restart_recovers_same_result_without_epoch_change() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    let a = action(&state, "remember");
    let first = state
        .remember_personal_memory_fact(a.clone(), "retained".into(), "r1".into(), None)
        .await
        .unwrap();
    writer.stop();
    writer.joined().await.unwrap();
    let (restored, mut writer) = self::state(&dir, Arc::new(FsPersistenceSink));
    *AppState::lock(&restored.stores) = persist::Stores::load(&dir.0).unwrap();
    let epoch = restored.personal_memory_exposure_epoch();
    let retry = restored
        .remember_personal_memory_fact(a, "retained".into(), "r1".into(), None)
        .await
        .unwrap();
    assert_eq!(retry, first);
    assert_eq!(restored.personal_memory_exposure_epoch(), epoch);
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn concurrent_identical_withdrawals_complete_once() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    let a = action(&state, "off");
    let first = state.set_personal_memory_use(a.clone(), UseChoice::Off, "r1".into());
    let second = state.set_personal_memory_use(a, UseChoice::Off, "r1".into());
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.as_ref().unwrap(), second.as_ref().unwrap());
    let store = persist::Stores::load(&dir.0).unwrap();
    assert_eq!(
        store.personal_memory[&subject_key("g", "u")].outcomes.len(),
        1
    );
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn capacity_withdrawal_fences_without_inserting_subject() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    {
        let mut stores = AppState::lock(&state.stores);
        for index in 0..MAX_SUBJECTS {
            stores.personal_memory.insert(
                subject_key("g", &format!("other{index}")),
                PersonalMemorySubject::default(),
            );
        }
    }
    let a = action(&state, "off");
    assert_eq!(
        state
            .set_personal_memory_use(a, UseChoice::Off, "r1".into())
            .await,
        Err(MemoryConsentError::Bounds)
    );
    {
        let stores = AppState::lock(&state.stores);
        assert_eq!(stores.personal_memory.len(), MAX_SUBJECTS);
        assert!(!stores.personal_memory.contains_key(&subject_key("g", "u")));
        assert!(state.personal_memory_blocked.load(Ordering::Acquire));
    }
    writer.stop();
    writer.joined().await.unwrap();
}

struct FailProjectionOnce(std::sync::atomic::AtomicBool);
impl PersistenceSink for FailProjectionOnce {
    fn publish(&self, dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        if path == persist::Stores::wdbx_path(dir) && self.0.swap(false, Ordering::AcqRel) {
            Err(PersistErrorCategory::SyncTemporary)
        } else {
            FsPersistenceSink.publish(dir, path, bytes)
        }
    }
}
#[tokio::test]
async fn provisional_remember_and_forget_retry_repairs_all_surfaces() {
    let dir = Directory::new();
    let sink = Arc::new(FailProjectionOnce(std::sync::atomic::AtomicBool::new(true)));
    let (state, mut writer) = state(&dir, sink.clone());
    let remember = action(&state, "remember-retry");
    assert_eq!(
        state
            .remember_personal_memory_fact(
                remember.clone(),
                "retained".into(),
                "remember-retry".into(),
                None
            )
            .await,
        Err(MemoryConsentError::Persistence)
    );
    assert!(state.memory_service().facts("g", "u").is_empty());
    let disk = persist::Stores::load(&dir.0).unwrap();
    assert!(!disk.personal_memory[&subject_key("g", "u")].outcomes["remember-retry"].completed);
    state
        .remember_personal_memory_fact(remember, "retained".into(), "remember-retry".into(), None)
        .await
        .unwrap();
    assert_eq!(state.memory_service().facts("g", "u"), ["retained"]);
    assert!(!state.personal_memory_blocked.load(Ordering::Acquire));
    sink.0.store(true, Ordering::Release);
    let forget = action(&state, "forget-retry");
    assert_eq!(
        state
            .forget_personal_memory_fact(forget.clone(), "retained".into(), "forget-retry".into())
            .await,
        Err(MemoryConsentError::Persistence)
    );
    assert_eq!(state.memory_service().facts("g", "u"), ["retained"]);
    state
        .forget_personal_memory_fact(forget, "retained".into(), "forget-retry".into())
        .await
        .unwrap();
    assert!(state.memory_service().facts("g", "u").is_empty());
    let disk = persist::Stores::load(&dir.0).unwrap();
    assert!(disk.memory.facts("g", "u").is_empty());
    assert!(disk.personal_memory[&subject_key("g", "u")].outcomes["forget-retry"].completed);
    let projection = crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0)).unwrap();
    assert!(projection.all_memory_facts().is_empty());
    assert!(!state.personal_memory_blocked.load(Ordering::Acquire));
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn failed_off_original_stamp_retry_retires_durable_denial() {
    let dir = Directory::new();
    let sink = Arc::new(FailProjectionOnce(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let (state, mut writer) = state(&dir, sink.clone());
    state
        .set_personal_memory_use(
            action(&state, "initial-on"),
            UseChoice::On,
            "initial-on".into(),
        )
        .await
        .unwrap();
    let off = action(&state, "off-retry");
    sink.0.store(true, Ordering::Release);
    assert_eq!(
        state
            .set_personal_memory_use(off.clone(), UseChoice::Off, "off-retry".into())
            .await,
        Err(MemoryConsentError::Persistence)
    );
    assert_ne!(state.personal_memory_status("g", "u").stamp, off.expected);
    state
        .set_personal_memory_use(off, UseChoice::Off, "off-retry".into())
        .await
        .unwrap();
    assert_eq!(
        state.personal_memory_status("g", "u").choice,
        UseChoice::Off
    );
    assert!(journal::load(&dir.0).unwrap().withdrawals.is_empty());
    assert!(!state.personal_memory_blocked.load(Ordering::Acquire));
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn stale_higher_exposure_cannot_replace_cooperating_writer_facts() {
    let dir = Directory::new();
    let (a, mut wa) = state(&dir, Arc::new(FsPersistenceSink));
    let (b, mut wb) = state(&dir, Arc::new(FsPersistenceSink));
    a.memory_service()
        .remember("g", "other", "newer disk fact", 1)
        .unwrap();
    let a_snapshot = (
        AppState::lock(&a.stores).clone(),
        AppState::lock(&a.recall).clone(),
    );
    assert_eq!(
        a.persist_snapshot(a_snapshot).canonical_state,
        persist::PersistComponentOutcome::Committed
    );
    b.memory_service()
        .remember("g", "u", "local one", 2)
        .unwrap();
    b.memory_service()
        .remember("g", "u", "local two", 3)
        .unwrap();
    let b_snapshot = (
        AppState::lock(&b.stores).clone(),
        AppState::lock(&b.recall).clone(),
    );
    assert!(matches!(
        b.persist_snapshot(b_snapshot).canonical_state,
        persist::PersistComponentOutcome::Failed(_)
    ));
    assert_eq!(
        b.set_personal_memory_use(action(&b, "stale-off"), UseChoice::Off, "stale-off".into())
            .await,
        Err(MemoryConsentError::Stale)
    );
    assert_eq!(
        persist::Stores::load(&dir.0)
            .unwrap()
            .memory
            .facts("g", "other"),
        ["newer disk fact"]
    );
    wa.stop();
    wb.stop();
    wa.joined().await.unwrap();
    wb.joined().await.unwrap();
}
#[tokio::test]
async fn stronger_existing_marker_minima_survive_new_off_and_restart() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    state
        .set_personal_memory_use(action(&state, "on-base"), UseChoice::On, "on-base".into())
        .await
        .unwrap();
    let minimum_cutoff = crate::runtime::now();
    let mut marker = journal::load(&dir.0).unwrap();
    journal::add_withdrawal(
        &mut marker,
        "g",
        "u",
        journal::Withdrawal {
            guild: "g".into(),
            user: "u".into(),
            epoch: 80,
            minimum_revision: 90,
            exposure_epoch: 100,
            cutoff: crate::runtime::now(),
            request_id: "older-off".into(),
            payload_digest: "a".repeat(64),
            at: crate::runtime::now(),
        },
    )
    .unwrap();
    journal::publish(&dir.0, marker.revision, marker).unwrap();
    state
        .set_personal_memory_use(action(&state, "new-off"), UseChoice::Off, "new-off".into())
        .await
        .unwrap();
    assert!(journal::load(&dir.0).unwrap().withdrawals.is_empty());
    let mut disk = persist::Stores::load(&dir.0).unwrap();
    let s = &disk.personal_memory[&subject_key("g", "u")];
    assert!(s.revision >= 90 && s.consent_epoch >= 80);
    assert!(disk.personal_memory_exposure.epoch >= 100);
    assert!(disk.personal_memory_exposure.cutoff >= minimum_cutoff);
    journal::recover(&mut disk, &dir.0).unwrap();
    assert!(disk.personal_memory[&subject_key("g", "u")].revision >= 90);
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn provisional_forget_restart_qualifies_before_use_and_retry() {
    let dir = Directory::new();
    let sink = Arc::new(FailProjectionOnce(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let (first, mut writer) = state(&dir, sink.clone());
    first
        .remember_personal_memory_fact(action(&first, "seed"), "erase".into(), "seed".into(), None)
        .await
        .unwrap();
    let forget = action(&first, "restart-forget");
    sink.0.store(true, Ordering::Release);
    assert_eq!(
        first
            .forget_personal_memory_fact(forget.clone(), "erase".into(), "restart-forget".into())
            .await,
        Err(MemoryConsentError::Persistence)
    );
    writer.stop();
    writer.joined().await.unwrap();
    let mut recovered = persist::Stores::load(&dir.0).unwrap();
    journal::recover(&mut recovered, &dir.0).unwrap();
    assert!(recovered.memory.facts("g", "u").is_empty());
    assert!(recovered.personal_memory[&subject_key("g", "u")].outcomes["restart-forget"].completed);
    let (restarted, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    *AppState::lock(&restarted.stores) = recovered;
    restarted
        .forget_personal_memory_fact(forget, "erase".into(), "restart-forget".into())
        .await
        .unwrap();
    assert!(restarted.memory_service().facts("g", "u").is_empty());
    assert!(
        crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0))
            .unwrap()
            .all_memory_facts()
            .is_empty()
    );
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn provisional_remember_restart_repairs_projection_and_completed_receipt() {
    let dir = Directory::new();
    let (first, mut writer) = state(
        &dir,
        Arc::new(FailProjectionOnce(std::sync::atomic::AtomicBool::new(true))),
    );
    let remember = action(&first, "restart-remember");
    assert_eq!(
        first
            .remember_personal_memory_fact(
                remember.clone(),
                "durable fact".into(),
                "restart-remember".into(),
                None
            )
            .await,
        Err(MemoryConsentError::Persistence)
    );
    writer.stop();
    writer.joined().await.unwrap();
    let mut recovered = persist::Stores::load(&dir.0).unwrap();
    assert!(
        !recovered.personal_memory[&subject_key("g", "u")].outcomes["restart-remember"].completed
    );
    journal::recover(&mut recovered, &dir.0).unwrap();
    assert_eq!(recovered.memory.facts("g", "u"), ["durable fact"]);
    assert!(
        recovered.personal_memory[&subject_key("g", "u")].outcomes["restart-remember"].completed
    );
    let (restarted, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    *AppState::lock(&restarted.stores) = recovered;
    *AppState::lock(&restarted.recall) =
        crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0)).unwrap();
    restarted
        .remember_personal_memory_fact(
            remember,
            "durable fact".into(),
            "restart-remember".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(restarted.memory_service().facts("g", "u"), ["durable fact"]);
    assert_eq!(
        AppState::lock(&restarted.recall).all_memory_facts().len(),
        1
    );
    writer.stop();
    writer.joined().await.unwrap();
}

struct ReplaySink {
    fail_projection: std::sync::atomic::AtomicBool,
    hold_projection: std::sync::atomic::AtomicBool,
    held: Arc<HeldSink>,
}
impl PersistenceSink for ReplaySink {
    fn publish(&self, dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        if path == persist::Stores::wdbx_path(dir) {
            if self.fail_projection.swap(false, Ordering::AcqRel) {
                return Err(PersistErrorCategory::SyncTemporary);
            }
            if self.hold_projection.swap(false, Ordering::AcqRel) {
                self.held.entered.notify_one();
                let (lock, wake) = &self.held.release;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
            }
        }
        FsPersistenceSink.publish(dir, path, bytes)
    }
}
#[tokio::test]
async fn replay_never_undoes_same_or_other_subject_immediate_off() {
    for withdrawn in ["u", "other"] {
        let dir = Directory::new();
        let sink = Arc::new(ReplaySink {
            fail_projection: std::sync::atomic::AtomicBool::new(false),
            hold_projection: std::sync::atomic::AtomicBool::new(false),
            held: Arc::new(HeldSink {
                entered: tokio::sync::Notify::new(),
                release: (Mutex::new(false), Condvar::new()),
            }),
        });
        let _release = Release(sink.held.clone());
        let (state, mut writer) = state(&dir, sink.clone());
        state
            .remember_personal_memory_fact(
                action(&state, "seed"),
                "seed fact".into(),
                "seed".into(),
                None,
            )
            .await
            .unwrap();
        state
            .set_personal_memory_use(action(&state, "on"), UseChoice::On, "on".into())
            .await
            .unwrap();
        let remembered = action(&state, "held-retry");
        sink.fail_projection.store(true, Ordering::Release);
        assert_eq!(
            state
                .remember_personal_memory_fact(
                    remembered.clone(),
                    "new retained fact".into(),
                    "held-retry".into(),
                    None
                )
                .await,
            Err(MemoryConsentError::Persistence)
        );
        sink.hold_projection.store(true, Ordering::Release);
        let owned = state.clone();
        let replay = tokio::spawn(async move {
            owned
                .remember_personal_memory_fact(
                    remembered,
                    "new retained fact".into(),
                    "held-retry".into(),
                    None,
                )
                .await
        });
        sink.held.entered.notified().await;
        let off = SelfAuthorizedFactAction::new(
            MemberProof {
                actor: withdrawn.into(),
                subject: withdrawn.into(),
                guild: "g".into(),
                interaction_id: "withdraw-during-replay".into(),
                platform: "discord".into(),
                at: crate::runtime::now(),
                policy_version: 1,
            },
            state.personal_memory_status("g", withdrawn).stamp,
        )
        .unwrap();
        let before = state.personal_memory_exposure_epoch();
        let owned = state.clone();
        let withdrawal = tokio::spawn(async move {
            owned
                .set_personal_memory_use(off, UseChoice::Off, "withdraw-during-replay".into())
                .await
        });
        for _ in 0..1000 {
            if state.personal_memory_exposure_epoch() > before {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(state.personal_memory_exposure_epoch() > before);
        let admitted_epoch = state.personal_memory_exposure_epoch();
        assert_eq!(
            state.personal_memory_status("g", withdrawn).choice,
            UseChoice::Off
        );
        assert!(state.personal_memory_blocked.load(Ordering::Acquire));
        sink.held.release();
        assert_eq!(replay.await.unwrap(), Err(MemoryConsentError::Stale));
        withdrawal.await.unwrap().unwrap();
        assert_eq!(
            state.personal_memory_status("g", withdrawn).choice,
            UseChoice::Off
        );
        assert_eq!(
            state.personal_memory_status("g", withdrawn).eligible_facts,
            0
        );
        assert!(state.personal_memory_exposure_epoch() >= admitted_epoch);
        assert_eq!(
            state.memory_service().facts("g", "u"),
            ["seed fact", "new retained fact"]
        );
        let mut disk = persist::Stores::load(&dir.0).unwrap();
        journal::recover(&mut disk, &dir.0).unwrap();
        assert_eq!(
            disk.personal_memory[&subject_key("g", withdrawn)].choice,
            UseChoice::Off
        );
        assert!(disk.personal_memory_exposure.epoch >= admitted_epoch);
        assert!(journal::load(&dir.0).unwrap().withdrawals.is_empty());
        let snapshot = (
            AppState::lock(&state.stores).clone(),
            AppState::lock(&state.recall).clone(),
        );
        assert_eq!(
            state.persist_snapshot(snapshot).canonical_state,
            persist::PersistComponentOutcome::Committed
        );
        writer.stop();
        writer.joined().await.unwrap();
    }
}

#[tokio::test]
async fn sequential_real_filesystem_fifo_and_runtime_flushes_keep_lineage() {
    let dir = Directory::new();
    let (state, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
    for fact in ["first flush", "second flush"] {
        state
            .memory_service()
            .remember("g", "u", fact, crate::runtime::now())
            .unwrap();
        assert_eq!(
            state.request_persistence().await.unwrap().canonical_state,
            persist::PersistComponentOutcome::Committed
        );
        assert!(writer.last_completed().is_some());
        assert!(
            persist::Stores::load(&dir.0)
                .unwrap()
                .memory
                .facts("g", "u")
                .contains(&fact.to_owned())
        );
    }
    assert_eq!(
        state.persist_all_gated().await.canonical_state,
        persist::PersistComponentOutcome::Committed
    );
    assert_eq!(
        state.persist_all().canonical_state,
        persist::PersistComponentOutcome::Committed
    );
    assert_eq!(
        persist::Stores::load(&dir.0)
            .unwrap()
            .memory
            .facts("g", "u"),
        ["first flush", "second flush"]
    );
    writer.stop();
    writer.joined().await.unwrap();
}

mod publication_failures;
mod round4;
