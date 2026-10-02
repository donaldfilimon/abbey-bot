//! Retained public planning; uses the existing delivery owner and canonical store.
use super::{AppState, engagement_delivery::EngagementTransport};
use crate::{
    engagement::{CommunityFeature, EngagementKind, EngagementScope, community::*},
    work::WorkError,
};
use std::sync::Arc;
/// Current startup requests non-privileged intents plus voice, optionally content.
/// It does not request GUILD_MEMBERS. Never infer join availability from scans.
pub(crate) const fn join_events_available() -> bool {
    false
}
impl AppState {
    pub(crate) async fn advance_community_cursor(
        &self,
        source: crate::engagement::SourceRef,
    ) -> Result<(), WorkError> {
        self.commit_engagement_invalidation(move |store| {
            store.community_cursor = Some(source);
            store.validate()
        })
        .await
    }

    pub(crate) fn engagement_public_gate(&self, scope: &EngagementScope, now: u64) -> bool {
        self.engagement_guild_gate(scope, now, false)
    }
    pub(crate) fn community_observation_allowed(&self, scope: &EngagementScope) -> bool {
        let EngagementScope::Guild { guild, channel } = scope else {
            return false;
        };
        let enabled = {
            let stores = Self::lock(&self.stores);
            stores
                .work
                .engagement
                .guild_features
                .get(guild)
                .is_some_and(|p| {
                    [CommunityFeature::Starters, CommunityFeature::Questions]
                        .iter()
                        .any(|f| {
                            p.enabled.contains(f)
                                && p.channels.get(f).is_some_and(|c| c.contains(channel))
                        })
                })
        };
        enabled && self.engagement_guild_gate(scope, super::now(), false)
    }
    pub(crate) async fn plan_community<T: EngagementTransport>(
        self: Arc<Self>,
        transport: &T,
        now: u64,
    ) -> Result<(), WorkError> {
        let configured = {
            let stores = Self::lock(&self.stores);
            stores
                .work
                .engagement
                .guild_features
                .values()
                .any(|p| !p.enabled.is_empty())
        };
        if !configured {
            return Ok(());
        }
        let mut facts = CommunityFacts::default();
        if let Ok(result) = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            transport.community_facts(&self, now, &mut facts),
        )
        .await
        {
            result?;
        }
        // On timeout the active incomplete proof is dropped; fully proved
        // metadata remains owned and is proposed under current store policy.
        let facts = CommunityFacts {
            rows: facts
                .rows
                .into_iter()
                .filter(|f| self.engagement_guild_gate(&f.scope, now, false))
                .collect(),
            join_events_available: facts.join_events_available,
        };
        self.commit_work_owned(move |work| work.engagement.propose_community(&facts, now))
            .await?;
        Ok(())
    }
    pub(crate) async fn community_join(
        &self,
        guild: u64,
        member: u64,
        joined_at: u64,
        is_bot: bool,
    ) -> Result<bool, WorkError> {
        let configured = {
            let stores = Self::lock(&self.stores);
            stores
                .work
                .engagement
                .guild_features
                .get(&guild)
                .is_some_and(|p| p.enabled.contains(&CommunityFeature::Welcomes))
        };
        if !configured {
            return Ok(false);
        }
        // Configured new behavior owns this event, even when capability is absent.
        if is_bot || !join_events_available() {
            return Ok(true);
        }
        let channels = {
            let stores = Self::lock(&self.stores);
            stores
                .work
                .engagement
                .guild_features
                .get(&guild)
                .and_then(|p| p.channels.get(&CommunityFeature::Welcomes))
                .cloned()
                .unwrap_or_default()
        };
        let now = super::now();
        // One configured destination per event: no fan-out welcome.
        if let Some(channel) = channels.into_iter().next() {
            let scope = EngagementScope::Guild { guild, channel };
            if self.engagement_guild_gate(&scope, now, false) {
                let facts = CommunityFacts {
                    join_events_available: true,
                    rows: vec![CommunityFact {
                        kind: EngagementKind::Welcome,
                        scope,
                        source: None,
                        evidence: CommunityEvidence::Join { member, joined_at },
                        at: joined_at,
                        useful: true,
                        current: true,
                    }],
                };
                self.commit_engagement(move |store| store.propose_community(&facts, now))
                    .await?;
            }
        }
        Ok(true)
    }
}
#[cfg(test)]
mod tests;
