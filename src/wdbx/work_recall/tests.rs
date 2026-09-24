use super::*;
use crate::{
    wdbx::{HEADER, Recall},
    work::{
        WorkAccess, WorkStatus, WorkStore, WorkTask,
        recall::{RecallAudience, WorkAdmission, WorkSourceKey},
    },
};

fn access(actor: u64) -> WorkAccess {
    WorkAccess {
        actor,
        guild: None,
        channel: actor + 100,
        can_view: true,
        can_manage: false,
    }
}
fn add(work: &mut WorkStore, actor: u64, name: &str) -> (u64, u64, WorkSourceKey) {
    let a = access(actor);
    let project = work.create_project(a, name, name).unwrap();
    let task = WorkTask {
        id: 0,
        project_id: project,
        title: name.into(),
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
    };
    let id = work.add_task(a, task, &format!("task-{name}")).unwrap();
    let source = WorkSourceKey::Task { project, id };
    let (attempt, payload) = work
        .prepare_recall(&source, a, 1, 1, "a".repeat(64))
        .unwrap();
    let scope = payload.scope.recall_gate_scope().0;
    let row = work
        .recall
        .settle_add(
            attempt,
            payload,
            WorkAdmission::Uncovered {
                scoped_guild: scope,
            },
            2,
        )
        .unwrap();
    (project, row, source)
}
fn allowed(work: &WorkStore, actor: u64, project: u64) -> BTreeSet<u64> {
    work.eligible_recall_ids(
        access(actor),
        project,
        RecallAudience::Private { principal: actor },
    )
    .unwrap()
}
fn read_entry(recall: &Recall, id: u64) -> Entry {
    entry(&key(id), recall.store.get_kv(&key(id)).unwrap()).unwrap()
}

#[test]
fn stable_scoped_search_preserves_generic_and_unrelated_roundtrip() {
    let mut work = WorkStore::default();
    let (p, a, _) = add(&mut work, 1, "alpha");
    let (q, b, _) = add(&mut work, 2, "beta");
    let mut recall = Recall::new();
    recall.remember("discord:dm:1", "discord:1", "private fact", 1);
    recall.store.put_kv("settings:theme", "dark");
    recall
        .store
        .unknown
        .push("{\"type\":\"future\",\"text\":\"preserve me\"}".into());
    let unrelated = recall.store.insert_vector(vec![0.25, 0.75]);
    let generic = recall.recall_for_user("discord:dm:1", "discord:1", "fact", 8);
    recall.reconcile_work_evidence(&work.recall).unwrap();
    let first = recall.clone();
    recall.reconcile_work_evidence(&work.recall).unwrap();
    assert_eq!(recall, first);
    assert!(recall.work_projection_current(&work.recall).unwrap());
    assert_eq!(
        recall.recall_for_user("discord:dm:1", "discord:1", "fact", 8),
        generic
    );
    assert_eq!(
        recall.store.vector(unrelated),
        Some([0.25, 0.75].as_slice())
    );
    assert_eq!(
        recall
            .search_work_evidence(&work.recall, &allowed(&work, 1, p), "beta", 8)
            .unwrap()[0]
            .evidence_id,
        a
    );
    assert_eq!(
        recall
            .search_work_evidence(&work.recall, &allowed(&work, 2, q), "alpha", 8)
            .unwrap()[0]
            .evidence_id,
        b
    );
    assert!(
        work.eligible_recall_ids(access(2), p, RecallAudience::Private { principal: 2 })
            .is_err()
    );
    let rendered = recall.store.try_render().unwrap();
    assert!(rendered.starts_with(HEADER));
    let reloaded = Recall::from_store(WdbxStore::parse(&rendered).unwrap());
    assert_eq!(reloaded.store.try_render().unwrap(), rendered);
    assert!(reloaded.work_projection_current(&work.recall).unwrap());
    assert!(matches!(
        recall.search_work_evidence(&work.recall, &BTreeSet::new(), &"x".repeat(513), 8),
        Err(ProjectionError::QueryTooLarge)
    ));
}
#[test]
fn retired_rows_remain_until_admitted_deletion_and_stale_disk_is_detected() {
    let mut work = WorkStore::default();
    let (p, row, source) = add(&mut work, 1, "alpha");
    let mut recall = Recall::new();
    recall.reconcile_work_evidence(&work.recall).unwrap();
    work.disable_recall_source(&source, access(1)).unwrap();
    recall.reconcile_work_evidence(&work.recall).unwrap();
    assert!(recall.store.get_kv(&key(row)).is_some());
    assert!(
        recall
            .search_work_evidence(&work.recall, &allowed(&work, 1, p), "alpha", 8)
            .unwrap()
            .is_empty()
    );
    let directory = std::env::temp_dir().join(format!("abbey-work-recall-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let disk = directory.join("projection.wdbx");
    let blocked_tmp = directory.join("projection.wdbx.tmp");
    recall.save(&disk).unwrap();
    std::fs::create_dir(&blocked_tmp).unwrap();
    let attempt = work
        .prepare_recall_forget(row, access(1), 3, 3, "b".repeat(64))
        .unwrap();
    work.recall
        .settle_forget(
            attempt,
            WorkAdmission::Uncovered {
                scoped_guild: "discord:dm:1".into(),
            },
            4,
        )
        .unwrap();
    assert!(!recall.work_projection_current(&work.recall).unwrap());
    recall.reconcile_work_evidence(&work.recall).unwrap();
    assert_eq!(recall.store.vector_count(), 0);
    assert!(recall.work_projection_current(&work.recall).unwrap());
    assert!(recall.save(&disk).is_err());
    let mut stale_disk = Recall::load(&disk).unwrap();
    assert!(!stale_disk.work_projection_current(&work.recall).unwrap());
    assert!(
        stale_disk
            .search_work_evidence(&work.recall, &BTreeSet::from([row]), "alpha", 8)
            .unwrap()
            .is_empty()
    );
    stale_disk.reconcile_work_evidence(&work.recall).unwrap();
    assert_eq!(stale_disk, recall);
    std::fs::remove_dir(&blocked_tmp).unwrap();
    stale_disk.save(&disk).unwrap();
    assert!(
        Recall::load(&disk)
            .unwrap()
            .work_projection_current(&work.recall)
            .unwrap()
    );
    std::fs::remove_file(&disk).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}
#[test]
fn corrupt_unknown_missing_and_orphan_cache_rows_never_grant_authority() {
    let mut work = WorkStore::default();
    let (p, row, _) = add(&mut work, 1, "alpha");
    let mut good = Recall::new();
    good.reconcile_work_evidence(&work.recall).unwrap();
    for corruption in 0..5 {
        let mut recall = good.clone();
        let mut e = read_entry(&recall, row);
        match corruption {
            0 => {
                recall.store.remove_vector(e.vector_id);
            }
            1 => {
                recall.store.put_kv(key(row), "broken");
            }
            2 => {
                e.version = 2;
                recall.store.put_kv(key(row), encode(&e).unwrap());
            }
            3 => {
                e.row.payload.text = "forged".into();
                recall.store.put_kv(key(row), encode(&e).unwrap());
            }
            _ => {
                recall.store.vectors[0].1 = vec![1.0];
            }
        }
        assert!(
            recall
                .search_work_evidence(&work.recall, &allowed(&work, 1, p), "alpha", 8)
                .unwrap()
                .is_empty()
        );
        assert!(!recall.work_projection_current(&work.recall).unwrap());
        recall.reconcile_work_evidence(&work.recall).unwrap();
        assert_eq!(
            recall
                .search_work_evidence(&work.recall, &allowed(&work, 1, p), "alpha", 8)
                .unwrap()
                .len(),
            1
        );
    }
    let mut e = read_entry(&good, row);
    e.row.id = 999;
    good.store.put_kv(key(999), encode(&e).unwrap());
    assert!(
        good.search_work_evidence(&work.recall, &BTreeSet::from([999]), "alpha", 8)
            .unwrap()
            .is_empty()
    );
    good.reconcile_work_evidence(&work.recall).unwrap();
    assert!(good.store.get_kv(&key(999)).is_none());
    let mut invalid = work.recall.clone();
    invalid.schema_version = 2;
    let before = good.clone();
    assert_eq!(
        good.reconcile_work_evidence(&invalid),
        Err(ProjectionError::InvalidAuthority)
    );
    assert_eq!(good, before);
}
#[test]
fn shared_vectors_are_rejected_preserved_and_replaced_without_churn() {
    let mut work = WorkStore::default();
    let (p, row, _) = add(&mut work, 1, "alpha");
    for kind in 0..5 {
        let mut recall = Recall::new();
        recall.reconcile_work_evidence(&work.recall).unwrap();
        let e = read_entry(&recall, row);
        match kind {
            0 => recall
                .store
                .put_kv(format!("mem:discord:other:{}", e.vector_id), "{}"),
            1 => recall
                .store
                .put_kv("foreign:vector", e.vector_id.to_string()),
            2 => recall.store.unknown.push(format!(
                "{{\"type\":\"future\",\"vector_id\":{}}}",
                e.vector_id
            )),
            3 => recall
                .store
                .put_kv("foreign:encoded", r#"{"vector_id":"\u0031"}"#),
            _ => recall
                .store
                .unknown
                .push(r#"{"type":"future","vector_id":1e0}"#.into()),
        }
        assert!(
            recall
                .search_work_evidence(&work.recall, &allowed(&work, 1, p), "alpha", 8)
                .unwrap()
                .is_empty()
        );
        let old_vector = recall.store.vector(e.vector_id).unwrap().to_vec();
        recall.reconcile_work_evidence(&work.recall).unwrap();
        assert_eq!(
            recall.store.vector(e.vector_id),
            Some(old_vector.as_slice())
        );
        assert_ne!(read_entry(&recall, row).vector_id, e.vector_id);
        let once = recall.clone();
        recall.reconcile_work_evidence(&work.recall).unwrap();
        assert_eq!(recall, once);
    }
}
#[test]
fn checked_batch_allocation_exhaustion_collision_and_no_partial_mutation() {
    let mut work = WorkStore::default();
    add(&mut work, 1, "alpha");
    add(&mut work, 1, "beta");
    for next_id in [0, u64::MAX, u64::MAX - 1] {
        let mut recall = Recall::new();
        recall.store.next_id = next_id;
        recall.store.put_kv("workrecall:v1:bad", "bad");
        let before = recall.clone();
        assert_eq!(
            recall.reconcile_work_evidence(&work.recall),
            Err(ProjectionError::AllocationUnavailable)
        );
        assert_eq!(recall, before);
    }
    let mut recall = Recall::new();
    recall.store.insert_vector(vec![1.0]);
    recall.store.next_id = 1;
    let before = recall.clone();
    assert_eq!(
        recall.reconcile_work_evidence(&work.recall),
        Err(ProjectionError::AllocationUnavailable)
    );
    assert_eq!(recall, before);
    recall.store.next_id = u64::MAX - 2;
    recall.reconcile_work_evidence(&work.recall).unwrap();
    assert_eq!(recall.store.next_id, u64::MAX);
    let before = recall.clone();
    recall.reconcile_work_evidence(&work.recall).unwrap();
    assert_eq!(recall, before);
}

#[test]
fn indexed_query_matches_conservative_ownership_for_opaque_references() {
    let mut work = WorkStore::default();
    let (project, row, _) = add(&mut work, 1, "alpha");
    let mut baseline = Recall::new();
    baseline.reconcile_work_evidence(&work.recall).unwrap();
    let e = read_entry(&baseline, row);
    let id = e.vector_id;
    let escaped = id
        .to_string()
        .chars()
        .map(|c| format!("\\u{:04x}", c as u32))
        .collect::<String>();
    for (k, value, unknown) in [
        (
            "foreign".to_string(),
            format!("{{\"reference\":\"{escaped}\"}}"),
            false,
        ),
        (
            "foreign".to_string(),
            format!("{{\"reference\":{id}e0}}"),
            false,
        ),
        (format!("mem:g:u:{id}"), "generic".into(), false),
        ("workrecall:v1:broken".into(), format!("future {id}"), false),
        ("unused".into(), format!("{{\"future\":{id}}}"), true),
        ("foreign".into(), "unrelated".into(), false),
    ] {
        let mut recall = baseline.clone();
        if unknown {
            recall.store.unknown.push(value);
        } else {
            recall.store.put_kv(k, value);
        }
        let expected = intact(&recall.store, &key(row), &e);
        assert_eq!(
            Ownership::new(&recall.store).intact(&key(row), &e),
            expected
        );
        assert_eq!(
            recall.work_projection_current(&work.recall).unwrap(),
            expected
        );
        let mut repaired = recall.clone();
        repaired.reconcile_work_evidence(&work.recall).unwrap();
        assert!(repaired.work_projection_current(&work.recall).unwrap());
        if !expected {
            assert!(repaired.store.vector(id).is_some());
        }
        let hits = recall
            .search_work_evidence(&work.recall, &allowed(&work, 1, project), "alpha", 8)
            .unwrap();
        assert_eq!(!hits.is_empty(), expected);
    }
}
