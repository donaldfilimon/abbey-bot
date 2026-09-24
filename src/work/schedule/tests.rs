use super::*;
use crate::work::tests::{personal, task, team};

fn timestamp(value: &str) -> u64 {
    u64::try_from(DateTime::parse_from_rfc3339(value).unwrap().timestamp()).unwrap()
}

fn fixture() -> (WorkStore, WorkScope, WorkAccess, u64) {
    let mut store = WorkStore::default();
    let access = personal(1);
    let scope = WorkScope::Personal { owner: 1 };
    let project = store.create_project(access, "One", "one").unwrap();
    store
        .configure_automation(
            &scope,
            access,
            WorkAutomationPolicy {
                enabled: true,
                destination: Some(access.channel),
                timezone: "America/New_York".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let id = store.add_task(access, task(project), "task").unwrap();
    (store, scope, access, id)
}

#[test]
fn defaults_are_disabled_with_four_deliveries_and_night_quiet_hours() {
    let policy = WorkAutomationPolicy::default();
    assert!(!policy.enabled);
    assert_eq!(
        (policy.daily_limit, policy.quiet_start, policy.quiet_end),
        (4, 22, 8)
    );
    policy.validate().unwrap();
    let (mut store, scope, access, _) = fixture();
    store.scope_automation.clear();
    assert_eq!(
        store.next_batch(&scope, access, timestamp("2026-09-24T13:00:00Z")),
        Ok(None)
    );
}

#[test]
fn timezone_destination_and_ceiling_are_validated() {
    let (mut store, scope, access, _) = fixture();
    let original = store.scope_automation[&scope.key()].clone();
    for timezone in ["", "Mars/Olympus", "EST+5"] {
        let mut bad = original.clone();
        bad.timezone = timezone.into();
        assert_eq!(
            store.configure_automation(&scope, access, bad),
            Err(WorkError::Invalid)
        );
    }
    let mut bad = original.clone();
    bad.daily_limit = 5;
    assert_eq!(
        store.configure_automation(&scope, access, bad),
        Err(WorkError::Invalid)
    );
    let mut bad = original.clone();
    bad.destination = Some(999);
    assert_eq!(
        store.configure_automation(&scope, access, bad),
        Err(WorkError::Denied)
    );
    assert_eq!(store.scope_automation[&scope.key()], original);
}

#[test]
fn due_date_alone_never_schedules_a_reminder() {
    let (mut store, scope, access, id) = fixture();
    let now = timestamp("2026-09-24T12:00:00Z"); // 08:00, before briefing
    store.tasks.get_mut(&id).unwrap().due_at = Some(now - 60);
    assert_eq!(store.next_batch(&scope, access, now), Ok(None));
    store.set_reminder(access, id, 0, Some(now)).unwrap();
    let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
    assert_eq!(batch.kind, WorkDeliveryKind::Reminder);
    assert_eq!(batch.task_ids, vec![id]);
    assert_eq!(store.tasks[&id].due_at, Some(now - 60));
}

#[test]
fn quiet_hours_delay_until_eight_across_both_dst_transitions() {
    for (before, after) in [
        ("2026-03-08T06:30:00Z", "2026-03-08T12:00:00Z"),
        ("2026-11-01T05:30:00Z", "2026-11-01T13:00:00Z"),
    ] {
        let (mut store, scope, access, id) = fixture();
        store
            .set_reminder(access, id, 0, Some(timestamp(before)))
            .unwrap();
        assert_eq!(
            store.next_batch(&scope, access, timestamp(before)),
            Ok(None)
        );
        assert_eq!(
            store.next_batch(&scope, access, timestamp(after) - 1),
            Ok(None)
        );
        let batch = store
            .next_batch(&scope, access, timestamp(after))
            .unwrap()
            .unwrap();
        assert_eq!(batch.kind, WorkDeliveryKind::Reminder);
        store
            .reserve_batch(access, &batch, timestamp(after))
            .unwrap();
        assert_eq!(store.next_batch(&scope, access, timestamp(after)), Ok(None));
    }
}

#[test]
fn calendar_spring_gap_and_fall_overlap_fire_once() {
    for (day, hour, expected) in [
        ("2026-03-08", 2, "2026-03-08T07:00:00Z"),
        ("2026-11-01", 1, "2026-11-01T05:00:00Z"),
    ] {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        assert_eq!(
            local_hour(chrono_tz::America::New_York, date, hour).unwrap(),
            utc(timestamp(expected)).unwrap()
        );
        let (mut store, scope, access, _) = fixture();
        let policy = store.scope_automation.get_mut(&scope.key()).unwrap();
        policy.briefing_hour = hour;
        policy.quiet_start = 0;
        policy.quiet_end = 0;
        let now = timestamp(expected);
        assert_eq!(store.next_batch(&scope, access, now - 1), Ok(None));
        let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
        store.reserve_batch(access, &batch, now).unwrap();
        assert_eq!(store.next_batch(&scope, access, now + 3600), Ok(None));
    }
}

#[test]
fn missed_days_and_projects_combine_in_one_current_briefing() {
    let (mut store, scope, access, first) = fixture();
    let project = store.create_project(access, "Two", "two").unwrap();
    let second = store.add_task(access, task(project), "second").unwrap();
    let now = timestamp("2026-09-24T15:00:00Z");
    for id in [first, second] {
        store
            .set_reminder(access, id, 0, Some(now - 86400 * 10))
            .unwrap();
    }
    let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
    assert_eq!(batch.kind, WorkDeliveryKind::Briefing);
    assert_eq!(batch.local_day, "2026-09-24");
    assert_eq!(batch.task_ids, vec![first, second]);
    assert_eq!(batch.coverage.len(), 2);
    assert_eq!(batch.dedupe_keys.len(), 3);
    store.reserve_batch(access, &batch, now).unwrap();
    assert_eq!(store.next_batch(&scope, access, now), Ok(None));
    assert_eq!(store.deliveries.len(), 1);
}

#[test]
fn reminder_revision_coverage_survives_restart_and_next_day() {
    let (mut store, scope, access, id) = fixture();
    let now = timestamp("2026-09-24T12:00:00Z");
    store.set_reminder(access, id, 0, Some(now)).unwrap();
    let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
    let receipt = store.reserve_batch(access, &batch, now).unwrap();
    for state in [
        DeliveryState::Attempting,
        DeliveryState::Sent,
        DeliveryState::ReviewRequired,
    ] {
        store.deliveries.get_mut(&receipt).unwrap().state = state;
        let json = serde_json::to_string(&store).unwrap();
        let loaded: WorkStore = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.next_batch(&scope, access, now + 86400), Ok(None));
    }
    store
        .update_task(access, id, 1, WorkStatus::InProgress, None)
        .unwrap();
    assert_eq!(store.next_batch(&scope, access, now + 86400), Ok(None));
    store
        .set_reminder(access, id, 2, Some(now + 86400))
        .unwrap();
    assert!(
        store
            .next_batch(&scope, access, now + 86400)
            .unwrap()
            .is_some()
    );
}

#[test]
fn default_quota_counts_briefing_and_attempts_across_projects() {
    let (mut store, scope, access, id) = fixture();
    let project = store.create_project(access, "Two", "two").unwrap();
    let second = store.add_task(access, task(project), "second").unwrap();
    let now = timestamp("2026-09-24T13:00:00Z");
    let briefing = store.next_batch(&scope, access, now).unwrap().unwrap();
    store.reserve_batch(access, &briefing, now).unwrap();
    for offset in 1..=3 {
        let target = if offset == 2 { second } else { id };
        store
            .set_reminder(
                access,
                target,
                store.tasks[&target].revision,
                Some(now + offset),
            )
            .unwrap();
        let batch = store
            .next_batch(&scope, access, now + offset)
            .unwrap()
            .unwrap();
        store.reserve_batch(access, &batch, now + offset).unwrap();
    }
    assert_eq!(store.deliveries.len(), 4);
    store
        .set_reminder(access, id, store.tasks[&id].revision, Some(now + 4))
        .unwrap();
    assert_eq!(store.next_batch(&scope, access, now + 4), Ok(None));
    assert!(
        store
            .next_batch(&scope, access, now + 86400)
            .unwrap()
            .is_some()
    );
}

#[test]
fn reservation_rechecks_policy_membership_and_current_records() {
    let (mut store, scope, access, id) = fixture();
    let now = timestamp("2026-09-24T13:00:00Z");
    let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
    store
        .update_task(access, id, 0, WorkStatus::Done, None)
        .unwrap();
    assert_eq!(
        store.reserve_batch(access, &batch, now),
        Err(WorkError::Stale)
    );
    assert!(store.deliveries.is_empty());
    store
        .update_task(access, id, 1, WorkStatus::Open, None)
        .unwrap();
    let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
    store
        .scope_automation
        .get_mut(&scope.key())
        .unwrap()
        .enabled = false;
    assert_eq!(
        store.reserve_batch(access, &batch, now),
        Err(WorkError::Stale)
    );
    store
        .scope_automation
        .get_mut(&scope.key())
        .unwrap()
        .enabled = true;
    let project = store.tasks[&id].project_id;
    store.projects.get_mut(&project).unwrap().members.clear();
    assert_eq!(
        store.reserve_batch(access, &batch, now),
        Err(WorkError::Denied)
    );
}

#[test]
fn team_policy_is_channel_bound_and_requires_all_scope_managers() {
    let mut store = WorkStore::default();
    let access = team(1, 99, true, true);
    let project = store.create_project(access, "Team", "team").unwrap();
    let scope = store.projects[&project].scope.clone();
    let mut policy = WorkAutomationPolicy {
        enabled: true,
        destination: Some(100),
        timezone: "Europe/London".into(),
        ..Default::default()
    };
    assert_eq!(
        store.configure_automation(&scope, access, policy.clone()),
        Err(WorkError::Denied)
    );
    policy.destination = Some(99);
    store
        .configure_automation(&scope, access, policy.clone())
        .unwrap();
    store.set_member(project, access, 2, true).unwrap();
    assert_eq!(
        store.configure_automation(&scope, team(2, 99, true, true), policy),
        Err(WorkError::Denied)
    );
}

#[test]
fn old_tasks_receipts_and_project_policies_load_without_opt_in() {
    let (mut store, scope, access, id) = fixture();
    let project = store.tasks[&id].project_id;
    store.automation.insert(
        project,
        store.scope_automation.remove(&scope.key()).unwrap(),
    );
    let mut json = serde_json::to_value(&store).unwrap();
    json.as_object_mut().unwrap().remove("scope_automation");
    let task = json["tasks"][id.to_string()].as_object_mut().unwrap();
    task.remove("remind_at");
    task.remove("reminder_revision");
    json["deliveries"] = serde_json::json!({"100": {"id":100,"project_id":project,
        "recipient":101,"local_day":"2026-09-24","at":1790254800u64,
        "state":"Attempting","message_id":null}});
    let loaded: WorkStore = serde_json::from_value(json).unwrap();
    assert_eq!(loaded.tasks[&id].remind_at, None);
    assert_eq!(loaded.tasks[&id].reminder_revision, 0);
    assert!(loaded.deliveries[&100].scope.is_none());
    assert!(loaded.deliveries[&100].coverage.is_empty());
    assert_eq!(
        loaded.next_batch(&scope, access, timestamp("2026-09-24T13:00:00Z")),
        Ok(None)
    );
}

#[test]
fn invalid_time_snoozed_and_finished_tasks_are_not_scheduled() {
    let (mut store, scope, access, id) = fixture();
    assert_eq!(
        store.next_batch(&scope, access, u64::MAX),
        Err(WorkError::Invalid)
    );
    assert_eq!(
        store.set_reminder(access, id, 0, Some(u64::MAX)),
        Err(WorkError::Invalid)
    );
    let extreme = u64::try_from(DateTime::<Utc>::MAX_UTC.timestamp()).unwrap();
    assert_eq!(
        store.next_batch(&scope, access, extreme),
        Err(WorkError::Invalid)
    );
    assert_eq!(
        store.set_reminder(access, id, 0, Some(extreme)),
        Err(WorkError::Invalid)
    );
    let now = timestamp("2026-09-24T13:00:00Z");
    store.set_reminder(access, id, 0, Some(now)).unwrap();
    store
        .update_task(access, id, 1, WorkStatus::Open, Some(now + 60))
        .unwrap();
    assert_eq!(store.next_batch(&scope, access, now), Ok(None));
    assert!(
        store
            .next_batch(&scope, access, now + 60)
            .unwrap()
            .is_some()
    );
    store
        .update_task(access, id, 2, WorkStatus::Cancelled, None)
        .unwrap();
    assert_eq!(store.next_batch(&scope, access, now + 60), Ok(None));
}

#[test]
fn evening_briefing_delays_across_midnight_and_dst_with_stable_occurrence() {
    for (evening, morning, occurrence) in [
        ("2026-09-25T03:00:00Z", "2026-09-25T12:00:00Z", "2026-09-24"),
        ("2026-03-08T04:00:00Z", "2026-03-08T12:00:00Z", "2026-03-07"),
        ("2026-11-01T03:00:00Z", "2026-11-01T13:00:00Z", "2026-10-31"),
    ] {
        let (mut store, scope, access, id) = fixture();
        let policy = store.scope_automation.get_mut(&scope.key()).unwrap();
        policy.briefing_hour = 23;
        policy.daily_limit = 1;
        let at = timestamp(morning);
        assert_eq!(
            store.next_batch(&scope, access, timestamp(evening)),
            Ok(None)
        );
        assert_eq!(store.next_batch(&scope, access, at - 1), Ok(None));
        let batch = store.next_batch(&scope, access, at).unwrap().unwrap();
        assert_eq!(batch.kind, WorkDeliveryKind::Briefing);
        assert!(
            batch
                .dedupe_keys
                .contains(&format!("{}:{occurrence}:briefing:scope", scope.key()))
        );
        assert_ne!(batch.local_day, occurrence);
        let receipt = store.reserve_batch(access, &batch, at).unwrap();
        assert_eq!(store.deliveries[&receipt].at, at);
        assert_eq!(store.deliveries[&receipt].local_day, batch.local_day);
        let mut loaded: WorkStore =
            serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(loaded.next_batch(&scope, access, at), Ok(None));
        // Quota is charged to this morning, not the previous occurrence date.
        loaded.set_reminder(access, id, 0, Some(at + 1)).unwrap();
        assert_eq!(loaded.next_batch(&scope, access, at + 1), Ok(None));
        // A policy edit cannot create a second briefing on this delivery day.
        let policy = loaded.scope_automation.get_mut(&scope.key()).unwrap();
        policy.daily_limit = 4;
        policy.briefing_hour = 9;
        loaded.set_reminder(access, id, 1, None).unwrap();
        assert_eq!(loaded.next_batch(&scope, access, at + 3600), Ok(None));
    }
}

#[test]
fn normal_briefing_does_not_repeat_yesterday_before_today_is_due() {
    let (mut store, scope, access, _) = fixture();
    let yesterday = timestamp("2026-09-24T13:00:00Z");
    let batch = store
        .next_batch(&scope, access, yesterday)
        .unwrap()
        .unwrap();
    store.reserve_batch(access, &batch, yesterday).unwrap();
    let loaded: WorkStore = serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    assert_eq!(
        loaded.next_batch(&scope, access, timestamp("2026-09-25T12:00:00Z")),
        Ok(None)
    );
    let next = loaded
        .next_batch(&scope, access, yesterday + 86400)
        .unwrap()
        .unwrap();
    assert!(
        next.dedupe_keys
            .contains("personal:1:2026-09-25:briefing:scope")
    );
}

#[test]
fn delayed_briefing_skips_backlog_and_repeated_tick_after_reload() {
    let (mut store, scope, access, _) = fixture();
    store
        .scope_automation
        .get_mut(&scope.key())
        .unwrap()
        .briefing_hour = 23;
    let now = timestamp("2026-09-25T12:00:00Z");
    let batch = store.next_batch(&scope, access, now).unwrap().unwrap();
    store.reserve_batch(access, &batch, now).unwrap();
    // Days of missed runs yield just the latest overnight occurrence.
    let later = now + 86400 * 5;
    let next = store.next_batch(&scope, access, later).unwrap().unwrap();
    assert!(
        next.dedupe_keys
            .contains("personal:1:2026-09-29:briefing:scope")
    );
    store.reserve_batch(access, &next, later).unwrap();
    let loaded: WorkStore = serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    assert_eq!(loaded.next_batch(&scope, access, later + 60), Ok(None));
    assert_eq!(loaded.deliveries.len(), 2);
}

#[test]
fn deliveries_never_consume_another_scopes_quota_or_reminder_coverage() {
    let (mut store, first_scope, first_access, first_id) = fixture();
    let second_access = personal(2);
    let project = store
        .create_project(second_access, "Other owner", "other")
        .unwrap();
    let second_scope = store.projects[&project].scope.clone();
    let second_id = store
        .add_task(second_access, task(project), "other task")
        .unwrap();
    let mut policy = store.scope_automation[&first_scope.key()].clone();
    policy.daily_limit = 1;
    policy.destination = Some(second_access.channel);
    store
        .configure_automation(&second_scope, second_access, policy)
        .unwrap();
    store
        .scope_automation
        .get_mut(&first_scope.key())
        .unwrap()
        .daily_limit = 1;
    let now = timestamp("2026-09-24T12:00:00Z");
    store
        .set_reminder(first_access, first_id, 0, Some(now))
        .unwrap();
    store
        .set_reminder(second_access, second_id, 0, Some(now))
        .unwrap();
    let first = store
        .next_batch(&first_scope, first_access, now)
        .unwrap()
        .unwrap();
    store.reserve_batch(first_access, &first, now).unwrap();
    assert_eq!(store.next_batch(&first_scope, first_access, now), Ok(None));
    let other = store
        .next_batch(&second_scope, second_access, now)
        .unwrap()
        .unwrap();
    assert_eq!(other.kind, WorkDeliveryKind::Reminder);
    assert_eq!(other.task_ids, vec![second_id]);
    assert_eq!(other.coverage.len(), 1);
    assert_eq!(other.coverage[0].task_id, second_id);
    store.reserve_batch(second_access, &other, now).unwrap();
    assert_eq!(store.deliveries.len(), 2);
}

#[test]
fn explicit_snooze_rearms_only_existing_reminders_and_preserves_deadline() {
    let (mut store, scope, access, id) = fixture();
    let first = timestamp("2026-09-24T13:00:00Z");
    let later = first + 3600;
    store.set_reminder(access, id, 0, Some(first)).unwrap();
    let batch = store.next_batch(&scope, access, first).unwrap().unwrap();
    store.reserve_batch(access, &batch, first).unwrap();
    store.snooze_task(access, id, 1, later).unwrap();
    assert!(
        store
            .next_batch(&scope, access, first + 1)
            .unwrap()
            .is_none()
    );
    let next = store.next_batch(&scope, access, later).unwrap().unwrap();
    assert_eq!(next.kind, WorkDeliveryKind::Reminder);
    store.reserve_batch(access, &next, later).unwrap();
    assert!(
        store
            .next_batch(&scope, access, later + 1)
            .unwrap()
            .is_none()
    );
    let project = store.tasks[&id].project_id;
    let fresh = store
        .add_task(access, task(project), "no reminder")
        .unwrap();
    let deadline = store.tasks[&fresh].due_at;
    store.snooze_task(access, fresh, 0, later + 100).unwrap();
    assert_eq!(store.tasks[&fresh].remind_at, None);
    assert_eq!(store.tasks[&fresh].due_at, deadline);
}

#[test]
fn missing_legacy_automation_authorizer_never_sends() {
    let (mut store, scope, access, _) = fixture();
    assert_eq!(
        store.scope_automation_actors.get(&scope.key()),
        Some(&access.actor)
    );
    store.scope_automation_actors.clear();
    assert_eq!(
        store.next_batch(&scope, access, timestamp("2026-09-24T13:00:00Z")),
        Ok(None)
    );
}
