use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    FirstCanonical,
    Projection,
    FinalCanonical,
    MarkerRetirement,
}
const PHASES: [Phase; 4] = [
    Phase::FirstCanonical,
    Phase::Projection,
    Phase::FinalCanonical,
    Phase::MarkerRetirement,
];
#[derive(Default)]
struct Fault {
    phase: Option<Phase>,
    canonical_calls: usize,
    hit: bool,
    marker_obstruction: Option<PathBuf>,
}
#[derive(Default)]
struct PhaseSink(Mutex<Fault>);
impl PhaseSink {
    fn arm(&self, phase: Phase) {
        let mut fault = self.0.lock().unwrap();
        assert!(fault.marker_obstruction.is_none());
        *fault = Fault {
            phase: Some(phase),
            ..Default::default()
        };
    }
    fn clear(&self) {
        let mut fault = self.0.lock().unwrap();
        assert!(fault.hit, "requested publication failure was not exercised");
        if let Some(path) = fault.marker_obstruction.take() {
            fs::remove_dir(path).unwrap();
        }
        fault.phase = None;
    }
}
impl PersistenceSink for PhaseSink {
    fn publish(&self, dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        let mut fault = self.0.lock().unwrap();
        if path == persist::Stores::state_path(dir) {
            fault.canonical_calls += 1;
        }
        let fail = match fault.phase {
            Some(Phase::FirstCanonical) => {
                path == persist::Stores::state_path(dir) && fault.canonical_calls == 1
            }
            Some(Phase::Projection) => path == persist::Stores::wdbx_path(dir),
            Some(Phase::FinalCanonical) => {
                path == persist::Stores::state_path(dir) && fault.canonical_calls == 2
            }
            _ => false,
        };
        if fail {
            fault.phase = None;
            fault.hit = true;
            return Err(PersistErrorCategory::SyncTemporary);
        }
        FsPersistenceSink.publish(dir, path, bytes)?;
        if fault.phase == Some(Phase::MarkerRetirement)
            && path == persist::Stores::state_path(dir)
            && fault.canonical_calls == 2
        {
            // Occupy the next journal temporary path after the final canonical
            // publication. The real create_new retirement write must fail;
            // the existing durable withdrawal remains untouched.
            let marker = journal::load(dir).unwrap();
            assert!(!marker.withdrawals.is_empty());
            let temporary = dir.join(format!(
                ".personal-memory-withdrawals.json.{}.{}",
                std::process::id(),
                marker.revision + 1
            ));
            fs::create_dir(&temporary).unwrap();
            fault.marker_obstruction = Some(temporary);
            fault.phase = None;
            fault.hit = true;
        }
        Ok(())
    }
}
async fn seeded(
    dir: &Directory,
    sink: Arc<PhaseSink>,
) -> (
    Arc<AppState>,
    crate::service::persistence::PersistenceWriter,
) {
    let (state, writer) = state(dir, sink);
    state
        .remember_personal_memory_fact(
            action(&state, "seed"),
            "retained fact".into(),
            "seed".into(),
            None,
        )
        .await
        .unwrap();
    state
        .set_personal_memory_use(action(&state, "on"), UseChoice::On, "on".into())
        .await
        .unwrap();
    assert!(
        state
            .personal_memory_permits("g", "u")
            .authorizes_personal_memory()
    );
    (state, writer)
}
async fn fail_withdrawal(
    state: &AppState,
    sink: &PhaseSink,
    withdrawal: &SelfAuthorizedFactAction,
    phase: Phase,
) {
    sink.arm(phase);
    assert_eq!(
        state
            .set_personal_memory_use(withdrawal.clone(), UseChoice::Off, "off".into())
            .await,
        Err(MemoryConsentError::Persistence),
        "{phase:?}"
    );
    sink.clear();
}
fn assert_fenced(state: &AppState, dir: &Directory, before: &MemoryUsePermitSet, phase: Phase) {
    assert!(state.personal_memory_blocked.load(Ordering::Acquire));
    assert!(!state.validate_personal_memory_permits(before));
    assert!(
        !state
            .personal_memory_permits("g", "u")
            .authorizes_personal_memory()
    );
    let current = state.personal_memory_status("g", "u");
    assert_eq!(current.choice, UseChoice::Off);
    assert_eq!(current.eligible_facts, 0);
    assert_eq!(state.memory_service().facts("g", "u"), ["retained fact"]);
    let live = AppState::lock(&state.stores);
    assert!(
        live.personal_memory[&subject_key("g", "u")]
            .outcomes
            .get("off")
            .is_none_or(|outcome| !outcome.completed)
    );
    drop(live);
    let marker = journal::load(&dir.0).unwrap();
    assert_eq!(marker.withdrawals[&subject_key("g", "u")].request_id, "off");
    let disk = persist::Stores::load(&dir.0).unwrap();
    // The final canonical image precedes marker retirement. Only that late
    // failure may retain a completed disk outcome while live use stays denied.
    let outcome = disk.personal_memory[&subject_key("g", "u")]
        .outcomes
        .get("off");
    assert_eq!(
        outcome.is_some_and(|outcome| outcome.completed),
        phase == Phase::MarkerRetirement,
        "{phase:?}"
    );
}
async fn assert_retry_qualifies(
    state: &AppState,
    dir: &Directory,
    withdrawal: SelfAuthorizedFactAction,
) {
    let result = state
        .set_personal_memory_use(withdrawal.clone(), UseChoice::Off, "off".into())
        .await
        .unwrap();
    assert_eq!(result.choice, UseChoice::Off);
    assert_eq!(
        state
            .set_personal_memory_use(withdrawal, UseChoice::Off, "off".into())
            .await
            .unwrap(),
        result
    );
    assert!(!state.personal_memory_blocked.load(Ordering::Acquire));
    assert!(journal::load(&dir.0).unwrap().withdrawals.is_empty());
    assert!(
        AppState::lock(&state.stores).personal_memory[&subject_key("g", "u")].outcomes["off"]
            .completed
    );
    let disk = persist::Stores::load(&dir.0).unwrap();
    assert_eq!(
        disk.personal_memory[&subject_key("g", "u")].choice,
        UseChoice::Off
    );
    assert!(disk.personal_memory[&subject_key("g", "u")].outcomes["off"].completed);
    assert_eq!(disk.memory.facts("g", "u"), ["retained fact"]);
    let projection = crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0)).unwrap();
    assert_eq!(projection.all_memory_facts().len(), 1);
}
#[tokio::test]
async fn fresh_publication_failure_keeps_withdrawal_fenced_until_exact_retry() {
    for phase in PHASES {
        let dir = Directory::new();
        let sink = Arc::new(PhaseSink::default());
        let (state, mut writer) = seeded(&dir, sink.clone()).await;
        let permits = state.personal_memory_permits("g", "u");
        let withdrawal = action(&state, "off");
        fail_withdrawal(&state, &sink, &withdrawal, phase).await;
        assert_fenced(&state, &dir, &permits, phase);
        assert_retry_qualifies(&state, &dir, withdrawal).await;
        writer.stop();
        writer.joined().await.unwrap();
    }
}
#[tokio::test]
async fn replay_publication_failure_keeps_withdrawal_fenced_until_exact_retry() {
    for phase in PHASES {
        let dir = Directory::new();
        let sink = Arc::new(PhaseSink::default());
        let (state, mut writer) = seeded(&dir, sink.clone()).await;
        let permits = state.personal_memory_permits("g", "u");
        let withdrawal = action(&state, "off");
        // A durable provisional outcome and marker force the next request
        // through replay rather than applying a second fresh mutation.
        fail_withdrawal(&state, &sink, &withdrawal, Phase::Projection).await;
        let provisional = persist::Stores::load(&dir.0).unwrap().personal_memory
            [&subject_key("g", "u")]
            .outcomes["off"]
            .clone();
        fail_withdrawal(&state, &sink, &withdrawal, phase).await;
        assert_fenced(&state, &dir, &permits, phase);
        assert_retry_qualifies(&state, &dir, withdrawal).await;
        let qualified = AppState::lock(&state.stores).personal_memory[&subject_key("g", "u")]
            .outcomes["off"]
            .clone();
        assert_eq!(qualified.payload_digest, provisional.payload_digest);
        assert_eq!(qualified.result, provisional.result);
        writer.stop();
        writer.joined().await.unwrap();
    }
}
#[tokio::test]
async fn fresh_and_replay_publication_failures_restart_with_denial_and_retained_facts() {
    for replay in [false, true] {
        for phase in PHASES {
            let dir = Directory::new();
            let sink = Arc::new(PhaseSink::default());
            let (first, mut writer) = seeded(&dir, sink.clone()).await;
            let withdrawal = action(&first, "off");
            if replay {
                fail_withdrawal(&first, &sink, &withdrawal, Phase::Projection).await;
            }
            fail_withdrawal(&first, &sink, &withdrawal, phase).await;
            let admitted = first.personal_memory_status("g", "u").stamp;
            writer.stop();
            writer.joined().await.unwrap();
            let mut recovered = persist::Stores::load(&dir.0).unwrap();
            journal::recover(&mut recovered, &dir.0).unwrap();
            let subject = &recovered.personal_memory[&subject_key("g", "u")];
            assert_eq!(subject.choice, UseChoice::Off, "{replay:?} {phase:?}");
            assert!(subject.revision >= admitted.revision);
            assert!(subject.consent_epoch >= admitted.consent_epoch);
            assert!(recovered.personal_memory_exposure.epoch >= admitted.exposure_epoch);
            assert_eq!(recovered.memory.facts("g", "u"), ["retained fact"]);
            assert!(journal::load(&dir.0).unwrap().withdrawals.is_empty());
            let (restarted, mut writer) = state(&dir, Arc::new(FsPersistenceSink));
            *AppState::lock(&restarted.stores) = recovered;
            *AppState::lock(&restarted.recall) =
                crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0)).unwrap();
            assert!(
                !restarted
                    .personal_memory_permits("g", "u")
                    .authorizes_personal_memory()
            );
            assert_eq!(restarted.personal_memory_status("g", "u").eligible_facts, 0);
            writer.stop();
            writer.joined().await.unwrap();
        }
    }
}
