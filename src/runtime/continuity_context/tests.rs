//! Real canonical cards with fake fresh access; no provider or Discord calls.
use super::*;
use crate::work::{WorkContentRef, continuity::*};

struct Access(WorkAccess);
impl ContinuityAccessProvider for Access {
    fn authorize<'a>(
        &'a self,
        _: &'a WorkScope,
        _: u64,
        _: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
        Box::pin(async move { Ok(self.0) })
    }
}

fn seed(state: &AppState, access: WorkAccess) {
    let now = crate::runtime::now();
    let mut stores = AppState::lock(&state.stores);
    let project = stores
        .work
        .create_project(
            WorkAccess {
                can_manage: true,
                ..access
            },
            "Allocator",
            "create",
        )
        .unwrap();
    let id = stores
        .work
        .record_decision(project, access, "Keep checks", now, "decision")
        .unwrap();
    let mut registry = ProposalRegistry::new([1; 16]);
    let p = registry
        .propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Continue checked allocator".into(),
                source_refs: BTreeSet::from([WorkContentRef::Decision {
                    project,
                    id,
                    revision: 1,
                }]),
            },
            &access,
            &stores.work,
            now,
        )
        .unwrap();
    let grant = registry
        .resolve_confirmation(p.id, access.actor, &access.scope(), now)
        .unwrap();
    let crate::persist::Stores {
        work, continuity, ..
    } = &mut *stores;
    continuity.confirm(grant, &access, work, now).unwrap();
}

fn personal() -> WorkAccess {
    WorkAccess {
        actor: 7,
        guild: None,
        channel: 70,
        can_view: true,
        can_manage: false,
    }
}

#[tokio::test]
async fn continuity_owner_dm_requires_current_native_access() {
    let state = AppState::in_memory();
    let access = personal();
    seed(&state, access);
    assert!(
        state
            .prepare_continuity_context(access.scope(), 7, 70, ContinuityAudience::OwnerDm)
            .await
            .is_none()
    );
    state
        .attach_continuity_access(Arc::new(Access(access)))
        .unwrap();
    let admitted = state
        .prepare_continuity_context(access.scope(), 7, 70, ContinuityAudience::OwnerDm)
        .await
        .unwrap();
    assert_eq!(admitted.text(), "Continue checked allocator");
}

#[tokio::test]
async fn continuity_two_dm_users_and_guild_never_share_card() {
    let state = AppState::in_memory();
    let access = personal();
    seed(&state, access);
    state
        .attach_continuity_access(Arc::new(Access(access)))
        .unwrap();
    for scope in [
        WorkScope::Personal { owner: 8 },
        WorkScope::Team {
            guild: 10,
            channel: 70,
        },
    ] {
        assert!(
            state
                .prepare_continuity_context(scope, 8, 70, ContinuityAudience::OwnerDm)
                .await
                .is_none()
        );
    }
    assert!(
        state
            .prepare_continuity_context(access.scope(), 8, 70, ContinuityAudience::OwnerDm)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn continuity_team_card_needs_private_delivery() {
    let state = AppState::in_memory();
    let access = WorkAccess {
        guild: Some(10),
        channel: 20,
        ..personal()
    };
    seed(&state, access);
    state
        .attach_continuity_access(Arc::new(Access(access)))
        .unwrap();
    assert!(
        state
            .prepare_continuity_context(access.scope(), 7, 20, ContinuityAudience::OwnerDm)
            .await
            .is_none()
    );
    assert_eq!(
        state
            .prepare_continuity_context(
                access.scope(),
                7,
                20,
                ContinuityAudience::PrivateInteraction
            )
            .await
            .unwrap()
            .text(),
        "Continue checked allocator"
    );
}

#[tokio::test]
async fn continuity_changed_access_response_or_canonical_source_denies() {
    for returned in [
        WorkAccess {
            actor: 8,
            ..personal()
        },
        WorkAccess {
            channel: 80,
            ..personal()
        },
        WorkAccess {
            can_view: false,
            ..personal()
        },
    ] {
        let state = AppState::in_memory();
        seed(&state, personal());
        state
            .attach_continuity_access(Arc::new(Access(returned)))
            .unwrap();
        assert!(
            state
                .prepare_continuity_context(personal().scope(), 7, 70, ContinuityAudience::OwnerDm)
                .await
                .is_none()
        );
    }
    let state = AppState::in_memory();
    seed(&state, personal());
    state
        .attach_continuity_access(Arc::new(Access(personal())))
        .unwrap();
    AppState::lock(&state.stores).work.decisions.clear();
    assert!(
        state
            .prepare_continuity_context(personal().scope(), 7, 70, ContinuityAudience::OwnerDm)
            .await
            .is_none()
    );
}

struct RacingAccess {
    state: std::sync::Weak<AppState>,
    change: u8,
}
impl ContinuityAccessProvider for RacingAccess {
    fn authorize<'a>(
        &'a self,
        _: &'a WorkScope,
        _: u64,
        _: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.state.upgrade().unwrap();
            // Taking these locks also proves the binder released both before
            // crossing the transport seam. No sleep or live network is needed.
            match self.change {
                0 => AppState::lock(&state.stores).work.decisions.clear(),
                1 => AppState::lock(&state.continuity_safety).generation = Some(2),
                2 => {
                    AppState::lock(&state.continuity_safety)
                        .blocked
                        .insert(personal().scope());
                }
                3 | 4 => {
                    let mut stores = AppState::lock(&state.stores);
                    let mut json = serde_json::to_value(&stores.continuity).unwrap();
                    if self.change == 3 {
                        json["cards"][0]["confirmed_text"] = "Changed during access".into();
                    } else {
                        json["cards"][0]["expires_at"] = crate::runtime::now().into();
                    }
                    stores.continuity = serde_json::from_value(json).unwrap();
                }
                _ => unreachable!(),
            }
            Ok(personal())
        })
    }
}

#[tokio::test]
async fn continuity_access_await_cannot_cross_card_source_expiry_or_erasure_change() {
    for change in 0..5 {
        let state = AppState::in_memory();
        seed(&state, personal());
        state
            .attach_continuity_access(Arc::new(RacingAccess {
                state: Arc::downgrade(&state),
                change,
            }))
            .unwrap();
        assert!(
            state
                .prepare_continuity_context(personal().scope(), 7, 70, ContinuityAudience::OwnerDm)
                .await
                .is_none(),
            "change {change} admitted stale authority"
        );
    }
}

#[tokio::test]
async fn continuity_guard_binds_subject_channel_and_private_session() {
    use crate::generation::{Ask, SessionMode, consent::GenerationGuard};
    let state = AppState::in_memory();
    let access = WorkAccess {
        guild: Some(10),
        channel: 20,
        ..personal()
    };
    seed(&state, access);
    state
        .attach_continuity_access(Arc::new(Access(access)))
        .unwrap();
    let mut context =
        state
            .memory_service()
            .context_for("discord:10", "discord:7", "discord:20", "", 0, 0.5);
    context.continuity = state
        .prepare_continuity_context(
            access.scope(),
            7,
            20,
            ContinuityAudience::PrivateInteraction,
        )
        .await;
    assert!(context.continuity.is_some());
    let request = Ask {
        subject: Some(("discord:10", "discord:7")),
        session_mode: SessionMode::Ephemeral,
        scope: "discord:20",
        context: &context,
        user_input: "Continue",
        now: super::super::now(),
    };
    assert!(GenerationGuard::capture(&state, &request).is_ok());
    for wrong in [
        Ask {
            scope: "discord:21",
            ..request
        },
        Ask {
            session_mode: SessionMode::Shared,
            ..request
        },
        Ask {
            session_mode: SessionMode::SourceOnly,
            ..request
        },
    ] {
        assert!(GenerationGuard::capture(&state, &wrong).is_err());
    }
    let json = serde_json::to_value(&context).unwrap();
    assert!(json.get("continuity").is_none());
    let mut injected = json;
    injected["continuity"] = serde_json::json!({"text":"FORGED_CARD"});
    let restored: crate::memory::PersonaContext = serde_json::from_value(injected).unwrap();
    assert!(restored.continuity.is_none());
}

struct HeldAccess {
    entered: tokio::sync::Notify,
    dropped: std::sync::atomic::AtomicUsize,
}
impl ContinuityAccessProvider for HeldAccess {
    fn authorize<'a>(
        &'a self,
        _: &'a WorkScope,
        _: u64,
        _: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
        Box::pin(async move {
            struct Done<'a>(&'a std::sync::atomic::AtomicUsize);
            impl Drop for Done<'_> {
                fn drop(&mut self) {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _done = Done(&self.dropped);
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn continuity_native_read_releases_on_service_cancellation() {
    let state = AppState::in_memory();
    seed(&state, personal());
    let provider = Arc::new(HeldAccess {
        entered: tokio::sync::Notify::new(),
        dropped: std::sync::atomic::AtomicUsize::new(0),
    });
    state.attach_continuity_access(provider.clone()).unwrap();
    let mut service = crate::service::ServiceSupervisor::new();
    let mut writer = state.attach_service(service.operations());
    service.finish_startup();
    let mut future = {
        let state = state.clone();
        tokio::spawn(async move {
            state
                .prepare_continuity_context(personal().scope(), 7, 70, ContinuityAudience::OwnerDm)
                .await
        })
    };
    provider.entered.notified().await;
    service.begin_draining(
        crate::service::ShutdownReason::Signal,
        tokio::time::Instant::now(),
    );
    service.request_cancellation();
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), &mut future).await;
    if result.is_err() {
        future.abort();
        assert!(future.await.unwrap_err().is_cancelled());
    }
    writer.stop();
    writer.joined().await.unwrap();
    assert!(result.is_ok(), "native proof ignored service cancellation");
    assert!(result.unwrap().unwrap().is_none());
    assert_eq!(
        provider.dropped.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuity_canonical_guard_rechecks_revocation_after_waiting_for_stores() {
    let state = AppState::in_memory();
    seed(&state, personal());
    state
        .attach_continuity_access(Arc::new(Access(personal())))
        .unwrap();
    let admitted = state
        .prepare_continuity_context(personal().scope(), 7, 70, ContinuityAudience::OwnerDm)
        .await
        .unwrap();
    let stores = AppState::lock(&state.stores);
    let (started, entered) = std::sync::mpsc::channel();
    let (done, completed) = std::sync::mpsc::channel();
    let worker = {
        let state = state.clone();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            done.send(admitted.current(&state)).unwrap();
        })
    };
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(
        completed
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    AppState::lock(&state.continuity_safety).generation = Some(2);
    drop(stores);
    let current = completed
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    worker.join().unwrap();
    assert!(
        !current,
        "canonical guard ignored revocation while waiting for its read"
    );
}
