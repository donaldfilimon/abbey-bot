//! Exact native task evidence for the existing retained Engagement owner.
//! Source and task text are transient; no Work reservation, tools or quota.
use super::*;
use crate::{
    calendar::utc,
    engagement::{EngagementKind, EngagementStore},
    work::{
        WorkAccess, WorkDestination, WorkStatus, WorkStore,
        follow_up::{FollowUpDecision, FollowUpIntent, preferred_destination, work_scope},
    },
};
use std::collections::BTreeSet;

pub(super) struct TaskEvidence {
    intent: FollowUpIntent,
    source: SourceRef,
    response: u64,
    audience: BTreeSet<u64>,
    access: Option<WorkAccess>,
    channel: Option<u64>,
    failure_reason: std::sync::Mutex<Option<FollowUpDecision>>,
}

fn origin(scope: &EngagementScope) -> u64 {
    match *scope {
        EngagementScope::Guild { channel, .. } | EngagementScope::Dm { channel, .. } => channel,
    }
}

fn intent(candidate: &Candidate) -> Result<FollowUpIntent, WorkError> {
    candidate.validate_task_follow_up()?;
    let member = candidate
        .member
        .filter(|id| *id != 0)
        .ok_or(WorkError::Invalid)?;
    let scope = work_scope(&candidate.scope);
    let destination = match (&scope, candidate.destination) {
        (crate::work::WorkScope::Personal { owner }, _) if *owner == member => {
            WorkDestination::Personal { principal: member }
        }
        (crate::work::WorkScope::Team { channel, .. }, DestinationPreference::Origin) => {
            WorkDestination::TeamChannel { channel: *channel }
        }
        (crate::work::WorkScope::Team { .. }, DestinationPreference::Private) => {
            WorkDestination::TeamPrivate { principal: member }
        }
        _ => return Err(WorkError::Invalid),
    };
    Ok(FollowUpIntent {
        scope,
        task: candidate.work_ref.clone().ok_or(WorkError::Invalid)?,
        destination,
        expires_at: candidate.expires_at.ok_or(WorkError::Invalid)?,
    })
}

/// Negative canonical checks require no invented positive native WorkAccess.
/// They retire stale work before prospective policy reserve can skip it forever.
pub(crate) fn invalidated(
    work: &WorkStore,
    candidate: &Candidate,
    at: u64,
) -> Option<FollowUpDecision> {
    let intent = match intent(candidate) {
        Ok(intent) => intent,
        Err(_) => return Some(FollowUpDecision::StaleTask),
    };
    if utc(at).is_err() || at >= intent.expires_at {
        return Some(FollowUpDecision::Expired);
    }
    let Some(source) = candidate.source.as_ref() else {
        return Some(FollowUpDecision::StaleTask);
    };
    let member = source.author;
    if work
        .engagement
        .observations
        .get(&source.scope)
        .and_then(|rows| rows.get(&member))
        != Some(source)
        || !work
            .engagement
            .eligibility
            .get(&member)
            .is_some_and(|rows| rows.contains(source))
        || !work
            .engagement
            .responses
            .get(&source.message)
            .is_some_and(|id| *id != 0)
    {
        return Some(FollowUpDecision::StaleTask);
    }
    let crate::work::WorkContentRef::Task {
        project,
        id,
        revision,
    } = intent.task
    else {
        return Some(FollowUpDecision::StaleTask);
    };
    let Some(project_record) = work.projects.get(&project) else {
        return Some(FollowUpDecision::StaleTask);
    };
    if project_record.id != project || project_record.scope != intent.scope {
        return Some(FollowUpDecision::StaleTask);
    }
    if !project_record.members.contains(&member) {
        return Some(FollowUpDecision::AccessDenied);
    }
    if !work.tasks.get(&id).is_some_and(|task| {
        task.id == id
            && task.project_id == project
            && task.revision == revision
            && !matches!(task.status, WorkStatus::Done | WorkStatus::Cancelled)
    }) {
        return Some(FollowUpDecision::StaleTask);
    }
    if preferred_destination(&work.engagement, &source.scope, member)
        .ok()
        .as_ref()
        != Some(&intent.destination)
    {
        return Some(FollowUpDecision::OptedOut);
    }
    match work
        .engagement
        .task_follow_up_member_decision(candidate, at)
    {
        Ok(reason @ (FollowUpDecision::Disabled | FollowUpDecision::OptedOut)) => Some(reason),
        // Quiet and budget are temporary policy suppression, not native staleness.
        Ok(_) => None,
        Err(_) => Some(FollowUpDecision::AccessDenied),
    }
}

pub(super) fn cancel_invalidated(work: &mut WorkStore, at: u64) -> Result<(), WorkError> {
    let rows: Vec<_> = work
        .engagement
        .candidates
        .values()
        .filter(|candidate| {
            candidate.work_ref.is_some()
                && matches!(
                    candidate.state,
                    CandidateState::Pending | CandidateState::Reserved
                )
        })
        .filter_map(|candidate| {
            invalidated(work, candidate, at)
                .map(|reason| (candidate.id, candidate.revision, reason))
        })
        .collect();
    for (id, revision, reason) in rows {
        work.engagement
            .cancel_task_follow_up(id, revision, reason)?;
    }
    Ok(())
}

impl TaskEvidence {
    pub(super) fn resolve(work: &WorkStore, candidate: &Candidate) -> Result<Self, WorkError> {
        if candidate.kind != EngagementKind::FollowUp || candidate.introduction_id.is_some() {
            return Err(WorkError::Invalid);
        }
        let intent = intent(candidate)?;
        let source = candidate.source.clone().ok_or(WorkError::Invalid)?;
        let response = work
            .engagement
            .responses
            .get(&source.message)
            .copied()
            .filter(|id| *id != 0)
            .ok_or(WorkError::Stale)?;
        let audience = work.delivery_audience(&intent.scope);
        Ok(Self {
            intent,
            source,
            response,
            audience,
            access: None,
            channel: None,
            failure_reason: std::sync::Mutex::new(None),
        })
    }

    fn note_failure(&self, reason: FollowUpDecision) {
        *AppState::lock(&self.failure_reason) = Some(reason);
    }

    fn canonical(
        &self,
        work: &WorkStore,
        candidate: &Candidate,
        access: WorkAccess,
        at: u64,
    ) -> Result<(), WorkError> {
        let current = work
            .engagement
            .candidates
            .get(&candidate.id)
            .ok_or(WorkError::Stale)?;
        if current.revision != candidate.revision
            || !matches!(
                current.state,
                CandidateState::Pending | CandidateState::Reserved
            )
            || current.kind != EngagementKind::FollowUp
            || current.member != Some(self.source.author)
            || current.scope != self.source.scope
            || current.source.as_ref() != Some(&self.source)
            || current.work_ref.as_ref() != Some(&self.intent.task)
            || current.expires_at != Some(self.intent.expires_at)
            || current.destination != candidate.destination
            || current.due_at != candidate.due_at
            || current.dedupe_key != candidate.dedupe_key
            || current.introduction_id.is_some()
            || work.engagement.responses.get(&self.source.message) != Some(&self.response)
            || work.delivery_audience(&self.intent.scope) != self.audience
            || invalidated(work, current, at).is_some()
        {
            self.note_failure(invalidated(work, current, at).unwrap_or_else(|| {
                if work.delivery_audience(&self.intent.scope) != self.audience {
                    FollowUpDecision::AccessDenied
                } else {
                    FollowUpDecision::StaleTask
                }
            }));
            return Err(WorkError::Stale);
        }
        work.current_follow_up_task(&self.intent, &self.source, access)
            .inspect_err(|error| {
                self.note_failure(if *error == WorkError::Denied {
                    FollowUpDecision::AccessDenied
                } else {
                    FollowUpDecision::StaleTask
                });
            })?;
        if current.state == CandidateState::Reserved {
            // The recorded policy revision belongs to the durable reservation;
            // recheck consent/quiet without charging its own attempt again.
            work.engagement
                .validate_reserved(
                    &EngagementReservation {
                        introduction: None,
                        candidate_id: current.id,
                        revision: current.revision,
                        policy_revision: current.policy_revision,
                        scope: current.scope.clone(),
                        member: current.member,
                        destination: current.destination,
                    },
                    at,
                )
                .inspect_err(|_| {
                    if let Ok(reason) = work.engagement.task_follow_up_member_decision(current, at)
                        && reason != FollowUpDecision::Allowed
                        && reason != FollowUpDecision::Budget
                    {
                        self.note_failure(reason);
                    }
                })?;
        }
        Ok(())
    }

    pub(super) fn current(&self, work: &WorkStore, candidate: &Candidate, at: u64) -> bool {
        self.access
            .is_some_and(|access| self.canonical(work, candidate, access, at).is_ok())
    }

    fn accepts(&self, access: WorkAccess, channel: u64) -> bool {
        access.actor == self.source.author
            && access.scope() == self.intent.scope
            && access.channel == origin(&self.source.scope)
            && access.can_view
            && channel != 0
            && self.channel.is_none_or(|expected| expected == channel)
            && match self.intent.destination {
                WorkDestination::Personal { .. } | WorkDestination::TeamChannel { .. } => {
                    channel == origin(&self.source.scope)
                }
                WorkDestination::TeamPrivate { .. } => channel != origin(&self.source.scope),
            }
    }

    async fn work_proof<T: EngagementTransport>(
        &self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
        now: &impl Fn() -> u64,
    ) -> Result<(WorkAccess, u64), FinalCheckFailure> {
        let (access, channel) = final_bounded(
            cancel,
            transport.authorize_work(
                &self.intent.scope,
                self.source.author,
                origin(&self.source.scope),
                &self.intent.destination,
                &self.audience,
            ),
        )
        .await
        .inspect_err(|failure| {
            if !matches!(failure, FinalCheckFailure::Cancelled) {
                self.note_failure(FollowUpDecision::AccessDenied);
            }
        })?;
        if !self.accepts(access, channel) {
            self.note_failure(FollowUpDecision::AccessDenied);
            return Err(FinalCheckFailure::Rejected);
        }
        self.canonical(
            &AppState::lock(&state.stores).work,
            candidate,
            access,
            now(),
        )
        .map_err(FinalCheckFailure::policy)?;
        Ok((access, channel))
    }

    async fn source_proof<T: EngagementTransport>(
        &self,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
    ) -> Result<(), FinalCheckFailure> {
        let source = final_bounded(cancel, transport.source_exists(&self.source)).await;
        self.source_result(source)?;
        let exchange = final_bounded(
            cancel,
            transport.candidate_current(candidate, Some(self.response)),
        )
        .await;
        self.source_result(exchange)
    }

    fn source_result(
        &self,
        result: Result<bool, FinalCheckFailure>,
    ) -> Result<(), FinalCheckFailure> {
        match result {
            Ok(true) => Ok(()),
            Ok(false) => {
                self.note_failure(FollowUpDecision::StaleTask);
                Err(FinalCheckFailure::Rejected)
            }
            Err(failure) => {
                if !matches!(failure, FinalCheckFailure::Cancelled) {
                    self.note_failure(match failure {
                        FinalCheckFailure::Failed(WorkError::Denied | WorkError::Stale) => {
                            FollowUpDecision::StaleTask
                        }
                        _ => FollowUpDecision::AccessDenied,
                    });
                }
                Err(failure)
            }
        }
    }

    pub(super) async fn preflight<T: EngagementTransport>(
        &mut self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
        now: &impl Fn() -> u64,
    ) -> Result<(), FinalCheckFailure> {
        self.source_proof(transport, candidate, cancel).await?;
        let (access, channel) = self
            .work_proof(state, transport, candidate, cancel, now)
            .await?;
        self.access = Some(access);
        self.channel = Some(channel);
        Ok(())
    }

    pub(super) fn matches_destination(&self, destination: &AuthorizedDestination) -> bool {
        self.channel == Some(destination.channel)
            && destination.member == Some(self.source.author)
            && destination.scope == self.source.scope
    }

    pub(super) async fn body<T: EngagementTransport>(
        &self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
        now: &impl Fn() -> u64,
    ) -> Result<String, WorkError> {
        let exchange =
            bounded(cancel, transport.hydrate_exchange(candidate, self.response)).await?;
        if exchange.trim().is_empty() || exchange.len() > 128_000 {
            return Err(WorkError::Invalid);
        }
        // The immutable writer wait may have outlived the pre-reserve native
        // proof. Re-prove before reading any private native task plaintext.
        let (access, _) = self
            .work_proof(state, transport, candidate, cancel, now)
            .await
            .map_err(|failure| match failure {
                FinalCheckFailure::Failed(error) => error,
                _ => WorkError::Denied,
            })?;
        let task = {
            let stores = AppState::lock(&state.stores);
            self.canonical(&stores.work, candidate, access, now())?;
            stores
                .work
                .render_follow_up_task(&self.intent, &self.source, access)?
        };
        let quoted = format!("{exchange}\n\n{task}");
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(WorkError::Denied),
            body = transport.generate(state, candidate, &quoted, now()) => body,
        }
    }

    pub(super) async fn prove<T: EngagementTransport>(
        &self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
        now: &impl Fn() -> u64,
    ) -> Result<(), FinalCheckFailure> {
        self.source_proof(transport, candidate, cancel).await?;
        self.work_proof(state, transport, candidate, cancel, now)
            .await
            .map(|_| ())
    }

    pub(super) fn reject_pending(
        &self,
        store: &mut EngagementStore,
        candidate: &Candidate,
    ) -> Result<(), WorkError> {
        let current = store
            .candidates
            .get_mut(&candidate.id)
            .ok_or(WorkError::Missing)?;
        if current.state == CandidateState::Pending && current.revision == candidate.revision {
            current.state = CandidateState::Rejected;
            current.follow_up_reason =
                (*AppState::lock(&self.failure_reason)).or(Some(FollowUpDecision::AccessDenied));
        }
        Ok(())
    }

    pub(super) fn annotate_failure(
        &self,
        work: &mut WorkStore,
        candidate: &Candidate,
        outcome: DeliveryOutcome,
        at: u64,
    ) {
        if !matches!(
            outcome,
            DeliveryOutcome::Rejected | DeliveryOutcome::Cancelled
        ) {
            return;
        }
        let reason = work
            .engagement
            .candidates
            .get(&candidate.id)
            .and_then(|current| invalidated(work, current, at));
        if let Some(current) = work.engagement.candidates.get_mut(&candidate.id)
            && current.work_ref.as_ref() == Some(&self.intent.task)
            && matches!(
                current.state,
                CandidateState::Reserved | CandidateState::Cancelled
            )
        {
            current.follow_up_reason = current
                .follow_up_reason
                .or(reason)
                .or(*AppState::lock(&self.failure_reason));
        }
    }
}
