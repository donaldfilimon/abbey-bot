//! Bounded fresh public evidence and closed Work audience proofs; no member scans.
use super::engagement_delivery::DiscordEngagementDelivery;
use crate::{
    engagement::{
        Candidate, DestinationPreference, EngagementKind, EngagementScope, SourceRef,
        community::assessment::{PUBLIC_PROMPT, PublicOutcome, parse_public_assessment},
        community::*,
        lifecycle::EngagementReservation,
    },
    runtime::{AppState, engagement_delivery::EngagementTransport},
    work::{WorkAccess, WorkContentRef, WorkError, WorkScope},
};
use serenity::all::{
    ChannelId, ChannelType, GuildId, PermissionOverwriteType, Permissions, UserId,
};
use std::collections::BTreeSet;
impl DiscordEngagementDelivery {
    pub(super) async fn public_message_current(&self, s: &SourceRef) -> Result<bool, WorkError> {
        self.source(s).await?;
        let after = ChannelId::new(channel(&s.scope))
            .messages(
                &self.0,
                serenity::all::GetMessages::new()
                    .after(s.message)
                    .limit(100),
            )
            .await
            .map_err(|_| WorkError::Denied)?;
        Ok(public_exchange_current(
            s.message,
            &after
                .iter()
                .map(|m| (m.id.get(), m.author.bot))
                .collect::<Vec<_>>(),
        ))
    }
    pub(super) async fn public_facts(
        &self,
        state: &AppState,
        now: u64,
        facts: &mut CommunityFacts,
    ) -> Result<(), WorkError> {
        let (store, work) = {
            let stores = AppState::lock(&state.stores);
            (stores.work.engagement.clone(), stores.work.clone())
        };
        collect_public_source_facts(state, self, now, &mut facts.rows).await?;
        for project in work.projects.values().take(1000) {
            let WorkScope::Team { guild, channel } = project.scope else {
                continue;
            };
            let scope = EngagementScope::Guild { guild, channel };
            if !enabled(&store, &scope, EngagementKind::ProjectCheckIn)
                || !state.engagement_public_gate(&scope, now)
            {
                continue;
            }
            let Some(actor) = project
                .managers
                .iter()
                .find(|m| {
                    store
                        .member_policies
                        .get(m)
                        .is_some_and(|p| p.personalized_enabled())
                })
                .copied()
            else {
                continue;
            };
            let content: BTreeSet<_> = work
                .tasks
                .values()
                .filter(|t| {
                    t.project_id == project.id
                        && matches!(
                            t.status,
                            crate::work::WorkStatus::Open
                                | crate::work::WorkStatus::InProgress
                                | crate::work::WorkStatus::Blocked
                        )
                })
                .take(8)
                .map(|t| WorkContentRef::Task {
                    project: project.id,
                    id: t.id,
                    revision: t.revision,
                })
                .collect();
            if content.is_empty() {
                continue;
            }
            let evidence = CommunityEvidence::Project {
                project: project.id,
                revision: project.revision,
                actor,
                audience: project.members.clone(),
                content,
            };
            if self
                .project_context(state, &scope, &evidence)
                .await
                .is_err()
            {
                continue;
            }
            facts.rows.push(CommunityFact {
                kind: EngagementKind::ProjectCheckIn,
                scope,
                source: None,
                evidence,
                at: now,
                useful: true,
                current: true,
            });
        }
        Ok(())
    }
    pub(super) async fn project_context(
        &self,
        state: &AppState,
        scope: &EngagementScope,
        evidence: &CommunityEvidence,
    ) -> Result<String, WorkError> {
        let CommunityEvidence::Project {
            actor, audience, ..
        } = evidence
        else {
            return Err(WorkError::Denied);
        };
        let EngagementScope::Guild { guild, channel } = *scope else {
            return Err(WorkError::Denied);
        };
        self.closed_audience(guild, channel, *actor, audience)
            .await?;
        let stores = AppState::lock(&state.stores);
        project_text(&stores.work, scope, evidence)
    }

    async fn closed_audience(
        &self,
        guild: u64,
        channel: u64,
        actor: u64,
        audience: &BTreeSet<u64>,
    ) -> Result<(), WorkError> {
        if audience.is_empty()
            || audience.len() > 1000
            || audience.contains(&0)
            || !audience.contains(&actor)
        {
            return Err(WorkError::Denied);
        }
        let guild = GuildId::new(guild)
            .to_partial_guild(&self.0)
            .await
            .map_err(|_| WorkError::Denied)?;
        let native = ChannelId::new(channel)
            .to_channel(&self.0)
            .await
            .map_err(|_| WorkError::Denied)?
            .guild()
            .ok_or(WorkError::Denied)?;
        let bot = self
            .0
            .get_current_user()
            .await
            .map_err(|_| WorkError::Denied)?
            .id;
        if native.guild_id != guild.id
            || native.kind != ChannelType::Text
            || !closed_overwrites(
                guild.id.get(),
                guild.owner_id.get(),
                bot.get(),
                audience,
                &native.permission_overwrites,
            )
            || guild
                .roles
                .values()
                .any(|r| r.permissions.contains(Permissions::ADMINISTRATOR))
        {
            return Err(WorkError::Denied);
        }
        for user in audience.iter().copied().chain(std::iter::once(bot.get())) {
            let member = guild
                .member(&self.0, UserId::new(user))
                .await
                .map_err(|_| WorkError::Denied)?;
            let required = Permissions::VIEW_CHANNEL
                | Permissions::READ_MESSAGE_HISTORY
                | if user == bot.get() {
                    Permissions::SEND_MESSAGES
                } else {
                    Permissions::empty()
                };
            if !guild
                .user_permissions_in(&native, &member)
                .contains(required)
            {
                return Err(WorkError::Denied);
            }
        }
        Ok(())
    }
    pub(super) async fn community_context(
        &self,
        state: &AppState,
        c: &Candidate,
    ) -> Result<String, WorkError> {
        let receipt = {
            let stores = AppState::lock(&state.stores);
            let store = &stores.work.engagement;
            if !store.community_receipt_current(c) {
                return Err(WorkError::Stale);
            }
            store
                .community_receipts
                .get(&c.id)
                .cloned()
                .ok_or(WorkError::Denied)?
        };
        match receipt.evidence {
            CommunityEvidence::Message => {
                let s = c.source.as_ref().ok_or(WorkError::Denied)?;
                if !self.public_message_current(s).await? {
                    return Err(WorkError::Stale);
                }
                self.source(s).await
            }
            CommunityEvidence::Join { .. } => {
                if !crate::runtime::engagement_community::join_events_available() {
                    return Err(WorkError::Denied);
                }
                Ok("An actual new human member joined this server. Welcome them without addressing or identifying anyone.".into())
            }
            evidence @ CommunityEvidence::Project { .. } => {
                self.project_context(state, &c.scope, &evidence).await
            }
        }
    }
}
fn channel(scope: &EngagementScope) -> u64 {
    match scope {
        EngagementScope::Guild { channel, .. } | EngagementScope::Dm { channel, .. } => *channel,
    }
}
/// Everyone denied, no role viewer grants, no individual outside the exact Work
/// audience, and owner included: a closed upper bound without guild enumeration.
fn closed_overwrites(
    guild: u64,
    owner: u64,
    bot: u64,
    audience: &BTreeSet<u64>,
    rows: &[serenity::all::PermissionOverwrite],
) -> bool {
    let owner_in_audience = audience.contains(&owner);
    let everyone_denied = rows.iter().any(|r| {
        matches!(r.kind,PermissionOverwriteType::Role(id) if id.get()==guild)
            && r.deny.contains(Permissions::VIEW_CHANNEL)
            && !r.allow.contains(Permissions::VIEW_CHANNEL)
    });
    let no_outside_grants = rows.iter().all(|r| {
        if !r.allow.contains(Permissions::VIEW_CHANNEL) {
            return true;
        }
        match r.kind {
            PermissionOverwriteType::Member(id) => audience.contains(&id.get()) || id.get() == bot,
            _ => false,
        }
    });
    owner_in_audience && everyone_denied && no_outside_grants
}
#[cfg(test)]
mod tests;

fn project_text(
    work: &crate::work::WorkStore,
    scope: &EngagementScope,
    evidence: &CommunityEvidence,
) -> Result<String, WorkError> {
    let CommunityEvidence::Project {
        project,
        revision,
        actor,
        audience,
        content,
    } = evidence
    else {
        return Err(WorkError::Denied);
    };
    let EngagementScope::Guild { guild, channel } = *scope else {
        return Err(WorkError::Denied);
    };
    let p = work.projects.get(project).ok_or(WorkError::Missing)?;
    if p.revision != *revision
        || p.members != *audience
        || p.scope != (WorkScope::Team { guild, channel })
    {
        return Err(WorkError::Stale);
    }
    p.authorize(
        WorkAccess {
            actor: *actor,
            guild: Some(guild),
            channel,
            can_view: true,
            can_manage: false,
        },
        true,
    )?;
    let mut evidence = Vec::new();
    for source in content {
        let WorkContentRef::Task {
            project: source_project,
            id,
            revision,
        } = source
        else {
            return Err(WorkError::Denied);
        };
        let t = work.tasks.get(id).ok_or(WorkError::Missing)?;
        if source_project != project
            || t.project_id != *project
            || t.revision != *revision
            || !matches!(
                t.status,
                crate::work::WorkStatus::Open
                    | crate::work::WorkStatus::InProgress
                    | crate::work::WorkStatus::Blocked
            )
        {
            return Err(WorkError::Stale);
        }
        evidence.push(serde_json::json!({"task":t.title,"status":t.status.label()}));
    }
    if evidence.is_empty() {
        return Err(WorkError::Denied);
    }
    Ok(serde_json::json!({"project":p.name,"tasks":evidence}).to_string())
}

fn public_exchange_current(source: u64, after: &[(u64, bool)]) -> bool {
    after.len() < 100 && after.iter().all(|(id, bot)| *id > source && *bot)
}

/// Only explicit configured observations enter the bounded assessment inventory.
fn public_sources(store: &crate::engagement::EngagementStore, now: u64) -> Vec<SourceRef> {
    let mut sources: Vec<_> = store
        .observations
        .values()
        .flat_map(|rows| rows.values())
        .filter(|source| {
            if store.candidates.values().any(|c| {
                c.scope == source.scope
                    && matches!(
                        c.kind,
                        EngagementKind::ConversationStarter | EngagementKind::UnansweredQuestion
                    )
                    && c.source
                        .as_ref()
                        .is_some_and(|s| s.message == source.message)
            }) {
                return false;
            }
            [
                EngagementKind::ConversationStarter,
                EngagementKind::UnansweredQuestion,
            ]
            .into_iter()
            .any(|kind| {
                let facts = CommunityFacts {
                    join_events_available: false,
                    rows: vec![CommunityFact {
                        kind,
                        scope: source.scope.clone(),
                        source: Some((*source).clone()),
                        evidence: CommunityEvidence::Message,
                        at: source.at,
                        useful: true,
                        current: true,
                    }],
                };
                !community_candidates(store, &facts, now).is_empty()
            })
        })
        .cloned()
        .collect();
    sources.sort();
    if let Some(cursor) = &store.community_cursor {
        let pivot = sources.partition_point(|s| s <= cursor);
        sources.rotate_left(pivot);
    }
    sources.truncate(8);
    sources
}
fn assessed_kind(
    store: &crate::engagement::EngagementStore,
    source: &SourceRef,
    raw: &str,
    text: &str,
) -> Option<EngagementKind> {
    let assessment = parse_public_assessment(raw, source, text).ok()?;
    match assessment.outcome {
        PublicOutcome::UnansweredQuestion
            if enabled(store, &source.scope, EngagementKind::UnansweredQuestion) =>
        {
            Some(EngagementKind::UnansweredQuestion)
        }
        PublicOutcome::UnansweredQuestion | PublicOutcome::UsefulContext
            if enabled(store, &source.scope, EngagementKind::ConversationStarter) =>
        {
            Some(EngagementKind::ConversationStarter)
        }
        _ => None,
    }
}
trait PublicSourceTransport: Send + Sync {
    fn context(
        &self,
        source: &SourceRef,
    ) -> impl std::future::Future<Output = Result<String, WorkError>> + Send;
    fn assessment(
        &self,
        state: &AppState,
        source: &SourceRef,
        text: &str,
    ) -> impl std::future::Future<Output = Result<String, WorkError>> + Send;
}
impl PublicSourceTransport for DiscordEngagementDelivery {
    async fn context(&self, source: &SourceRef) -> Result<String, WorkError> {
        let r = EngagementReservation {
            introduction: None,
            candidate_id: 0,
            revision: 1,
            policy_revision: 0,
            scope: source.scope.clone(),
            member: Some(source.author),
            destination: DestinationPreference::Origin,
        };
        self.authorize(&r).await?;
        if !self.public_message_current(source).await? {
            return Err(WorkError::Stale);
        }
        self.source(source).await
    }
    async fn assessment(
        &self,
        state: &AppState,
        source: &SourceRef,
        text: &str,
    ) -> Result<String, WorkError> {
        state
            .chat(
                PUBLIC_PROMPT,
                &[crate::llm::ChatTurn::user(
                    serde_json::json!([{"message":source.message,"text":text}]).to_string(),
                )],
            )
            .await
            .map(|result| result.0)
            .map_err(|_| WorkError::Denied)
    }
}
#[cfg(test)]
async fn public_source_facts<T: PublicSourceTransport>(
    state: &AppState,
    transport: &T,
    now: u64,
) -> Result<Vec<CommunityFact>, WorkError> {
    let mut facts = Vec::new();
    collect_public_source_facts(state, transport, now, &mut facts).await?;
    Ok(facts)
}
async fn collect_public_source_facts<T: PublicSourceTransport>(
    state: &AppState,
    transport: &T,
    now: u64,
    facts: &mut Vec<CommunityFact>,
) -> Result<(), WorkError> {
    let store = {
        let stores = AppState::lock(&state.stores);
        stores.work.engagement.clone()
    };
    for source in public_sources(&store, now) {
        // Persist progress before any fallible await. A timeout/dropped assessment
        // still advances the next retained tick and a restarted process.
        state.advance_community_cursor(source.clone()).await?;
        if !state.engagement_public_gate(&source.scope, now) {
            continue;
        }
        let Ok(text) = transport.context(&source).await else {
            continue;
        };
        if text.trim().is_empty() || text.len() > 64_000 {
            continue;
        }
        let Ok(raw) = transport.assessment(state, &source, &text).await else {
            continue;
        };
        let Some(kind) = assessed_kind(&store, &source, &raw, &text) else {
            continue;
        };
        if !transport
            .context(&source)
            .await
            .is_ok_and(|fresh| fresh == text)
        {
            continue;
        }
        facts.push(CommunityFact {
            kind,
            scope: source.scope.clone(),
            source: Some(source.clone()),
            evidence: CommunityEvidence::Message,
            at: source.at,
            useful: true,
            current: true,
        });
    }
    Ok(())
}
