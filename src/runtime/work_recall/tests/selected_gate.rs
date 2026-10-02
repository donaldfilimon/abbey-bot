//! Offline real gate selection, ABI protocol and cumulative submission evidence.
use super::*;
use std::os::unix::fs::PermissionsExt;

struct Transport(std::path::PathBuf);
impl Transport {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-recall-transport-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("abi");
        std::fs::write(&program, r#"#!/bin/sh
base=$(dirname "$0")
cat "$4" >> "$base/writes"
printf '\n' >> "$base/writes"
mode=$(cat "$base/mode")
case "$mode" in
 reject) echo 'FailedPrecondition: episode_storage_budget_exhausted' >&2; exit 1 ;;
 unknown) echo 'ambiguous response'; exit 0 ;;
 *) echo '{"decision":"appended","episode_digest":"abababababababababababababababababababababababababababababababab","sequence":"1"}' ;;
esac
"#).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let this = Self(dir);
        this.mode("append");
        this
    }
    fn mode(&self, mode: &str) {
        std::fs::write(self.0.join("mode"), mode).unwrap();
    }
    fn gate(&self, scopes: &[&str]) -> Arc<crate::episode_gate::EpisodeGate> {
        Arc::new(crate::episode_gate::EpisodeGate::new(
            crate::episode_gate::EpisodeGateConfig::from_json(
                &serde_json::json!({
                    "abi_cli": self.0.join("abi"), "token_file": self.0.join("unused-token"),
                    "endpoint": "http://127.0.0.1:1", "policy_version": "policy_v1",
                    "contract_revision": 2, "contract_digest": "01".repeat(32),
                    "timeout_secs": 5, "guilds": scopes,
                })
                .to_string(),
            )
            .unwrap(),
        ))
    }
    fn writes(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.0.join("writes"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn team(actor: u64, guild: u64) -> WorkAccess {
    WorkAccess {
        actor,
        guild: Some(guild),
        channel: 500,
        can_view: true,
        can_manage: true,
    }
}
fn configured(
    gate: Arc<crate::episode_gate::EpisodeGate>,
) -> (
    Arc<AppState>,
    ServiceSupervisor,
    crate::service::persistence::PersistenceWriter,
) {
    let sink = Sink::isolated();
    let mut state = AppState::in_memory_with_persistence(sink.directory.clone(), sink);
    Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = state.attach_service(supervisor.operations());
    (state, supervisor, writer)
}
async fn decision(state: &AppState, access: WorkAccess, name: &'static str) -> WorkSourceKey {
    state
        .commit_work(move |s| {
            let project = s.create_project(access, name, name)?;
            let id = s.record_decision(project, access, "Use rust", 1, name)?;
            Ok(WorkSourceKey::Decision { project, id })
        })
        .await
        .unwrap()
}
async fn task(state: &AppState, access: WorkAccess, name: &'static str) -> WorkSourceKey {
    state
        .commit_work(move |s| {
            let project = s.create_project(access, name, name)?;
            let id = s.add_task(
                access,
                WorkTask {
                    id: 0,
                    project_id: project,
                    title: "Use rust".into(),
                    owner: 0,
                    assignee: None,
                    goal_id: None,
                    priority: 2,
                    status: WorkStatus::Open,
                    due_at: None,
                    remind_at: None,
                    reminder_revision: 0,
                    snoozed_until: None,
                    source: None,
                    github: None,
                    revision: 0,
                },
                name,
            )?;
            Ok(WorkSourceKey::Task { project, id })
        })
        .await
        .unwrap()
}
async fn admit(state: &AppState, key: &WorkSourceKey, access: WorkAccess) -> AdmissionResult {
    state
        .admit_work_recall(
            key.clone(),
            access,
            3,
            &std::collections::BTreeSet::from([access.scope()]),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn selected_gate_covers_exact_scopes_and_preserves_cumulative_outcomes() {
    let transport = Transport::new();
    let gate = transport.gate(&["discord:dm:1", "discord:42"]);
    let (state, supervisor, writer) = configured(gate.clone());
    let personal = decision(&state, access(), "personal").await;
    let shared = decision(&state, team(1, 42), "team").await;
    let sibling = decision(&state, team(1, 43), "uncovered").await;
    let personal_sibling = task(
        &state,
        WorkAccess {
            actor: 2,
            ..access()
        },
        "personal-uncovered",
    )
    .await;
    for (key, a) in [
        (&personal, access()),
        (&shared, team(1, 42)),
        (&sibling, team(1, 43)),
        (
            &personal_sibling,
            WorkAccess {
                actor: 2,
                ..access()
            },
        ),
    ] {
        assert!(matches!(
            admit(&state, key, a).await,
            AdmissionResult::Committed { .. }
        ));
    }
    let writes = transport.writes();
    assert_eq!(writes.len(), 2);
    for (write, expected, member) in [
        (&writes[0], "discord-dm-1", true),
        (&writes[1], "discord-42", false),
    ] {
        assert_eq!(write["guild_ref"], expected);
        assert_eq!(write["source_type"], "discord_guild");
        assert_eq!(write["token_cost"], 1);
        let c = &write["event"]["candidate"];
        assert_eq!(c["member_scoped"], member);
        assert_eq!(c["class"], "fact");
        assert_eq!(c["retention"], "durable");
        assert!(c["payload_bytes"].as_u64().unwrap() > 0);
    }
    transport.mode("reject");
    let refused = decision(&state, access(), "refused").await;
    assert_eq!(
        admit(&state, &refused, access()).await,
        AdmissionResult::Rejected
    );
    transport.mode("unknown");
    let unknown = decision(&state, access(), "unknown").await;
    assert_eq!(
        admit(&state, &unknown, access()).await,
        AdmissionResult::Unknown
    );
    assert!(
        state
            .admit_work_recall(
                unknown.clone(),
                access(),
                4,
                &std::collections::BTreeSet::from([access().scope()])
            )
            .await
            .is_err()
    );
    let counters = gate.counters();
    assert_eq!(
        (counters.appended, counters.rejected, counters.unavailable),
        (2, 1, 1)
    );
    assert_eq!(transport.writes().len(), 4);
    assert_eq!(
        transport
            .writes()
            .iter()
            .map(|w| w["token_cost"].as_u64().unwrap())
            .sum::<u64>(),
        4
    );
    // Refusing cleanup leaves an honest partial status after durable disable.
    transport.mode("reject");
    let status = state
        .forget_work_recall(shared.clone(), team(1, 42), 5)
        .await
        .unwrap();
    assert_eq!(status.retained_rows, 1);
    assert_eq!(status.unresolved, 0);
    assert!(!AppState::lock(&state.stores).work.recall.source_versions[&shared].recall_enabled);
    let writes = transport.writes();
    assert_eq!(writes.len(), 5);
    assert_eq!(writes[4]["event"]["candidate"]["payload_bytes"], 0);
    assert_eq!(
        writes[4]["event"]["candidate"]["forgets"],
        serde_json::json!([0xab; 32].to_vec())
    );
    let bytes: u64 = writes
        .iter()
        .map(|w| w["event"]["candidate"]["payload_bytes"].as_u64().unwrap())
        .sum();
    assert!(bytes > 0); // submissions remain cumulative, with no refund/replay entry
    let canonical = AppState::lock(&state.stores).clone();
    stop(supervisor, writer).await;

    // Simulated restart with expanded exact coverage; loading does not propose.
    let covered = transport.gate(&["discord:43", "discord:dm:2"]);
    let (state, supervisor, writer) = configured(covered.clone());
    *AppState::lock(&state.stores) =
        serde_json::from_slice(&serde_json::to_vec(&canonical).unwrap()).unwrap();
    transport.mode("append");
    let status = state
        .forget_work_recall(sibling, team(1, 43), 6)
        .await
        .unwrap();
    assert_eq!(status.retained_rows, 0);
    assert_eq!(covered.counters().ungated_forgets, 1);
    assert_eq!(transport.writes().len(), 5); // no invented ledger edge for uncovered row
    // A changed uncovered source now proposes a normal covered candidate.
    let WorkSourceKey::Task { project, id } = personal_sibling.clone() else {
        unreachable!()
    };
    state
        .commit_work(move |s| {
            let revision = s.tasks[&id].revision;
            s.update_task(
                WorkAccess {
                    actor: 2,
                    ..access()
                },
                id,
                revision,
                WorkStatus::Done,
                None,
            )
        })
        .await
        .unwrap();
    assert!(matches!(
        admit(
            &state,
            &personal_sibling,
            WorkAccess {
                actor: 2,
                ..access()
            }
        )
        .await,
        AdmissionResult::Committed { .. }
    ));
    assert_eq!(covered.counters().ungated_forgets, 2);
    let writes = transport.writes();
    assert_eq!(writes.len(), 6);
    assert!(writes[5]["event"]["candidate"]["supersedes"].is_null());
    assert_eq!(writes[5]["guild_ref"], "discord-dm-2");
    assert_eq!(
        state
            .recall_project(
                WorkAccess {
                    actor: 2,
                    ..access()
                },
                project,
                2,
                "rust"
            )
            .unwrap()
            .evidence
            .len(),
        1
    );
    stop(supervisor, writer).await;
}

#[tokio::test]
async fn source_management_is_project_specific_in_a_shared_channel() {
    let transport = Transport::new();
    let (state, supervisor, writer) = configured(transport.gate(&["discord:42"]));
    let a = team(1, 42);
    let b = team(2, 42);
    let owned = decision(&state, a, "owned").await;
    let other = decision(&state, b, "other").await;
    assert!(matches!(
        admit(&state, &owned, a).await,
        AdmissionResult::Committed { .. }
    ));
    assert!(
        state
            .admit_work_recall(
                other.clone(),
                a,
                3,
                &std::collections::BTreeSet::from([a.scope()])
            )
            .await
            .is_err()
    );
    assert!(state.work_recall_status(&other, a).is_err());
    assert!(state.forget_work_recall(other.clone(), a, 4).await.is_err());
    transport.mode("reject");
    assert_eq!(
        state
            .forget_work_recall(owned.clone(), a, 4)
            .await
            .unwrap()
            .retained_rows,
        1
    );
    assert_eq!(
        state.work_recall_status(&owned, a).unwrap().retained_rows,
        1
    );
    transport.mode("append");
    assert_eq!(
        state
            .forget_work_recall(owned.clone(), a, 5)
            .await
            .unwrap()
            .retained_rows,
        0
    );
    assert_eq!(
        state
            .forget_work_recall(owned, a, 6)
            .await
            .unwrap()
            .retained_rows,
        0
    );
    assert!(state.work_recall_status(&other, a).is_err());
    stop(supervisor, writer).await;
}
