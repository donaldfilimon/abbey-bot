//! Native human requests use the retained Engagement owner and Work publisher.
//! Only exact canonical source metadata survives; hydrated text is transient.
use super::{AppState, engagement_delivery::EngagementTransport};
use crate::{
    engagement::{Candidate, EngagementScope},
    work::{
        WorkContentRef, WorkError,
        follow_up::{self, FollowUpDecision, FollowUpIntent},
    },
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct TaskFollowUpRequest {
    pub member: u64,
    pub origin: EngagementScope,
    pub task: u64,
    pub revision: u64,
    pub source_message: u64,
    pub expiry_seconds: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TaskFollowUpResult {
    Saved {
        candidate: u64,
        due_at: u64,
        expires_at: u64,
    },
    Refused(FollowUpDecision),
}

async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = Result<T, WorkError>>,
) -> Result<T, WorkError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(WorkError::Denied),
        result = tokio::time::timeout(Duration::from_secs(30),future) => result.unwrap_or(Err(WorkError::Missing)),
    }
}

impl AppState {
    pub(crate) async fn request_task_follow_up<T: EngagementTransport + 'static>(
        self: &Arc<Self>,
        request: TaskFollowUpRequest,
        transport: Arc<T>,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Result<TaskFollowUpResult, WorkError> {
        if self.data_dir.is_none() {
            return Err(WorkError::Persistence);
        }
        let registry = self.service.get().ok_or(WorkError::Persistence)?;
        let cancel = registry.cancellation();
        let admitted_at = clock();
        let state = self.clone();
        let result = registry
            .spawn_result(
                crate::service::OperationKind::EngagementDelivery,
                async move {
                    state
                        .request_task_follow_up_owned(
                            request,
                            transport.as_ref(),
                            &cancel,
                            admitted_at,
                            &clock,
                        )
                        .await
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        result.await.map_err(|_| WorkError::Persistence)?
    }

    async fn request_task_follow_up_owned<T: EngagementTransport>(
        &self,
        request: TaskFollowUpRequest,
        transport: &T,
        cancel: &CancellationToken,
        admitted_at: u64,
        clock: &impl Fn() -> u64,
    ) -> Result<TaskFollowUpResult, WorkError> {
        let refused = |reason| Ok(TaskFollowUpResult::Refused(reason));
        if cancel.is_cancelled()
            || !self
                .engagement_events_healthy
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return refused(FollowUpDecision::AccessDenied);
        }
        let expires_at = follow_up::checked_expiry_seconds(admitted_at, request.expiry_seconds)?;
        if request.member == 0 || request.task == 0 || request.source_message == 0 {
            return Err(WorkError::Invalid);
        }
        let selected = {
            let stores = Self::lock(&self.stores);
            let work = &stores.work;
            work.engagement
                .eligibility
                .get(&request.member)
                .and_then(|rows| {
                    rows.iter().find(|source| {
                        source.scope == request.origin
                            && source.author == request.member
                            && source.message == request.source_message
                    })
                })
                .cloned()
                .and_then(|source| {
                    let task = work.tasks.get(&request.task)?;
                    let response = *work.engagement.responses.get(&source.message)?;
                    let intent = FollowUpIntent {
                        scope: follow_up::work_scope(&request.origin),
                        task: WorkContentRef::Task {
                            project: task.project_id,
                            id: task.id,
                            revision: request.revision,
                        },
                        destination: follow_up::preferred_destination(
                            &work.engagement,
                            &request.origin,
                            request.member,
                        )
                        .ok()?,
                        expires_at,
                    };
                    let audience = work.delivery_audience(&intent.scope);
                    let preference = work
                        .engagement
                        .member_policies
                        .get(&request.member)
                        .and_then(|p| p.destinations.get(&request.origin))
                        .copied();
                    Some((intent, source, response, audience, preference))
                })
        };
        let Some((intent, source, response, audience, preference)) = selected else {
            return refused(FollowUpDecision::AccessDenied);
        };
        let due_at = follow_up::follow_up_due(&source)?;
        if due_at >= expires_at {
            return Err(WorkError::Invalid);
        }
        let candidate = follow_up::preview(&intent, &source, request.member, due_at);
        // A real Work proof precedes source text reads. Pure preview facts cannot
        // substitute for the fetched origin/member/full-audience authority.
        let proof = bounded(
            cancel,
            transport.authorize_work(
                &intent.scope,
                request.member,
                origin(&request.origin),
                &intent.destination,
                &audience,
            ),
        )
        .await;
        let Ok((access, destination)) = proof else {
            return refused(FollowUpDecision::AccessDenied);
        };
        {
            let stores = Self::lock(&self.stores);
            let facts =
                stores
                    .work
                    .task_follow_up_facts(&intent, &source, access, response, clock())?;
            let reason = follow_up::evaluate_follow_up(&facts, clock());
            if reason != FollowUpDecision::Allowed {
                return refused(reason);
            }
        }
        let text = bounded(cancel, transport.hydrate_exchange(&candidate, response)).await;
        if text.as_ref().is_err()
            || text
                .as_ref()
                .is_ok_and(|t| t.trim().is_empty() || t.len() > 64_000)
        {
            return refused(FollowUpDecision::StaleTask);
        }
        drop(text);
        if !bounded(cancel, transport.source_exists(&source))
            .await
            .unwrap_or(false)
            || !bounded(
                cancel,
                transport.candidate_current(&candidate, Some(response)),
            )
            .await
            .unwrap_or(false)
        {
            return refused(FollowUpDecision::StaleTask);
        }
        let fresh = bounded(
            cancel,
            transport.authorize_work(
                &intent.scope,
                request.member,
                origin(&request.origin),
                &intent.destination,
                &audience,
            ),
        )
        .await;
        let Ok((fresh_access, fresh_destination)) = fresh else {
            return refused(FollowUpDecision::AccessDenied);
        };
        if destination == 0 || fresh_destination != destination {
            return refused(FollowUpDecision::AccessDenied);
        }
        self.commit_work_owned_at(
            move |work| {
                if cancel.is_cancelled() || work.delivery_audience(&intent.scope) != audience {
                    return Ok(TaskFollowUpResult::Refused(FollowUpDecision::AccessDenied));
                }
                if work
                    .engagement
                    .member_policies
                    .get(&request.member)
                    .and_then(|p| p.destinations.get(&request.origin))
                    .copied()
                    != preference
                {
                    return Ok(TaskFollowUpResult::Refused(FollowUpDecision::OptedOut));
                }
                let now = clock();
                let facts =
                    work.task_follow_up_facts(&intent, &source, fresh_access, response, now)?;
                let reason = follow_up::evaluate_follow_up(&facts, now);
                if reason != FollowUpDecision::Allowed {
                    return Ok(TaskFollowUpResult::Refused(reason));
                }
                match work.propose_task_follow_up(intent, source, fresh_access, response, now)? {
                    Some(candidate) => Ok(TaskFollowUpResult::Saved {
                        candidate,
                        due_at,
                        expires_at,
                    }),
                    None => Ok(TaskFollowUpResult::Refused(
                        FollowUpDecision::AlreadyAttempted,
                    )),
                }
            },
            admitted_at,
        )
        .await
        .map(|(result, _)| result)
    }

    /// Private current status never charges or writes. Exact-origin ownership
    /// precedes fresh Work proof; unknown access receives fixed content-free copy.
    pub(crate) async fn task_follow_up_status<T: EngagementTransport>(
        &self,
        member: u64,
        scope: &EngagementScope,
        transport: &T,
        clock: impl Fn() -> u64,
    ) -> String {
        let rows: Vec<Candidate> = Self::lock(&self.stores)
            .work
            .engagement
            .candidates
            .values()
            .rev()
            .filter(|c| c.member == Some(member) && &c.scope == scope && c.work_ref.is_some())
            .take(5)
            .cloned()
            .collect();
        if rows.is_empty() {
            return String::new();
        }
        let mut lines = vec!["Your task follow-ups in this origin:".to_string()];
        let mut receipts = crate::engagement::TaskReceiptAggregate::default();
        for c in rows {
            let Some(source) = c.source.clone() else {
                continue;
            };
            let Some(task) = c.work_ref.clone() else {
                continue;
            };
            let intent = FollowUpIntent {
                scope: follow_up::work_scope(scope),
                task,
                destination: match (&c.scope, c.destination) {
                    (EngagementScope::Dm { .. }, _) => {
                        crate::work::WorkDestination::Personal { principal: member }
                    }
                    (
                        EngagementScope::Guild { channel, .. },
                        crate::engagement::DestinationPreference::Origin,
                    ) => crate::work::WorkDestination::TeamChannel { channel: *channel },
                    _ => crate::work::WorkDestination::TeamPrivate { principal: member },
                },
                expires_at: c.expires_at.unwrap_or(0),
            };
            let audience = Self::lock(&self.stores)
                .work
                .delivery_audience(&intent.scope);
            let proof = tokio::time::timeout(
                Duration::from_secs(5),
                transport.authorize_work(
                    &intent.scope,
                    member,
                    origin(scope),
                    &intent.destination,
                    &audience,
                ),
            )
            .await;
            let Ok(Ok((access, _))) = proof else {
                lines.push(
                    "A task follow-up is unavailable: current access could not be confirmed."
                        .into(),
                );
                continue;
            };
            let now = clock();
            let reason = {
                let stores = Self::lock(&self.stores);
                let work = &stores.work;
                if work.engagement.candidates.get(&c.id) != Some(&c)
                    || work.delivery_audience(&intent.scope) != audience
                    || work
                        .current_follow_up_task(&intent, &source, access)
                        .is_err()
                {
                    lines.push("A task follow-up is unavailable: current access or task revision could not be confirmed.".into());
                    continue;
                }
                if !receipts.observe(
                    member,
                    scope,
                    &c,
                    work.engagement
                        .feedback
                        .get(&c.id)
                        .and_then(|rows| rows.get(&member)),
                ) {
                    lines.push("A task follow-up is unavailable: current access or task revision could not be confirmed.".into());
                    continue;
                }
                c.follow_up_reason.unwrap_or_else(|| {
                    if let Some(reason) =
                        super::engagement_delivery::task::invalidated(work, &c, now)
                    {
                        reason
                    } else {
                        work.engagement
                            .task_follow_up_member_decision(&c, now)
                            .unwrap_or(FollowUpDecision::AccessDenied)
                    }
                })
            };
            let reason = if reason == FollowUpDecision::Allowed {
                self.task_follow_up_contact_reason(scope, now)
            } else {
                reason
            };
            let label = if c.follow_up_reason.is_some() {
                "Recorded suppression"
            } else {
                "Current policy"
            };
            lines.push(format!(
                "Candidate {}: {:?}; expires <t:{}:R>. {label}: {}",
                c.id,
                c.state,
                intent.expires_at,
                reason.message()
            ));
        }
        let summary = receipts.render();
        if !summary.is_empty() {
            lines.push(summary);
        }
        lines.join("\n")
    }

    /// Read-only projection of the existing guild/channel guards and capacity.
    /// Lock order and budget-before-cooldown precedence match RateLimits.
    fn task_follow_up_contact_reason(&self, scope: &EngagementScope, now: u64) -> FollowUpDecision {
        if !self
            .engagement_events_healthy
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return FollowUpDecision::AccessDenied;
        }
        if self.quiet {
            return FollowUpDecision::Quiet;
        }
        let EngagementScope::Guild { guild, channel } = scope else {
            return FollowUpDecision::Allowed;
        };
        let guild = format!("discord:{guild}");
        let channel = format!("discord:{channel}");
        let settings = {
            let stores = Self::lock(&self.stores);
            Self::lock(&self.guilds).lookup(&guild, &*stores)
        };
        let Some(settings) = settings else {
            return FollowUpDecision::Disabled;
        };
        if !settings.enabled
            || !settings.unsolicited
            || !settings.unsolicited_channel_allowed(&channel)
        {
            return FollowUpDecision::Disabled;
        }
        let cooldown = Self::lock(&self.cooldown);
        let budget = Self::lock(&self.budget);
        if budget.tokens_left(&guild, settings.unsolicited_per_hour, now) < 1.0 {
            FollowUpDecision::Budget
        } else if !cooldown.permitted(&channel, settings.reply_cooldown_seconds, now) {
            FollowUpDecision::Cooldown
        } else {
            FollowUpDecision::Allowed
        }
    }
}
fn origin(scope: &EngagementScope) -> u64 {
    match scope {
        EngagementScope::Guild { channel, .. } | EngagementScope::Dm { channel, .. } => *channel,
    }
}

#[cfg(test)]
mod tests;
