use super::policy::{DeliveryDecision, LocalDeliveryTime};
use super::*;

fn personal(user: u64) -> WorkAccess {
    WorkAccess {
        actor: user,
        guild: None,
        channel: user + 100,
        can_view: true,
        can_manage: false,
    }
}

fn team(user: u64, channel: u64, can_view: bool, can_manage: bool) -> WorkAccess {
    WorkAccess {
        actor: user,
        guild: Some(9),
        channel,
        can_view,
        can_manage,
    }
}

fn task(project_id: u64) -> WorkTask {
    WorkTask {
        id: 0,
        project_id,
        title: "Ship the release".into(),
        owner: 0,
        assignee: None,
        goal_id: None,
        priority: 2,
        status: WorkStatus::Open,
        due_at: None,
        snoozed_until: None,
        source: None,
        revision: 0,
    }
}

#[test]
fn personal_and_team_records_require_current_access_and_membership() {
    let mut store = WorkStore::default();
    let private = store.create_project(personal(1), "Personal", "a").unwrap();
    assert_eq!(store.project(private, personal(2)), Err(WorkError::Denied));
    let lead = team(1, 99, true, true);
    let shared = store.create_project(lead, "Shared", "b").unwrap();
    store.set_member(shared, lead, 2, true).unwrap();
    assert!(store.project(shared, team(2, 99, true, false)).is_ok());
    assert_eq!(
        store.project(shared, team(2, 99, false, false)),
        Err(WorkError::Denied)
    );
    assert_eq!(
        store.project(shared, team(2, 100, true, false)),
        Err(WorkError::Denied)
    );
    store.set_member(shared, lead, 2, false).unwrap();
    assert_eq!(
        store.project(shared, team(2, 99, true, false)),
        Err(WorkError::Denied)
    );
    assert!(store.tasks.is_empty());
}

#[test]
fn task_request_is_idempotent_and_stale_update_is_refused() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store
        .create_project(access, "Release", "new-project")
        .unwrap();
    let first = store
        .add_task(access, task(project), "interaction-1")
        .unwrap();
    assert_eq!(
        store
            .add_task(access, task(project), "interaction-1")
            .unwrap(),
        first
    );
    store
        .update_task(access, first, 0, WorkStatus::Done, None)
        .unwrap();
    assert_eq!(
        store.update_task(access, first, 0, WorkStatus::Cancelled, None),
        Err(WorkError::Stale)
    );
    assert_eq!(store.tasks.len(), 1);
}

#[test]
fn preference_requires_observations_and_stays_per_user() {
    let mut profiles = std::collections::BTreeMap::<u64, WorkPreferenceProfile>::new();
    let profile = profiles.entry(1).or_default();
    for id in 1..=4 {
        profile
            .observe(PreferenceEvidence {
                delivery_id: id,
                feedback: WorkFeedback::Snoozed { hour: 10 },
                at: id,
            })
            .unwrap();
    }
    assert_eq!(profile.effective_hour(9), 9);
    profile
        .observe(PreferenceEvidence {
            delivery_id: 5,
            feedback: WorkFeedback::Useful,
            at: 5,
        })
        .unwrap();
    assert_eq!(profile.effective_hour(9), 10);
    profile.explicit_hour = Some(8);
    assert_eq!(profile.effective_hour(9), 8);
    profile.reset();
    assert_eq!(profile.learned_hour, None);
    assert_eq!(profiles.get(&2), None);
}

#[test]
fn quiet_hours_and_ambiguous_attempts_consume_the_ceiling() {
    let mut store = WorkStore::default();
    let policy = WorkAutomationPolicy {
        enabled: true,
        destination: Some(123),
        timezone: "America/New_York".into(),
        daily_limit: 1,
        ..Default::default()
    };
    policy.validate().unwrap();
    store.automation.insert(4, policy);
    assert_eq!(
        store.reserve_delivery(
            4,
            1,
            1,
            LocalDeliveryTime {
                day: "2026-09-24",
                hour: 23
            }
        ),
        Err(DeliveryDecision::Quiet)
    );
    let id = store
        .reserve_delivery(
            4,
            1,
            2,
            LocalDeliveryTime {
                day: "2026-09-24",
                hour: 9,
            },
        )
        .unwrap();
    assert_eq!(store.deliveries[&id].state, DeliveryState::Attempting);
    assert_eq!(
        store.reserve_delivery(
            4,
            1,
            3,
            LocalDeliveryTime {
                day: "2026-09-24",
                hour: 10
            }
        ),
        Err(DeliveryDecision::Limited)
    );
}

#[test]
fn legacy_state_defaults_to_empty_work_store() {
    let state: WorkStore = serde_json::from_str("{}").unwrap();
    assert_eq!(state, WorkStore::default());
}
