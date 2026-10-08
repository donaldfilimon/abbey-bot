//! Actual retained canonical owner regressions, with native access injected.
use super::*;
use crate::work::{WorkAccess, WorkScope, continuity::*};
use std::{collections::BTreeSet, future::Future, pin::Pin};
struct Access;
impl super::super::continuity_context::ContinuityAccessProvider for Access {
    fn authorize<'a>(
        &'a self,
        scope: &'a WorkScope,
        actor: u64,
        channel: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(WorkAccess {
                actor,
                channel,
                guild: match scope {
                    WorkScope::Team { guild, .. } => Some(*guild),
                    _ => None,
                },
                can_view: true,
                can_manage: false,
            })
        })
    }
}
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "abbey-continuity-owner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn grant(state: &AppState, actor: u64, channel: u64, text: &str) -> ResolvedConfirmation {
    let access = WorkAccess {
        actor,
        channel,
        guild: None,
        can_view: true,
        can_manage: false,
    };
    let mut stores = AppState::lock(&state.stores);
    if stores.work.projects.is_empty() {
        stores
            .work
            .create_project(access, "Allocator", "project")
            .unwrap();
    }
    let mut registry = ProposalRegistry::new([1; 16]);
    let p = registry
        .propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: stores
                    .continuity
                    .card(&access.scope())
                    .map_or(0, |c| c.revision),
                presented_text: text.into(),
                source_refs: BTreeSet::new(),
            },
            &access,
            &stores.work,
            now(),
        )
        .unwrap();
    registry
        .resolve_confirmation(p.id, actor, &access.scope(), now())
        .unwrap()
}
#[tokio::test]
async fn continuity_owner_confirm_clear_reopen_preserves_native_work() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    state.attach_continuity_access(Arc::new(Access)).unwrap();
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let g = grant(&state, 7, 70, "Resume checked allocator");
    let result = state.confirm_continuity(g, 1, 7, 70).await;
    writer.stop();
    writer.joined().await.unwrap();
    assert!(result.is_ok(), "{result:?}");
    let disk = Stores::load(&dir.0).unwrap();
    assert_eq!(
        disk.continuity
            .card(&WorkScope::Personal { owner: 7 })
            .unwrap()
            .confirmed_text,
        "Resume checked allocator"
    );
    // A fresh retained writer is not needed: exercise clear before stopping in its own test.
    assert_eq!(disk.work.projects.len(), 1);
}
#[tokio::test]
async fn continuity_owner_clear_and_member_erase_remove_canonical_cards() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    state.attach_continuity_access(Arc::new(Access)).unwrap();
    let g = grant(&state, 7, 70, "Resume checked allocator");
    {
        let mut stores = AppState::lock(&state.stores);
        let work = stores.work.clone();
        stores
            .continuity
            .confirm(
                g,
                &WorkAccess {
                    actor: 7,
                    channel: 70,
                    guild: None,
                    can_view: true,
                    can_manage: false,
                },
                &work,
                now(),
            )
            .unwrap();
    }
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    state.request_persistence().await.unwrap();
    let result = state
        .clear_continuity(WorkScope::Personal { owner: 7 }, 7, 70)
        .await;
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(result, Ok(1));
    assert!(
        Stores::load(&dir.0)
            .unwrap()
            .continuity
            .card(&WorkScope::Personal { owner: 7 })
            .is_none()
    );
}
#[tokio::test]
async fn continuity_learning_erasure_reports_and_durably_removes_card() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    let g = grant(&state, 7, 70, "Resume checked allocator");
    {
        let mut stores = AppState::lock(&state.stores);
        let work = stores.work.clone();
        stores
            .continuity
            .confirm(
                g,
                &WorkAccess {
                    actor: 7,
                    channel: 70,
                    guild: None,
                    can_view: true,
                    can_manage: false,
                },
                &work,
                now(),
            )
            .unwrap();
    }
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    state.request_persistence().await.unwrap();
    let report = state
        .erase_personal_learning("discord:dm:7".into(), 7)
        .await;
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(report.unwrap().continuity, 1);
    assert!(
        Stores::load(&dir.0)
            .unwrap()
            .continuity
            .card(&WorkScope::Personal { owner: 7 })
            .is_none()
    );
}

#[cfg(unix)]
mod covered {
    mod recovery;
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
    struct Sink(AtomicU8);
    impl PersistenceSink for Sink {
        fn publish(
            &self,
            dir: &std::path::Path,
            path: &std::path::Path,
            bytes: &[u8],
        ) -> Result<(), crate::persist::PersistErrorCategory> {
            if path
                .file_name()
                .is_some_and(|p| p == crate::persist::STATE_FILE)
            {
                match self.0.load(Ordering::SeqCst) {
                    1 => return Err(crate::persist::PersistErrorCategory::SyncTemporary),
                    2 => {
                        FsPersistenceSink.publish(dir, path, bytes)?;
                        return Err(crate::persist::PersistErrorCategory::SyncTemporary);
                    }
                    _ => {}
                }
            }
            FsPersistenceSink.publish(dir, path, bytes)
        }
    }
    fn gate(dir: &Directory, body: &str) -> Arc<crate::episode_gate::EpisodeGate> {
        let path = dir.0.join("abi");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Arc::new(crate::episode_gate::EpisodeGate::new(crate::episode_gate::EpisodeGateConfig::from_json(&serde_json::json!({"abi_cli":path,"endpoint":"http://127.0.0.1:50051","token_file":dir.0.join("token"),"policy_version":"v1","contract_revision":2,"contract_digest":"ab".repeat(32),"timeout_secs":5}).to_string()).unwrap()))
    }
    const APPEND: &str = "printf '%s\\n' '{\"decision\":\"appended\",\"episode_digest\":\"abababababababababababababababababababababababababababababababab\",\"sequence\":\"1\"}'";
    const LIVE: &str = "printf '%s\\n' '{\"found\":\"true\",\"guild_ref\":\"discord-dm-7\",\"event_kind\":\"memory_candidate\",\"signature_status\":\"unsigned\",\"memory_forgotten\":\"false\"}'";
    struct Revoked(AtomicUsize);
    impl super::super::super::continuity_context::ContinuityAccessProvider for Revoked {
        fn authorize<'a>(
            &'a self,
            _: &'a WorkScope,
            actor: u64,
            channel: u64,
        ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
            Box::pin(async move {
                if self.0.fetch_add(1, Ordering::SeqCst) > 0 {
                    Err(WorkError::Denied)
                } else {
                    Ok(WorkAccess {
                        actor,
                        channel,
                        guild: None,
                        can_view: true,
                        can_manage: false,
                    })
                }
            })
        }
    }
    #[tokio::test]
    async fn continuity_confirm_rechecks_native_authority_after_append() {
        let dir = Directory::new();
        let mut state =
            AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, APPEND));
        state
            .attach_continuity_access(Arc::new(Revoked(AtomicUsize::new(0))))
            .unwrap();
        let g = grant(&state, 7, 70, "Exact preview");
        let mut sup = crate::service::ServiceSupervisor::new();
        sup.finish_startup();
        let mut writer = state.attach_service(sup.operations());
        let result = state.confirm_continuity(g, 1, 7, 70).await;
        writer.stop();
        writer.joined().await.unwrap();
        assert_eq!(result, Err(WorkError::Denied));
        assert!(
            AppState::lock(&state.stores)
                .continuity
                .card(&WorkScope::Personal { owner: 7 })
                .is_none()
        );
        assert!(!Stores::state_path(&dir.0).exists());
    }
    #[tokio::test]
    async fn continuity_write_then_error_keeps_appended_card_in_future_snapshots() {
        let dir = Directory::new();
        let sink = Arc::new(Sink(AtomicU8::new(2)));
        let mut state = AppState::in_memory_with_persistence(Some(dir.0.clone()), sink.clone());
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, APPEND));
        state.attach_continuity_access(Arc::new(Access)).unwrap();
        let g = grant(&state, 7, 70, "Exact preview");
        let mut sup = crate::service::ServiceSupervisor::new();
        sup.finish_startup();
        let mut writer = state.attach_service(sup.operations());
        assert_eq!(
            state.confirm_continuity(g, 1, 7, 70).await,
            Err(WorkError::Persistence)
        );
        assert!(
            Stores::load(&dir.0)
                .unwrap()
                .continuity
                .card(&WorkScope::Personal { owner: 7 })
                .is_some()
        );
        sink.0.store(0, Ordering::SeqCst);
        state.request_persistence().await.unwrap();
        writer.stop();
        writer.joined().await.unwrap();
        let disk = Stores::load(&dir.0).unwrap();
        assert_eq!(
            disk.continuity
                .card(&WorkScope::Personal { owner: 7 })
                .unwrap()
                .episode_receipt,
            Some("ab".repeat(32))
        );
    }
    #[tokio::test]
    async fn continuity_receipt_context_requires_positive_live_remote_verification() {
        let dir = Directory::new();
        let mut state = AppState::in_memory();
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, "exit 1"));
        state.attach_continuity_access(Arc::new(Access)).unwrap();
        let g = grant(&state, 7, 70, "Exact preview");
        {
            let mut stores = AppState::lock(&state.stores);
            let work = stores.work.clone();
            let card = stores
                .continuity
                .confirm(
                    g,
                    &WorkAccess {
                        actor: 7,
                        channel: 70,
                        guild: None,
                        can_view: true,
                        can_manage: false,
                    },
                    &work,
                    now(),
                )
                .unwrap();
            stores
                .continuity
                .install_receipt(&card, "ab".repeat(32))
                .unwrap();
        }
        let ctx = state
            .prepare_continuity_context(
                WorkScope::Personal { owner: 7 },
                7,
                70,
                super::super::super::continuity_context::ContinuityAudience::OwnerDm,
            )
            .await;
        assert!(ctx.is_none());
    }
    #[tokio::test]
    async fn continuity_appended_forget_local_failure_retries_only_publication() {
        let dir = Directory::new();
        let sink = Arc::new(Sink(AtomicU8::new(0)));
        let mut state = AppState::in_memory_with_persistence(Some(dir.0.clone()), sink.clone());
        let count = dir.0.join("forget-count");
        let body = format!(
            "if [ \"$3\" = verify ]; then {LIVE}; else printf x >> '{}'; {APPEND}; fi",
            count.display()
        );
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, &body));
        state.attach_continuity_access(Arc::new(Access)).unwrap();
        let g = grant(&state, 7, 70, "Exact preview");
        {
            let mut stores = AppState::lock(&state.stores);
            let work = stores.work.clone();
            let card = stores
                .continuity
                .confirm(
                    g,
                    &WorkAccess {
                        actor: 7,
                        channel: 70,
                        guild: None,
                        can_view: true,
                        can_manage: false,
                    },
                    &work,
                    now(),
                )
                .unwrap();
            stores
                .continuity
                .install_receipt(&card, "ab".repeat(32))
                .unwrap();
        }
        let mut sup = crate::service::ServiceSupervisor::new();
        sup.finish_startup();
        let mut writer = state.attach_service(sup.operations());
        state.request_persistence().await.unwrap();
        sink.0.store(1, Ordering::SeqCst);
        assert_eq!(
            state
                .clear_continuity(WorkScope::Personal { owner: 7 }, 7, 70)
                .await,
            Err(WorkError::Persistence)
        );
        assert_eq!(std::fs::read(&count).unwrap(), b"x");
        assert!(
            state
                .prepare_continuity_context(
                    WorkScope::Personal { owner: 7 },
                    7,
                    70,
                    super::super::super::continuity_context::ContinuityAudience::OwnerDm
                )
                .await
                .is_none()
        );
        sink.0.store(0, Ordering::SeqCst);
        assert_eq!(
            state
                .clear_continuity(WorkScope::Personal { owner: 7 }, 7, 70)
                .await
                .unwrap(),
            1
        );
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
    }
    struct DenyFourth(AtomicUsize);
    impl super::super::super::continuity_context::ContinuityAccessProvider for DenyFourth {
        fn authorize<'a>(
            &'a self,
            _: &'a WorkScope,
            actor: u64,
            channel: u64,
        ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
            Box::pin(async move {
                if self.0.fetch_add(1, Ordering::SeqCst) == 3 {
                    Err(WorkError::Denied)
                } else {
                    Ok(WorkAccess {
                        actor,
                        channel,
                        guild: None,
                        can_view: true,
                        can_manage: false,
                    })
                }
            })
        }
    }
    #[tokio::test]
    async fn continuity_failed_clear_retry_cannot_release_unresolved_fence() {
        let dir = Directory::new();
        let mut state =
            AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, "exit 1"));
        state
            .attach_continuity_access(Arc::new(DenyFourth(AtomicUsize::new(0))))
            .unwrap();
        let g = grant(&state, 7, 70, "Exact preview");
        {
            let mut stores = AppState::lock(&state.stores);
            let work = stores.work.clone();
            let card = stores
                .continuity
                .confirm(
                    g,
                    &WorkAccess {
                        actor: 7,
                        channel: 70,
                        guild: None,
                        can_view: true,
                        can_manage: false,
                    },
                    &work,
                    now(),
                )
                .unwrap();
            stores
                .continuity
                .install_receipt(&card, "ab".repeat(32))
                .unwrap();
        }
        let mut sup = crate::service::ServiceSupervisor::new();
        sup.finish_startup();
        let mut writer = state.attach_service(sup.operations());
        state.request_persistence().await.unwrap();
        let scope = WorkScope::Personal { owner: 7 };
        assert!(state.clear_continuity(scope.clone(), 7, 70).await.is_err());
        assert!(state.continuity_generation(&scope).is_none());
        assert_eq!(
            state.clear_continuity(scope.clone(), 7, 70).await,
            Err(WorkError::Denied)
        );
        writer.stop();
        writer.joined().await.unwrap();
        assert!(state.continuity_generation(&scope).is_none());
    }
    #[tokio::test]
    async fn continuity_member_erasure_includes_source_linked_orphan_receipts() {
        let dir = Directory::new();
        let mut state =
            AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
        let body = format!(
            "if [ \"$3\" = verify ]; then {}; else {APPEND}; fi",
            LIVE.replace("discord-dm-7", "discord-1")
        );
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, &body));
        let access = WorkAccess {
            actor: 7,
            channel: 2,
            guild: Some(1),
            can_view: true,
            can_manage: true,
        };
        let (project, id) = {
            let mut stores = AppState::lock(&state.stores);
            let project = stores
                .work
                .create_project(access, "Shared", "project")
                .unwrap();
            stores.work.set_member(project, access, 8, true).unwrap();
            let id = stores
                .work
                .add_task(
                    access,
                    crate::work::WorkTask {
                        id: 0,
                        project_id: project,
                        title: "Checks".into(),
                        owner: 0,
                        assignee: Some(8),
                        goal_id: None,
                        priority: 1,
                        status: crate::work::WorkStatus::Open,
                        due_at: None,
                        remind_at: None,
                        reminder_revision: 0,
                        snoozed_until: None,
                        source: None,
                        github: None,
                        revision: 0,
                    },
                    "task",
                )
                .unwrap();
            (project, id)
        };
        let orphan = ContinuityCard {
            schema_version: 1,
            scope: access.scope(),
            confirmed_by: 7,
            revision: 1,
            confirmed_text: "Exact human preview".into(),
            source_refs: BTreeSet::from([crate::work::WorkContentRef::Task {
                project,
                id,
                revision: 0,
            }]),
            expires_at: now() + 604800,
            episode_receipt: Some("ab".repeat(32)),
        };
        let mut unrelated = orphan.clone();
        unrelated.scope = WorkScope::Team {
            guild: 1,
            channel: 3,
        };
        unrelated.confirmed_by = 9;
        unrelated.source_refs.clear();
        {
            let mut safety = AppState::lock(&state.continuity_safety);
            safety.orphans.insert(orphan.scope.clone(), orphan.clone());
            safety
                .orphans
                .insert(unrelated.scope.clone(), unrelated.clone());
            safety.blocked.insert(orphan.scope.clone());
            safety.blocked.insert(unrelated.scope.clone());
        }
        let mut sup = crate::service::ServiceSupervisor::new();
        sup.finish_startup();
        let mut writer = state.attach_service(sup.operations());
        let result = state.erase_personal_learning("discord:1".into(), 8).await;
        writer.stop();
        writer.joined().await.unwrap();
        assert!(result.is_ok());
        let safety = AppState::lock(&state.continuity_safety);
        assert!(!safety.orphans.contains_key(&orphan.scope));
        assert_eq!(safety.orphans.get(&unrelated.scope), Some(&unrelated));
    }
    #[tokio::test]
    async fn continuity_revoked_during_old_receipt_verify_never_proposes_replacement() {
        let dir = Directory::new();
        let mut state =
            AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
        let count = dir.0.join("proposal-count");
        let body = format!(
            "if [ \"$3\" = verify ]; then {LIVE}; else printf x >> '{}'; {APPEND}; fi",
            count.display()
        );
        Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate(&dir, &body));
        state
            .attach_continuity_access(Arc::new(Revoked(AtomicUsize::new(0))))
            .unwrap();
        let g = grant(&state, 7, 70, "Old confirmed text");
        {
            let mut stores = AppState::lock(&state.stores);
            let work = stores.work.clone();
            let card = stores
                .continuity
                .confirm(
                    g,
                    &WorkAccess {
                        actor: 7,
                        channel: 70,
                        guild: None,
                        can_view: true,
                        can_manage: false,
                    },
                    &work,
                    now(),
                )
                .unwrap();
            stores
                .continuity
                .install_receipt(&card, "cd".repeat(32))
                .unwrap();
        }
        let g = grant(&state, 7, 70, "Replacement preview");
        let mut sup = crate::service::ServiceSupervisor::new();
        sup.finish_startup();
        let mut writer = state.attach_service(sup.operations());
        assert_eq!(
            state.confirm_continuity(g, 1, 7, 70).await,
            Err(WorkError::Denied)
        );
        writer.stop();
        writer.joined().await.unwrap();
        assert!(!count.exists());
    }
}

#[tokio::test]
async fn personal_memory_publication_preserves_pending_continuity_delete() {
    use crate::personal_memory::{MemberProof, SelfAuthorizedFactAction};
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    state.attach_continuity_access(Arc::new(Access)).unwrap();
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let target = WorkScope::Personal { owner: 7 };
    let g = grant(&state, 7, 70, "REMOVE_PENDING_CARD");
    let expected = state
        .confirm_continuity(g, state.continuity_generation(&target).unwrap(), 7, 70)
        .await
        .unwrap();
    let other_scope = WorkScope::Personal { owner: 8 };
    {
        let access = WorkAccess {
            actor: 8,
            channel: 80,
            guild: None,
            can_view: true,
            can_manage: false,
        };
        AppState::lock(&state.stores)
            .work
            .create_project(access, "Other project", "other-project")
            .unwrap();
    }
    let g = grant(&state, 8, 80, "KEEP_UNRELATED_CARD");
    let other = state
        .confirm_continuity(g, state.continuity_generation(&other_scope).unwrap(), 8, 80)
        .await
        .unwrap();
    {
        let _serial = state.persistence_preparation.lock().await;
        state
            .stage_continuity(&target, Some(expected.clone()), None, None)
            .unwrap();
    }
    assert_eq!(
        AppState::lock(&state.stores).continuity.card(&target),
        Some(&expected)
    );
    let action = SelfAuthorizedFactAction::new(
        MemberProof {
            actor: "u".into(),
            subject: "u".into(),
            guild: "g".into(),
            interaction_id: "personal-overlay-remember".into(),
            platform: "discord".into(),
            at: now(),
            policy_version: 1,
        },
        state.personal_memory_status("g", "u").stamp,
    )
    .unwrap();
    // Public wrapper delegates to the real retained commit_personal_memory.
    state
        .remember_personal_memory_fact(
            action,
            "uses rust".into(),
            "personal-overlay-r1".into(),
            None,
        )
        .await
        .unwrap();
    let reopened = Stores::load(&dir.0).unwrap();
    assert!(
        reopened.continuity.card(&target).is_none(),
        "direct personal canonical publication must apply pending deletion"
    );
    assert_eq!(reopened.continuity.card(&other_scope), Some(&other));
    assert_eq!(
        reopened.memory.facts("g", "u"),
        vec!["uses rust".to_string()]
    );
    {
        let safety = AppState::lock(&state.continuity_safety);
        assert!(safety.pending.contains_key(&target));
        assert!(
            safety.blocked.contains(&target),
            "personal commit cannot clear recovery fence"
        );
    }
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn continuity_pending_conflict_prevents_canonical_and_projection_publication() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    state.attach_continuity_access(Arc::new(Access)).unwrap();
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let scope = WorkScope::Personal { owner: 7 };
    let grant = grant(&state, 7, 70, "KEEP_CURRENT_CARD");
    let mut unexpected = state.confirm_continuity(grant, 1, 7, 70).await.unwrap();
    let before = std::fs::read(Stores::state_path(&dir.0)).unwrap();
    let projection = std::fs::read(Stores::wdbx_path(&dir.0)).unwrap();
    unexpected.confirmed_text = "DIFFERENT_EXPECTED_PARENT".into();
    {
        let _serial = state.persistence_preparation.lock().await;
        state
            .stage_continuity(&scope, Some(unexpected), None, None)
            .unwrap();
    }
    let report = state.request_persistence().await.unwrap();
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(
        report.canonical_state,
        crate::persist::PersistComponentOutcome::Failed(
            crate::persist::PersistErrorCategory::SnapshotEncode
        )
    );
    assert_eq!(
        report.wdbx_projection,
        crate::persist::PersistComponentOutcome::SkippedCanonicalFailure
    );
    assert_eq!(std::fs::read(Stores::state_path(&dir.0)).unwrap(), before);
    assert_eq!(
        std::fs::read(Stores::wdbx_path(&dir.0)).unwrap(),
        projection
    );
    assert!(state.continuity_generation(&scope).is_none());
}
#[tokio::test]
async fn continuity_member_erase_preserves_unrelated_pending_target_and_fence() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let access = WorkAccess {
        actor: 8,
        channel: 80,
        guild: Some(1),
        can_view: true,
        can_manage: true,
    };
    let card = {
        let mut stores = AppState::lock(&state.stores);
        stores
            .work
            .create_project(access, "Other project", "other-project")
            .unwrap();
        let mut registry = ProposalRegistry::new([8; 16]);
        let proposal = registry
            .propose(
                ContinuityDraft {
                    scope: access.scope(),
                    base_revision: 0,
                    presented_text: "UNRELATED_MEMBER_CARD".into(),
                    source_refs: BTreeSet::new(),
                },
                &access,
                &stores.work,
                now(),
            )
            .unwrap();
        let grant = registry
            .resolve_confirmation(proposal.id, 8, &access.scope(), now())
            .unwrap();
        let work = stores.work.clone();
        stores
            .continuity
            .confirm(grant, &access, &work, now())
            .unwrap()
    };
    state.request_persistence().await.unwrap();
    {
        let _serial = state.persistence_preparation.lock().await;
        state
            .stage_continuity(&card.scope, Some(card.clone()), None, None)
            .unwrap();
        assert_eq!(
            state
                .erase_continuity_owned("discord:1", Some(7))
                .await
                .unwrap(),
            0
        );
    }
    writer.stop();
    writer.joined().await.unwrap();
    let safety = AppState::lock(&state.continuity_safety);
    assert!(
        safety.pending.contains_key(&card.scope),
        "erasing another member must not settle this target"
    );
    assert!(safety.blocked.contains(&card.scope));
}
#[tokio::test]
async fn continuity_failed_preparation_cannot_settle_equal_disk_bytes() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    state.request_persistence().await.unwrap();
    let scope = WorkScope::Personal { owner: 7 };
    {
        let _serial = state.persistence_preparation.lock().await;
        state.stage_continuity(&scope, None, None, None).unwrap();
        let mut snapshot = state.snapshot_without_proposals();
        snapshot.stores.canonical_preparation_failed = true;
        assert_eq!(
            state.publish_continuity(snapshot, &scope).await,
            Err(WorkError::Persistence)
        );
    }
    writer.stop();
    writer.joined().await.unwrap();
    let safety = AppState::lock(&state.continuity_safety);
    assert!(
        safety.pending.contains_key(&scope),
        "a refused snapshot cannot retire recovery metadata through equal bytes"
    );
    assert!(safety.blocked.contains(&scope));
}
