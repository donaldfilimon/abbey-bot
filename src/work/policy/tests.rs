use super::*;

fn access(actor: u64) -> WorkAccess {
    WorkAccess {
        actor,
        guild: None,
        channel: 55,
        can_view: true,
        can_manage: false,
    }
}
fn fixture() -> WorkStore {
    let mut store = WorkStore::default();
    let project = store.create_project(access(1), "Private", "p").unwrap();
    for id in 10..16 {
        store.deliveries.insert(
            id,
            WorkDeliveryReceipt {
                id,
                project_id: project,
                recipient: 55,
                local_day: "2026-09-24".into(),
                at: 100,
                state: DeliveryState::Sent,
                message_id: Some(id),
                scope: Some(access(1).scope()),
                kind: Some(WorkDeliveryKind::Briefing),
                coverage: vec![],
                task_ids: vec![],
                dedupe_keys: BTreeSet::new(),
            },
        );
    }
    store
}

#[test]
fn corrections_reset_disable_recompute_every_derived_setting() {
    let mut store = fixture();
    for id in 10..15 {
        store
            .feedback(access(1), id, Some(WorkFeedback::Dismissed), false, 101)
            .unwrap();
    }
    let profile = store.preference_profile(access(1)).unwrap();
    assert!(profile.reduce_followups);
    assert_eq!(profile.briefing_rank, -1);
    store.feedback(access(1), 10, None, true, 102).unwrap();
    let profile = store.preference_profile(access(1)).unwrap();
    assert!(!profile.reduce_followups);
    assert_eq!(profile.briefing_rank, 0);
    store
        .feedback(access(1), 10, Some(WorkFeedback::Useful), false, 103)
        .unwrap();
    store
        .control_preferences(access(1), Some(false), Some(Some(7)), false)
        .unwrap();
    let profile = store.preference_profile(access(1)).unwrap();
    assert_eq!(profile.effective_hour(9), 7);
    assert!(!profile.reduce_followups);
    assert_eq!(profile.briefing_rank, 0);
    store
        .control_preferences(access(1), Some(true), None, true)
        .unwrap();
    let profile = store.preference_profile(access(1)).unwrap();
    assert!(profile.evidence.is_empty());
    assert_eq!(profile.explicit_hour, Some(7));
}

#[test]
fn unknown_unfinished_forged_legacy_and_duplicate_feedback_fail_closed() {
    let mut store = fixture();
    assert!(
        store
            .feedback(access(2), 10, Some(WorkFeedback::Useful), false, 101)
            .is_err()
    );
    assert!(
        store
            .feedback(access(1), 99, Some(WorkFeedback::Useful), false, 101)
            .is_err()
    );
    store.deliveries.get_mut(&10).unwrap().state = DeliveryState::Attempting;
    assert!(
        store
            .feedback(access(1), 10, Some(WorkFeedback::Useful), false, 101)
            .is_err()
    );
    store.deliveries.get_mut(&10).unwrap().state = DeliveryState::Sent;
    store.deliveries.get_mut(&10).unwrap().scope = None;
    assert!(
        store
            .feedback(access(1), 10, Some(WorkFeedback::Useful), false, 101)
            .is_err()
    );
    for _ in 0..8 {
        store
            .feedback(access(1), 11, Some(WorkFeedback::Useful), false, 101)
            .unwrap();
    }
    assert_eq!(
        store.preference_profile(access(1)).unwrap().evidence.len(),
        1
    );
    assert_eq!(
        store.preference_profile(access(1)).unwrap().briefing_rank,
        0
    );
    let profile = store.preferences.get_mut(&access(1).scope().key()).unwrap();
    for id in 20..30 {
        profile
            .observe(PreferenceEvidence {
                actor: None,
                scope: None,
                kind: None,
                delivery_id: id,
                feedback: WorkFeedback::Snoozed { hour: 12 },
                at: 100,
            })
            .unwrap();
    }
    assert_eq!(profile.learned_hour, None);
}

#[test]
fn shared_controls_and_feedback_recheck_all_project_membership() {
    let manager = WorkAccess {
        guild: Some(9),
        can_manage: true,
        ..access(1)
    };
    let member = WorkAccess {
        actor: 2,
        can_manage: false,
        ..manager
    };
    let mut store = WorkStore::default();
    let p = store.create_project(manager, "Team", "a").unwrap();
    store.set_member(p, manager, 2, true).unwrap();
    assert!(store.preference_profile(member).is_ok());
    assert_eq!(
        store.control_preferences(member, Some(false), None, true),
        Err(WorkError::Denied)
    );
    store
        .control_preferences(manager, Some(false), None, true)
        .unwrap();
    store.set_member(p, manager, 2, false).unwrap();
    assert_eq!(store.preference_profile(member), Err(WorkError::Denied));
    assert!(store.preference_profile(access(1)).is_err());
}

#[test]
fn timezone_default_is_identity_bound_and_never_shared() {
    assert_eq!(
        initial_timezone(access(1), None, Some("1")).unwrap(),
        "America/New_York"
    );
    assert!(initial_timezone(access(2), None, Some("1")).is_err());
    assert!(initial_timezone(access(1), None, None).is_err());
    assert!(
        initial_timezone(
            WorkAccess {
                guild: Some(9),
                ..access(1)
            },
            None,
            Some("1")
        )
        .is_err()
    );
    assert_eq!(
        initial_timezone(access(1), Some("Europe/London".into()), Some("1")).unwrap(),
        "Europe/London"
    );
    assert!(initial_timezone(access(1), None, Some("0")).is_err());
}

#[test]
fn unrelated_feedback_never_unlocks_timing_and_correction_clears_it() {
    let mut store = fixture();
    for id in 10..13 {
        store
            .feedback(
                access(1),
                id,
                Some(WorkFeedback::Snoozed { hour: 11 }),
                false,
                101,
            )
            .unwrap();
    }
    for id in 13..15 {
        store
            .feedback(access(1), id, Some(WorkFeedback::Useful), false, 101)
            .unwrap();
    }
    assert_eq!(
        store.preference_profile(access(1)).unwrap().learned_hour,
        None
    );
    for id in 13..15 {
        store
            .feedback(
                access(1),
                id,
                Some(WorkFeedback::Snoozed { hour: 11 }),
                true,
                102,
            )
            .unwrap();
    }
    assert_eq!(
        store.preference_profile(access(1)).unwrap().learned_hour,
        Some(11)
    );
    store.feedback(access(1), 10, None, true, 103).unwrap();
    assert_eq!(
        store.preference_profile(access(1)).unwrap().learned_hour,
        None
    );
}

#[test]
fn rolling_window_retains_durable_replay_protection_across_correction_and_reset() {
    let mut store = fixture();
    let template = store.deliveries[&10].clone();
    for id in 10..111 {
        store.deliveries.insert(
            id,
            WorkDeliveryReceipt {
                id,
                message_id: Some(id),
                ..template.clone()
            },
        );
        store
            .feedback(access(1), id, Some(WorkFeedback::Useful), false, 101)
            .unwrap();
    }
    let profile = store.preference_profile(access(1)).unwrap();
    assert_eq!(profile.evidence.len(), 100);
    assert_eq!(profile.evidence[0].delivery_id, 11);
    assert_eq!(profile.evidence.last().unwrap().delivery_id, 110);
    // The durable serialization round-trip preserves the evicted identity.
    let mut store: WorkStore =
        serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    store
        .feedback(access(1), 10, Some(WorkFeedback::Dismissed), false, 102)
        .unwrap();
    assert_eq!(store.preference_profile(access(1)).unwrap(), profile);
    assert_eq!(
        store.feedback(access(1), 10, Some(WorkFeedback::Dismissed), true, 102),
        Err(WorkError::Missing)
    );
    store
        .feedback(access(1), 11, Some(WorkFeedback::Dismissed), true, 102)
        .unwrap();
    assert_eq!(
        store.preference_profile(access(1)).unwrap().evidence[0].feedback,
        WorkFeedback::Dismissed
    );
    store.feedback(access(1), 11, None, true, 103).unwrap();
    store
        .feedback(access(1), 11, Some(WorkFeedback::Useful), false, 104)
        .unwrap();
    assert_eq!(
        store.preference_profile(access(1)).unwrap().evidence.len(),
        99
    );
    store
        .control_preferences(access(1), None, Some(Some(7)), true)
        .unwrap();
    store
        .feedback(access(1), 110, Some(WorkFeedback::Useful), false, 105)
        .unwrap();
    let profile = store.preference_profile(access(1)).unwrap();
    assert!(profile.evidence.is_empty());
    assert_eq!(profile.explicit_hour, Some(7));
    assert_eq!(profile.briefing_rank, 0);
    assert_eq!(profile.observed_deliveries.len(), 101);
}

#[test]
fn legacy_full_window_does_not_block_new_evidence_or_lose_old_attributed_identity() {
    let mut store = fixture();
    let mut profile = WorkPreferenceProfile::default();
    for id in 0..100 {
        profile.evidence.push(PreferenceEvidence {
            actor: None,
            scope: None,
            kind: None,
            delivery_id: id,
            feedback: WorkFeedback::Dismissed,
            at: 100,
        });
    }
    store.preferences.insert(access(1).scope().key(), profile);
    store
        .feedback(access(1), 10, Some(WorkFeedback::Useful), false, 101)
        .unwrap();
    let profile = store.preference_profile(access(1)).unwrap();
    assert_eq!(profile.evidence.len(), 1);
    assert_eq!(profile.briefing_rank, 0);
    // Old serialized profiles had no observation identity set. Reset migrates it.
    store
        .preferences
        .get_mut(&access(1).scope().key())
        .unwrap()
        .observed_deliveries
        .clear();
    store
        .control_preferences(access(1), None, None, true)
        .unwrap();
    store
        .feedback(access(1), 10, Some(WorkFeedback::Useful), false, 102)
        .unwrap();
    assert!(
        store
            .preference_profile(access(1))
            .unwrap()
            .evidence
            .is_empty()
    );
}

#[test]
fn authorized_partial_updates_preserve_saved_policy_and_inspection_fallback() {
    for guild in [None, Some(9)] {
        let manager = WorkAccess {
            guild,
            can_manage: true,
            ..access(1)
        };
        let mut store = WorkStore::default();
        let project = store.create_project(manager, "Scope", "p").unwrap();
        let initial = WorkAutomationUpdate {
            enabled: true,
            timezone: Some("Europe/London".into()),
            briefing_hour: Some(15),
            quiet_start: Some(21),
            quiet_end: Some(6),
            daily_limit: Some(1),
        };
        let saved = store.update_automation(manager, initial, None).unwrap();
        let disabled = store
            .update_automation(manager, WorkAutomationUpdate::default(), None)
            .unwrap();
        assert_eq!(
            disabled,
            WorkAutomationPolicy {
                enabled: false,
                ..saved.clone()
            }
        );
        let updated = store
            .update_automation(
                manager,
                WorkAutomationUpdate {
                    enabled: true,
                    briefing_hour: Some(16),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        assert_eq!(
            updated,
            WorkAutomationPolicy {
                briefing_hour: 16,
                ..saved.clone()
            }
        );
        store
            .update_automation(
                manager,
                WorkAutomationUpdate {
                    enabled: true,
                    briefing_hour: Some(15),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        store
            .control_preferences(manager, None, Some(None), false)
            .unwrap();
        let snapshot = store.preference_snapshot(manager).unwrap();
        assert_eq!(snapshot.effective_hour, 15);
        assert_eq!(
            snapshot.effective_hour,
            snapshot
                .profile
                .effective_hour(store.scope_automation[&manager.scope().key()].briefing_hour)
        );
        assert_eq!(snapshot.timezone.as_deref(), Some("Europe/London"));
        let before = store.clone();
        assert!(
            store
                .update_automation(
                    manager,
                    WorkAutomationUpdate {
                        enabled: true,
                        daily_limit: Some(5),
                        ..Default::default()
                    },
                    None
                )
                .is_err()
        );
        assert_eq!(store, before);
        if guild.is_some() {
            store.set_member(project, manager, 2, true).unwrap();
            let before = store.clone();
            assert_eq!(
                store.update_automation(
                    WorkAccess {
                        actor: 2,
                        can_manage: false,
                        ..manager
                    },
                    WorkAutomationUpdate::default(),
                    None
                ),
                Err(WorkError::Denied)
            );
            assert_eq!(store, before);
        }
    }
}
