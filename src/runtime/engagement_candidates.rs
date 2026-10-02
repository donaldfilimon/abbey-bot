//! Owned source metadata and read-only assessment; no transcript copies or tools.
use super::{AppState, engagement_delivery::EngagementTransport};
use crate::{
    engagement::{
        Candidate, CandidateState, DestinationPreference, EngagementKind, EngagementScope,
        SourceRef,
        classifier::{ConversationOutcome, PROMPT, parse_assessment},
        lifecycle::EngagementReservation,
        schedule::CandidateProposal,
    },
    work::WorkError,
};
use std::sync::Arc;

pub(crate) fn event_source(event: &crate::platform::SocialEvent) -> Result<SourceRef, WorkError> {
    if event.is_bot || event.network != crate::platform::SocialNetwork::Discord {
        return Err(WorkError::Denied);
    }
    let member = event
        .native_user_id
        .parse()
        .map_err(|_| WorkError::Invalid)?;
    let channel = event
        .native_channel_id
        .parse()
        .map_err(|_| WorkError::Invalid)?;
    let scope = match &event.native_guild_id {
        Some(guild) => EngagementScope::Guild {
            guild: guild.parse().map_err(|_| WorkError::Invalid)?,
            channel,
        },
        None => EngagementScope::Dm { member, channel },
    };
    Ok(SourceRef {
        scope,
        message: event
            .native_message_id
            .parse()
            .map_err(|_| WorkError::Invalid)?,
        author: member,
        revision: 1,
        at: event.timestamp,
    })
}
fn current(store: &crate::engagement::EngagementStore, source: &SourceRef) -> bool {
    store
        .observations
        .get(&source.scope)
        .and_then(|rows| rows.get(&source.author))
        == Some(source)
}
fn enabled(
    store: &crate::engagement::EngagementStore,
    member: u64,
    scope: &EngagementScope,
) -> bool {
    store.member_policies.get(&member).is_some_and(|p| p.personalized_enabled() && !p.stopped_scopes.contains(scope) && !matches!(scope, EngagementScope::Guild {guild,..} if p.stopped_guilds.contains(guild)))
}
fn preview(source: &SourceRef, kind: EngagementKind, due: u64) -> Candidate {
    Candidate {
        id: 0,
        kind,
        source: Some(source.clone()),
        member: Some(source.author),
        scope: source.scope.clone(),
        due_at: due,
        revision: 1,
        state: CandidateState::Pending,
        dedupe_key: String::new(),
        policy_revision: 0,
        destination: DestinationPreference::Origin,
        message_id: None,
        introduction_id: None,
    }
}
impl AppState {
    /// The retained owner owns both publication and the sticky failure response.
    /// A dropped gateway result receiver cannot discard this safety transition.
    pub(super) async fn commit_engagement_invalidation<R: Send + 'static>(
        &self,
        change: impl FnOnce(&mut crate::engagement::EngagementStore) -> Result<R, WorkError>
        + Send
        + 'static,
    ) -> Result<R, WorkError> {
        let Some(state) = self.owned_state() else {
            self.engagement_events_healthy
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Err(WorkError::Persistence);
        };
        let Some(registry) = self.service.get() else {
            self.engagement_events_healthy
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Err(WorkError::Persistence);
        };
        let receiver = registry.spawn_result(
            crate::service::OperationKind::EngagementDelivery,
            async move {
                let result = state
                    .commit_work_owned(move |work| {
                        let value = change(&mut work.engagement)?;
                        work.engagement.validate()?;
                        Ok(value)
                    })
                    .await
                    .map(|(value, _)| value);
                if result.is_err() {
                    state
                        .engagement_events_healthy
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                }
                result
            },
        );
        let result = match receiver {
            Ok(receiver) => receiver.await.unwrap_or(Err(WorkError::Persistence)),
            Err(_) => Err(WorkError::Persistence),
        };
        if result.is_err() {
            self.engagement_events_healthy
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        result
    }
    pub(crate) async fn observe_engagement(&self, source: SourceRef) -> Result<bool, WorkError> {
        self.commit_engagement_invalidation(move |store| {
            source.validate()?;
            if store
                .observations
                .get(&source.scope)
                .and_then(|r| r.get(&source.author))
                .is_some_and(|old| old.message >= source.message)
            {
                return Ok(false);
            }
            store.cancel_public_origin(&source.scope, source.at.saturating_add(1));
            store.cancel_member_origin(source.author, &source.scope, source.at.saturating_add(1));
            let exists = store
                .observations
                .get(&source.scope)
                .is_some_and(|rows| rows.contains_key(&source.author));
            if !exists
                && store
                    .observations
                    .values()
                    .map(std::collections::BTreeMap::len)
                    .sum::<usize>()
                    >= 10_000
            {
                return Ok(false);
            }
            store
                .observations
                .entry(source.scope.clone())
                .or_default()
                .insert(source.author, source);
            Ok(true)
        })
        .await
    }
    pub(crate) async fn delete_engagement_source(
        &self,
        channel: u64,
        message: u64,
    ) -> Result<(), WorkError> {
        self.commit_engagement_invalidation(move |store| {
            let affected: std::collections::BTreeSet<_> = store.responses.iter().filter(|(source,response)| **source == message || **response == message).map(|(source,_)| *source).chain(std::iter::once(message)).collect();
            store.responses.retain(|source,response| !affected.contains(source) && *response != message);
            for (scope, rows) in &mut store.observations {
                let origin = match scope { EngagementScope::Guild {channel,..} | EngagementScope::Dm {channel,..} => *channel };
                if origin == channel { for source in rows.values_mut().filter(|s| affected.contains(&s.message)) { source.revision = source.revision.checked_add(1).ok_or(WorkError::Full)?; } }
            }
            for sources in store.eligibility.values_mut() { sources.retain(|s| { let origin = match s.scope { EngagementScope::Guild {channel,..} | EngagementScope::Dm {channel,..} => channel }; origin != channel || !affected.contains(&s.message) }); }
            let scopes: Vec<_> = store.candidates.values().filter_map(|c| c.source.as_ref()).filter(|s| affected.contains(&s.message) && matches!(s.scope, EngagementScope::Guild { channel: ch,.. } | EngagementScope::Dm {channel: ch,..} if ch == channel)).map(|s| s.scope.clone()).collect();
            for scope in scopes { for source in &affected { store.cancel_source(&scope, *source); } }
            Ok(())
        }).await
    }
    pub(crate) async fn complete_engagement<T: EngagementTransport + 'static>(
        self: Arc<Self>,
        source: SourceRef,
        response: u64,
        transport: Arc<T>,
    ) -> Result<(), WorkError> {
        let state = self.clone();
        let result = self
            .service
            .get()
            .ok_or(WorkError::Persistence)?
            .spawn_result(
                crate::service::OperationKind::EngagementDelivery,
                async move {
                    state
                        .complete_engagement_owned(source, response, transport.as_ref())
                        .await
                },
            )
            .map_err(|_| WorkError::Persistence)?;
        result.await.map_err(|_| WorkError::Persistence)?
    }
    async fn complete_engagement_owned<T: EngagementTransport>(
        self: Arc<Self>,
        source: SourceRef,
        response: u64,
        transport: &T,
    ) -> Result<(), WorkError> {
        let eligible = source.clone();
        self.commit_work_owned(move |work| {
            let store = &mut work.engagement;
            if !current(store, &eligible) {
                return Err(WorkError::Stale);
            }
            if store.responses.contains_key(&eligible.message) {
                return Err(WorkError::Stale);
            }
            store.responses.insert(eligible.message, response);
            let sources = store.eligibility.entry(eligible.author).or_default();
            sources.insert(eligible.clone());
            let mut same: Vec<_> = sources
                .iter()
                .filter(|s| s.scope == eligible.scope)
                .cloned()
                .collect();
            same.sort_by_key(|s| (s.at, s.message));
            for old in same.iter().take(same.len().saturating_sub(4)) {
                sources.remove(old);
                store.responses.remove(&old.message);
            }
            store.validate()
        })
        .await?;
        let sources = Self::lock(&self.stores)
            .work
            .engagement
            .eligibility
            .get(&source.author)
            .into_iter()
            .flatten()
            .filter(|s| s.scope == source.scope)
            .cloned()
            .collect();
        self.assess_engagement(source.scope, source.author, sources, transport, None)
            .await
    }
    pub(crate) async fn assess_engagement<T: EngagementTransport>(
        self: Arc<Self>,
        scope: EngagementScope,
        member: u64,
        mut sources: Vec<SourceRef>,
        transport: &T,
        weekly: Option<u64>,
    ) -> Result<(), WorkError> {
        sources.sort_by_key(|s| (s.at, s.message));
        if sources.len() > 4
            || sources.is_empty()
            || sources
                .iter()
                .any(|s| s.scope != scope || s.author != member)
        {
            return Err(WorkError::Invalid);
        }
        let latest = sources.last().cloned().ok_or(WorkError::Invalid)?;
        let policy_revision = {
            let stores = Self::lock(&self.stores);
            let store = &stores.work.engagement;
            if !enabled(store, member, &scope)
                || !current(store, &latest)
                || sources.iter().any(|s| {
                    store
                        .eligibility
                        .get(&member)
                        .is_none_or(|rows| !rows.contains(s))
                })
            {
                return Err(WorkError::Denied);
            }
            store.member_policies[&member].revision
        };
        let kind = if weekly.is_some() {
            EngagementKind::WeeklyCheckIn
        } else {
            EngagementKind::FollowUp
        };
        if kind == EngagementKind::FollowUp && self.follow_up_reduced(&scope, super::now()) {
            return Err(WorkError::Denied);
        }
        let mut evidence = Vec::new();
        for source in &sources {
            let candidate = preview(source, kind, weekly.unwrap_or(source.at));
            let r = EngagementReservation {
                introduction: None,
                candidate_id: 0,
                revision: 1,
                policy_revision: 0,
                scope: scope.clone(),
                member: Some(member),
                destination: DestinationPreference::Origin,
            };
            tokio::time::timeout(std::time::Duration::from_secs(30), transport.authorize(&r))
                .await
                .map_err(|_| WorkError::Denied)??;
            let response = Self::lock(&self.stores)
                .work
                .engagement
                .responses
                .get(&source.message)
                .copied()
                .ok_or(WorkError::Denied)?;
            let text = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                transport.hydrate_exchange(&candidate, response),
            )
            .await
            .map_err(|_| WorkError::Denied)??;
            if text.trim().is_empty() || text.len() > 64_000 {
                return Err(WorkError::Denied);
            }
            evidence.push(serde_json::json!({"message":source.message,"text":text}));
        }
        let input = serde_json::to_string(&evidence).map_err(|_| WorkError::Invalid)?;
        let raw = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.chat(PROMPT, &[crate::llm::ChatTurn::user(input)]),
        )
        .await
        .map_err(|_| WorkError::Denied)?
        .map_err(|_| WorkError::Denied)?
        .0;
        let assessment = parse_assessment(&raw, &scope, &sources)?;
        // Fresh exact source validation after the provider await; edits/deletions fail closed.
        for (index, source) in sources.iter().enumerate() {
            let response = Self::lock(&self.stores)
                .work
                .engagement
                .responses
                .get(&source.message)
                .copied()
                .ok_or(WorkError::Stale)?;
            let fresh = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                transport.hydrate_exchange(
                    &preview(source, kind, weekly.unwrap_or(source.at)),
                    response,
                ),
            )
            .await
            .map_err(|_| WorkError::Denied)??;
            if evidence[index]["text"].as_str() != Some(fresh.as_str()) {
                return Err(WorkError::Stale);
            }
            if !tokio::time::timeout(
                std::time::Duration::from_secs(30),
                transport.source_exists(source),
            )
            .await
            .map_err(|_| WorkError::Denied)??
            {
                return Err(WorkError::Stale);
            }
        }
        let reduction_state = self.clone();
        self.commit_work_owned(move |work| {
            let store = &mut work.engagement;
            if (kind == EngagementKind::FollowUp
                && reduction_state.follow_up_reduced(&scope, super::now()))
                || store
                    .member_policies
                    .get(&member)
                    .is_none_or(|p| p.revision != policy_revision)
                || !enabled(store, member, &scope)
                || !current(store, &latest)
                || sources.iter().any(|s| {
                    store
                        .eligibility
                        .get(&member)
                        .is_none_or(|rows| !rows.contains(s))
                })
            {
                return Err(WorkError::Stale);
            }
            if assessment.outcome == ConversationOutcome::Resolved {
                store.cancel_member_origin(member, &scope, latest.at.saturating_add(1));
                return Ok(());
            }
            if assessment.outcome != ConversationOutcome::Unresolved {
                return Ok(());
            }
            let source = sources
                .iter()
                .filter(|s| assessment.source_messages.contains(&s.message))
                .max_by_key(|s| (s.at, s.message))
                .cloned()
                .ok_or(WorkError::Invalid)?;
            if source != latest {
                return Ok(());
            }
            let due_at = weekly.unwrap_or(source.at.checked_add(86_400).ok_or(WorkError::Invalid)?);
            if let Some(due) = weekly
                && store
                    .member_policies
                    .get(&member)
                    .and_then(|p| p.weekly_subscription.as_ref())
                    .is_none_or(|w| w.scope != scope)
            {
                let _ = due;
                return Err(WorkError::Stale);
            }
            store.propose(
                CandidateProposal {
                    kind,
                    source: Some(source),
                    member: Some(member),
                    scope,
                    due_at,
                    introduction_id: None,
                },
                super::now(),
            )?;
            Ok(())
        })
        .await
        .map(|(value, _)| value)
    }
    pub(crate) async fn plan_weekly<T: EngagementTransport>(
        self: Arc<Self>,
        transport: &T,
        now: u64,
    ) -> Result<(), WorkError> {
        let plans = {
            let stores = Self::lock(&self.stores);
            let store = &stores.work.engagement;
            store
                .member_policies
                .iter()
                .filter_map(|(member, p)| {
                    let w = p.weekly_subscription.as_ref()?;
                    if !enabled(store, *member, &w.scope) {
                        return None;
                    }
                    // Include the active one-hour window; never enumerate older missed weeks.
                    let due = store
                        .next_weekly(*member, now.saturating_sub(3600))
                        .ok()
                        .flatten()?;
                    if store
                        .weekly_assessments
                        .get(member)
                        .is_some_and(|old| *old >= due)
                        || due > now
                    {
                        return None;
                    }
                    if store.candidates.values().any(|c| {
                        c.kind == EngagementKind::WeeklyCheckIn
                            && c.member == Some(*member)
                            && c.scope == w.scope
                            && c.due_at == due
                    }) {
                        return None;
                    }
                    let sources: Vec<_> = store
                        .eligibility
                        .get(member)?
                        .iter()
                        .filter(|s| s.scope == w.scope)
                        .cloned()
                        .collect();
                    if sources.is_empty() {
                        return None;
                    }
                    Some((*member, w.scope.clone(), sources, due))
                })
                .collect::<Vec<_>>()
        };
        for (member, scope, sources, due) in plans {
            self.commit_work_owned(|work| {
                work.engagement.weekly_assessments.insert(member, due);
                work.engagement.validate()
            })
            .await?;
            let _ = self
                .clone()
                .assess_engagement(scope, member, sources, transport, Some(due))
                .await;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
