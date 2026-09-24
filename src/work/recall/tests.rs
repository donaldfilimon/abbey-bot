use super::*;
use crate::work::tests::{personal, task, team};

fn setup(access: WorkAccess) -> (WorkStore, u64, WorkSourceKey) {
    let mut s = WorkStore::default();
    let project = s.create_project(access, "Work", "p").unwrap();
    let id = s.add_task(access, task(project), "t").unwrap();
    (s, project, WorkSourceKey::Task { project, id })
}
fn add(s: &mut WorkStore, key: &WorkSourceKey, a: WorkAccess) -> u64 {
    let (attempt, payload) = s.prepare_recall(key, a, 10, 1, "a".repeat(64)).unwrap();
    let scope = payload.scope.recall_gate_scope().0;
    s.recall
        .settle_add(
            attempt,
            payload,
            WorkAdmission::Appended {
                digest_hex: "b".repeat(64),
                scoped_guild: scope,
            },
            11,
        )
        .unwrap()
}
fn eligible(s: &WorkStore, a: WorkAccess, project: u64) -> BTreeSet<u64> {
    s.eligible_recall_ids(a, project, RecallAudience::Private { principal: a.actor })
        .unwrap()
}
fn reload(s: &WorkStore) -> WorkStore {
    serde_json::from_slice(&serde_json::to_vec(s).unwrap()).unwrap()
}
fn feedback_source(
    s: &mut WorkStore,
    a: WorkAccess,
    projects: &[u64],
    delivery: u64,
) -> WorkSourceKey {
    let refs: Vec<_> = s
        .content_refs(&a.scope())
        .into_iter()
        .filter(|r| match r {
            WorkContentRef::Task { project, .. } | WorkContentRef::Decision { project, .. } => {
                projects.contains(project)
            }
        })
        .collect();
    let (_, provenance) = s.render_delivery(&refs, 0).unwrap();
    let task_ids = refs
        .iter()
        .filter_map(|r| {
            if let WorkContentRef::Task { id, .. } = r {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    s.deliveries.insert(
        delivery,
        WorkDeliveryReceipt {
            id: delivery,
            project_id: *s.projects.keys().next().unwrap(),
            recipient: a.channel,
            destination: None,
            policy_revision: 0,
            provenance: Some(provenance),
            local_day: "2026-09-24".into(),
            at: 1,
            state: DeliveryState::Sent,
            message_id: Some(delivery),
            scope: Some(a.scope()),
            kind: Some(WorkDeliveryKind::Briefing),
            coverage: vec![],
            task_ids,
            dedupe_keys: BTreeSet::new(),
        },
    );
    s.feedback(a, delivery, Some(WorkFeedback::Useful), false, 2)
        .unwrap();
    WorkSourceKey::Preference {
        scope: a.scope(),
        delivery,
        actor: a.actor,
    }
}

#[test]
fn owner_scope_and_audience_filter_before_scoring() {
    let a = personal(1);
    let (mut s, p, key) = setup(a);
    let row = add(&mut s, &key, a);
    assert_eq!(eligible(&s, a, p), BTreeSet::from([row]));
    assert!(
        s.eligible_recall_ids(personal(2), p, RecallAudience::Private { principal: 2 })
            .is_err()
    );
    assert!(
        s.eligible_recall_ids(a, p, RecallAudience::Private { principal: 2 })
            .is_err()
    );
    assert!(
        s.eligible_recall_ids(a, p, RecallAudience::Channel)
            .is_err()
    );
    assert_eq!(a.scope().recall_gate_scope(), ("discord:dm:1".into(), true));
    assert_eq!(
        team(1, 99, true, true).scope().recall_gate_scope(),
        ("discord:9".into(), false)
    );
}
#[test]
fn native_task_edit_reminder_and_github_invalidate_without_removing_receipts() {
    let a = personal(1);
    let (mut s, p, key) = setup(a);
    let WorkSourceKey::Task { id, .. } = key else {
        panic!()
    };
    let first = add(&mut s, &key, a);
    s.update_task(a, id, 0, WorkStatus::Done, None).unwrap();
    assert!(eligible(&s, a, p).is_empty());
    assert!(s.recall.records.contains_key(&first));
    add(&mut s, &key, a);
    s.set_reminder(a, id, 1, Some(1_800_000_000)).unwrap();
    assert!(eligible(&reload(&s), a, p).is_empty());
    assert_eq!(s.recall.source_versions[&key].revision, 2);
}
#[test]
fn provenance_second_project_and_whole_multi_project_identity() {
    let a = team(1, 99, true, true);
    let (mut s, p1, _) = setup(a);
    let p2 = s.create_project(a, "Second", "p2").unwrap();
    s.add_task(a, task(p2), "t2").unwrap();
    let key = feedback_source(&mut s, a, &[p2], 500);
    let row = add(&mut s, &key, a);
    assert!(eligible(&s, a, p1).is_empty());
    assert_eq!(eligible(&s, a, p2), BTreeSet::from([row]));
    let key2 = feedback_source(&mut s, a, &[p1, p2], 501);
    let row2 = add(&mut s, &key2, a);
    assert_eq!(eligible(&s, a, p1), BTreeSet::from([row2]));
    assert_eq!(eligible(&s, a, p2), BTreeSet::from([row, row2]));
    s.set_member(p1, a, 2, true).unwrap();
    let other = team(2, 99, true, false);
    assert!(eligible(&s, other, p1).is_empty());
    s.set_member(p2, a, 2, true).unwrap();
    assert_eq!(eligible(&s, other, p1), BTreeSet::from([row2]));
    s.set_member(p2, a, 2, false).unwrap();
    assert!(eligible(&s, other, p1).is_empty());
    assert!(s.recall_candidate(&key2, team(1, 100, true, true)).is_err());
    let mut wrong_guild = a;
    wrong_guild.guild = Some(10);
    assert!(s.recall_candidate(&key2, wrong_guild).is_err());
    println!("{}", s.recall.records[&row2].payload.text);
}
#[test]
fn learning_disable_reenable_reset_and_corrections_preserve_native_replay() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    let row = add(&mut s, &key, a);
    s.control_preferences(a, Some(false), None, false).unwrap();
    assert!(eligible(&s, a, p).is_empty());
    s.control_preferences(a, Some(true), None, false).unwrap();
    assert_eq!(eligible(&s, a, p), BTreeSet::from([row]));
    s.feedback(a, 500, Some(WorkFeedback::Dismissed), true, 3)
        .unwrap();
    assert!(eligible(&s, a, p).is_empty());
    let second = add(&mut s, &key, a);
    assert_ne!(row, second);
    s.control_preferences(a, None, None, true).unwrap();
    assert!(eligible(&reload(&s), a, p).is_empty());
    assert_eq!(s.recall.records.len(), 2);
    assert_eq!(s.recall.scope_controls[&a.scope()].generation, 1);
    s.feedback(a, 500, Some(WorkFeedback::Useful), false, 4)
        .unwrap();
    assert!(s.preferences[&a.scope().key()].evidence.is_empty());
    assert!(
        s.preferences[&a.scope().key()]
            .observed_deliveries
            .contains(&(500, 1))
    );
}
#[test]
fn native_removal_and_window_eviction_retire_rows_and_keep_tombstones() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let removed = feedback_source(&mut s, a, &[p], 500);
    add(&mut s, &removed, a);
    s.feedback(a, 500, None, true, 3).unwrap();
    assert!(eligible(&s, a, p).is_empty());
    let old = feedback_source(&mut s, a, &[p], 501);
    add(&mut s, &old, a);
    for id in 502..602 {
        feedback_source(&mut s, a, &[p], id);
    }
    assert!(eligible(&reload(&s), a, p).is_empty());
    assert_eq!(s.preferences[&a.scope().key()].evidence.len(), 100);
    assert_eq!(s.recall.source_versions[&old].revision, 2);
    s.feedback(a, 501, Some(WorkFeedback::Useful), false, 4)
        .unwrap();
    assert!(
        !s.preferences[&a.scope().key()]
            .evidence
            .iter()
            .any(|e| e.delivery_id == 501)
    );
}
#[test]
fn legacy_and_corrupt_provenance_are_never_backfilled() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    s.deliveries.get_mut(&500).unwrap().provenance = None;
    assert!(s.recall_candidate(&key, a).is_err());
    let mut json = serde_json::to_value(&s).unwrap();
    json.as_object_mut().unwrap().remove("recall");
    let legacy: WorkStore = serde_json::from_value(json).unwrap();
    assert!(legacy.recall.records.is_empty());
    assert!(legacy.recall.attempts.is_empty());
    assert!(legacy.recall_candidate(&key, a).is_err());
}
#[test]
fn loading_rejects_duplicate_typed_keys_unknown_schema_and_wrong_admission_scope() {
    let a = personal(1);
    let (mut s, _, key) = setup(a);
    add(&mut s, &key, a);
    let json = serde_json::to_value(&s).unwrap();
    let mut duplicate = json.clone();
    let entries = duplicate["recall"]["source_versions"]
        .as_array_mut()
        .unwrap();
    entries.push(entries[0].clone());
    assert!(serde_json::from_value::<WorkStore>(duplicate).is_err());
    let mut unknown = json.clone();
    unknown["recall"]["schema_version"] = 99.into();
    assert!(serde_json::from_value::<WorkStore>(unknown).is_err());
    let mut receipt = json;
    receipt["recall"]["records"][0]["value"]["admission"]["Appended"]["scoped_guild"] =
        "discord:1".into();
    assert!(serde_json::from_value::<WorkStore>(receipt).is_err());
}
#[test]
fn prepared_recovers_unknown_with_reservation_and_no_candidate_text() {
    let a = personal(1);
    let (mut s, _, key) = setup(a);
    let (attempt, payload) = s.prepare_recall(&key, a, 10, 42, "a".repeat(64)).unwrap();
    let encoded = serde_json::to_string(&s.recall.attempts).unwrap();
    assert!(!encoded.contains(&payload.text));
    assert!(!encoded.contains("Ship the release"));
    let mut loaded = reload(&s);
    assert_eq!(
        loaded.recall.attempts[&attempt].state,
        AttemptState::Unknown
    );
    assert!(
        loaded
            .prepare_recall(&key, a, 11, 43, "a".repeat(64))
            .is_err()
    );
    assert!(loaded.recall.records.is_empty());
    loaded.recall.reject(attempt, 12).unwrap();
    let next = add(&mut loaded, &key, a);
    assert!(next > attempt);
}
#[test]
fn delayed_appended_after_reset_is_retained_but_ineligible() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    let (attempt, payload) = s.prepare_recall(&key, a, 10, 1, "a".repeat(64)).unwrap();
    s.control_preferences(a, None, None, true).unwrap();
    s.recall
        .settle_add(
            attempt,
            payload,
            WorkAdmission::Appended {
                digest_hex: "a".repeat(64),
                scoped_guild: "discord:dm:1".into(),
            },
            11,
        )
        .unwrap();
    assert!(eligible(&reload(&s), a, p).is_empty());
    assert_eq!(s.recall.records.len(), 1);
}

#[test]
fn explicit_disable_survives_admitted_deletion_compaction_and_restart() {
    let a = personal(1);
    let (mut s, p, key) = setup(a);
    let row = add(&mut s, &key, a);
    assert!(
        s.prepare_recall_forget(row, a, 12, 2, "a".repeat(64))
            .is_err()
    );
    s.disable_recall_source(&key, a).unwrap();
    assert!(eligible(&s, a, p).is_empty());
    let id = s
        .prepare_recall_forget(row, a, 12, 2, "a".repeat(64))
        .unwrap();
    s.recall
        .settle_forget(
            id,
            WorkAdmission::Appended {
                digest_hex: "c".repeat(64),
                scoped_guild: "discord:dm:1".into(),
            },
            13,
        )
        .unwrap();
    assert!(s.recall.records.is_empty());
    s.compact_recall(Some(s.recall.projection_revision), &BTreeSet::new());
    let s = reload(&s);
    assert!(!s.recall.source_versions[&key].recall_enabled);
    assert!(s.recall_candidate(&key, a).is_err());
    assert!(s.recall.scope_controls.contains_key(&a.scope()));
}
#[test]
fn compaction_needs_disk_and_draft_proofs_and_never_reuses_ids() {
    let a = personal(1);
    let (mut s, _, key) = setup(a);
    let (id, _) = s.prepare_recall(&key, a, 10, 1, "a".repeat(64)).unwrap();
    s.recall.reject(id, 11).unwrap();
    s.compact_recall(None, &BTreeSet::new());
    assert!(s.recall.source_versions.contains_key(&key));
    s.compact_recall(
        Some(s.recall.projection_revision),
        &BTreeSet::from([key.clone()]),
    );
    assert!(s.recall.source_versions.contains_key(&key));
    s.compact_recall(Some(s.recall.projection_revision), &BTreeSet::new());
    assert!(s.recall.source_versions.is_empty());
    assert!(s.recall.scope_controls.is_empty());
    let mut s = reload(&s);
    assert!(add(&mut s, &key, a) > id);
}
#[test]
fn checked_exhaustion_stays_sticky_but_native_controls_work() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    add(&mut s, &key, a);
    s.recall.source_versions.get_mut(&key).unwrap().revision = u64::MAX;
    s.feedback(a, 500, Some(WorkFeedback::Dismissed), true, 3)
        .unwrap();
    assert!(s.recall.source_versions[&key].recall_disabled_exhausted);
    s.recall
        .scope_controls
        .get_mut(&a.scope())
        .unwrap()
        .generation = u64::MAX;
    s.control_preferences(a, None, None, true).unwrap();
    assert!(s.recall.scope_controls[&a.scope()].recall_disabled_exhausted);
    s.control_preferences(a, Some(false), None, false).unwrap();
    assert!(!s.preferences[&a.scope().key()].learning_enabled);
    assert!(eligible(&reload(&s), a, p).is_empty());
    s.recall.sequence = u64::MAX;
    let before = s.clone();
    assert!(s.prepare_recall(&key, a, 10, 1, "a".repeat(64)).is_err());
    assert_eq!(s, before);
    let (mut task_store, _, task_key) = setup(a);
    task_store.recall.sequence = u64::MAX;
    let before = task_store.clone();
    assert!(
        task_store
            .prepare_recall(&task_key, a, 10, 1, "a".repeat(64))
            .is_err()
    );
    assert_eq!(before, task_store);
}
#[test]
fn source_and_scope_capacity_do_not_block_reset_or_disable() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    add(&mut s, &key, a);
    for id in 1..MAX_SOURCES as u64 {
        s.recall.source_versions.insert(
            WorkSourceKey::Task {
                project: p,
                id: id + 100_000,
            },
            SourceVersion {
                revision: 0,
                generation: 0,
                recall_enabled: true,
                recall_disabled_exhausted: false,
            },
        );
    }
    for owner in 2..=MAX_SCOPES as u64 {
        s.recall
            .scope_controls
            .insert(WorkScope::Personal { owner }, ScopeRecallControl::default());
    }
    assert!(s.recall.validate().is_ok());
    let new_key = feedback_source(&mut s, a, &[p], 501);
    let before = s.recall.clone();
    assert!(
        s.prepare_recall(&new_key, a, 10, 1, "a".repeat(64))
            .is_err()
    );
    assert_eq!(before, s.recall);
    assert!(s.disable_recall_source(&new_key, a).is_err());
    s.control_preferences(a, None, None, true).unwrap();
    s.control_preferences(a, Some(false), None, false).unwrap();
    assert!(s.preferences[&a.scope().key()].evidence.is_empty());
    assert_eq!(s.recall.scope_controls[&a.scope()].generation, 1);
    assert_eq!(s.recall.source_versions.len(), MAX_SOURCES);
    assert_eq!(s.recall.scope_controls.len(), MAX_SCOPES);
    s.recall.scope_controls.insert(
        WorkScope::Personal { owner: u64::MAX },
        ScopeRecallControl::default(),
    );
    assert_eq!(s.recall.validate(), Err(WorkError::Full));
}
#[test]
fn deterministic_payloads_and_bounded_byte_reservations() {
    let a = personal(1);
    let (mut s, p, key) = setup(a);
    let payload = s.recall_candidate(&key, a).unwrap();
    assert_eq!(payload.at, None);
    assert_eq!(
        payload.encoded().unwrap(),
        reload(&s)
            .recall_candidate(&key, a)
            .unwrap()
            .encoded()
            .unwrap()
    );
    let mut oversized = payload.clone();
    oversized.text = "🙂".repeat(1025);
    assert_eq!(oversized.encoded(), Err(WorkError::Full));
    oversized = payload;
    oversized.contributing_projects = (1..=128).collect();
    oversized.text = "🙂".repeat(1024);
    // Exact serialized identity/provenance is never clipped to admit it.
    oversized.delivered_source_digest = Some("a".repeat(8000));
    assert_eq!(oversized.encoded(), Err(WorkError::Full));
    let decision = s.record_decision(p, a, &"x".repeat(1000), 7, "d").unwrap();
    let d = s
        .recall_candidate(
            &WorkSourceKey::Decision {
                project: p,
                id: decision,
            },
            a,
        )
        .unwrap();
    assert_eq!(d.at, Some(7));
    assert!(d.text.chars().count() <= 1024);
}

#[test]
fn admitted_row_and_payload_byte_caps_reserve_before_prepared() {
    let a = personal(1);
    let (mut s, _, key) = setup(a);
    let row = add(&mut s, &key, a);
    let record = s.recall.records[&row].clone();
    s.recall.records.clear();
    for id in 1..=MAX_ROWS as u64 {
        let mut record = record.clone();
        record.id = id;
        s.recall.records.insert(id, record);
    }
    s.recall.terminal.clear();
    s.recall.sequence = MAX_ROWS as u64;
    assert!(s.recall.validate().is_ok());
    let sequence = s.recall.sequence;
    assert_eq!(
        s.prepare_recall(&key, a, 10, 1, "a".repeat(64)),
        Err(WorkError::Full)
    );
    assert_eq!(s.recall.sequence, sequence);
    assert!(s.recall.attempts.is_empty());
    // Row headers/receipts also consume control metadata, separately from payloads.
    for id in 100_000..120_000 {
        s.recall.source_versions.insert(
            WorkSourceKey::Task { project: 1, id },
            SourceVersion {
                revision: 0,
                generation: 0,
                recall_enabled: true,
                recall_disabled_exhausted: false,
            },
        );
    }
    // Keep exactly the source-count cap: combined bytes, not count, must reject.
    s.recall.source_versions.remove(&WorkSourceKey::Task {
        project: 1,
        id: 100_000,
    });
    assert_eq!(s.recall.source_versions.len(), MAX_SOURCES);
    assert_eq!(s.recall.validate(), Err(WorkError::Full));
    s.recall.source_versions.retain(|k, _| k == &key);
    for record in s.recall.records.values_mut() {
        record.payload.text = "🙂".repeat(1024);
        record.payload_digest = digest(&record.payload.encoded().unwrap());
    }
    assert_eq!(s.recall.validate(), Err(WorkError::Full));
}
#[test]
fn unresolved_limit_and_terminal_fifo_preserve_unknown_reservations() {
    let a = personal(1);
    let (mut s, p, key) = setup(a);
    let (unknown, _) = s.prepare_recall(&key, a, 10, 1, "a".repeat(64)).unwrap();
    let template = s.recall.attempts[&unknown].clone();
    s.recall.recover_prepared();
    for n in 1..MAX_ATTEMPTS as u64 {
        let source = WorkSourceKey::Task {
            project: p,
            id: 100_000 + n,
        };
        s.recall.source_versions.insert(
            source.clone(),
            SourceVersion {
                revision: 0,
                generation: 0,
                recall_enabled: true,
                recall_disabled_exhausted: false,
            },
        );
        let id = n * 2 + 2;
        let mut item = template.clone();
        item.id = id;
        item.source = source;
        item.operation = ProjectionOperation::Add { row: id - 1 };
        s.recall.attempts.insert(id, item);
    }
    s.recall.sequence = MAX_ATTEMPTS as u64 * 2;
    assert!(s.recall.validate().is_ok());
    s.recall.attempts.insert(s.recall.sequence + 2, template);
    assert_eq!(s.recall.validate(), Err(WorkError::Full));
    let (mut s, p, key) = setup(a);
    let (unknown, _) = s.prepare_recall(&key, a, 10, 1, "a".repeat(64)).unwrap();
    let d = s.record_decision(p, a, "Decision", 1, "d").unwrap();
    let other = WorkSourceKey::Decision { project: p, id: d };
    let mut last = 0;
    for _ in 0..MAX_TERMINALS + 4 {
        let (id, _) = s.prepare_recall(&other, a, 10, 1, "a".repeat(64)).unwrap();
        s.recall.reject(id, 11).unwrap();
        last = id;
    }
    assert_eq!(s.recall.terminal.len(), MAX_TERMINALS);
    assert_eq!(s.recall.terminal.back().unwrap().attempt, last);
    assert!(s.recall.terminal.front().unwrap().attempt > unknown + 2);
    assert!(s.recall.attempts.contains_key(&unknown));
    assert_eq!(
        reload(&s).recall.attempts[&unknown].state,
        AttemptState::Unknown
    );
}

#[test]
fn maximum_width_scope_entry_remains_loadable_after_exhausted_reset() {
    let a = WorkAccess {
        actor: u64::MAX,
        guild: Some(u64::MAX),
        channel: u64::MAX,
        can_view: true,
        can_manage: true,
    };
    let mut s = WorkStore {
        sequence: u64::MAX - 2,
        ..Default::default()
    };
    let project = s.create_project(a, "Maximum IDs", "p").unwrap();
    let id = s.add_task(a, task(project), "t").unwrap();
    let key = WorkSourceKey::Task { project, id };
    add(&mut s, &key, a);
    let preference = feedback_source(&mut s, a, &[project], u64::MAX);
    add(&mut s, &preference, a);
    for (key, value) in &s.recall.source_versions {
        assert!(serde_json::to_vec(key).unwrap().len() <= 256);
        assert!(
            serde_json::to_vec(&serde_json::json!({"key": key, "value": value}))
                .unwrap()
                .len()
                <= 512
        );
    }
    s.recall
        .scope_controls
        .get_mut(&a.scope())
        .unwrap()
        .generation = u64::MAX;
    let entry = serde_json::json!({"key":a.scope(),"value":s.recall.scope_controls[&a.scope()]});
    assert!(serde_json::to_vec(&entry).unwrap().len() <= 128);
    s.control_preferences(a, None, None, true).unwrap();
    assert!(reload(&s).recall.scope_controls[&a.scope()].recall_disabled_exhausted);
}

#[test]
fn temporary_learning_disable_cannot_authorize_forget_or_resurrection() {
    let a = personal(1);
    let (mut s, p, _) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    let row = add(&mut s, &key, a);
    s.control_preferences(a, Some(false), None, false).unwrap();
    assert_eq!(
        s.prepare_recall_forget(row, a, 12, 1, "a".repeat(64)),
        Err(WorkError::Denied)
    );
    assert!(s.recall.attempts.is_empty());
    s.disable_recall_source(&key, a).unwrap();
    let attempt = s
        .prepare_recall_forget(row, a, 12, 1, "a".repeat(64))
        .unwrap();
    s.control_preferences(a, Some(true), None, false).unwrap();
    assert!(eligible(&s, a, p).is_empty());
    assert!(s.prepare_recall(&key, a, 13, 2, "a".repeat(64)).is_err());
    s.recall
        .settle_forget(
            attempt,
            WorkAdmission::Appended {
                digest_hex: "b".repeat(64),
                scoped_guild: "discord:dm:1".into(),
            },
            14,
        )
        .unwrap();
    s.compact_recall(Some(s.recall.projection_revision), &BTreeSet::new());
    let mut s = reload(&s);
    s.control_preferences(a, Some(false), None, false).unwrap();
    s.control_preferences(a, Some(true), None, false).unwrap();
    assert!(!s.recall.source_versions[&key].recall_enabled);
    assert!(s.prepare_recall(&key, a, 15, 3, "a".repeat(64)).is_err());
    assert_eq!(s.preferences[&a.scope().key()].evidence.len(), 1);
    assert!(s.recall.records.is_empty());
}

#[test]
fn native_obsolescence_allows_cleanup_but_candidate_errors_do_not() {
    let a = personal(1);
    let (mut s, p, task_key) = setup(a);
    let key = feedback_source(&mut s, a, &[p], 500);
    let row = add(&mut s, &key, a);
    s.deliveries.get_mut(&500).unwrap().provenance = None;
    assert_eq!(
        s.prepare_recall_forget(row, a, 12, 1, "a".repeat(64)),
        Err(WorkError::Invalid)
    );
    s.feedback(a, 500, None, true, 3).unwrap();
    assert!(
        s.prepare_recall_forget(row, a, 12, 1, "a".repeat(64))
            .is_ok()
    );
    let task_row = add(&mut s, &task_key, a);
    let WorkSourceKey::Task { id, .. } = task_key else {
        panic!()
    };
    s.update_task(a, id, 0, WorkStatus::Done, None).unwrap();
    assert!(
        s.prepare_recall_forget(task_row, a, 12, 2, "a".repeat(64))
            .is_ok()
    );
}

#[test]
fn scope_exhaustion_blocks_all_sources_but_normal_reset_only_preferences() {
    let a = personal(1);
    let (mut s, p, task_key) = setup(a);
    let task_row = add(&mut s, &task_key, a);
    let decision = s
        .record_decision(p, a, "Keep native authority", 1, "d")
        .unwrap();
    let decision_key = WorkSourceKey::Decision {
        project: p,
        id: decision,
    };
    let decision_row = add(&mut s, &decision_key, a);
    let pref_key = feedback_source(&mut s, a, &[p], 500);
    add(&mut s, &pref_key, a);
    s.control_preferences(a, None, None, true).unwrap();
    assert_eq!(eligible(&s, a, p), BTreeSet::from([task_row, decision_row]));
    let fresh_pref = feedback_source(&mut s, a, &[p], 501);
    add(&mut s, &fresh_pref, a);
    s.recall
        .scope_controls
        .get_mut(&a.scope())
        .unwrap()
        .generation = u64::MAX;
    s.control_preferences(a, None, None, true).unwrap();
    s.control_preferences(a, Some(false), None, false).unwrap();
    s.control_preferences(a, Some(true), None, false).unwrap();
    s.compact_recall(Some(s.recall.projection_revision), &BTreeSet::new());
    let mut s = reload(&s);
    assert!(eligible(&s, a, p).is_empty());
    for key in [task_key, decision_key, fresh_pref] {
        assert_eq!(s.recall_candidate(&key, a), Err(WorkError::Full));
        assert_eq!(
            s.prepare_recall(&key, a, 12, 1, "a".repeat(64)),
            Err(WorkError::Full)
        );
    }
    s.control_preferences(a, None, None, true).unwrap();
    s.control_preferences(a, Some(false), None, false).unwrap();
    assert!(reload(&s).recall.scope_controls[&a.scope()].recall_disabled_exhausted);
}

#[test]
fn unresolved_observed_receipt_is_bounded_validated_and_backward_compatible() {
    let a = personal(1);
    let (mut s, _, key) = setup(a);
    let (id, _) = s.prepare_recall(&key, a, 10, 1, "a".repeat(64)).unwrap();
    let legacy = serde_json::to_vec(&s).unwrap();
    let legacy: WorkStore = serde_json::from_slice(&legacy).unwrap();
    assert_eq!(legacy.recall.attempts[&id].observed_admission, None);
    let attempt = s.recall.attempts.get_mut(&id).unwrap();
    attempt.state = AttemptState::Unknown;
    attempt.observed_admission = Some(WorkAdmission::Appended {
        digest_hex: "b".repeat(64),
        scoped_guild: "discord:dm:1".into(),
    });
    assert_eq!(reload(&s).recall.attempts[&id], s.recall.attempts[&id]);
    let mut worst = s.recall.attempts[&id].clone();
    worst.id = u64::MAX;
    worst.source = WorkSourceKey::Preference {
        scope: WorkScope::Team {
            guild: u64::MAX,
            channel: u64::MAX,
        },
        delivery: u64::MAX,
        actor: u64::MAX,
    };
    worst.operation = ProjectionOperation::Forget { row: u64::MAX };
    worst.revision = u64::MAX;
    worst.generation = u64::MAX;
    worst.at = u64::MAX;
    worst.nonce = u64::MAX;
    worst.payload_bytes = MAX_PAYLOAD_BYTES;
    worst.observed_admission = Some(WorkAdmission::Appended {
        digest_hex: "b".repeat(64),
        scoped_guild: format!("discord:dm:{}", u64::MAX),
    });
    assert!(bytes(&worst).unwrap().len() <= 1024);
    // The full 1024-byte reservation already includes any observed receipt.
    for n in 1..MAX_SOURCES as u64 {
        s.recall.source_versions.insert(
            WorkSourceKey::Task {
                project: 1,
                id: 100_000 + n,
            },
            SourceVersion {
                revision: 0,
                generation: 0,
                recall_enabled: true,
                recall_disabled_exhausted: false,
            },
        );
    }
    let template = s.recall.attempts[&id].clone();
    for n in 1..MAX_ATTEMPTS as u64 {
        let mut pending = template.clone();
        pending.id = n * 2 + 2;
        pending.source = WorkSourceKey::Task {
            project: 1,
            id: 100_000 + n,
        };
        pending.operation = ProjectionOperation::Add {
            row: pending.id - 1,
        };
        s.recall.attempts.insert(pending.id, pending);
    }
    s.recall.sequence = MAX_ATTEMPTS as u64 * 2;
    assert!(s.recall.validate().is_ok());
    for owner in 2..2002 {
        s.recall
            .scope_controls
            .insert(WorkScope::Personal { owner }, Default::default());
    }
    assert_eq!(s.recall.validate(), Err(WorkError::Full));
    let mut bad = reload(&legacy);
    bad.recall.attempts.get_mut(&id).unwrap().observed_admission = Some(WorkAdmission::Appended {
        digest_hex: "bad".into(),
        scoped_guild: "discord:dm:1".into(),
    });
    assert!(serde_json::from_slice::<WorkStore>(&serde_json::to_vec(&bad).unwrap()).is_err());
    bad.recall.attempts.get_mut(&id).unwrap().observed_admission = Some(WorkAdmission::Appended {
        digest_hex: "b".repeat(64),
        scoped_guild: "discord:2".into(),
    });
    assert!(serde_json::from_slice::<WorkStore>(&serde_json::to_vec(&bad).unwrap()).is_err());
}
