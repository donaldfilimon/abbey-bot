//! Exact member-authored introductions. Approval never grants retrieval authority.
use super::*;

pub fn description_valid(text: &str) -> bool {
    !text.trim().is_empty() && text.chars().count() <= 300
}
pub fn enabled(s: &EngagementStore, scope: &EngagementScope) -> bool {
    let EngagementScope::Guild { guild, channel } = scope else {
        return false;
    };
    s.guild_features.get(guild).is_some_and(|p| {
        p.enabled.contains(&CommunityFeature::Introductions)
            && p.channels
                .get(&CommunityFeature::Introductions)
                .is_some_and(|c| c.contains(channel))
    })
}
impl EngagementStore {
    /// Exact own approval supplies this introduction’s one-time eligibility.
    /// It never adds a fabricated source or enables other engagement kinds.
    pub(crate) fn introduction_member_enabled(&self, member: u64, scope: &EngagementScope) -> bool {
        let Some(p) = self.member_policies.get(&member) else {
            return false;
        };
        p.personalized_enabled()
            && !p.stopped_scopes.contains(scope)
            && !matches!(scope,EngagementScope::Guild {guild,..} if p.stopped_guilds.contains(guild))
    }
    pub fn create_introduction(
        &mut self,
        members: [u64; 2],
        scope: EngagementScope,
        description: String,
        now: u64,
    ) -> Result<u64, WorkError> {
        scope.validate()?;
        crate::calendar::utc(now)?;
        if members.contains(&0) || members[0] == members[1] || !description_valid(&description) {
            return Err(WorkError::Invalid);
        }
        if !enabled(self, &scope)
            || members
                .iter()
                .any(|m| !self.introduction_member_enabled(*m, &scope))
        {
            return Err(WorkError::Denied);
        }
        if self.candidates.len() + self.introductions.len() + 2 > 10_000
            || self
                .candidates
                .values()
                .filter(|c| matches!(c.state, CandidateState::Pending | CandidateState::Reserved))
                .count()
                >= 1000
        {
            return Err(WorkError::Full);
        }
        // A pair has one durable identity per guild: neither edits nor restart solicit a repeat.
        if self.introductions.values().any(|i| {
            matches!((&i.scope, &scope),
                (EngagementScope::Guild { guild: a, .. }, EngagementScope::Guild { guild: b, .. }) if a == b)
                && members.iter().all(|m| i.members.contains(m))
        }) {
            return Err(WorkError::Stale);
        }
        let EngagementScope::Guild { guild, channel } = scope else {
            return Err(WorkError::Invalid);
        };
        let id = self.sequence.checked_add(1).ok_or(WorkError::Full)?;
        let candidate_id = id.checked_add(1).ok_or(WorkError::Full)?;
        let scope = EngagementScope::Guild { guild, channel };
        let mut next = self.clone();
        next.sequence = candidate_id;
        next.introductions.insert(
            id,
            Introduction {
                id,
                revision: 1,
                scope: scope.clone(),
                members,
                approved_self_descriptions: [Some(description), None],
                approvals: [None; 2],
                destination: channel,
                state: IntroductionState::Pending,
            },
        );
        next.candidates.insert(
            candidate_id,
            Candidate {
                id: candidate_id,
                kind: EngagementKind::Introduction,
                source: None,
                member: None,
                scope,
                due_at: now,
                revision: 1,
                state: CandidateState::Pending,
                dedupe_key: format!("introduction:{id}"),
                policy_revision: self.guild_features[&guild].revision,
                destination: DestinationPreference::Origin,
                message_id: None,
                introduction_id: Some(id),
            },
        );
        next.validate()?;
        *self = next;
        Ok(id)
    }
    pub fn edit_introduction(
        &mut self,
        id: u64,
        member: u64,
        revision: u64,
        description: String,
        destination: Option<u64>,
    ) -> Result<u64, WorkError> {
        if !description_valid(&description) {
            return Err(WorkError::Invalid);
        }
        let mut next = self.clone();
        let i = next.introductions.get_mut(&id).ok_or(WorkError::Missing)?;
        let index = i
            .members
            .iter()
            .position(|m| *m == member)
            .ok_or(WorkError::Denied)?;
        if i.revision != revision
            || !matches!(
                i.state,
                IntroductionState::Pending | IntroductionState::Ready
            )
        {
            return Err(WorkError::Stale);
        }
        let revision = revision.checked_add(1).ok_or(WorkError::Full)?;
        if let Some(destination) = destination {
            if destination == 0 {
                return Err(WorkError::Invalid);
            }
            let EngagementScope::Guild { guild, .. } = i.scope else {
                return Err(WorkError::Invalid);
            };
            i.scope = EngagementScope::Guild {
                guild,
                channel: destination,
            };
            i.destination = destination;
        }
        i.revision = revision;
        i.approved_self_descriptions[index] = Some(description);
        i.approvals = [None; 2];
        i.state = IntroductionState::Pending;
        let scope = i.scope.clone();
        if !enabled(&next, &scope) {
            return Err(WorkError::Denied);
        }
        let EngagementScope::Guild { guild, .. } = scope else {
            return Err(WorkError::Invalid);
        };
        for c in next
            .candidates
            .values_mut()
            .filter(|c| c.introduction_id == Some(id))
        {
            if c.state != CandidateState::Pending {
                return Err(WorkError::Stale);
            }
            c.revision = revision;
            c.scope = scope.clone();
            c.policy_revision = next.guild_features[&guild].revision;
        }
        next.validate()?;
        *self = next;
        Ok(revision)
    }
    pub fn approve_introduction(
        &mut self,
        id: u64,
        member: u64,
        revision: u64,
    ) -> Result<bool, WorkError> {
        let i = self.introductions.get(&id).ok_or(WorkError::Missing)?;
        let index = i
            .members
            .iter()
            .position(|m| *m == member)
            .ok_or(WorkError::Denied)?;
        if i.revision != revision
            || !matches!(
                i.state,
                IntroductionState::Pending | IntroductionState::Ready
            )
        {
            return Err(WorkError::Stale);
        }
        if i.approved_self_descriptions[index].is_none()
            || !enabled(self, &i.scope)
            || i.members
                .iter()
                .any(|m| !self.introduction_member_enabled(*m, &i.scope))
        {
            return Err(WorkError::Denied);
        }
        let c = self
            .candidates
            .values()
            .find(|c| c.introduction_id == Some(id))
            .ok_or(WorkError::Missing)?;
        let EngagementScope::Guild { guild, .. } = i.scope else {
            return Err(WorkError::Invalid);
        };
        if c.state != CandidateState::Pending
            || c.policy_revision != self.guild_features[&guild].revision
        {
            return Err(WorkError::Stale);
        }
        let i = self.introductions.get_mut(&id).ok_or(WorkError::Missing)?;
        i.approvals[index] = Some(revision);
        let ready = i.approvals == [Some(revision); 2];
        if ready {
            i.state = IntroductionState::Ready;
        }
        Ok(ready)
    }
    pub fn withdraw_introduction(
        &mut self,
        id: u64,
        member: u64,
        revision: u64,
    ) -> Result<(), WorkError> {
        let i = self.introductions.get(&id).ok_or(WorkError::Missing)?;
        if !i.members.contains(&member) {
            return Err(WorkError::Denied);
        }
        if i.revision != revision
            || !self.candidates.values().any(|c| {
                c.introduction_id == Some(id)
                    && matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
            })
        {
            return Err(WorkError::Stale);
        }
        let revision = revision.checked_add(1).ok_or(WorkError::Full)?;
        let i = self.introductions.get_mut(&id).ok_or(WorkError::Missing)?;
        i.revision = revision;
        i.state = IntroductionState::Cancelled;
        i.approvals = [None; 2];
        for c in self
            .candidates
            .values_mut()
            .filter(|c| c.introduction_id == Some(id))
        {
            c.state = CandidateState::Cancelled;
            c.revision = revision;
        }
        Ok(())
    }
    pub(crate) fn introduction_receipt_current(&self, c: &Candidate) -> bool {
        c.kind == EngagementKind::Introduction
            && c.source.is_none()
            && c.member.is_none()
            && c.destination == DestinationPreference::Origin
            && enabled(self, &c.scope)
            && c.introduction_id
                .and_then(|id| self.introductions.get(&id))
                .is_some_and(|i| {
                    let EngagementScope::Guild { guild, channel } = i.scope else {
                        return false;
                    };
                    i.scope == c.scope
                        && i.destination == channel
                        && i.revision == c.revision
                        && i.approvals == [Some(i.revision); 2]
                        && i.approved_self_descriptions
                            .iter()
                            .all(|s| s.as_ref().is_some_and(|s| description_valid(s)))
                        && matches!(
                            (i.state, c.state),
                            (IntroductionState::Ready, CandidateState::Pending)
                                | (IntroductionState::Consumed, CandidateState::Reserved)
                        )
                        && self
                            .guild_features
                            .get(&guild)
                            .is_some_and(|p| p.revision == c.policy_revision)
                        && i.members
                            .iter()
                            .all(|m| self.introduction_member_enabled(*m, &i.scope))
                })
    }
}
/// Pre-approval copies reveal only the invoker's own supplied description.
pub fn private_preview(i: &Introduction, member: u64) -> Result<String, WorkError> {
    let index = i
        .members
        .iter()
        .position(|m| *m == member)
        .ok_or(WorkError::Denied)?;
    Ok(format!(
        "Introduction {} revision {}. Public destination channel {}.\nYour exact description:\n{}\nYour approval: {}. State: {:?}.\nApproval publishes your description together with another member’s separately approved description. Edits clear both approvals. No other participant’s identity or description is shown here.",
        i.id,
        i.revision,
        i.destination,
        i.approved_self_descriptions[index]
            .as_deref()
            .unwrap_or("No description supplied; add your own with `/engage introduction`."),
        i.approvals[index] == Some(i.revision),
        i.state
    ))
}
pub fn publication(i: &Introduction) -> Result<String, WorkError> {
    if i.state != IntroductionState::Consumed || i.approvals != [Some(i.revision); 2] {
        return Err(WorkError::Denied);
    }
    let [Some(a), Some(b)] = &i.approved_self_descriptions else {
        return Err(WorkError::Denied);
    };
    if !description_valid(a) || !description_valid(b) {
        return Err(WorkError::Invalid);
    }
    Ok(format!(
        "Two members have approved sharing these introductions here:\n\n{a}\n\n{b}\n\nYou’re welcome to connect around what you’ve shared."
    ))
}
#[cfg(test)]
mod tests;
impl EngagementStore {
    pub(super) fn reserve_introduction(
        &mut self,
        id: u64,
        revision: u64,
        now: u64,
    ) -> Result<lifecycle::EngagementReservation, WorkError> {
        let c = self.candidates.get(&id).ok_or(WorkError::Missing)?;
        if !self.introduction_receipt_current(c) || c.revision != revision {
            return Err(WorkError::Denied);
        }
        let mut i = self.introductions[&c.introduction_id.ok_or(WorkError::Invalid)?].clone();
        let mut charges = Vec::new();
        let mut revisions = [0; 2];
        if self.charges.len() + 2 > 20_000 {
            return Err(WorkError::Full);
        }
        for (index, member) in i.members.iter().enumerate() {
            let b = self.allowed(c, *member, now)?;
            self.capacity(*member, now, &b)?;
            revisions[index] = self.member_policies[member].revision;
            charges.push(ContactCharge {
                candidate_id: id,
                member: *member,
                local_day: b.0,
                local_week: b.1,
                at: now,
            });
        }
        i.state = IntroductionState::Consumed;
        let r = lifecycle::EngagementReservation {
            candidate_id: id,
            revision,
            policy_revision: c.policy_revision,
            scope: c.scope.clone(),
            member: None,
            destination: DestinationPreference::Origin,
            introduction: Some(lifecycle::IntroductionReservation {
                introduction: i.clone(),
                policy_revisions: revisions,
            }),
        };
        self.charges.extend(charges);
        self.introductions.insert(i.id, i);
        self.candidates
            .get_mut(&id)
            .ok_or(WorkError::Missing)?
            .state = CandidateState::Reserved;
        Ok(r)
    }
    pub(super) fn validate_introduction_reservation(
        &self,
        r: &lifecycle::EngagementReservation,
        now: u64,
    ) -> Result<(), WorkError> {
        let snapshot = r.introduction.as_ref().ok_or(WorkError::Denied)?;
        let c = self
            .candidates
            .get(&r.candidate_id)
            .ok_or(WorkError::Missing)?;
        if !self.introduction_receipt_current(c)
            || self.introductions.get(&snapshot.introduction.id) != Some(&snapshot.introduction)
        {
            return Err(WorkError::Stale);
        }
        for (index, member) in snapshot.introduction.members.iter().enumerate() {
            if self
                .member_policies
                .get(member)
                .is_none_or(|p| p.revision != snapshot.policy_revisions[index])
            {
                return Err(WorkError::Stale);
            }
            self.allowed(c, *member, now)?;
        }
        Ok(())
    }
}
