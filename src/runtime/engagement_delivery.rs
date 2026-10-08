//! Retained engagement attempts: canonical reservation precedes every send.
use super::AppState;
use crate::{
    engagement::{
        Candidate, CandidateState, DestinationPreference, EngagementScope, SourceRef,
        lifecycle::{DeliveryOutcome, EngagementReservation},
    },
    work::WorkError,
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
mod plan;
mod preflight;
pub(super) mod task;
use plan::{DeliveryPlan, PlanAdmission};
use preflight::PreflightOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorizedDestination {
    pub channel: u64,
    pub member: Option<u64>,
    pub scope: EngagementScope,
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum SendFailure {
    Rejected,
    Uncertain,
}
pub(crate) trait EngagementTransport: Send + Sync {
    fn authorize_work(
        &self,
        _scope: &crate::work::WorkScope,
        _actor: u64,
        _origin: u64,
        _target: &crate::work::WorkDestination,
        _audience: &std::collections::BTreeSet<u64>,
    ) -> impl Future<Output = Result<(crate::work::WorkAccess, u64), WorkError>> + Send {
        async { Err(WorkError::Denied) }
    }
    fn authorize(
        &self,
        reservation: &EngagementReservation,
    ) -> impl Future<Output = Result<AuthorizedDestination, WorkError>> + Send;
    fn source_exists(
        &self,
        source: &SourceRef,
    ) -> impl Future<Output = Result<bool, WorkError>> + Send;
    /// Hydrate only this candidate's exact source; text remains transient.
    fn hydrate(
        &self,
        candidate: &Candidate,
    ) -> impl Future<Output = Result<String, WorkError>> + Send;
    fn hydrate_exchange(
        &self,
        candidate: &Candidate,
        _response: u64,
    ) -> impl Future<Output = Result<String, WorkError>> + Send {
        self.hydrate(candidate)
    }
    fn candidate_current(
        &self,
        candidate: &Candidate,
        _response: Option<u64>,
    ) -> impl Future<Output = Result<bool, WorkError>> + Send {
        async move {
            if let Some(source) = &candidate.source {
                self.source_exists(source).await
            } else {
                Ok(false)
            }
        }
    }
    /// Append only fully verified metadata as each item completes. The caller
    /// retains completed rows if its aggregate deadline drops the active proof.
    fn community_facts(
        &self,
        _state: &AppState,
        _now: u64,
        _completed: &mut crate::engagement::community::CommunityFacts,
    ) -> impl Future<Output = Result<(), WorkError>> + Send {
        async { Ok(()) }
    }
    fn hydrate_community(
        &self,
        _state: &AppState,
        candidate: &Candidate,
    ) -> impl Future<Output = Result<String, WorkError>> + Send {
        self.hydrate(candidate)
    }
    fn community_current(
        &self,
        _state: &AppState,
        candidate: &Candidate,
    ) -> impl Future<Output = Result<bool, WorkError>> + Send {
        self.candidate_current(candidate, None)
    }
    fn generate(
        &self,
        state: &AppState,
        candidate: &Candidate,
        source: &str,
        now: u64,
    ) -> impl Future<Output = Result<String, WorkError>> + Send;
    fn send(
        &self,
        channel: u64,
        body: &str,
    ) -> impl Future<Output = Result<u64, SendFailure>> + Send;
}
fn work_failure(error: WorkError) -> Option<crate::observability::OperationalErrorCategory> {
    use crate::observability::OperationalErrorCategory as Category;
    match error {
        // Adapters erase provider/guard details into Denied: no proven category.
        WorkError::Denied => None,
        WorkError::Missing => Some(Category::Unavailable),
        WorkError::Invalid => Some(Category::Protocol),
        WorkError::Full => Some(Category::Capacity),
        WorkError::Stale => Some(Category::Authorization),
        WorkError::Persistence => Some(Category::Persistence),
    }
}
#[derive(Clone, Copy)]
enum FinalCheckFailure {
    Rejected,
    Failed(WorkError),
    Timeout,
    Cancelled,
}
impl FinalCheckFailure {
    fn policy(error: WorkError) -> Self {
        if error == WorkError::Denied {
            Self::Rejected
        } else {
            Self::Failed(error)
        }
    }
    fn category(self) -> Option<crate::observability::OperationalErrorCategory> {
        use crate::observability::OperationalErrorCategory as Category;
        match self {
            Self::Rejected => Some(Category::Authorization),
            Self::Failed(error) => work_failure(error),
            Self::Timeout => Some(Category::Timeout),
            Self::Cancelled => None,
        }
    }
}
fn verified(current: bool) -> Result<(), FinalCheckFailure> {
    if current {
        Ok(())
    } else {
        Err(FinalCheckFailure::Rejected)
    }
}
async fn final_bounded<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = Result<T, WorkError>>,
) -> Result<T, FinalCheckFailure> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(FinalCheckFailure::Cancelled),
        result = tokio::time::timeout(Duration::from_secs(30), future) => match result {
            Ok(result) => result.map_err(FinalCheckFailure::Failed),
            Err(_) => Err(FinalCheckFailure::Timeout),
        }
    }
}
async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = Result<T, WorkError>>,
) -> Result<T, WorkError> {
    tokio::select! { biased; () = cancel.cancelled() => Err(WorkError::Denied), result = tokio::time::timeout(Duration::from_secs(30), future) => result.unwrap_or(Err(WorkError::Denied)) }
}
fn matches(r: &EngagementReservation, d: &AuthorizedDestination) -> bool {
    let origin = match r.scope {
        EngagementScope::Guild { channel, .. } | EngagementScope::Dm { channel, .. } => channel,
    };
    d.channel != 0
        && d.scope == r.scope
        && d.member == r.member
        && match (&r.scope, r.destination) {
            (EngagementScope::Guild { .. }, DestinationPreference::Private) => d.channel != origin,
            _ => d.channel == origin,
        }
}
impl AppState {
    pub(crate) fn voice_invitation_available(&self, scope: &EngagementScope) -> bool {
        if let EngagementScope::Guild { guild, .. } = scope {
            let settings = {
                let mut stores = Self::lock(&self.stores);
                Self::lock(&self.guilds).refresh(&format!("discord:{guild}"), &mut *stores)
            };
            if !settings.voice_enabled {
                return false;
            }
        }

        let voice = match scope {
            EngagementScope::Guild { guild, .. } => self.voice_registry.get(*guild),
            _ => None,
        };
        let config = voice
            .as_ref()
            .map(|v| v.config.template())
            .or_else(|| self.voice_registry.template());
        config.is_some_and(|config| {
            let mode = voice.as_ref().map_or(config.mode(), |v| v.effective_mode());
            mode == crate::voice::VoiceMode::Local
                && config.backend_for(mode).is_some()
                && self.providers.local_voice_route().is_some()
        })
    }
    pub(crate) fn engagement_guild_gate(
        &self,
        scope: &EngagementScope,
        at: u64,
        acquire: bool,
    ) -> bool {
        self.engagement_guild_check(scope, at, acquire).is_ok()
    }
    fn engagement_guild_check(
        &self,
        scope: &EngagementScope,
        at: u64,
        acquire: bool,
    ) -> Result<(), WorkError> {
        if !self
            .engagement_events_healthy
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(WorkError::Missing);
        }
        let EngagementScope::Guild { guild, channel } = scope else {
            return if self.quiet {
                Err(WorkError::Denied)
            } else {
                Ok(())
            };
        };
        let guild = format!("discord:{guild}");
        let channel = format!("discord:{channel}");
        let settings = {
            let mut stores = Self::lock(&self.stores);
            Self::lock(&self.guilds).config(&guild, &mut *stores)
        };
        if self.quiet
            || !settings.enabled
            || !settings.unsolicited
            || !settings.unsolicited_channel_allowed(&channel)
        {
            return Err(WorkError::Denied);
        }
        if acquire {
            crate::pipeline::RateLimits {
                cooldown: &self.cooldown,
                budget: &self.budget,
            }
            .try_acquire(&guild, &channel, &settings, at)
            .map_err(|_| WorkError::Denied)?;
        }
        Ok(())
    }
    pub(crate) async fn deliver_engagement<T: EngagementTransport>(
        self: Arc<Self>,
        transport: &T,
        cancel: CancellationToken,
        now: impl Fn() -> u64,
    ) -> Result<(), WorkError> {
        let Ok(_single) = self.engagement_delivery_running.try_lock() else {
            return Ok(());
        };
        self.commit_work_owned(|store| {
            task::cancel_invalidated(store, now())?;
            store.engagement.expire_missed_weekly(now())
        })
        .await?;
        self.clone().plan_weekly(transport, now()).await?;
        self.clone().plan_community(transport, now()).await?;
        let due = Self::lock(&self.stores).work.engagement.due(now());
        for id in due {
            if cancel.is_cancelled() {
                break;
            }
            // Clone a prospective store to preflight policy without charging or I/O.
            let preview = {
                let stores = Self::lock(&self.stores);
                let mut work = stores.work.clone();
                let candidate = work
                    .engagement
                    .candidates
                    .get(&id)
                    .cloned()
                    .ok_or(WorkError::Missing)?;
                work.engagement
                    .reserve(id, candidate.revision, now())
                    .and_then(|r| {
                        DeliveryPlan::resolve(&work, &candidate, &r)
                            .map(|plan| (candidate, r, plan))
                    })
            };
            let Ok((candidate, preview, mut plan)) = preview else {
                continue;
            };
            match plan.admission(&self, &candidate, now()) {
                PlanAdmission::Ready => {}
                PlanAdmission::Deferred => continue,
                PlanAdmission::Rejected => {
                    self.commit_work_owned(|s| plan.reject_pending(&mut s.engagement, &candidate))
                        .await?;
                    continue;
                }
            }
            if let Err(failure) =
                Box::pin(plan.work_preflight(&self, transport, &candidate, &cancel, &now)).await
            {
                match failure {
                    FinalCheckFailure::Rejected
                    | FinalCheckFailure::Failed(
                        WorkError::Denied | WorkError::Invalid | WorkError::Stale,
                    ) => {
                        self.commit_work_owned(|work| {
                            task::cancel_invalidated(work, now())?;
                            plan.reject_pending(&mut work.engagement, &candidate)
                        })
                        .await?;
                    }
                    // No canonical member charge or provider request exists.
                    // Missing proof, deadline and cancellation never grant access.
                    _ => {}
                }
                continue;
            }
            let destination = match preflight::authorize(transport, &preview, &cancel).await {
                PreflightOutcome::Authorized(destination) => destination,
                // No attempt exists: temporary missing proof, cancellation and
                // deadline leave the candidate and any mutual approvals intact.
                PreflightOutcome::Unavailable
                | PreflightOutcome::Cancelled
                | PreflightOutcome::Timeout => continue,
                PreflightOutcome::Rejected => {
                    self.commit_work_owned(|s| plan.reject_pending(&mut s.engagement, &candidate))
                        .await?;
                    continue;
                }
            };
            if !plan.matches_work_destination(&destination) {
                self.commit_work_owned(|work| {
                    plan.reject_pending(&mut work.engagement, &candidate)
                })
                .await?;
                continue;
            }
            if !self.engagement_guild_gate(&candidate.scope, now(), true) {
                continue;
            }
            let attempt_started = tokio::time::Instant::now();
            let reservation = match self
                .commit_work_owned(|s| {
                    if cancel.is_cancelled()
                        || plan.reduced(&self, &candidate, now())
                        || !plan.evidence_current(s, &candidate, now())
                    {
                        return Err(WorkError::Denied);
                    }
                    s.engagement.reserve(id, candidate.revision, now())
                })
                .await
            {
                Ok((r, _)) => r,
                Err(WorkError::Persistence) => return Err(WorkError::Persistence),
                Err(_) => continue,
            };
            crate::generation::timing::record(
                &self,
                crate::observability::EventCode::EngagementQueue,
                crate::observability::EventOutcome::Succeeded,
                Duration::from_secs(now().saturating_sub(candidate.due_at)),
                None,
            );
            self.engagement_metric(
                &candidate.scope,
                crate::observability::EventCode::EngagementQueue,
                Duration::from_secs(now().saturating_sub(candidate.due_at)),
            );
            let body = plan.body(&self, transport, &candidate, &cancel, &now).await;
            let mut error = body.as_ref().err().and_then(|error| work_failure(*error));
            let malformed_body = body
                .as_ref()
                .is_ok_and(|body| body.trim().is_empty() || body.chars().count() > 1900);
            let outcome = if let Ok(body) = body
                && !body.trim().is_empty()
                && body.chars().count() <= 1900
            {
                let proof = plan
                    .prove(&self, transport, &candidate, &cancel, &now)
                    .await;
                // Recipient access must be fresher than every external source proof.
                let fresh = final_bounded(&cancel, transport.authorize(&reservation))
                    .await
                    .and_then(|d| verified(d == destination && matches(&reservation, &d)));
                let checked = proof.and(fresh).and_then(|()| {
                    if cancel.is_cancelled() {
                        return Err(FinalCheckFailure::Cancelled);
                    }
                    plan.ready_after_proofs(&self, &candidate, now())?;
                    self.engagement_guild_check(&candidate.scope, now(), false)
                        .map_err(FinalCheckFailure::policy)?;
                    let stores = Self::lock(&self.stores);
                    // Exact task/source/audience and reservation share this last
                    // short canonical check after all external proofs.
                    verified(plan.evidence_current(&stores.work, &candidate, now()))?;
                    stores
                        .work
                        .engagement
                        .validate_reserved(&reservation, now())
                        .map_err(FinalCheckFailure::policy)
                });
                if checked.is_ok() {
                    let send = tokio::select! {
                        biased;
                        () = cancel.cancelled() => Err(SendFailure::Uncertain),
                        result = tokio::time::timeout(Duration::from_secs(30),transport.send(destination.channel,&body)) => result.unwrap_or(Err(SendFailure::Uncertain)),
                    };
                    match send {
                        Ok(message_id) if message_id != 0 => {
                            crate::generation::timing::record(
                                &self,
                                crate::observability::EventCode::DiscordFirstPost,
                                crate::observability::EventOutcome::Succeeded,
                                attempt_started.elapsed(),
                                None,
                            );
                            self.engagement_metric(
                                &candidate.scope,
                                crate::observability::EventCode::DiscordFirstPost,
                                attempt_started.elapsed(),
                            );
                            DeliveryOutcome::Sent { message_id }
                        }
                        Err(SendFailure::Rejected) => {
                            error =
                                Some(crate::observability::OperationalErrorCategory::Unavailable);
                            DeliveryOutcome::Rejected
                        }
                        _ => DeliveryOutcome::ReviewRequired,
                    }
                } else if cancel.is_cancelled() {
                    DeliveryOutcome::Cancelled
                } else {
                    error = checked.err().and_then(FinalCheckFailure::category);
                    DeliveryOutcome::Rejected
                }
            } else if cancel.is_cancelled() {
                DeliveryOutcome::Cancelled
            } else {
                if malformed_body {
                    error = Some(crate::observability::OperationalErrorCategory::Protocol);
                }
                DeliveryOutcome::Rejected
            };
            let succeeded = matches!(outcome, DeliveryOutcome::Sent { .. });
            if matches!(
                outcome,
                DeliveryOutcome::Cancelled | DeliveryOutcome::ReviewRequired
            ) {
                error = None;
            }
            let settled = self
                .commit_work_owned(|s| {
                    // Erasure won canonical publication while generation was
                    // held. Its two replay commitments are the only authority
                    // for this no-op; never recreate a deleted receipt.
                    if !s.engagement.candidates.contains_key(&id)
                        && matches!(
                            outcome,
                            DeliveryOutcome::Rejected | DeliveryOutcome::Cancelled
                        )
                        && s.engagement.task_follow_up_erased(&candidate)
                    {
                        return Ok(());
                    }
                    let state = s
                        .engagement
                        .candidates
                        .get(&id)
                        .ok_or(WorkError::Missing)?
                        .state;
                    if state == CandidateState::Cancelled
                        && matches!(
                            outcome,
                            DeliveryOutcome::Rejected | DeliveryOutcome::Cancelled
                        )
                    {
                        return Ok(());
                    }
                    plan.annotate_task_failure(s, &candidate, outcome, now());
                    s.engagement.settle(id, outcome)
                })
                .await;
            if let Err(error) = settled {
                self.engagement_metric(
                    &candidate.scope,
                    crate::observability::EventCode::EngagementFailure,
                    attempt_started.elapsed(),
                );
                crate::generation::timing::record(
                    &self,
                    crate::observability::EventCode::EngagementFailure,
                    crate::observability::EventOutcome::Failed,
                    attempt_started.elapsed(),
                    Some(crate::observability::OperationalErrorCategory::Persistence),
                );
                return Err(error);
            }
            self.engagement_metric(
                &candidate.scope,
                if succeeded {
                    crate::observability::EventCode::EngagementCompleted
                } else {
                    crate::observability::EventCode::EngagementFailure
                },
                attempt_started.elapsed(),
            );
            crate::generation::timing::record(
                &self,
                if succeeded {
                    crate::observability::EventCode::EngagementCompleted
                } else {
                    crate::observability::EventCode::EngagementFailure
                },
                if succeeded {
                    crate::observability::EventOutcome::Succeeded
                } else if matches!(outcome, DeliveryOutcome::Cancelled) {
                    crate::observability::EventOutcome::Cancelled
                } else {
                    crate::observability::EventOutcome::Failed
                },
                attempt_started.elapsed(),
                error,
            );
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
