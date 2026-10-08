//! Real covered child/canonical retry tests; no live gateway or credentials.
use super::*;
#[tokio::test]
async fn continuity_refused_candidate_is_never_reproposed_by_snapshot_persistence() {
    let dir = Directory::new();
    let count = dir.0.join("proposal-count");
    let body = format!(
        "printf x >> '{}'; printf '%s\\n' 'FailedPrecondition: learning_disabled' >&2; exit 1",
        count.display()
    );
    let mut state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, &body));
    state.attach_continuity_access(Arc::new(Access)).unwrap();
    let g = grant(&state, 7, 70, "REFUSED_PRIVATE_CARD");
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    assert_eq!(
        state.confirm_continuity(g, 1, 7, 70).await,
        Err(WorkError::Persistence)
    );
    assert_eq!(std::fs::read(&count).unwrap(), b"x");
    state.request_persistence().await.unwrap();
    state.request_persistence().await.unwrap();
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(std::fs::read(&count).unwrap(), b"x");
    assert!(
        Stores::load(&dir.0)
            .unwrap()
            .continuity
            .card(&WorkScope::Personal { owner: 7 })
            .is_none()
    );
    let safety = AppState::lock(&state.continuity_safety);
    assert!(safety.pending.is_empty() && safety.orphans.is_empty());
    assert!(
        !String::from_utf8(std::fs::read(Stores::state_path(&dir.0)).unwrap())
            .unwrap()
            .contains("REFUSED_PRIVATE_CARD")
    );
}
#[tokio::test]
async fn continuity_restart_forgotten_receipt_excludes_context_and_clears_without_reproposal() {
    let dir = Directory::new();
    let count = dir.0.join("proposal-count");
    let body = format!(
        "if [ \"$3\" = verify ]; then {LIVE}; else printf x >> '{}'; {APPEND}; fi",
        count.display()
    );
    let sink = Arc::new(Sink(AtomicU8::new(0)));
    let mut state = AppState::in_memory_with_persistence(Some(dir.0.clone()), sink.clone());
    Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, &body));
    state.attach_continuity_access(Arc::new(Access)).unwrap();
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let g = grant(&state, 7, 70, "FORGOTTEN_PRIVATE_CARD");
    state.confirm_continuity(g, 1, 7, 70).await.unwrap();
    let scope = WorkScope::Personal { owner: 7 };
    sink.0.store(1, Ordering::SeqCst);
    assert_eq!(
        state.clear_continuity(scope.clone(), 7, 70).await,
        Err(WorkError::Persistence)
    );
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(std::fs::read(&count).unwrap(), b"xx");
    let loaded = Stores::load(&dir.0).unwrap();
    assert!(loaded.continuity.card(&scope).is_some());
    drop(state);
    let forgotten = LIVE.replace(
        "\"memory_forgotten\":\"false\"",
        "\"memory_forgotten\":\"true\"",
    );
    let body = format!(
        "if [ \"$3\" = verify ]; then {forgotten}; else printf x >> '{}'; {APPEND}; fi",
        count.display()
    );
    let mut restarted =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    *AppState::lock(&restarted.stores) = loaded;
    Arc::get_mut(&mut restarted).unwrap().episode_gate = Some(gate(&dir, &body));
    restarted
        .attach_continuity_access(Arc::new(Access))
        .unwrap();
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = restarted.attach_service(supervisor.operations());
    assert!(
        restarted
            .prepare_continuity_context(
                scope.clone(),
                7,
                70,
                crate::runtime::continuity_context::ContinuityAudience::OwnerDm
            )
            .await
            .is_none()
    );
    assert_eq!(
        restarted
            .clear_continuity(scope.clone(), 7, 70)
            .await
            .unwrap(),
        1
    );
    restarted.request_persistence().await.unwrap();
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(std::fs::read(&count).unwrap(), b"xx");
    assert!(
        Stores::load(&dir.0)
            .unwrap()
            .continuity
            .card(&scope)
            .is_none()
    );
}
