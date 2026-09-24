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
