use super::*;

fn access(actor: u64, team: bool) -> WorkAccess {
    WorkAccess {
        actor,
        guild: team.then_some(42),
        channel: 99,
        can_view: true,
        can_manage: true,
    }
}

#[test]
fn rollout_is_strict_bounded_and_defaults_empty() {
    let personal = WorkScope::Personal { owner: 1 };
    assert!(!RecallRollout::parse(None).unwrap().contains(&personal));
    for input in [
        "",
        "null",
        "{}",
        r#"["discord:dm:1"]"#,
        r#"[{"Personal":{"owner":0}}]"#,
        r#"[{"Personal":{"owner":1,"admin":true}}]"#,
        r#"[{"Personal":{"owner":1}},{"Personal":{"owner":1}}]"#,
        r#"[{"Team":{"guild":42,"channel":0}}]"#,
        r#"[{"Personal":{"owner":1,"owner":2}}]"#,
    ] {
        assert!(RecallRollout::parse(Some(input)).is_err(), "{input}");
    }
    let value = serde_json::to_string(
        &(1..=1000)
            .map(|owner| WorkScope::Personal { owner })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let allowed = RecallRollout::parse(Some(&value)).unwrap();
    assert!(allowed.contains(&personal));
    assert!(!allowed.contains(&WorkScope::Personal { owner: 1001 }));
    let value = serde_json::to_string(
        &(1..=1001)
            .map(|owner| WorkScope::Personal { owner })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(RecallRollout::parse(Some(&value)), Err(WorkError::Full));
    let limit = format!("[]{}", " ".repeat(MAX_ROLLOUT_BYTES - 2));
    assert!(RecallRollout::parse(Some(&limit)).is_ok());
    assert_eq!(
        RecallRollout::parse(Some(&(limit + " "))),
        Err(WorkError::Full)
    );
}

#[test]
fn default_off_checked_revision_and_configuring_principal_round_trip() {
    let mut store = WorkStore::default();
    let actor = access(1, false);
    store.create_project(actor, "Private", "one").unwrap();
    assert_eq!(store.recall_policy(actor).unwrap(), RecallPolicy::default());
    let recall = store.recall.clone();
    let enabled = store.configure_recall(actor, true, 0).unwrap();
    assert_eq!(
        enabled,
        RecallPolicy {
            enabled: true,
            configured_by: 1,
            revision: 1
        }
    );
    assert_eq!(store.configure_recall(actor, true, 1).unwrap(), enabled);
    assert_eq!(
        store.configure_recall(actor, false, 0),
        Err(WorkError::Stale)
    );
    assert_eq!(store.configure_recall(actor, false, 1).unwrap().revision, 2);
    assert_eq!(
        store.recall, recall,
        "policy must not propose, delete or alter receipts"
    );
    let mut encoded = serde_json::to_value(&store).unwrap();
    let restored: WorkStore = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(restored, store);
    encoded.as_object_mut().unwrap().remove("recall_policies");
    let legacy: WorkStore = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        legacy.recall_policy(actor).unwrap(),
        RecallPolicy::default()
    );
}

#[test]
fn configuration_needs_every_project_manager_and_current_view() {
    let mut store = WorkStore::default();
    let actor = access(1, true);
    let first = store.create_project(actor, "A", "a").unwrap();
    let second = store.create_project(access(2, true), "B", "b").unwrap();
    assert_eq!(
        store.configure_recall(actor, true, 0),
        Err(WorkError::Denied)
    );
    store.projects.get_mut(&second).unwrap().members.insert(1);
    assert_eq!(
        store.configure_recall(actor, true, 0),
        Err(WorkError::Denied)
    );
    store.projects.get_mut(&second).unwrap().managers.insert(1);
    store.configure_recall(actor, true, 0).unwrap();
    store.projects.get_mut(&first).unwrap().managers.remove(&1);
    assert_eq!(
        store.configure_recall(actor, false, 1),
        Err(WorkError::Denied)
    );
    assert_eq!(
        store.recall_policy(WorkAccess {
            can_view: false,
            ..actor
        }),
        Err(WorkError::Denied)
    );
    assert_eq!(
        store.recall_policy(WorkAccess {
            channel: 100,
            ..actor
        }),
        Err(WorkError::Missing)
    );
    assert_eq!(
        store.recall_policy(access(2, false)),
        Err(WorkError::Missing)
    );
    store.projects.get_mut(&first).unwrap().members.remove(&1);
    assert_eq!(store.recall_policy(actor), Err(WorkError::Denied));
}

#[test]
fn malformed_or_duplicate_policies_fail_loading() {
    let mut store = WorkStore::default();
    let actor = access(1, false);
    store.create_project(actor, "A", "a").unwrap();
    store.configure_recall(actor, true, 0).unwrap();
    let original = serde_json::to_value(store).unwrap();
    let mut duplicate = original.clone();
    let entry = duplicate["recall_policies"][0].clone();
    duplicate["recall_policies"]
        .as_array_mut()
        .unwrap()
        .push(entry);
    assert!(serde_json::from_value::<WorkStore>(duplicate).is_err());
    for (field, value) in [("revision", 0), ("configured_by", 0), ("configured_by", 2)] {
        let mut invalid = original.clone();
        invalid["recall_policies"][0]["value"][field] = value.into();
        assert!(serde_json::from_value::<WorkStore>(invalid).is_err());
    }
    let mut invalid = original;
    invalid["recall_policies"][0]["key"]["Personal"]["owner"] = 0.into();
    assert!(serde_json::from_value::<WorkStore>(invalid).is_err());
}

#[test]
fn capacity_preserves_existing_disable_and_reserves_maximum_width() {
    let mut store = WorkStore::default();
    let actor = access(1, false);
    store.create_project(actor, "A", "a").unwrap();
    store.configure_recall(actor, true, 0).unwrap();
    for n in 1..MAX_POLICIES {
        store.recall_policies.0.insert(
            WorkScope::Team {
                guild: u64::MAX,
                channel: n as u64,
            },
            RecallPolicy {
                enabled: true,
                configured_by: u64::MAX,
                revision: u64::MAX,
            },
        );
    }
    store.recall_policies.validate().unwrap();
    assert!(serde_json::to_vec(&store.recall_policies).unwrap().len() <= MAX_POLICY_BYTES);
    let other = access(2, false);
    store.create_project(other, "B", "b").unwrap();
    let before = store.clone();
    assert_eq!(store.configure_recall(other, true, 0), Err(WorkError::Full));
    assert_eq!(store, before);
    assert!(!store.configure_recall(actor, false, 1).unwrap().enabled);
    let restored: WorkStore = serde_json::from_slice(&serde_json::to_vec(&store).unwrap()).unwrap();
    assert_eq!(restored.recall_policies, store.recall_policies);
}

#[test]
fn revision_exhaustion_never_wraps_or_mutates_policy() {
    let actor = access(1, false);
    let mut store = WorkStore::default();
    store.create_project(actor, "A", "a").unwrap();
    store.recall_policies.0.insert(
        actor.scope(),
        RecallPolicy {
            enabled: false,
            configured_by: actor.actor,
            revision: u64::MAX,
        },
    );
    let old = store.clone();
    assert_eq!(
        store.configure_recall(actor, true, u64::MAX),
        Err(WorkError::Full)
    );
    assert_eq!(store, old);
    assert!(store.configure_recall(actor, false, u64::MAX).is_ok());
}
