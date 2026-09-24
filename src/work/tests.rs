use super::*;

pub(super) fn personal(user: u64) -> WorkAccess {
    WorkAccess {
        actor: user,
        guild: None,
        channel: user + 100,
        can_view: true,
        can_manage: false,
    }
}

pub(super) fn team(user: u64, channel: u64, can_view: bool, can_manage: bool) -> WorkAccess {
    WorkAccess {
        actor: user,
        guild: Some(9),
        channel,
        can_view,
        can_manage,
    }
}

pub(super) fn task(project_id: u64) -> WorkTask {
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
        remind_at: None,
        reminder_revision: 0,
        snoozed_until: None,
        source: None,
        github: None,
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
    assert_eq!(store.visible_projects(team(2, 99, true, false)).len(), 0);
    assert_eq!(store.visible_projects(personal(1)).len(), 1);
}

#[test]
fn preference_keys_isolate_users_guild_channels_and_personal_space() {
    let mut store = WorkStore::default();
    let personal_key = personal(1).preference_key();
    let team_key = team(1, 99, true, false).preference_key();
    store
        .preferences
        .entry(team_key.clone())
        .or_default()
        .learned_hour = Some(10);
    assert_ne!(personal_key, team_key);
    assert_ne!(team_key, team(1, 100, true, false).preference_key());
    assert_ne!(team_key, team(2, 99, true, false).preference_key());
    assert!(!store.preferences.contains_key(&personal_key));
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
                actor: Some(1),
                scope: Some(WorkScope::Personal { owner: 1 }),
                kind: Some(WorkDeliveryKind::Briefing),
                feedback: WorkFeedback::Snoozed { hour: 10 },
                at: id,
            })
            .unwrap();
    }
    assert_eq!(profile.effective_hour(9), 9);
    profile
        .observe(PreferenceEvidence {
            delivery_id: 5,
            actor: Some(1),
            scope: Some(WorkScope::Personal { owner: 1 }),
            kind: Some(WorkDeliveryKind::Briefing),
            feedback: WorkFeedback::Snoozed { hour: 10 },
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
fn legacy_state_defaults_to_empty_work_store() {
    let state: WorkStore = serde_json::from_str("{}").unwrap();
    assert_eq!(state, WorkStore::default());
}

#[test]
fn older_project_and_task_rows_default_new_github_fields() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store.create_project(access, "Personal", "project").unwrap();
    let task_id = store.add_task(access, task(project), "task").unwrap();
    let mut old = serde_json::to_value(&store).unwrap();
    old["projects"][project.to_string()]
        .as_object_mut()
        .unwrap()
        .remove("allowed_github_repositories");
    old["tasks"][task_id.to_string()]
        .as_object_mut()
        .unwrap()
        .remove("github");
    old.as_object_mut().unwrap().remove("github_snapshots");
    let restored: WorkStore = serde_json::from_value(old).unwrap();
    assert!(
        restored.projects[&project]
            .allowed_github_repositories
            .is_empty()
    );
    assert_eq!(restored.tasks[&task_id].github, None);
    assert!(restored.github_snapshots.is_empty());
}

#[test]
fn github_link_requires_manager_allowlist_and_current_channel_access() {
    let mut store = WorkStore::default();
    let lead = team(1, 99, true, true);
    let project = store.create_project(lead, "Release", "project").unwrap();
    store.set_member(project, lead, 2, true).unwrap();
    let member = team(2, 99, true, false);
    let task_id = store.add_task(member, task(project), "task").unwrap();
    let repository = GitHubRepository {
        installation: 7,
        owner: "team".into(),
        name: "repo".into(),
    };
    let reference = GitHubReference {
        repository: repository.clone(),
        kind: GitHubItemKind::Issue,
        number: 3,
    };
    assert_eq!(
        store.link_github(task_id, 0, member, reference.clone()),
        Err(WorkError::Denied)
    );
    assert_eq!(
        store.allow_github_repository(project, member, repository.clone(), true),
        Err(WorkError::Denied)
    );
    store
        .allow_github_repository(project, lead, repository.clone(), true)
        .unwrap();
    assert_eq!(
        store.link_github(task_id, 0, team(2, 99, false, false), reference.clone()),
        Err(WorkError::Denied)
    );
    store
        .link_github(task_id, 0, member, reference.clone())
        .unwrap();
    store
        .record_github_snapshot(
            &reference,
            GitHubSnapshot {
                title: "Release issue".into(),
                state: GitHubState::Open,
                refreshed_at: 42,
                stale: false,
                etag: Some("v1".into()),
            },
        )
        .unwrap();
    store.mark_github_stale(&reference);
    let briefing = store.briefing(project, member, 50).unwrap();
    assert!(briefing.contains("stale <t:42:R>"), "{briefing}");
    assert!(briefing.contains(&reference.url()), "{briefing}");
    assert_eq!(
        store.confirm_github_not_modified(&reference, 41),
        Err(WorkError::Stale)
    );
    store.confirm_github_not_modified(&reference, 50).unwrap();
    let briefing = store.briefing(project, member, 51).unwrap();
    assert!(briefing.contains("refreshed <t:50:R>"), "{briefing}");
    store
        .allow_github_repository(project, lead, repository, false)
        .unwrap();
    assert!(
        !store
            .briefing(project, member, 50)
            .unwrap()
            .contains("GitHub:")
    );
}

#[test]
fn github_snapshot_text_cannot_create_extra_briefing_lines() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store.create_project(access, "Personal", "project").unwrap();
    let task_id = store.add_task(access, task(project), "task").unwrap();
    let repository = GitHubRepository {
        installation: 7,
        owner: "team".into(),
        name: "repo".into(),
    };
    store
        .allow_github_repository(project, access, repository.clone(), true)
        .unwrap();
    let reference = GitHubReference {
        repository,
        kind: GitHubItemKind::PullRequest,
        number: 8,
    };
    store
        .link_github(task_id, 0, access, reference.clone())
        .unwrap();
    store
        .record_github_snapshot(
            &reference,
            GitHubSnapshot {
                title: "Normal\n@everyone **Fake instruction**".into(),
                state: GitHubState::Open,
                refreshed_at: 42,
                stale: false,
                etag: None,
            },
        )
        .unwrap();
    let briefing = store.briefing(project, access, 50).unwrap();
    assert!(
        briefing.contains("Normal@\u{200b}everyone \\*\\*Fake instruction\\*\\*"),
        "{briefing}"
    );
    assert!(!briefing.contains("Normal\nFake"));
}

#[test]
fn prepopulated_github_link_cannot_bypass_allowlist() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store.create_project(access, "Private", "project").unwrap();
    let reference = GitHubReference {
        repository: GitHubRepository {
            installation: 7,
            owner: "team".into(),
            name: "repo".into(),
        },
        kind: GitHubItemKind::Issue,
        number: 3,
    };
    let mut draft = task(project);
    draft.github = Some(reference.clone());
    assert_eq!(
        store.add_task(access, draft.clone(), "task"),
        Err(WorkError::Invalid)
    );
    assert!(store.tasks.is_empty());
    store
        .allow_github_repository(project, access, reference.repository.clone(), true)
        .unwrap();
    assert!(store.add_task(access, draft, "task").is_ok());
}

#[test]
fn legacy_github_source_migration_is_explicit_scoped_and_unambiguous() {
    let mut store = WorkStore::default();
    let lead = team(1, 99, true, true);
    let project = store.create_project(lead, "Shared", "project").unwrap();
    store.set_member(project, lead, 2, true).unwrap();
    let member = team(2, 99, true, false);
    let mut draft = task(project);
    draft.source = Some("https://github.com/Team/Repo/issues/42".into());
    let id = store.add_task(member, draft, "task").unwrap();
    let other = store
        .create_project(personal(1), "Personal", "personal")
        .unwrap();
    let mut private = task(other);
    private.source = Some("https://github.com/Team/Repo/issues/42".into());
    let private_id = store
        .add_task(personal(1), private, "private-task")
        .unwrap();
    assert_eq!(
        store.migrate_github_sources(project, member),
        Err(WorkError::Denied)
    );
    assert_eq!(store.migrate_github_sources(project, lead).unwrap(), 0);
    let repo = GitHubRepository {
        installation: 7,
        owner: "Team".into(),
        name: "Repo".into(),
    };
    store
        .allow_github_repository(project, lead, repo.clone(), true)
        .unwrap();
    assert_eq!(store.migrate_github_sources(project, lead).unwrap(), 1);
    assert_eq!(
        store.tasks[&id].github.as_ref().unwrap().repository,
        repo.canonical()
    );
    assert_eq!(store.tasks[&id].revision, 1);
    assert!(store.tasks[&private_id].github.is_none());
    assert_eq!(store.migrate_github_sources(project, lead).unwrap(), 0);
    assert_eq!(
        store.migrate_github_sources(project, team(1, 99, false, true)),
        Err(WorkError::Denied)
    );
}

#[test]
fn repository_identity_is_case_insensitive_for_persisted_links_and_revocation() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store.create_project(access, "Private", "project").unwrap();
    let mixed = GitHubRepository {
        installation: 7,
        owner: "Team".into(),
        name: "Repo".into(),
    };
    let lower = mixed.canonical();
    store
        .allow_github_repository(project, access, mixed.clone(), true)
        .unwrap();
    let id = store.add_task(access, task(project), "task").unwrap();
    let reference = GitHubReference {
        repository: mixed.clone(),
        kind: GitHubItemKind::Issue,
        number: 42,
    };
    store.link_github(id, 0, access, reference.clone()).unwrap();
    assert_eq!(store.tasks[&id].github.as_ref().unwrap().repository, lower);
    // Simulate a persisted row and cache written before canonicalization.
    store.tasks.get_mut(&id).unwrap().github = Some(reference.clone());
    store
        .projects
        .get_mut(&project)
        .unwrap()
        .allowed_github_repositories = BTreeSet::from([mixed.clone()]);
    store.github_snapshots.insert(
        "7:Team/Repo/issues/42".to_string(),
        GitHubSnapshot {
            title: "Private issue".into(),
            state: GitHubState::Open,
            refreshed_at: 10,
            stale: false,
            etag: Some("v1".into()),
        },
    );
    let lower_reference = GitHubReference {
        repository: lower.clone(),
        ..reference.clone()
    };
    assert!(store.active_github_link(&lower_reference));
    assert!(
        store
            .briefing(project, access, 11)
            .unwrap()
            .contains("Private issue")
    );
    store
        .allow_github_repository(project, access, lower.clone(), false)
        .unwrap();
    assert!(
        store.projects[&project]
            .allowed_github_repositories
            .is_empty()
    );
    assert!(!store.active_github_link(&reference));
    assert!(
        !store
            .briefing(project, access, 11)
            .unwrap()
            .contains("Private issue")
    );
    store
        .allow_github_repository(project, access, lower, true)
        .unwrap();
    store
        .confirm_github_not_modified(&lower_reference, 12)
        .unwrap();
    assert!(store.github_snapshots.contains_key(&lower_reference.key()));
    assert!(!store.github_snapshots.contains_key("7:Team/Repo/issues/42"));
}

#[test]
fn legacy_migration_requires_canonical_digits_and_unique_installation() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store.create_project(access, "Private", "project").unwrap();
    let repo = GitHubRepository {
        installation: 7,
        owner: "Team".into(),
        name: "Repo".into(),
    };
    store
        .allow_github_repository(project, access, repo.clone(), true)
        .unwrap();
    let invalid = [
        "https://github.com/Team/Repo/issues/+42",
        "https://github.com/Team/Repo/issues/042",
        "https://github.com/Team/Repo/issues/42?x=1",
        "https://github.com/Team/Repo/issues/42#comment",
    ];
    let mut invalid_ids = Vec::new();
    for (index, source) in invalid.into_iter().enumerate() {
        let mut draft = task(project);
        draft.source = Some(source.into());
        invalid_ids.push(
            store
                .add_task(access, draft, &format!("invalid-{index}"))
                .unwrap(),
        );
    }
    let mut draft = task(project);
    draft.source = Some("https://github.com/Team/Repo/pull/42".into());
    let ambiguous = store.add_task(access, draft, "ambiguous").unwrap();
    store
        .projects
        .get_mut(&project)
        .unwrap()
        .allowed_github_repositories
        .insert(GitHubRepository {
            installation: 8,
            ..repo.clone()
        });
    assert_eq!(store.migrate_github_sources(project, access).unwrap(), 0);
    assert!(store.tasks[&ambiguous].github.is_none());
    store
        .projects
        .get_mut(&project)
        .unwrap()
        .allowed_github_repositories
        .retain(|candidate| candidate.installation == 7);
    assert_eq!(store.migrate_github_sources(project, access).unwrap(), 1);
    assert_eq!(
        store.tasks[&ambiguous].github.as_ref().unwrap().kind,
        GitHubItemKind::PullRequest
    );
    for id in invalid_ids {
        assert!(store.tasks[&id].github.is_none());
    }
}

#[test]
fn github_snapshot_is_hidden_from_other_projects_and_revoked_repositories() {
    let mut store = WorkStore::default();
    let owner = personal(1);
    let private = store.create_project(owner, "Private", "private").unwrap();
    let other = store.create_project(personal(2), "Other", "other").unwrap();
    let repository = GitHubRepository {
        installation: 7,
        owner: "team".into(),
        name: "repo".into(),
    };
    store
        .allow_github_repository(private, owner, repository.clone(), true)
        .unwrap();
    let reference = GitHubReference {
        repository: repository.clone(),
        kind: GitHubItemKind::Issue,
        number: 1,
    };
    let id = store.add_task(owner, task(private), "task").unwrap();
    store.link_github(id, 0, owner, reference.clone()).unwrap();
    store
        .record_github_snapshot(
            &reference,
            GitHubSnapshot {
                title: "Secret title".into(),
                state: GitHubState::Open,
                refreshed_at: 10,
                stale: false,
                etag: None,
            },
        )
        .unwrap();
    assert!(
        !store
            .briefing(other, personal(2), 11)
            .unwrap()
            .contains("Secret title")
    );
    assert_eq!(
        store.briefing(private, personal(2), 11),
        Err(WorkError::Denied)
    );
    store
        .allow_github_repository(private, owner, repository, false)
        .unwrap();
    assert!(!store.active_github_link(&reference));
    assert!(
        !store
            .briefing(private, owner, 11)
            .unwrap()
            .contains("Secret title")
    );
}

#[test]
fn quiet_hours_and_ambiguous_attempts_consume_the_ceiling() {
    let mut store = WorkStore::default();
    let access = personal(1);
    let project = store.create_project(access, "Policy", "policy").unwrap();
    let scope = store.projects[&project].scope.clone();
    store.add_task(access, task(project), "task").unwrap();
    store
        .configure_automation(
            &scope,
            access,
            WorkAutomationPolicy {
                enabled: true,
                destination: Some(access.channel),
                timezone: "America/New_York".into(),
                daily_limit: 1,
                ..Default::default()
            },
        )
        .unwrap();
    let now = |s: &str| {
        u64::try_from(chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()).unwrap()
    };
    assert_eq!(
        store.next_batch(&scope, access, now("2026-09-24T03:00:00Z")),
        Ok(None)
    );
    let at = now("2026-09-24T13:00:00Z");
    let batch = store.next_batch(&scope, access, at).unwrap().unwrap();
    let id = store.reserve_batch(access, &batch, at).unwrap();
    assert_eq!(store.deliveries[&id].state, DeliveryState::Attempting);
    assert_eq!(store.next_batch(&scope, access, at + 3600), Ok(None));
}

#[test]
fn goal_and_decision_requests_survive_reload_and_reauthorize() {
    let mut store = WorkStore::default();
    let lead = team(1, 99, true, true);
    let member = team(2, 99, true, false);
    let project = store.create_project(lead, "Shared", "project").unwrap();
    store.set_member(project, lead, 2, true).unwrap();
    let goal = store
        .add_goal(project, member, "Deliver", "request")
        .unwrap();
    let decision = store
        .record_decision(project, member, "Approved", 10, "request")
        .unwrap();
    assert_ne!(goal, decision);
    let mut store: WorkStore =
        serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    assert_eq!(
        store
            .add_goal(project, member, "Deliver", "request")
            .unwrap(),
        goal
    );
    assert_eq!(
        store
            .record_decision(project, member, "Approved", 11, "request")
            .unwrap(),
        decision
    );
    assert_ne!(
        store.add_goal(project, member, "Deliver", "new").unwrap(),
        goal
    );
    assert_ne!(
        store
            .record_decision(project, member, "Approved", 10, "new")
            .unwrap(),
        decision
    );
    assert_ne!(
        store.add_goal(project, lead, "Deliver", "request").unwrap(),
        goal
    );
    let other = store
        .create_project(lead, "Other", "other-project")
        .unwrap();
    assert_ne!(
        store.add_goal(other, lead, "Deliver", "request").unwrap(),
        goal
    );
    store.set_member(project, lead, 2, false).unwrap();
    assert_eq!(
        store.add_goal(project, member, "Deliver", "request"),
        Err(WorkError::Denied)
    );
    assert_eq!(
        store.record_decision(project, member, "Approved", 10, "request"),
        Err(WorkError::Denied)
    );
}
