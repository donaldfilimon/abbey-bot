//! Apply as src/work/follow_up/tests.rs. Draft only; root observes actual RED.
use super::*;
use crate::engagement::{MemberPolicy, schedule::CandidateProposal};
use crate::work::WorkProject;
use std::collections::BTreeSet;

const NOW: u64 = 1_700_000_000;
const MEMBER: u64 = 7;
const PROJECT: u64 = 3;
const TASK: u64 = 4;

#[test]
fn follow_up_request_contract_preserves_both_explicit_owner_dm_choices() {
    for preference in [
        DestinationPreference::Origin,
        DestinationPreference::Private,
    ] {
        let (mut work, mut intent, mut source, mut access) = fixture();
        intent.scope = WorkScope::Personal { owner: MEMBER };
        intent.destination = WorkDestination::Personal { principal: MEMBER };
        work.projects.get_mut(&PROJECT).unwrap().scope = intent.scope.clone();
        source.scope = EngagementScope::Dm {
            member: MEMBER,
            channel: 2,
        };
        access.guild = None;
        work.engagement
            .member_policies
            .get_mut(&MEMBER)
            .unwrap()
            .destinations
            .insert(source.scope.clone(), preference);
        remember_source(&mut work, source.clone(), 11);
        let id = work
            .propose_task_follow_up(intent, source, access, 11, NOW)
            .unwrap()
            .unwrap();
        assert_eq!(work.engagement.candidates[&id].destination, preference);
    }
}

#[test]
fn follow_up_request_contract_sixty_seconds_does_not_restart_after_proof() {
    let (mut work, mut intent, mut source, access) = fixture();
    source.at = NOW - 86_400 - 1;
    remember_source(&mut work, source.clone(), 11);
    intent.expires_at = checked_expiry_seconds(NOW, 60).unwrap();
    let id = work
        .propose_task_follow_up(intent.clone(), source.clone(), access, 11, NOW + 1)
        .unwrap()
        .unwrap();
    assert_eq!(work.engagement.candidates[&id].expires_at, Some(NOW + 60));
    assert_eq!(
        work.engagement.reserve(id, 1, NOW + 60),
        Err(WorkError::Stale)
    );
}

fn access() -> WorkAccess {
    WorkAccess {
        actor: MEMBER,
        guild: Some(1),
        channel: 2,
        can_view: true,
        can_manage: false,
    }
}
fn fixture() -> (WorkStore, FollowUpIntent, SourceRef, WorkAccess) {
    let origin = EngagementScope::Guild {
        guild: 1,
        channel: 2,
    };
    let scope = WorkScope::Team {
        guild: 1,
        channel: 2,
    };
    let source = SourceRef {
        scope: origin.clone(),
        message: 10,
        author: MEMBER,
        revision: 1,
        at: NOW - 86_400 + 120,
    };
    let mut store = WorkStore {
        sequence: TASK,
        ..Default::default()
    };
    store.projects.insert(
        PROJECT,
        WorkProject {
            id: PROJECT,
            name: "Checked project".into(),
            scope: scope.clone(),
            managers: BTreeSet::from([MEMBER]),
            members: BTreeSet::from([MEMBER]),
            revision: 0,
            allowed_github_repositories: BTreeSet::new(),
        },
    );
    store.tasks.insert(
        TASK,
        WorkTask {
            id: TASK,
            project_id: PROJECT,
            title: "Use checked allocation".into(),
            owner: MEMBER,
            assignee: None,
            goal_id: None,
            priority: 0,
            status: WorkStatus::Open,
            due_at: None,
            remind_at: None,
            reminder_revision: 0,
            snoozed_until: None,
            source: None,
            github: None,
            revision: 0,
        },
    );
    store.engagement.member_policies.insert(
        MEMBER,
        MemberPolicy {
            daily_limit: Some(1),
            weekly_limit: Some(7),
            timezone: Some("UTC".into()),
            quiet_start: 0,
            quiet_end: 0,
            ..MemberPolicy::default()
        },
    );
    store
        .engagement
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(origin, DestinationPreference::Origin);
    remember_source(&mut store, source.clone(), 11);
    let intent = FollowUpIntent {
        scope,
        task: WorkContentRef::Task {
            project: PROJECT,
            id: TASK,
            revision: 0,
        },
        destination: WorkDestination::TeamChannel { channel: 2 },
        expires_at: checked_expiry_seconds(NOW, 300).unwrap(),
    };
    (store, intent, source, access())
}
fn remember_source(store: &mut WorkStore, source: SourceRef, response: u64) {
    store.engagement.responses.insert(source.message, response);
    store
        .engagement
        .eligibility
        .entry(source.author)
        .or_default()
        .insert(source.clone());
    store
        .engagement
        .observations
        .entry(source.scope.clone())
        .or_default()
        .insert(source.author, source);
}
fn propose(
    store: &mut WorkStore,
    intent: FollowUpIntent,
    source: SourceRef,
    access: WorkAccess,
) -> Result<Option<u64>, WorkError> {
    let response = store
        .engagement
        .responses
        .get(&source.message)
        .copied()
        .unwrap_or(0);
    store.propose_task_follow_up(intent, source, access, response, NOW)
}
fn facts() -> FollowUpFacts {
    FollowUpFacts {
        authorized: true,
        source_current: true,
        completed: false,
        expires_at: NOW + 60,
        activity_ready: true,
        member_policy: FollowUpDecision::Allowed,
        already_attempted: false,
    }
}

#[test]
fn follow_up_native_revision_zero_is_valid_and_no_work_task_is_created_or_rewritten() {
    let (mut store, intent, source, access) = fixture();
    let tasks = store.tasks.clone();
    let id = propose(&mut store, intent.clone(), source.clone(), access)
        .unwrap()
        .unwrap();
    let candidate = &store.engagement.candidates[&id];
    assert_eq!(candidate.work_ref, Some(intent.task));
    assert_eq!(candidate.due_at, source.at + 86_400);
    assert_eq!(candidate.expires_at, Some(NOW + 300));
    assert_eq!(candidate.state, CandidateState::Pending);
    assert_eq!(store.tasks, tasks);
    assert_eq!(store.sequence, TASK);
    assert!(store.engagement.charges.is_empty());
    store.engagement.validate().unwrap();
}

#[test]
fn follow_up_exact_task_source_revision_and_current_exchange_are_required() {
    for change in 0..7 {
        let (mut store, mut intent, mut source, mut access) = fixture();
        match change {
            0 => store.tasks.get_mut(&TASK).unwrap().revision = 1,
            1 => store.tasks.get_mut(&TASK).unwrap().status = WorkStatus::Done,
            2 => store.tasks.get_mut(&TASK).unwrap().status = WorkStatus::Cancelled,
            3 => {
                intent.task = WorkContentRef::Decision {
                    project: PROJECT,
                    id: TASK,
                    revision: 1,
                }
            }
            4 => source.revision = 2,
            5 => {
                store.engagement.responses.insert(source.message, 99);
            }
            6 => access.can_view = false,
            _ => unreachable!(),
        }
        let before = serde_json::to_vec(&store.engagement).unwrap();
        assert!(
            store
                .propose_task_follow_up(intent, source, access, 11, NOW)
                .is_err(),
            "change {change}"
        );
        assert_eq!(serde_json::to_vec(&store.engagement).unwrap(), before);
    }
}

#[test]
fn follow_up_foreign_guild_dm_author_and_missing_source_refuse() {
    for change in 0..5 {
        let (mut store, intent, mut source, access) = fixture();
        match change {
            0 => {
                source.scope = EngagementScope::Guild {
                    guild: 99,
                    channel: 2,
                }
            }
            1 => {
                source.scope = EngagementScope::Dm {
                    member: MEMBER,
                    channel: 2,
                }
            }
            2 => source.author = 99,
            3 => {
                store.engagement.eligibility.clear();
            }
            4 => {
                store.engagement.observations.clear();
            }
            _ => unreachable!(),
        }
        assert!(propose(&mut store, intent, source, access).is_err());
        assert!(store.engagement.candidates.is_empty());
    }
}

#[test]
fn follow_up_guild_origin_permission_does_not_authorize_private_or_another_guild() {
    let (mut store, mut intent, source, access) = fixture();
    intent.destination = WorkDestination::TeamPrivate { principal: MEMBER };
    assert!(propose(&mut store, intent.clone(), source.clone(), access).is_err());
    store
        .engagement
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(
            EngagementScope::Guild {
                guild: 99,
                channel: 2,
            },
            DestinationPreference::Private,
        );
    assert!(propose(&mut store, intent.clone(), source.clone(), access).is_err());
    store
        .engagement
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(source.scope.clone(), DestinationPreference::Private);
    assert!(
        propose(&mut store, intent, source, access)
            .unwrap()
            .is_some()
    );
}

#[test]
fn follow_up_request_expiry_bounds_due_equality_and_overflow_are_checked() {
    assert!(checked_expiry_seconds(NOW, 59).is_err());
    assert_eq!(checked_expiry_seconds(NOW, 60).unwrap(), NOW + 60);
    assert_eq!(
        checked_expiry_seconds(NOW, 7 * 86_400).unwrap(),
        NOW + 7 * 86_400
    );
    assert!(checked_expiry_seconds(NOW, 7 * 86_400 + 1).is_err());
    assert!(checked_expiry_seconds(u64::MAX, 60).is_err());
    let (mut store, mut intent, source, access) = fixture();
    intent.expires_at = source.at + 86_400;
    assert_eq!(
        propose(&mut store, intent, source, access),
        Err(WorkError::Invalid)
    );
    let (_, _, mut source, _) = fixture();
    source.at = u64::MAX;
    assert!(follow_up_due(&source).is_err());
}

#[test]
fn follow_up_decisions_are_closed_and_authorization_precedes_private_task_state() {
    let mut f = facts();
    assert_eq!(evaluate_follow_up(&f, NOW), FollowUpDecision::Allowed);
    f.authorized = false;
    f.completed = true;
    f.expires_at = NOW;
    f.already_attempted = true;
    assert_eq!(evaluate_follow_up(&f, NOW), FollowUpDecision::AccessDenied);
    f = facts();
    f.completed = true;
    assert_eq!(evaluate_follow_up(&f, NOW), FollowUpDecision::StaleTask);
    f = facts();
    f.source_current = false;
    assert_eq!(evaluate_follow_up(&f, NOW), FollowUpDecision::StaleTask);
    f = facts();
    f.expires_at = NOW;
    assert_eq!(evaluate_follow_up(&f, NOW), FollowUpDecision::Expired);
    f = facts();
    f.already_attempted = true;
    assert_eq!(
        evaluate_follow_up(&f, NOW),
        FollowUpDecision::AlreadyAttempted
    );
    for reason in [
        FollowUpDecision::Disabled,
        FollowUpDecision::Quiet,
        FollowUpDecision::OptedOut,
        FollowUpDecision::Budget,
        FollowUpDecision::Cooldown,
    ] {
        f = facts();
        f.member_policy = reason;
        assert_eq!(evaluate_follow_up(&f, NOW), reason);
    }
    f = facts();
    f.activity_ready = false;
    assert_eq!(
        evaluate_follow_up(&f, NOW),
        FollowUpDecision::ActivityUnavailable
    );
}

#[test]
fn follow_up_plain_task_needs_no_activity_and_inspection_never_charges() {
    let (store, intent, source, access) = fixture();
    let before = serde_json::to_vec(&store.engagement).unwrap();
    let f = store
        .task_follow_up_facts(&intent, &source, access, 11, NOW)
        .unwrap();
    assert!(f.activity_ready);
    assert_eq!(evaluate_follow_up(&f, NOW), FollowUpDecision::Allowed);
    assert_eq!(serde_json::to_vec(&store.engagement).unwrap(), before);
}

#[test]
fn follow_up_two_human_sources_and_other_recipient_cannot_repeat_one_task_revision() {
    let (mut store, intent, source, access) = fixture();
    propose(&mut store, intent.clone(), source, access)
        .unwrap()
        .unwrap();
    store.projects.get_mut(&PROJECT).unwrap().members.insert(8);
    let policy = store.engagement.member_policies[&MEMBER].clone();
    store.engagement.member_policies.insert(8, policy);
    let source = SourceRef {
        scope: EngagementScope::Guild {
            guild: 1,
            channel: 2,
        },
        message: 20,
        author: 8,
        revision: 1,
        at: NOW - 86_400 + 120,
    };
    remember_source(&mut store, source.clone(), 21);
    let another = WorkAccess { actor: 8, ..access };
    assert_eq!(propose(&mut store, intent, source, another).unwrap(), None);
    assert_eq!(store.engagement.candidates.len(), 1);
}

#[test]
fn follow_up_conversation_candidate_is_never_upgraded_on_source_collision() {
    let (mut store, intent, source, access) = fixture();
    let id = store
        .engagement
        .propose(
            CandidateProposal {
                kind: EngagementKind::FollowUp,
                source: Some(source.clone()),
                member: Some(MEMBER),
                scope: source.scope.clone(),
                due_at: follow_up_due(&source).unwrap(),
                introduction_id: None,
            },
            NOW,
        )
        .unwrap()
        .unwrap();
    let before = serde_json::to_vec(&store.engagement).unwrap();
    assert_eq!(propose(&mut store, intent, source, access).unwrap(), None);
    assert_eq!(serde_json::to_vec(&store.engagement).unwrap(), before);
    assert!(store.engagement.candidates[&id].work_ref.is_none());
}

#[test]
fn follow_up_cancelled_rejected_sent_and_review_required_consume_task_identity() {
    for terminal in [
        CandidateState::Cancelled,
        CandidateState::Rejected,
        CandidateState::Sent,
        CandidateState::ReviewRequired,
    ] {
        let (mut store, intent, source, access) = fixture();
        let id = propose(&mut store, intent.clone(), source.clone(), access)
            .unwrap()
            .unwrap();
        let c = store.engagement.candidates.get_mut(&id).unwrap();
        c.state = terminal;
        c.message_id = (terminal == CandidateState::Sent).then_some(100);
        let fresh = SourceRef {
            message: 20,
            ..source
        };
        remember_source(&mut store, fresh.clone(), 21);
        assert_eq!(propose(&mut store, intent, fresh, access).unwrap(), None);
    }
}

#[test]
fn follow_up_erasure_retains_both_source_and_task_commitments_through_restart() {
    let (mut store, intent, source, access) = fixture();
    propose(&mut store, intent.clone(), source.clone(), access)
        .unwrap()
        .unwrap();
    store
        .engagement
        .erase_learning("discord:1", Some(MEMBER))
        .unwrap();
    assert!(store.engagement.candidates.is_empty());
    assert!(store.engagement.follow_up_source_attempted(&source, MEMBER));
    assert!(
        store
            .engagement
            .task_follow_up_attempted(&intent.scope, &intent.task)
    );
    let bytes = serde_json::to_vec(&store.engagement).unwrap();
    store.engagement = serde_json::from_slice(&bytes).unwrap();
    let fresh = SourceRef {
        message: 20,
        ..source
    };
    remember_source(&mut store, fresh.clone(), 21);
    assert_eq!(propose(&mut store, intent, fresh, access).unwrap(), None);
}

#[test]
fn follow_up_personal_task_dedupe_survives_different_native_dm_channel() {
    let (mut store, mut intent, mut source, mut access) = fixture();
    intent.scope = WorkScope::Personal { owner: MEMBER };
    intent.destination = WorkDestination::Personal { principal: MEMBER };
    store.projects.get_mut(&PROJECT).unwrap().scope = intent.scope.clone();
    source.scope = EngagementScope::Dm {
        member: MEMBER,
        channel: 2,
    };
    store
        .engagement
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(source.scope.clone(), DestinationPreference::Origin);
    access.guild = None;
    remember_source(&mut store, source.clone(), 11);
    propose(&mut store, intent.clone(), source.clone(), access)
        .unwrap()
        .unwrap();
    source.scope = EngagementScope::Dm {
        member: MEMBER,
        channel: 22,
    };
    store
        .engagement
        .member_policies
        .get_mut(&MEMBER)
        .unwrap()
        .destinations
        .insert(source.scope.clone(), DestinationPreference::Origin);
    source.message = 20;
    access.channel = 22;
    remember_source(&mut store, source.clone(), 21);
    assert_eq!(propose(&mut store, intent, source, access).unwrap(), None);
}

#[test]
fn follow_up_canonical_metadata_defaults_old_rows_and_rejects_half_links() {
    let (mut store, intent, source, access) = fixture();
    let id = propose(&mut store, intent, source, access)
        .unwrap()
        .unwrap();
    for change in 0..4 {
        let mut next = store.engagement.clone();
        let c = next.candidates.get_mut(&id).unwrap();
        match change {
            0 => c.work_ref = None,
            1 => c.expires_at = None,
            2 => c.due_at += 1,
            3 => c.dedupe_key = "engagement:forged".into(),
            _ => unreachable!(),
        }
        assert!(next.validate().is_err(), "change {change}");
        assert!(
            serde_json::from_slice::<EngagementStore>(&serde_json::to_vec(&next).unwrap()).is_err()
        );
    }
    let (mut old, _, source, _) = fixture();
    let id = old
        .engagement
        .propose(
            CandidateProposal {
                kind: EngagementKind::FollowUp,
                source: Some(source.clone()),
                member: Some(MEMBER),
                scope: source.scope.clone(),
                due_at: follow_up_due(&source).unwrap(),
                introduction_id: None,
            },
            NOW,
        )
        .unwrap()
        .unwrap();
    let mut value = serde_json::to_value(&old.engagement).unwrap();
    let row = value["candidates"][id.to_string()].as_object_mut().unwrap();
    row.remove("work_ref");
    row.remove("expires_at");
    row.remove("follow_up_reason");
    let loaded: EngagementStore = serde_json::from_value(value).unwrap();
    assert!(loaded.candidates[&id].work_ref.is_none());
    assert!(loaded.candidates[&id].expires_at.is_none());
    assert!(loaded.candidates[&id].follow_up_reason.is_none());
}

#[test]
fn follow_up_current_task_render_is_bounded_and_revocation_refuses_detail() {
    let (store, intent, source, access) = fixture();
    let text = store
        .render_follow_up_task(&intent, &source, access)
        .unwrap();
    println!("Actual authorized task context: {text}");
    assert!(text.contains("Current Task (quoted data)"));
    assert!(text.contains("Use checked allocation"));
    assert!(!text.contains("Checked project"));
    assert!(!text.contains("assignee"));
    let revoked = WorkAccess {
        can_view: false,
        ..access
    };
    assert_eq!(
        store.render_follow_up_task(&intent, &source, revoked),
        Err(WorkError::Denied)
    );
}
