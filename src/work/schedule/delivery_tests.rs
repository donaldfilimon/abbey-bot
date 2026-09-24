//! Cross-project/private delivery and revision coverage regressions.
use super::*;
use crate::work::tests::{personal, task, team};

const NOW: u64 = 1_790_265_600; // Injected timestamp; test policies use UTC and no quiet hours.

fn setup(private: bool) -> (WorkStore, WorkAccess, WorkScope, u64, u64) {
    let mut store = WorkStore::default();
    let access = team(1, 99, true, true);
    let a = store.create_project(access, "A", "a").unwrap();
    let b = store.create_project(access, "B", "b").unwrap();
    let scope = access.scope();
    store
        .configure_automation(
            &scope,
            access,
            WorkAutomationPolicy {
                enabled: true,
                destination: Some(99),
                delivery_target: Some(if private {
                    WorkDestination::TeamPrivate { principal: 1 }
                } else {
                    WorkDestination::TeamChannel { channel: 99 }
                }),
                timezone: "UTC".into(),
                quiet_start: 0,
                quiet_end: 0,
                briefing_hour: 0,
                ..Default::default()
            },
        )
        .unwrap();
    (store, access, scope, a, b)
}

fn reserve(store: &mut WorkStore, access: WorkAccess, batch: &WorkBatch, now: u64) -> u64 {
    match batch.target {
        WorkDestination::TeamPrivate { .. } => store
            .reserve_private_batch(access, batch, 777, now)
            .unwrap(),
        _ => store.reserve_batch(access, batch, now).unwrap(),
    }
}

#[test]
fn exact_sources_exclude_authorized_empty_a_and_include_both_when_rendered() {
    let (mut store, access, scope, a, b) = setup(true);
    let b_task = store.add_task(access, task(b), "btask").unwrap();
    let batch = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    assert_eq!(batch.provenance.contributing_projects, BTreeSet::from([b]));
    assert_eq!(batch.task_ids, vec![b_task]);
    assert!(batch.rendered_body.contains(&format!("Task #{b_task}")));
    let receipt = reserve(&mut store, access, &batch, NOW);
    assert_eq!(store.deliveries[&receipt].project_id, a); // Legacy anchor only.
    assert_eq!(store.deliveries[&receipt].recipient, 777);
    assert_eq!(store.deliveries[&receipt].scope, Some(scope.clone()));
    store.add_task(access, task(a), "atask").unwrap();
    let batch = store
        .next_batch(&scope, access, NOW + 86_400)
        .unwrap()
        .unwrap();
    assert_eq!(
        batch.provenance.contributing_projects,
        BTreeSet::from([a, b])
    );
    assert_eq!(batch.provenance.source_refs.len(), 2);
    println!("Private frozen body:\n{}", batch.rendered_body);
}

#[test]
fn private_feedback_requires_original_team_facts_and_exact_recipient() {
    let (mut store, access, scope, a, b) = setup(true);
    store.add_task(access, task(b), "task").unwrap();
    for p in [a, b] {
        store.set_member(p, access, 2, true).unwrap();
    }
    let batch = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    let id = reserve(&mut store, access, &batch, NOW);
    let receipt = store.deliveries.get_mut(&id).unwrap();
    receipt.state = DeliveryState::Sent;
    receipt.message_id = Some(8);
    assert_eq!(
        store.feedback(
            team(2, 99, true, true),
            id,
            Some(WorkFeedback::Useful),
            false,
            NOW
        ),
        Err(WorkError::Denied)
    );
    let dm = WorkAccess {
        channel: 777,
        ..personal(1)
    };
    assert!(
        store
            .feedback(dm, id, Some(WorkFeedback::Useful), false, NOW)
            .is_err()
    );
    store
        .feedback(access, id, Some(WorkFeedback::Useful), false, NOW)
        .unwrap();
    store.feedback(access, id, None, true, NOW).unwrap();
    store
        .feedback(access, id, Some(WorkFeedback::Useful), false, NOW)
        .unwrap();
    assert!(store.preferences[&scope.key()].evidence.is_empty());
    store.projects.get_mut(&a).unwrap().members.remove(&1);
    assert_eq!(
        store.feedback(access, id, Some(WorkFeedback::Useful), false, NOW),
        Err(WorkError::Denied)
    );
}

#[test]
fn recipient_policy_and_dm_changes_do_not_reset_scope_quota_or_occurrences() {
    let (mut store, access, scope, a, b) = setup(true);
    let id = store.add_task(access, task(b), "task").unwrap();
    store.set_reminder(access, id, 0, Some(NOW)).unwrap();
    let batch = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    reserve(&mut store, access, &batch, NOW);
    for p in [a, b] {
        let project = store.projects.get_mut(&p).unwrap();
        project.members.insert(2);
        project.managers.insert(2);
    }
    let new = team(2, 99, true, true);
    let old_revision = store.scope_automation[&scope.key()].revision;
    store
        .update_automation(
            new,
            WorkAutomationUpdate {
                enabled: true,
                private_to_me: Some(true),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(
        store.scope_automation[&scope.key()].revision,
        old_revision + 1
    );
    assert_eq!(store.next_batch(&scope, new, NOW), Ok(None));
    let revision = store.tasks[&id].revision;
    store
        .update_task(new, id, revision, WorkStatus::Done, None)
        .unwrap();
    let changed = store.next_batch(&scope, new, NOW + 1).unwrap().unwrap();
    assert_eq!(changed.kind, WorkDeliveryKind::Changes);
    store
        .reserve_private_batch(new, &changed, 888, NOW + 1)
        .unwrap();
    // Lowering the ceiling charges both subscribers' receipts.
    store
        .update_automation(
            new,
            WorkAutomationUpdate {
                enabled: true,
                daily_limit: Some(2),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    store
        .record_decision(a, new, "Choose the new implementation", NOW, "decision")
        .unwrap();
    assert_eq!(store.next_batch(&scope, new, NOW + 2), Ok(None));
    let roundtrip: WorkStore =
        serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    assert_eq!(roundtrip.next_batch(&scope, new, NOW + 2), Ok(None));
}

#[test]
fn self_opt_in_only_and_membership_or_manager_loss_prevents_reservation() {
    let (mut store, access, scope, a, b) = setup(false);
    assert!(matches!(
        store.scope_automation[&scope.key()].delivery_target,
        Some(WorkDestination::TeamChannel { .. })
    ));
    let mut bad = store.scope_automation[&scope.key()].clone();
    bad.delivery_target = Some(WorkDestination::TeamPrivate { principal: 2 });
    assert_eq!(
        store.configure_automation(&scope, access, bad),
        Err(WorkError::Denied)
    );
    store
        .update_automation(
            access,
            WorkAutomationUpdate {
                enabled: true,
                private_to_me: Some(true),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    store.add_task(access, task(b), "task").unwrap();
    let batch = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    assert_eq!(
        store.reserve_batch(access, &batch, NOW),
        Err(WorkError::Denied)
    );
    assert_eq!(
        store.reserve_private_batch(access, &batch, 99, NOW),
        Err(WorkError::Denied)
    );
    for member_loss in [false, true] {
        let mut denied = store.clone();
        let project = denied.projects.get_mut(&a).unwrap();
        if member_loss {
            project.members.remove(&1);
        } else {
            project.managers.remove(&1);
        }
        assert_eq!(
            denied.reserve_private_batch(access, &batch, 777, NOW),
            Err(WorkError::Denied)
        );
        assert!(denied.deliveries.is_empty());
    }
    store
        .update_automation(access, WorkAutomationUpdate::default(), None)
        .unwrap();
    assert_eq!(
        store.reserve_private_batch(access, &batch, 777, NOW),
        Err(WorkError::Stale)
    );
}

#[test]
fn history_is_seeded_then_terminal_tasks_and_decisions_consolidate_and_survive_reload() {
    let mut store = WorkStore::default();
    let access = team(1, 99, true, true);
    let project = store.create_project(access, "A", "a").unwrap();
    let id = store.add_task(access, task(project), "task").unwrap();
    store
        .update_task(access, id, 0, WorkStatus::Done, None)
        .unwrap();
    store
        .record_decision(project, access, "Historical decision", 1, "old")
        .unwrap();
    let scope = access.scope();
    store
        .update_automation(
            access,
            WorkAutomationUpdate {
                enabled: true,
                private_to_me: Some(true),
                timezone: Some("UTC".into()),
                quiet_start: Some(0),
                quiet_end: Some(0),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(store.next_batch(&scope, access, NOW), Ok(None));
    store
        .update_task(access, id, 1, WorkStatus::Open, None)
        .unwrap();
    store
        .update_task(access, id, 2, WorkStatus::Cancelled, None)
        .unwrap();
    let decision = store
        .record_decision(project, access, "New choice", NOW, "new")
        .unwrap();
    let mut store: WorkStore =
        serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    let batch = store
        .next_batch(&scope, access, NOW + 3 * 86_400)
        .unwrap()
        .unwrap();
    assert_eq!(batch.kind, WorkDeliveryKind::Changes);
    assert_eq!(batch.content_refs.len(), 2);
    assert!(batch.content_refs.contains(&WorkContentRef::Task {
        project,
        id,
        revision: 3
    }));
    assert!(batch.content_refs.contains(&WorkContentRef::Decision {
        project,
        id: decision,
        revision: 1
    }));
    assert!(batch.rendered_body.contains("cancelled"));
    reserve(&mut store, access, &batch, NOW + 3 * 86_400);
    assert_eq!(store.next_batch(&scope, access, NOW + 3 * 86_400), Ok(None));
    // Legacy missing coverage seeds at load, without announcing old decisions.
    let mut json = serde_json::to_value(&store).unwrap();
    json.as_object_mut().unwrap().remove("change_coverage");
    json.as_object_mut().unwrap().remove("change_fingerprints");
    json["deliveries"] = serde_json::json!({});
    let legacy: WorkStore = serde_json::from_value(json).unwrap();
    assert_eq!(
        legacy.next_batch(&scope, access, NOW + 4 * 86_400),
        Ok(None)
    );
}

#[test]
fn bounded_rendering_covers_only_included_content_and_digest_is_exact() {
    use sha2::{Digest, Sha256};
    let (mut store, access, scope, a, _) = setup(true);
    for i in 0..12 {
        store
            .record_decision(a, access, &format!("Decision {i}"), NOW, &format!("d{i}"))
            .unwrap();
    }
    let batch = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    assert_eq!(batch.content_refs.len(), 8);
    let digest = Sha256::digest(batch.rendered_body.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(batch.provenance.rendered_digest, digest);
    reserve(&mut store, access, &batch, NOW);
    let next = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    assert_eq!(next.content_refs.len(), 4);
    assert!(next.content_refs.is_disjoint(&batch.content_refs));
    reserve(&mut store, access, &next, NOW);
    assert_eq!(store.next_batch(&scope, access, NOW), Ok(None));
}

#[test]
fn malformed_provenance_or_policy_mismatch_is_rejected_on_load() {
    let (mut store, access, scope, _, b) = setup(true);
    store.add_task(access, task(b), "task").unwrap();
    let batch = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    let id = reserve(&mut store, access, &batch, NOW);
    let mut value = serde_json::to_value(&store).unwrap();
    value["deliveries"][id.to_string()]["provenance"]["contributing_projects"] =
        serde_json::json!([]);
    assert!(serde_json::from_value::<WorkStore>(value).is_err());
    let mut value = serde_json::to_value(&store).unwrap();
    value["scope_automation"][scope.key()]["delivery_target"] =
        serde_json::json!({"Personal": {"principal": 1}});
    assert!(serde_json::from_value::<WorkStore>(value).is_err());
    let mut value = serde_json::to_value(&store).unwrap();
    value["scope_automation"][scope.key()]["destination"] = serde_json::json!(100);
    assert!(serde_json::from_value::<WorkStore>(value).is_err());
    let mut value = serde_json::to_value(&store).unwrap();
    value["deliveries"][id.to_string()]["recipient"] = serde_json::json!(99);
    assert!(serde_json::from_value::<WorkStore>(value).is_err());
    let mut legacy = serde_json::to_value(&store).unwrap();
    legacy["scope_automation"][scope.key()]
        .as_object_mut()
        .unwrap()
        .remove("delivery_target");
    legacy["deliveries"][id.to_string()]
        .as_object_mut()
        .unwrap()
        .remove("provenance");
    let legacy: WorkStore = serde_json::from_value(legacy).unwrap();
    assert!(legacy.deliveries[&id].provenance.is_none());
    let batch = legacy
        .next_batch(&scope, access, NOW + 86_400)
        .unwrap()
        .unwrap();
    assert!(matches!(batch.target, WorkDestination::TeamChannel { .. }));
}

#[test]
fn learned_reduction_suppresses_only_optional_changes() {
    let (mut store, access, scope, a, _) = setup(true);
    let evidence = (1..=5)
        .map(|id| PreferenceEvidence {
            actor: Some(1),
            scope: Some(scope.clone()),
            kind: Some(WorkDeliveryKind::Changes),
            delivery_id: id,
            feedback: WorkFeedback::Dismissed,
            at: NOW,
        })
        .collect();
    store.preferences.insert(
        scope.key(),
        WorkPreferenceProfile {
            evidence,
            ..Default::default()
        },
    );
    store
        .record_decision(a, access, "Optional work update", NOW, "decision")
        .unwrap();
    assert!(store.preference_profile(access).unwrap().reduce_followups);
    assert_eq!(store.next_batch(&scope, access, NOW), Ok(None));
    let id = store.add_task(access, task(a), "task").unwrap();
    let briefing = store.next_batch(&scope, access, NOW).unwrap().unwrap();
    assert_eq!(briefing.kind, WorkDeliveryKind::Briefing);
    reserve(&mut store, access, &briefing, NOW);
    store.set_reminder(access, id, 0, Some(NOW + 1)).unwrap();
    let reminder = store.next_batch(&scope, access, NOW + 1).unwrap().unwrap();
    assert_eq!(reminder.kind, WorkDeliveryKind::Reminder);
    reserve(&mut store, access, &reminder, NOW + 1);
    assert_eq!(store.next_batch(&scope, access, NOW + 2), Ok(None));
    store
        .control_preferences(access, Some(false), None, false)
        .unwrap();
    assert_eq!(
        store
            .next_batch(&scope, access, NOW + 2)
            .unwrap()
            .unwrap()
            .kind,
        WorkDeliveryKind::Changes
    );
}

#[test]
fn policy_revision_is_monotonic_and_overflow_is_atomic() {
    let (mut store, access, scope, _, _) = setup(true);
    let policy = store.scope_automation[&scope.key()].clone();
    store
        .configure_automation(&scope, access, policy.clone())
        .unwrap();
    assert_eq!(
        store.scope_automation[&scope.key()].revision,
        policy.revision
    );
    store
        .scope_automation
        .get_mut(&scope.key())
        .unwrap()
        .revision = u64::MAX;
    let before = store.clone();
    assert_eq!(
        store.update_automation(access, WorkAutomationUpdate::default(), None),
        Err(WorkError::Full)
    );
    assert_eq!(store, before);
}

#[test]
fn saving_disabled_settings_does_not_turn_pre_enable_history_into_changes() {
    let (mut store, access, scope, a, _) = setup(true);
    store.scope_automation.clear();
    store.scope_automation_actors.clear();
    store.change_coverage.clear();
    store.change_fingerprints.clear();
    store
        .update_automation(
            access,
            WorkAutomationUpdate {
                timezone: Some("UTC".into()),
                private_to_me: Some(true),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    store
        .record_decision(a, access, "Before first enabling", NOW, "old")
        .unwrap();
    store
        .update_automation(
            access,
            WorkAutomationUpdate {
                enabled: true,
                quiet_start: Some(0),
                quiet_end: Some(0),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(store.next_batch(&scope, access, NOW), Ok(None));
    store
        .record_decision(a, access, "After enabling", NOW, "new")
        .unwrap();
    assert_eq!(
        store
            .next_batch(&scope, access, NOW)
            .unwrap()
            .unwrap()
            .content_refs
            .len(),
        1
    );
}
