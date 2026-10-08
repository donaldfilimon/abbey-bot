//! Scope-preserving removal of linkable engagement records and their indexes.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErasedContactCharge {
    pub member: u64,
    pub local_day: String,
    pub local_week: String,
    pub at: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErasedCommunityCharge {
    pub project: Option<String>,
    pub scope: EngagementScope,
    pub kind: EngagementKind,
    pub at: u64,
}

impl EngagementScope {
    pub(crate) fn learning_scope(&self) -> String {
        match self {
            Self::Guild { guild, .. } => format!("discord:{guild}"),
            Self::Dm { member, .. } => format!("discord:dm:{member}"),
        }
    }
}
impl EngagementStore {
    pub(crate) fn erase_learning(
        &mut self,
        scope: &str,
        member: Option<u64>,
    ) -> Result<usize, WorkError> {
        let mut removed = 0;
        let matches = |s: &EngagementScope| s.learning_scope() == scope;
        let own = |m: u64| member.is_none_or(|v| v == m);
        let introductions: BTreeSet<_> = self
            .introductions
            .iter()
            .filter(|(_, i)| {
                matches(&i.scope) && (member.is_none() || i.members.iter().any(|m| own(*m)))
            })
            .map(|(id, _)| *id)
            .collect();
        let candidates: BTreeSet<_> = self
            .candidates
            .iter()
            .filter(|(_, c)| {
                matches(&c.scope)
                    && (member.is_none()
                        || c.member.is_some_and(own)
                        || c.source.as_ref().is_some_and(|s| own(s.author))
                        || self
                            .community_receipts
                            .get(&c.id)
                            .is_some_and(|r| match &r.evidence {
                                community::CommunityEvidence::Join { member, .. } => own(*member),
                                community::CommunityEvidence::Project {
                                    actor, audience, ..
                                } => own(*actor) || audience.iter().any(|m| own(*m)),
                                community::CommunityEvidence::Message => false,
                            })
                        || c.introduction_id
                            .is_some_and(|id| introductions.contains(&id)))
            })
            .map(|(id, _)| *id)
            .collect();
        let mut identities = self.erased_identities.clone();
        for id in &introductions {
            let i = &self.introductions[id];
            identities.insert(erasure_identity::introduction(&i.scope, i.members));
        }
        for id in &candidates {
            let c = &self.candidates[id];
            if let Some(task) = &c.work_ref {
                identities.insert(erasure_identity::task_follow_up(
                    &crate::work::follow_up::work_scope(&c.scope),
                    task,
                ));
            }
            if c.kind != EngagementKind::Introduction
                && !invitations::invitation_kind(c.kind)
                && community::feature(c.kind).is_none()
            {
                identities.insert(erasure_identity::candidate(c));
            }
            if let Some(r) = self.community_receipts.get(id) {
                identities.insert(erasure_identity::community(
                    &c.scope,
                    c.kind,
                    &r.evidence,
                    c.source.as_ref(),
                ));
            }
            if let Some(r) = self.invitation_requests.get(id) {
                identities.insert(erasure_identity::invitation(r.interaction));
            }
        }
        for r in self.suppressed_invitation_requests.values().filter(|r| {
            candidates.contains(&r.blocking_candidate)
                || (matches(&r.request.scope) && own(r.request.member))
        }) {
            identities.insert(erasure_identity::invitation(r.request.interaction));
        }
        if identities.len() > 10_000 {
            return Err(WorkError::Full);
        }
        self.erased_identities = identities;
        let mut sources = BTreeSet::new();
        for (s, rows) in &mut self.observations {
            if matches(s) {
                rows.retain(|m, source| {
                    let keep = !own(*m);
                    if !keep {
                        sources.insert(source.message);
                        removed += 1;
                    }
                    keep
                });
            }
        }
        self.observations.retain(|_, r| !r.is_empty());
        for (m, rows) in &mut self.eligibility {
            if own(*m) {
                rows.retain(|source| {
                    let keep = !matches(&source.scope);
                    if !keep {
                        sources.insert(source.message);
                        removed += 1;
                    }
                    keep
                });
            }
        }
        self.eligibility.retain(|_, r| !r.is_empty());
        for c in self
            .candidates
            .values()
            .filter(|c| candidates.contains(&c.id))
        {
            if let Some(s) = &c.source {
                sources.insert(s.message);
            }
        }
        self.responses.retain(|s, _| {
            let keep = !sources.contains(s);
            removed += usize::from(!keep);
            keep
        });
        for c in self
            .charges
            .iter()
            .filter(|c| candidates.contains(&c.candidate_id))
        {
            self.erased_contact_charges.push(ErasedContactCharge {
                member: c.member,
                local_day: c.local_day.clone(),
                local_week: c.local_week.clone(),
                at: c.at,
            });
        }
        for (id, r) in self
            .community_receipts
            .iter()
            .filter(|(id, _)| candidates.contains(id))
        {
            if let Some(at) = r.attempted_at {
                let c = &self.candidates[id];
                self.erased_community_charges.push(ErasedCommunityCharge {
                    project: match r.evidence {
                        community::CommunityEvidence::Project { project, .. } => {
                            Some(erasure_identity::project(project))
                        }
                        _ => None,
                    },
                    scope: c.scope.clone(),
                    kind: c.kind,
                    at,
                });
            }
        }
        self.candidates.retain(|id, _| {
            let keep = !candidates.contains(id);
            removed += usize::from(!keep);
            keep
        });
        self.introductions.retain(|id, _| {
            let keep = !introductions.contains(id);
            removed += usize::from(!keep);
            keep
        });
        self.charges.retain(|c| {
            let keep = !candidates.contains(&c.candidate_id);
            removed += usize::from(!keep);
            keep
        });
        self.community_receipts.retain(|id, _| {
            let keep = !candidates.contains(id);
            removed += usize::from(!keep);
            keep
        });
        self.invitation_requests.retain(|_, r| {
            let keep = !(matches(&r.scope) && own(r.member));
            removed += usize::from(!keep);
            keep
        });
        self.suppressed_invitation_requests.retain(|_, r| {
            let keep = !candidates.contains(&r.blocking_candidate)
                && !(matches(&r.request.scope) && own(r.request.member));
            removed += usize::from(!keep);
            keep
        });
        self.feedback.retain(|id, rows| {
            if candidates.contains(id) {
                removed += rows.len();
                return false;
            }
            if self.candidates.get(id).is_some_and(|c| matches(&c.scope)) {
                rows.retain(|m, _| {
                    let keep = !own(*m);
                    removed += usize::from(!keep);
                    keep
                });
            }
            !rows.is_empty()
        });
        if self
            .community_cursor
            .as_ref()
            .is_some_and(|s| matches(&s.scope) && own(s.author))
        {
            self.community_cursor = None;
            removed += 1;
        }
        for (m, p) in &mut self.member_policies {
            if !own(*m) {
                continue;
            }
            // Global limits/timezone/stops are shared with other scopes and are
            // safety settings, not inferred learning. Remove scoped preferences.
            p.destinations.retain(|s, _| {
                let keep = !matches(s);
                removed += usize::from(!keep);
                keep
            });
            if p.weekly_subscription
                .as_ref()
                .is_some_and(|w| matches(&w.scope))
            {
                p.weekly_subscription = None;
                removed += 1;
                removed += usize::from(self.weekly_assessments.remove(m).is_some());
            }
        }
        Ok(removed)
    }
    /// A retained callback must still hold its original source identity. This
    /// also covers direct source observations queued behind the commit owner.
    pub(crate) fn erasure_admitted(
        &self,
        ledger: &crate::brain::erasure::ErasureLedger,
        before: &Self,
        admitted: u64,
    ) -> bool {
        if self
            .invitation_requests
            .values()
            .chain(
                self.suppressed_invitation_requests
                    .values()
                    .map(|r| &r.request),
            )
            .any(|r| {
                ledger.blocks(
                    &r.scope.learning_scope(),
                    &format!("discord:{}", r.member),
                    r.at,
                )
            })
        {
            return false;
        }
        let permitted = |s: &SourceRef| {
            !ledger.blocks(
                &s.scope.learning_scope(),
                &format!("discord:{}", s.author),
                s.at,
            )
        };
        if self
            .observations
            .values()
            .flat_map(|r| r.values())
            .chain(self.eligibility.values().flat_map(|r| r.iter()))
            .any(|s| {
                !permitted(s)
                    && !before
                        .observations
                        .values()
                        .flat_map(|r| r.values())
                        .chain(before.eligibility.values().flat_map(|r| r.iter()))
                        .any(|old| old == s)
            })
        {
            return false;
        }
        if self.community_cursor != before.community_cursor
            && self
                .community_cursor
                .as_ref()
                .is_some_and(|source| !permitted(source))
        {
            return false;
        }
        for (id, c) in &self.candidates {
            if before.candidates.get(id) == Some(c) {
                continue;
            }
            if c.source.as_ref().is_some_and(|source| !permitted(source)) {
                return false;
            }
            let scope = c.scope.learning_scope();
            let at = c.source.as_ref().map_or(admitted, |s| s.at);
            if ledger.blocks(
                &scope,
                &c.member
                    .map_or_else(String::new, |m| format!("discord:{m}")),
                at,
            ) {
                return false;
            }
            if let Some(i) = c.introduction_id.and_then(|id| self.introductions.get(&id))
                && i.members
                    .iter()
                    .any(|m| ledger.blocks(&scope, &format!("discord:{m}"), c.due_at))
            {
                return false;
            }
        }
        for (id, receipt) in &self.community_receipts {
            if before.community_receipts.get(id) == Some(receipt) {
                continue;
            }
            let Some(candidate) = self.candidates.get(id) else {
                return false;
            };
            let scope = candidate.scope.learning_scope();
            let blocked = |member, at| ledger.blocks(&scope, &format!("discord:{member}"), at);
            match &receipt.evidence {
                community::CommunityEvidence::Join { member, joined_at }
                    if blocked(*member, *joined_at) =>
                {
                    return false;
                }
                community::CommunityEvidence::Project {
                    actor, audience, ..
                } if blocked(*actor, admitted)
                    || audience.iter().any(|member| blocked(*member, admitted)) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        for (member, due) in &self.weekly_assessments {
            if before.weekly_assessments.get(member) == Some(due) {
                continue;
            }
            let Some(subscription) = self
                .member_policies
                .get(member)
                .and_then(|p| p.weekly_subscription.as_ref())
            else {
                return false;
            };
            if ledger.blocks(
                &subscription.scope.learning_scope(),
                &format!("discord:{member}"),
                admitted,
            ) {
                return false;
            }
        }
        for (member, p) in &self.member_policies {
            if before.member_policies.get(member) == Some(p) {
                continue;
            }
            for scope in p
                .destinations
                .keys()
                .chain(p.weekly_subscription.iter().map(|w| &w.scope))
            {
                if ledger.blocks(
                    &scope.learning_scope(),
                    &format!("discord:{member}"),
                    admitted,
                ) {
                    return false;
                }
            }
        }
        true
    }
}

impl EngagementStore {
    pub(super) fn validate_erased_charges(&self) -> Result<(), WorkError> {
        if self.erased_identities.len() > 10_000
            || self
                .erased_identities
                .iter()
                .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(WorkError::Full);
        }
        if self.erased_contact_charges.len() + self.charges.len() > 20_000
            || self.erased_community_charges.len() + self.community_receipts.len() > 10_000
        {
            return Err(WorkError::Full);
        }
        for c in &self.erased_contact_charges {
            if c.member == 0
                || crate::calendar::utc(c.at).is_err()
                || chrono::NaiveDate::parse_from_str(&c.local_day, "%Y-%m-%d").is_err()
                || chrono::NaiveDate::parse_from_str(&c.local_week, "%Y-%m-%d").is_err()
            {
                return Err(WorkError::Invalid);
            }
        }
        for c in &self.erased_community_charges {
            c.scope.validate()?;
            if crate::calendar::utc(c.at).is_err()
                || community::feature(c.kind).is_none()
                || (c.kind == EngagementKind::ProjectCheckIn) != c.project.is_some()
                || c.project
                    .as_ref()
                    .is_some_and(|p| p.len() != 64 || !p.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(WorkError::Invalid);
            }
        }
        Ok(())
    }
    /// Nine elapsed days exceed a local week plus the full UTC offset span,
    /// even across DST or a changed timezone. Saved bucket labels must also be
    /// older than every currently possible local calendar day/week. Clock
    /// rollback retains the row; no protective counter is evicted for capacity.
    pub(crate) fn prune_erasure_safety(&mut self, now: u64) {
        use chrono::Datelike;
        let before = self.erased_contact_charges.len() + self.erased_community_charges.len();
        let Ok(earliest) = crate::calendar::utc(now.saturating_sub(2 * 86400)) else {
            return;
        };
        let day = earliest.date_naive();
        let week = day - chrono::Duration::days(i64::from(day.weekday().num_days_from_monday()));
        self.erased_contact_charges.retain(|c| {
            now.saturating_sub(c.at) <= 9 * 86400
                || chrono::NaiveDate::parse_from_str(&c.local_day, "%Y-%m-%d")
                    .is_ok_and(|d| d >= day)
                || chrono::NaiveDate::parse_from_str(&c.local_week, "%Y-%m-%d")
                    .is_ok_and(|d| d >= week)
        });
        self.erased_community_charges
            .retain(|c| now.saturating_sub(c.at) <= community::WEEK);
        if before > self.erased_contact_charges.len() + self.erased_community_charges.len() {
            self.safety_pruned_through = self.safety_pruned_through.max(now);
        }
    }
}
#[cfg(test)]
mod tests;
