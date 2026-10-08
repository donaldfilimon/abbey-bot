//! Resolve metadata once; each plan owns its transient body and evidence proofs.
use super::*;
use crate::engagement::{
    EngagementKind, EngagementStore, InvitationRequest,
    community::{CommunityEvidence, CommunityReceipt},
    lifecycle::IntroductionReservation,
};

pub(super) enum DeliveryPlan {
    Task(task::TaskEvidence),
    Introduction(IntroductionReservation),
    Source {
        kind: ConversationKind,
        evidence: SourceEvidence,
    },
    Community(CommunityPlan),
    Invitation {
        kind: InvitationKind,
        evidence: InvitationEvidence,
    },
}
pub(super) enum ConversationKind {
    FollowUp,
    Weekly,
}
pub(super) enum InvitationKind {
    Activity,
    Voice,
}
pub(super) enum SourceEvidence {
    Message(SourceRef),
    Exchange { source: SourceRef, response: u64 },
}
pub(super) enum InvitationEvidence {
    Source(SourceEvidence),
    Request(InvitationRequest),
}
pub(super) enum CommunityPlan {
    Message {
        source: SourceRef,
        receipt: CommunityReceipt,
    },
    Welcome(CommunityReceipt),
    Project(CommunityReceipt),
}
pub(super) enum PlanAdmission {
    Ready,
    Deferred,
    Rejected,
}

impl SourceEvidence {
    fn resolve(store: &EngagementStore, candidate: &Candidate) -> Result<Self, WorkError> {
        let source = candidate.source.clone().ok_or(WorkError::Invalid)?;
        Ok(match store.responses.get(&source.message).copied() {
            Some(response) => Self::Exchange { source, response },
            None => Self::Message(source),
        })
    }
    fn current(&self, store: &EngagementStore, candidate: &Candidate) -> bool {
        let (source, response) = match self {
            Self::Message(source) => (source, None),
            Self::Exchange { source, response } => (source, Some(*response)),
        };
        store.candidates.get(&candidate.id).is_some_and(|current| {
            current.source.as_ref() == Some(source)
                && store.responses.get(&source.message).copied() == response
        })
    }
    async fn hydrate<T: EngagementTransport>(
        &self,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
    ) -> Result<String, WorkError> {
        match self {
            Self::Message(_) => bounded(cancel, transport.hydrate(candidate)).await,
            Self::Exchange { response, .. } => {
                bounded(cancel, transport.hydrate_exchange(candidate, *response)).await
            }
        }
    }
    async fn prove<T: EngagementTransport>(
        &self,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
    ) -> Result<(), FinalCheckFailure> {
        let (source, response) = match self {
            Self::Message(source) => (source, None),
            Self::Exchange { source, response } => (source, Some(*response)),
        };
        let source_check = final_bounded(cancel, transport.source_exists(source))
            .await
            .and_then(verified);
        let exchange_check =
            final_bounded(cancel, transport.candidate_current(candidate, response))
                .await
                .and_then(verified);
        source_check.and(exchange_check)
    }
}
impl CommunityPlan {
    fn resolve(store: &EngagementStore, candidate: &Candidate) -> Result<Self, WorkError> {
        let receipt = store
            .community_receipts
            .get(&candidate.id)
            .cloned()
            .ok_or(WorkError::Invalid)?;
        match (&receipt.evidence, candidate.kind, &candidate.source) {
            (
                CommunityEvidence::Message,
                EngagementKind::ConversationStarter | EngagementKind::UnansweredQuestion,
                Some(source),
            ) => Ok(Self::Message {
                source: source.clone(),
                receipt,
            }),
            (CommunityEvidence::Join { .. }, EngagementKind::Welcome, None) => {
                Ok(Self::Welcome(receipt))
            }
            (CommunityEvidence::Project { .. }, EngagementKind::ProjectCheckIn, None) => {
                Ok(Self::Project(receipt))
            }
            _ => Err(WorkError::Invalid),
        }
    }
    fn receipt(&self) -> &CommunityReceipt {
        match self {
            Self::Message { receipt, .. } | Self::Welcome(receipt) | Self::Project(receipt) => {
                receipt
            }
        }
    }
    fn current(&self, store: &EngagementStore, candidate: &Candidate) -> bool {
        store.community_receipt_current(candidate)
            && store
                .community_receipts
                .get(&candidate.id)
                .is_some_and(|current| {
                    current.policy_revision == self.receipt().policy_revision
                        && current.evidence == self.receipt().evidence
                })
    }
    async fn prove<T: EngagementTransport>(
        &self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
    ) -> Result<(), FinalCheckFailure> {
        let receipt_check =
            verified(self.current(&AppState::lock(&state.stores).work.engagement, candidate));
        let source_check = match self {
            Self::Message { source, .. } => final_bounded(cancel, transport.source_exists(source))
                .await
                .and_then(verified),
            Self::Welcome(_) | Self::Project(_) => Ok(()),
        };
        let current = final_bounded(cancel, transport.community_current(state, candidate))
            .await
            .and_then(verified);
        receipt_check.and(source_check).and(current)
    }
}
impl InvitationEvidence {
    fn current(&self, store: &EngagementStore, candidate: &Candidate) -> bool {
        match self {
            Self::Source(source) => source.current(store, candidate),
            Self::Request(request) => {
                store.invitation_receipt_current(candidate)
                    && store.invitation_requests.get(&candidate.id) == Some(request)
            }
        }
    }
    async fn hydrate<T: EngagementTransport>(
        &self,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
    ) -> Result<String, WorkError> {
        match self {
            Self::Source(source) => source.hydrate(transport, candidate, cancel).await,
            Self::Request(_) => bounded(cancel, transport.hydrate(candidate)).await,
        }
    }
    async fn prove<T: EngagementTransport>(
        &self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
    ) -> Result<(), FinalCheckFailure> {
        match self {
            Self::Source(source) => source.prove(transport, candidate, cancel).await,
            Self::Request(_) => {
                let receipt_check = {
                    let stores = AppState::lock(&state.stores);
                    verified(self.current(&stores.work.engagement, candidate))
                };
                let current = final_bounded(cancel, transport.candidate_current(candidate, None))
                    .await
                    .and_then(verified);
                receipt_check.and(current)
            }
        }
    }
}
impl DeliveryPlan {
    pub(super) fn evidence_current(
        &self,
        work: &crate::work::WorkStore,
        candidate: &Candidate,
        at: u64,
    ) -> bool {
        let store = &work.engagement;
        match self {
            Self::Task(task) => task.current(work, candidate, at),
            // Introduction content and both policy revisions are revalidated
            // by the canonical introduction reservation owner.
            Self::Introduction(_) => true,
            Self::Source { evidence, .. } => evidence.current(store, candidate),
            Self::Community(plan) => plan.current(store, candidate),
            Self::Invitation { evidence, .. } => evidence.current(store, candidate),
        }
    }
    pub(super) fn resolve(
        work: &crate::work::WorkStore,
        candidate: &Candidate,
        reservation: &EngagementReservation,
    ) -> Result<Self, WorkError> {
        let store = &work.engagement;
        candidate.validate_task_follow_up()?;
        if candidate.work_ref.is_some() {
            if reservation.introduction.is_some() {
                return Err(WorkError::Invalid);
            }
            return task::TaskEvidence::resolve(work, candidate).map(Self::Task);
        }
        if candidate.kind == EngagementKind::Introduction {
            let snapshot = reservation.introduction.clone().ok_or(WorkError::Invalid)?;
            if candidate.introduction_id != Some(snapshot.introduction.id)
                || candidate.source.is_some()
                || candidate.member.is_some()
            {
                return Err(WorkError::Invalid);
            }
            return Ok(Self::Introduction(snapshot));
        }
        if reservation.introduction.is_some() || candidate.introduction_id.is_some() {
            return Err(WorkError::Invalid);
        }
        match candidate.kind {
            EngagementKind::FollowUp | EngagementKind::WeeklyCheckIn => Ok(Self::Source {
                kind: match candidate.kind {
                    EngagementKind::FollowUp => ConversationKind::FollowUp,
                    _ => ConversationKind::Weekly,
                },
                evidence: SourceEvidence::resolve(store, candidate)?,
            }),
            EngagementKind::ConversationStarter
            | EngagementKind::UnansweredQuestion
            | EngagementKind::Welcome
            | EngagementKind::ProjectCheckIn => {
                CommunityPlan::resolve(store, candidate).map(Self::Community)
            }
            EngagementKind::ActivityInvite | EngagementKind::VoiceInvite => {
                let evidence = if candidate.source.is_some() {
                    InvitationEvidence::Source(SourceEvidence::resolve(store, candidate)?)
                } else {
                    InvitationEvidence::Request(
                        store
                            .invitation_requests
                            .get(&candidate.id)
                            .cloned()
                            .ok_or(WorkError::Invalid)?,
                    )
                };
                Ok(Self::Invitation {
                    kind: match candidate.kind {
                        EngagementKind::ActivityInvite => InvitationKind::Activity,
                        _ => InvitationKind::Voice,
                    },
                    evidence,
                })
            }
            EngagementKind::Introduction => Err(WorkError::Invalid),
        }
    }
    pub(super) fn reduced(&self, state: &AppState, candidate: &Candidate, now: u64) -> bool {
        matches!(
            self,
            Self::Task(_)
                | Self::Source {
                    kind: ConversationKind::FollowUp,
                    ..
                }
        ) && state.follow_up_reduced(&candidate.scope, now)
    }
    pub(super) fn admission(
        &self,
        state: &AppState,
        candidate: &Candidate,
        now: u64,
    ) -> PlanAdmission {
        if self.reduced(state, candidate, now) {
            PlanAdmission::Deferred
        } else if matches!(
            self,
            Self::Invitation {
                kind: InvitationKind::Voice,
                ..
            }
        ) && !state.voice_invitation_available(&candidate.scope)
        {
            PlanAdmission::Rejected
        } else {
            PlanAdmission::Ready
        }
    }
    pub(super) fn reject_pending(
        &self,
        store: &mut EngagementStore,
        candidate: &Candidate,
    ) -> Result<(), WorkError> {
        if let Self::Task(task) = self {
            return task.reject_pending(store, candidate);
        }
        let current = store
            .candidates
            .get_mut(&candidate.id)
            .ok_or(WorkError::Missing)?;
        if current.state == CandidateState::Pending && current.revision == candidate.revision {
            current.state = CandidateState::Rejected;
            if let Self::Introduction(snapshot) = self {
                let introduction = store
                    .introductions
                    .get_mut(&snapshot.introduction.id)
                    .ok_or(WorkError::Missing)?;
                introduction.state = crate::engagement::IntroductionState::Cancelled;
                introduction.approvals = [None; 2];
            }
        }
        Ok(())
    }
    pub(super) async fn body<T: EngagementTransport>(
        &self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
        now: &impl Fn() -> u64,
    ) -> Result<String, WorkError> {
        let hydrated = match self {
            Self::Task(task) => {
                return Box::pin(task.body(state, transport, candidate, cancel, now)).await;
            }
            Self::Introduction(snapshot) => {
                return crate::engagement::introductions::publication(&snapshot.introduction);
            }
            Self::Source { evidence, .. } => evidence.hydrate(transport, candidate, cancel).await,
            Self::Community(_) => {
                bounded(cancel, transport.hydrate_community(state, candidate)).await
            }
            Self::Invitation { evidence, .. } => {
                evidence.hydrate(transport, candidate, cancel).await
            }
        };
        // Source failures never grant permission to generate from absent context.
        let text = hydrated.map_err(|_| WorkError::Denied)?;
        if text.trim().is_empty() {
            return Err(WorkError::Denied);
        }
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(WorkError::Denied),
            body = transport.generate(state, candidate, &text, now()) => body,
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
        match self {
            Self::Task(task) => {
                Box::pin(task.prove(state, transport, candidate, cancel, now)).await
            }
            Self::Introduction(snapshot) => {
                let stores = AppState::lock(&state.stores);
                let store = &stores.work.engagement;
                verified(store.candidates.get(&candidate.id).is_some_and(|current| {
                    store.introduction_receipt_current(current)
                        && store.introductions.get(&snapshot.introduction.id)
                            == Some(&snapshot.introduction)
                }))
            }
            Self::Source { evidence, .. } => evidence.prove(transport, candidate, cancel).await,
            Self::Community(plan) => plan.prove(state, transport, candidate, cancel).await,
            Self::Invitation { kind, evidence } => {
                let evidence_check = evidence.prove(state, transport, candidate, cancel).await;
                let activity_check = match kind {
                    InvitationKind::Activity => final_bounded(
                        cancel,
                        super::super::activity_readiness::candidate_activity(
                            state,
                            candidate,
                            now(),
                        ),
                    )
                    .await
                    .map(|_| ()),
                    InvitationKind::Voice => Ok(()),
                };
                evidence_check.and(activity_check)
            }
        }
    }
    pub(super) async fn work_preflight<T: EngagementTransport>(
        &mut self,
        state: &AppState,
        transport: &T,
        candidate: &Candidate,
        cancel: &CancellationToken,
        now: &impl Fn() -> u64,
    ) -> Result<(), FinalCheckFailure> {
        match self {
            Self::Task(task) => {
                Box::pin(task.preflight(state, transport, candidate, cancel, now)).await
            }
            _ => Ok(()),
        }
    }
    pub(super) fn matches_work_destination(&self, destination: &AuthorizedDestination) -> bool {
        match self {
            Self::Task(task) => task.matches_destination(destination),
            _ => true,
        }
    }
    pub(super) fn annotate_task_failure(
        &self,
        work: &mut crate::work::WorkStore,
        candidate: &Candidate,
        outcome: DeliveryOutcome,
        at: u64,
    ) {
        if let Self::Task(task) = self {
            task.annotate_failure(work, candidate, outcome, at);
        }
    }
    pub(super) fn ready_after_proofs(
        &self,
        state: &AppState,
        candidate: &Candidate,
        now: u64,
    ) -> Result<(), FinalCheckFailure> {
        verified(self.evidence_current(&AppState::lock(&state.stores).work, candidate, now))?;
        if matches!(
            self,
            Self::Invitation {
                kind: InvitationKind::Voice,
                ..
            }
        ) && !state.voice_invitation_available(&candidate.scope)
        {
            return Err(FinalCheckFailure::Failed(WorkError::Denied));
        }
        verified(!self.reduced(state, candidate, now))
    }
}
