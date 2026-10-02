use super::*;

#[tokio::test]
async fn replay_queued_before_presnapshot_off_preserves_denial() {
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
        let (state, mut writer) = state(&dir, sink.clone());
        for subject in ["u", "other"] {
            let member_action = |id: &str| {
                let mut action = action(&state, id);
                action.proof.actor = subject.into();
                action.proof.subject = subject.into();
                action.expected = state.personal_memory_status("g", subject).stamp;
                action
            };
            state
                .remember_personal_memory_fact(
                    member_action("seed"),
                    "seed fact".into(),
                    "seed".into(),
                    None,
                )
                .await
                .unwrap();
            state
                .set_personal_memory_use(member_action("on"), UseChoice::On, "on".into())
                .await
                .unwrap();
        }
        let old_permits = state.personal_memory_permits("g", withdrawn);
        assert!(state.validate_personal_memory_permits(&old_permits));
        let original = action(&state, "retry");
        sink.fail_projection.store(true, Ordering::Release);
        assert_eq!(
            state
                .remember_personal_memory_fact(
                    original.clone(),
                    "published fact".into(),
                    "retry".into(),
                    None
                )
                .await,
            Err(MemoryConsentError::Persistence)
        );
        let held = state.persistence_preparation.lock().await;
        let owned = state.clone();
        let replay = tokio::spawn(async move {
            owned
                .remember_personal_memory_fact(
                    original,
                    "published fact".into(),
                    "retry".into(),
                    None,
                )
                .await
        });
        // Wait until the real request is admitted while preparation is held.
        loop {
            if AppState::lock(&state.personal_memory_pending)
                .get("g\u{1f}u\u{1f}retry")
                .is_some_and(|pending| pending.preparation_queued.load(Ordering::Acquire))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let off = SelfAuthorizedFactAction::new(
            MemberProof {
                actor: withdrawn.into(),
                subject: withdrawn.into(),
                guild: "g".into(),
                interaction_id: "off".into(),
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
                .set_personal_memory_use(off, UseChoice::Off, "off".into())
                .await
        });
        while state.personal_memory_exposure_epoch() == before {
            tokio::task::yield_now().await;
        }
        let admitted = state.personal_memory_exposure_epoch();
        assert!(!state.validate_personal_memory_permits(&old_permits));
        drop(held);
        assert_eq!(replay.await.unwrap(), Err(MemoryConsentError::Stale));
        assert_eq!(
            state.personal_memory_status("g", withdrawn).choice,
            UseChoice::Off
        );
        assert_eq!(
            state.personal_memory_status("g", withdrawn).eligible_facts,
            0
        );
        assert!(state.personal_memory_exposure_epoch() >= admitted);
        assert!(!state.validate_personal_memory_permits(&old_permits));
        withdrawal.await.unwrap().unwrap();
        let mut disk = persist::Stores::load(&dir.0).unwrap();
        journal::recover(&mut disk, &dir.0).unwrap();
        assert_eq!(
            disk.personal_memory[&subject_key("g", withdrawn)].choice,
            UseChoice::Off
        );
        assert!(disk.personal_memory_exposure.epoch >= admitted);
        assert!(journal::load(&dir.0).unwrap().withdrawals.is_empty());
        assert_eq!(disk.memory.facts("g", "u"), ["seed fact", "published fact"]);
        writer.stop();
        writer.joined().await.unwrap();
    }
}

#[tokio::test]
async fn off_rebase_preserves_newer_ordinary_remember_and_forget() {
    for forget in [false, true] {
        let dir = Directory::new();
        let sink = Arc::new(ReplaySink {
            fail_projection: std::sync::atomic::AtomicBool::new(false),
            hold_projection: std::sync::atomic::AtomicBool::new(false),
            held: Arc::new(HeldSink {
                entered: tokio::sync::Notify::new(),
                release: (Mutex::new(false), Condvar::new()),
            }),
        });
        let (state, mut writer) = state(&dir, sink.clone());
        state
            .memory_service()
            .remember("g", "other", "old fact", crate::runtime::now())
            .unwrap();
        assert_eq!(
            state.persist_all().canonical_state,
            persist::PersistComponentOutcome::Committed
        );
        sink.fail_projection.store(true, Ordering::Release);
        assert_eq!(
            state
                .remember_personal_memory_fact(
                    action(&state, "failed"),
                    "published fact".into(),
                    "failed".into(),
                    None
                )
                .await,
            Err(MemoryConsentError::Persistence)
        );
        if forget {
            assert!(state.memory_service().forget("g", "other", "old fact"));
        } else {
            state
                .memory_service()
                .remember("g", "other", "new live fact", crate::runtime::now())
                .unwrap();
        }
        state
            .set_personal_memory_use(action(&state, "off"), UseChoice::Off, "off".into())
            .await
            .unwrap();
        let expected: Vec<String> = if forget {
            vec![]
        } else {
            vec!["old fact".into(), "new live fact".into()]
        };
        assert_eq!(state.memory_service().facts("g", "other"), expected);
        assert_eq!(state.memory_service().facts("g", "u"), ["published fact"]);
        let mut disk = persist::Stores::load(&dir.0).unwrap();
        assert_eq!(disk.memory.facts("g", "other"), expected);
        assert_eq!(disk.memory.facts("g", "u"), ["published fact"]);
        let projection = crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0)).unwrap();
        assert_eq!(projection.all_memory_facts().len(), expected.len() + 1);
        journal::recover(&mut disk, &dir.0).unwrap();
        assert_eq!(disk.memory.facts("g", "other"), expected);
        assert_eq!(disk.memory.facts("g", "u"), ["published fact"]);
        writer.stop();
        writer.joined().await.unwrap();
    }
}

#[tokio::test]
async fn fifo_projection_failure_then_runtime_and_owner_save_keep_lineage() {
    let dir = Directory::new();
    let sink = Arc::new(ReplaySink {
        fail_projection: std::sync::atomic::AtomicBool::new(false),
        hold_projection: std::sync::atomic::AtomicBool::new(false),
        held: Arc::new(HeldSink {
            entered: tokio::sync::Notify::new(),
            release: (Mutex::new(false), Condvar::new()),
        }),
    });
    let (state, mut writer) = state(&dir, sink.clone());
    state
        .memory_service()
        .remember("g", "u", "first", crate::runtime::now())
        .unwrap();
    assert_eq!(
        state.request_persistence().await.unwrap().canonical_state,
        persist::PersistComponentOutcome::Committed
    );
    assert!(writer.last_completed().is_some());
    state
        .memory_service()
        .remember("g", "u", "second", crate::runtime::now())
        .unwrap();
    sink.fail_projection.store(true, Ordering::Release);
    let failed = state.request_persistence().await.unwrap();
    assert_eq!(
        failed.canonical_state,
        persist::PersistComponentOutcome::Committed
    );
    assert!(matches!(
        failed.wdbx_projection,
        persist::PersistComponentOutcome::Failed(_)
    ));
    assert_eq!(
        persist::Stores::load(&dir.0)
            .unwrap()
            .memory
            .facts("g", "u"),
        ["first", "second"]
    );
    assert_eq!(
        state.persist_all_gated().await.overall,
        persist::PersistOverall::Complete
    );
    assert_eq!(
        state.request_persistence().await.unwrap().overall,
        persist::PersistOverall::Complete
    );
    assert_eq!(
        state.persist_all().overall,
        persist::PersistOverall::Complete
    );
    // The immutable owner API advances itself across changed and unchanged saves.
    let mut owner = AppState::lock(&state.stores).clone();
    let stale = owner.clone();
    owner
        .memory
        .remember("g", "u", "owner third", crate::runtime::now());
    owner.save(&dir.0).unwrap();
    owner.save(&dir.0).unwrap();
    assert!(stale.save(&dir.0).is_err());
    assert_eq!(
        persist::Stores::load(&dir.0)
            .unwrap()
            .memory
            .facts("g", "u"),
        ["first", "second", "owner third"]
    );
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn failed_withdrawal_owner_keeps_denial_across_unrelated_replay() {
    let dir = Directory::new();
    let sink = Arc::new(ReplaySink {
        fail_projection: std::sync::atomic::AtomicBool::new(false),
        hold_projection: std::sync::atomic::AtomicBool::new(false),
        held: Arc::new(HeldSink {
            entered: tokio::sync::Notify::new(),
            release: (Mutex::new(false), Condvar::new()),
        }),
    });
    let (state, mut writer) = state(&dir, sink.clone());
    state
        .set_personal_memory_use(action(&state, "on"), UseChoice::On, "on".into())
        .await
        .unwrap();
    let remembered = action(&state, "remember");
    sink.fail_projection.store(true, Ordering::Release);
    assert_eq!(
        state
            .remember_personal_memory_fact(
                remembered.clone(),
                "retained".into(),
                "remember".into(),
                None
            )
            .await,
        Err(MemoryConsentError::Persistence)
    );
    let off = action(&state, "off");
    sink.fail_projection.store(true, Ordering::Release);
    assert_eq!(
        state
            .set_personal_memory_use(off.clone(), UseChoice::Off, "off".into())
            .await,
        Err(MemoryConsentError::Persistence)
    );
    assert!(AppState::lock(&state.personal_memory_pending).is_empty());
    let epoch = state.personal_memory_exposure_epoch();
    assert_eq!(
        state
            .remember_personal_memory_fact(remembered, "retained".into(), "remember".into(), None)
            .await,
        Err(MemoryConsentError::Stale)
    );
    assert!(state.personal_memory_blocked.load(Ordering::Acquire));
    assert_eq!(
        state.personal_memory_status("g", "u").choice,
        UseChoice::Off
    );
    assert!(state.personal_memory_exposure_epoch() >= epoch);
    state
        .set_personal_memory_use(off, UseChoice::Off, "off".into())
        .await
        .unwrap();
    assert_eq!(
        state.personal_memory_status("g", "u").choice,
        UseChoice::Off
    );
    assert_eq!(state.memory_service().facts("g", "u"), ["retained"]);
    assert!(!state.personal_memory_blocked.load(Ordering::Acquire));
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn imported_provisional_outcome_completes_before_fresh_on_and_permits() {
    for withdrawn in ["u", "other"] {
        for retry_off in [false, true] {
            for forget in [false, true] {
                let dir = Directory::new();
                let sink = Arc::new(ReplaySink {
                    fail_projection: std::sync::atomic::AtomicBool::new(false),
                    hold_projection: std::sync::atomic::AtomicBool::new(false),
                    held: Arc::new(HeldSink {
                        entered: tokio::sync::Notify::new(),
                        release: (Mutex::new(false), Condvar::new()),
                    }),
                });
                let (state, mut writer) = state(&dir, sink.clone());
                state
                    .memory_service()
                    .remember("g", "ordinary", "old", crate::runtime::now())
                    .unwrap();
                assert_eq!(
                    state.persist_all().overall,
                    persist::PersistOverall::Complete
                );
                let remembered = action(&state, "remember");
                let receipt = "ab".repeat(32);
                sink.fail_projection.store(true, Ordering::Release);
                assert_eq!(
                    state
                        .remember_personal_memory_fact(
                            remembered.clone(),
                            "retained".into(),
                            "remember".into(),
                            Some(receipt.clone())
                        )
                        .await,
                    Err(MemoryConsentError::Persistence)
                );
                let provisional = persist::Stores::load(&dir.0).unwrap().personal_memory
                    [&subject_key("g", "u")]
                    .outcomes["remember"]
                    .clone();
                assert!(!provisional.completed);
                let mut off = action(&state, "off");
                off.proof.actor = withdrawn.into();
                off.proof.subject = withdrawn.into();
                off.expected = state.personal_memory_status("g", withdrawn).stamp;
                if retry_off {
                    sink.fail_projection.store(true, Ordering::Release);
                    assert_eq!(
                        state
                            .set_personal_memory_use(off.clone(), UseChoice::Off, "off".into())
                            .await,
                        Err(MemoryConsentError::Persistence)
                    );
                }
                // Mutations after either retained publication must survive its reconciliation.
                if forget {
                    assert!(state.memory_service().forget("g", "ordinary", "old"));
                } else {
                    state
                        .memory_service()
                        .remember("g", "ordinary", "new", crate::runtime::now())
                        .unwrap();
                }
                if !retry_off {
                    // This is a fresh action, first admitted after the ordinary mutation.
                    // Replays keep their original stamp and payload unchanged.
                    off.expected = state.personal_memory_status("g", withdrawn).stamp;
                }
                if retry_off {
                    // Fail the replay itself too: no provisional outcome may report completion.
                    sink.fail_projection.store(true, Ordering::Release);
                    assert_eq!(
                        state
                            .set_personal_memory_use(off.clone(), UseChoice::Off, "off".into())
                            .await,
                        Err(MemoryConsentError::Persistence)
                    );
                    let disk = persist::Stores::load(&dir.0).unwrap();
                    assert!(
                        !disk.personal_memory[&subject_key("g", "u")].outcomes["remember"]
                            .completed
                    );
                    assert!(state.personal_memory_blocked.load(Ordering::Acquire));
                }
                state
                    .set_personal_memory_use(off, UseChoice::Off, "off".into())
                    .await
                    .unwrap();
                let expected = if forget {
                    vec![]
                } else {
                    vec!["old".to_string(), "new".to_string()]
                };
                assert_eq!(state.memory_service().facts("g", "ordinary"), expected);
                assert_eq!(state.memory_service().facts("g", "u"), ["retained"]);
                assert!(!state.personal_memory_blocked.load(Ordering::Acquire));
                {
                    let live = AppState::lock(&state.stores);
                    let imported =
                        &live.personal_memory[&subject_key("g", "u")].outcomes["remember"];
                    assert!(imported.completed);
                    assert_eq!(imported.result, provisional.result);
                    assert_eq!(imported.payload_digest, provisional.payload_digest);
                    assert!(
                        live.personal_memory[&subject_key("g", "u")]
                            .proofs
                            .contains_key(&fact_key("g", "u", "retained"))
                    );
                    assert_eq!(
                        live.memory_receipts[&receipt_key("g", "u", "retained")],
                        receipt
                    );
                }
                // Assert before any restart/journal recovery can hide live incompleteness.
                let on = state
                    .set_personal_memory_use(
                        action(&state, "fresh-on"),
                        UseChoice::On,
                        "fresh-on".into(),
                    )
                    .await
                    .unwrap();
                assert_eq!(on.choice, UseChoice::On);
                let status = state.personal_memory_status("g", "u");
                assert_eq!(status.choice, UseChoice::On);
                assert_eq!(status.eligible_facts, 1);
                let permits = state.personal_memory_permits("g", "u");
                assert!(permits.authorizes_personal_memory());
                assert!(state.validate_personal_memory_permits(&permits));
                assert_eq!(
                    state
                        .remember_personal_memory_fact(
                            remembered,
                            "retained".into(),
                            "remember".into(),
                            Some(receipt.clone())
                        )
                        .await
                        .unwrap(),
                    provisional.result
                );
                let disk = persist::Stores::load(&dir.0).unwrap();
                assert!(
                    disk.personal_memory[&subject_key("g", "u")].outcomes["remember"].completed
                );
                assert_eq!(disk.memory.facts("g", "ordinary"), expected);
                assert_eq!(
                    disk.memory_receipts[&receipt_key("g", "u", "retained")],
                    receipt
                );
                let projection =
                    crate::wdbx::Recall::load(&persist::Stores::wdbx_path(&dir.0)).unwrap();
                assert_eq!(projection.all_memory_facts().len(), expected.len() + 1);
                writer.stop();
                writer.joined().await.unwrap();
            }
        }
    }
}
