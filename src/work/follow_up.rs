//! Pure optional task follow-up authority. Callers provide current native proofs
//! and time; retained Work commits own publication. No task/source is invented.
use super::{
    WorkAccess, WorkContentRef, WorkDestination, WorkError, WorkScope, WorkStatus, WorkStore,
    WorkTask,
};
use crate::calendar::utc;
use crate::engagement::{
    Candidate, CandidateState, DestinationPreference, EngagementKind, EngagementScope,
    EngagementStore, SourceRef,
};
use serde::{Deserialize, Serialize};

pub const MIN_EXPIRY_SECONDS: u64 = 60;
pub const MAX_EXPIRY_SECONDS: u64 = 7 * 86_400;
const FOLLOW_UP_DELAY: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FollowUpDecision {
    Allowed,
    Disabled,
    Quiet,
    OptedOut,
    StaleTask,
    Expired,
    AccessDenied,
    Budget,
    Cooldown,
    AlreadyAttempted,
    ActivityUnavailable,
}
impl FollowUpDecision {
    pub fn message(self) -> &'static str {
        match self {
            Self::Allowed => "This task follow-up is eligible under your current settings.",
            Self::Disabled => {
                "Optional contact is disabled or lacks a positive limit and timezone."
            }
            Self::Quiet => "Optional contact is paused by snooze or quiet hours.",
            Self::OptedOut => {
                "A stop or missing or changed destination prevents this task follow-up."
            }
            Self::StaleTask => "The task or its selected human source is no longer current.",
            Self::Expired => "The requested task follow-up has expired.",
            Self::AccessDenied => "Current access to this task follow-up could not be confirmed.",
            Self::Budget => "The shared contact budget currently prevents this follow-up.",
            Self::Cooldown => "The existing channel cooldown currently prevents this follow-up.",
            Self::AlreadyAttempted => {
                "This task revision or human source already has a follow-up record."
            }
            Self::ActivityUnavailable => {
                "The requested Activity lacks current operator acceptance."
            }
        }
    }
}

/// The current policy decision includes the existing shared capacity and guild
/// limits. Inspecting these facts never reserves or charges an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowUpFacts {
    pub authorized: bool,
    pub source_current: bool,
    pub completed: bool,
    pub expires_at: u64,
    /// Plain native-task follow-ups set true: no Activity is requested.
    pub activity_ready: bool,
    pub member_policy: FollowUpDecision,
    pub already_attempted: bool,
}

pub fn evaluate_follow_up(facts: &FollowUpFacts, now: u64) -> FollowUpDecision {
    if !facts.authorized {
        FollowUpDecision::AccessDenied
    } else if !facts.source_current || facts.completed {
        FollowUpDecision::StaleTask
    } else if utc(now).is_err() || utc(facts.expires_at).is_err() || now >= facts.expires_at {
        FollowUpDecision::Expired
    } else if facts.already_attempted {
        FollowUpDecision::AlreadyAttempted
    } else if facts.member_policy != FollowUpDecision::Allowed {
        facts.member_policy
    } else if !facts.activity_ready {
        FollowUpDecision::ActivityUnavailable
    } else {
        FollowUpDecision::Allowed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpIntent {
    pub scope: WorkScope,
    pub task: WorkContentRef,
    pub destination: WorkDestination,
    pub expires_at: u64,
}

pub fn checked_expiry_seconds(now: u64, seconds: u64) -> Result<u64, WorkError> {
    utc(now)?;
    if !(MIN_EXPIRY_SECONDS..=MAX_EXPIRY_SECONDS).contains(&seconds) {
        return Err(WorkError::Invalid);
    }
    let expires = now.checked_add(seconds).ok_or(WorkError::Invalid)?;
    utc(expires)?;
    Ok(expires)
}

pub(crate) fn follow_up_due(source: &SourceRef) -> Result<u64, WorkError> {
    source.validate()?;
    utc(source.at)?;
    let due = source
        .at
        .checked_add(FOLLOW_UP_DELAY)
        .ok_or(WorkError::Invalid)?;
    utc(due)?;
    Ok(due)
}

pub(crate) fn work_scope(origin: &EngagementScope) -> WorkScope {
    match *origin {
        EngagementScope::Guild { guild, channel } => WorkScope::Team { guild, channel },
        EngagementScope::Dm { member, .. } => WorkScope::Personal { owner: member },
    }
}

/// Deterministic Work identity, deliberately independent of recipient, human
/// source and transport DM channel. Task revision zero is a native revision.
pub(crate) fn task_key(scope: &WorkScope, task: &WorkContentRef) -> Result<String, WorkError> {
    validate_scope(scope)?;
    let WorkContentRef::Task {
        project,
        id,
        revision,
    } = *task
    else {
        return Err(WorkError::Invalid);
    };
    if project == 0 || id == 0 {
        return Err(WorkError::Invalid);
    }
    Ok(format!(
        "task-follow-up\u{1f}{}\u{1f}{project}\u{1f}{id}\u{1f}{revision}",
        scope.key()
    ))
}

fn validate_scope(scope: &WorkScope) -> Result<(), WorkError> {
    if matches!(
        scope,
        WorkScope::Personal { owner: 0 }
            | WorkScope::Team { guild: 0, .. }
            | WorkScope::Team { channel: 0, .. }
    ) {
        Err(WorkError::Invalid)
    } else {
        Ok(())
    }
}

/// Exact saved Engagement choice; this does not inspect Discord or widen the
/// origin. TeamPrivate requires the saved choice for this exact guild/channel.
pub(crate) fn preferred_destination(
    store: &EngagementStore,
    origin: &EngagementScope,
    member: u64,
) -> Result<WorkDestination, WorkError> {
    let policy = store
        .member_policies
        .get(&member)
        .ok_or(WorkError::Denied)?;
    let preference = policy
        .destinations
        .get(origin)
        .copied()
        .ok_or(WorkError::Denied)?;
    Ok(match (origin.clone(), preference) {
        (EngagementScope::Dm { member: owner, .. }, _) if owner == member => {
            WorkDestination::Personal { principal: member }
        }
        (EngagementScope::Guild { channel, .. }, DestinationPreference::Origin) => {
            WorkDestination::TeamChannel { channel }
        }
        (EngagementScope::Guild { .. }, DestinationPreference::Private) => {
            WorkDestination::TeamPrivate { principal: member }
        }
        _ => return Err(WorkError::Denied),
    })
}

fn validate_request(
    intent: &FollowUpIntent,
    source: &SourceRef,
    member: u64,
    now: u64,
) -> Result<u64, WorkError> {
    validate_scope(&intent.scope)?;
    task_key(&intent.scope, &intent.task)?;
    source.validate()?;
    utc(now)?;
    utc(intent.expires_at)?;
    if member == 0
        || source.author != member
        || work_scope(&source.scope) != intent.scope
        || source.at > now
    {
        return Err(WorkError::Denied);
    }
    intent.destination.validate(&intent.scope, member)?;
    let due = follow_up_due(source)?;
    if due >= intent.expires_at {
        return Err(WorkError::Invalid);
    }
    Ok(due)
}

pub(crate) fn preview(
    intent: &FollowUpIntent,
    source: &SourceRef,
    member: u64,
    due: u64,
) -> Candidate {
    Candidate {
        id: 0,
        kind: EngagementKind::FollowUp,
        source: Some(source.clone()),
        member: Some(member),
        scope: source.scope.clone(),
        due_at: due,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: String::new(),
        policy_revision: 0,
        destination: match intent.destination {
            WorkDestination::TeamPrivate { .. } => DestinationPreference::Private,
            _ => DestinationPreference::Origin,
        },
        message_id: None,
        introduction_id: None,
        work_ref: Some(intent.task.clone()),
        expires_at: Some(intent.expires_at),
        follow_up_reason: None,
    }
}

/// Pinned Engagement signature. This validates only authority owned by
/// Engagement; every production caller uses WorkStore::propose_task_follow_up
/// in its retained commit to prove the canonical native task as well.
pub fn propose_follow_up(
    store: &mut EngagementStore,
    intent: FollowUpIntent,
    source: SourceRef,
    member: u64,
    now: u64,
) -> Result<Option<u64>, WorkError> {
    let due = validate_request(&intent, &source, member, now)?;
    let seconds = intent
        .expires_at
        .checked_sub(now)
        .ok_or(WorkError::Invalid)?;
    // The native request checked its original 60-second minimum. Time spent
    // proving access consumes that window; it must never restart the minimum.
    if seconds == 0 || seconds > MAX_EXPIRY_SECONDS {
        return Err(WorkError::Invalid);
    }
    if store.task_follow_up_attempted(&intent.scope, &intent.task)
        || store.follow_up_source_attempted(&source, member)
    {
        return Ok(None);
    }
    if preferred_destination(store, &source.scope, member)? != intent.destination {
        return Err(WorkError::Denied);
    }
    let candidate = configured_preview(store, &intent, &source, member, due);
    if store.task_follow_up_member_decision(&candidate, now)? != FollowUpDecision::Allowed {
        return Err(WorkError::Denied);
    }
    store.propose_linked_task_follow_up(&intent, source, member, due, now)
}

fn configured_preview(
    store: &EngagementStore,
    intent: &FollowUpIntent,
    source: &SourceRef,
    member: u64,
    due: u64,
) -> Candidate {
    let mut candidate = preview(intent, source, member, due);
    if matches!(source.scope, EngagementScope::Dm { .. })
        && let Some(choice) = store
            .member_policies
            .get(&member)
            .and_then(|p| p.destinations.get(&source.scope))
    {
        // Both explicit choices resolve to the owner DM. Keep their exact saved
        // policy value so reservation still refuses a later preference change.
        candidate.destination = *choice;
    }
    candidate
}

impl WorkStore {
    /// Access was freshly proved by the transport. No projected/guessed task
    /// may substitute for the exact canonical native record.
    pub(crate) fn current_follow_up_task(
        &self,
        intent: &FollowUpIntent,
        source: &SourceRef,
        access: WorkAccess,
    ) -> Result<&WorkTask, WorkError> {
        if access.actor == 0
            || !access.can_view
            || access.actor != source.author
            || access.scope() != intent.scope
            || access.channel
                != match source.scope {
                    EngagementScope::Guild { channel, .. }
                    | EngagementScope::Dm { channel, .. } => channel,
                }
            || work_scope(&source.scope) != intent.scope
        {
            return Err(WorkError::Denied);
        }
        intent.destination.validate(&intent.scope, access.actor)?;
        let WorkContentRef::Task {
            project,
            id,
            revision,
        } = intent.task
        else {
            return Err(WorkError::Invalid);
        };
        let native_project = self.project(project, access)?;
        if native_project.id != project || native_project.scope != intent.scope {
            return Err(WorkError::Denied);
        }
        let task = self.tasks.get(&id).ok_or(WorkError::Stale)?;
        if task.id != id
            || task.project_id != project
            || task.revision != revision
            || matches!(task.status, WorkStatus::Done | WorkStatus::Cancelled)
        {
            return Err(WorkError::Stale);
        }
        Ok(task)
    }

    /// This is also the bounded Current Task renderer for existing retained
    /// Engagement delivery. It reveals no other project, assignee or source.
    pub(crate) fn render_follow_up_task(
        &self,
        intent: &FollowUpIntent,
        source: &SourceRef,
        access: WorkAccess,
    ) -> Result<String, WorkError> {
        let task = self.current_follow_up_task(intent, source, access)?;
        let rendered = format!(
            "Current Task (quoted data):\n{}",
            serde_json::json!({
                "id": task.id,
                "project": task.project_id,
                "revision": task.revision,
                "title": task.title,
                "status": task.status.label(),
            })
        );
        if rendered.len() > 4096 {
            return Err(WorkError::Invalid);
        }
        Ok(rendered)
    }

    /// Pure current canonical facts. Expected response comes from the exact
    /// freshly hydrated native exchange; response IDs are never client inputs.
    pub(crate) fn task_follow_up_facts(
        &self,
        intent: &FollowUpIntent,
        source: &SourceRef,
        access: WorkAccess,
        expected_response: u64,
        now: u64,
    ) -> Result<FollowUpFacts, WorkError> {
        let due = validate_request(intent, source, access.actor, now)?;
        let current_task = self.current_follow_up_task(intent, source, access);
        let authorized = !matches!(current_task, Err(WorkError::Denied | WorkError::Missing));
        let source_current = expected_response != 0
            && self.engagement.responses.get(&source.message) == Some(&expected_response)
            && self
                .engagement
                .eligibility
                .get(&access.actor)
                .is_some_and(|s| s.contains(source))
            && self
                .engagement
                .observations
                .get(&source.scope)
                .and_then(|s| s.get(&access.actor))
                == Some(source);
        let candidate = configured_preview(&self.engagement, intent, source, access.actor, due);
        let mut member_policy = self
            .engagement
            .task_follow_up_member_decision(&candidate, now)?;
        if member_policy == FollowUpDecision::Allowed
            && preferred_destination(&self.engagement, &source.scope, access.actor)
                .ok()
                .as_ref()
                != Some(&intent.destination)
        {
            member_policy = FollowUpDecision::OptedOut;
        }
        Ok(FollowUpFacts {
            authorized,
            source_current,
            completed: current_task.is_err(),
            expires_at: intent.expires_at,
            activity_ready: true,
            member_policy,
            already_attempted: self
                .engagement
                .task_follow_up_attempted(&intent.scope, &intent.task)
                || self
                    .engagement
                    .follow_up_source_attempted(source, access.actor),
        })
    }

    /// Must execute in the existing retained WorkStore commit after fresh native
    /// source/access proof. No unrelated Work mutation or task-source rewrite.
    pub(crate) fn propose_task_follow_up(
        &mut self,
        intent: FollowUpIntent,
        source: SourceRef,
        access: WorkAccess,
        expected_response: u64,
        now: u64,
    ) -> Result<Option<u64>, WorkError> {
        let facts = self.task_follow_up_facts(&intent, &source, access, expected_response, now)?;
        match evaluate_follow_up(&facts, now) {
            FollowUpDecision::Allowed => {
                propose_follow_up(&mut self.engagement, intent, source, access.actor, now)
            }
            FollowUpDecision::AlreadyAttempted => Ok(None),
            FollowUpDecision::StaleTask | FollowUpDecision::Expired => Err(WorkError::Stale),
            _ => Err(WorkError::Denied),
        }
    }
}

#[cfg(test)]
mod tests;
