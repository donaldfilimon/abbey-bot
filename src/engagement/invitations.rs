//! Explicit command receipts prove request context without inventing a message source.
use super::*;
impl EngagementStore {
    pub fn validate_invitation_requests(&self) -> Result<(), WorkError> {
        if self.invitation_requests.len() + self.suppressed_invitation_requests.len() > 10_000
            || self.candidates.len()
                + self.introductions.len()
                + self.suppressed_invitation_requests.len()
                > 10_000
        {
            return Err(WorkError::Full);
        }
        let mut seen = BTreeSet::new();
        for (id, r) in &self.invitation_requests {
            let c = self.candidates.get(id).ok_or(WorkError::Invalid)?;
            if r.interaction == 0
                || r.member == 0
                || !seen.insert(r.interaction)
                || c.source.is_some()
                || c.member != Some(r.member)
                || c.scope != r.scope
                || c.due_at != r.at
                || !invitation_kind(c.kind)
                || (c.kind == EngagementKind::ActivityInvite) != r.activity.is_some()
                || r.activity.as_ref().is_some_and(|v| {
                    v.digest.len() != 64
                        || !v.digest.bytes().all(|b| b.is_ascii_hexdigit())
                        || v.origin.is_empty()
                })
                || crate::calendar::utc(r.at).is_err()
            {
                return Err(WorkError::Invalid);
            }
        }
        for (interaction, suppressed) in &self.suppressed_invitation_requests {
            let request = &suppressed.request;
            let blocker = self
                .candidates
                .get(&suppressed.blocking_candidate)
                .ok_or(WorkError::Invalid)?;
            request.scope.validate()?;
            if *interaction != request.interaction
                || *interaction == 0
                || !seen.insert(*interaction)
                || request.member == 0
                || blocker.member != Some(request.member)
                || blocker.scope != request.scope
                || !invitation_kind(blocker.kind)
                || !invitation_kind(suppressed.kind)
                || (suppressed.kind == EngagementKind::ActivityInvite) != request.activity.is_some()
                || crate::calendar::utc(request.at).is_err()
                || matches!(request.scope,EngagementScope::Dm{member,..} if member != request.member)
            {
                return Err(WorkError::Invalid);
            }
        }
        for c in self.candidates.values() {
            if invitation_kind(c.kind)
                && c.source.is_none()
                && !self.invitation_requests.contains_key(&c.id)
            {
                return Err(WorkError::Invalid);
            }
        }
        Ok(())
    }
    pub fn request_invitation(
        &mut self,
        kind: EngagementKind,
        request: InvitationRequest,
    ) -> Result<Option<u64>, WorkError> {
        if !invitation_kind(kind) || request.interaction == 0 {
            return Err(WorkError::Invalid);
        }
        request.scope.validate()?;
        crate::calendar::utc(request.at)?;
        let policy = self
            .member_policies
            .get(&request.member)
            .ok_or(WorkError::Denied)?;
        if !policy.personalized_enabled()
            || policy.stopped_scopes.contains(&request.scope)
            || matches!(request.scope,EngagementScope::Guild{guild,..} if policy.stopped_guilds.contains(&guild))
            || matches!(request.scope,EngagementScope::Dm{member,..} if member != request.member)
            || !self.invitation_eligible(request.member)
        {
            return Err(WorkError::Denied);
        }
        if self
            .invitation_requests
            .values()
            .any(|r| r.interaction == request.interaction)
            || self
                .suppressed_invitation_requests
                .contains_key(&request.interaction)
        {
            return Ok(None);
        }
        if let Some(blocker) = self.candidates.values().find(|c| {
            c.member == Some(request.member)
                && c.scope == request.scope
                && invitation_kind(c.kind)
                && matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
        }) {
            let mut next = self.clone();
            next.suppressed_invitation_requests.insert(
                request.interaction,
                SuppressedInvitationRequest {
                    blocking_candidate: blocker.id,
                    kind,
                    request,
                },
            );
            next.validate()?;
            *self = next;
            return Ok(None);
        }
        if self.candidates.len()
            + self.introductions.len()
            + self.suppressed_invitation_requests.len()
            >= 10_000
            || self
                .candidates
                .values()
                .filter(|c| matches!(c.state, CandidateState::Pending | CandidateState::Reserved))
                .count()
                >= 1000
        {
            return Err(WorkError::Full);
        }
        let id = self.sequence.checked_add(1).ok_or(WorkError::Full)?;
        let c = Candidate {
            id,
            kind,
            source: None,
            member: Some(request.member),
            scope: request.scope.clone(),
            due_at: request.at,
            revision: 1,
            state: CandidateState::Pending,
            dedupe_key: format!("invitation:{}", request.interaction),
            policy_revision: policy.revision,
            destination: policy
                .destinations
                .get(&request.scope)
                .copied()
                .unwrap_or(DestinationPreference::Origin),
            message_id: None,
            introduction_id: None,
        };
        let mut next = self.clone();
        next.sequence = id;
        next.candidates.insert(id, c);
        next.invitation_requests.insert(id, request);
        next.validate()?;
        *self = next;
        Ok(Some(id))
    }
    pub(super) fn invitation_eligible(&self, member: u64) -> bool {
        self.eligibility
            .get(&member)
            .is_some_and(|sources| !sources.is_empty())
            || self
                .member_policies
                .get(&member)
                .is_some_and(|p| p.weekly_subscription.is_some())
    }
    pub fn invitation_receipt_current(&self, c: &Candidate) -> bool {
        invitation_kind(c.kind)
            && c.source.is_none()
            && self.invitation_requests.get(&c.id).is_some_and(|r| {
                r.member != 0
                    && c.member == Some(r.member)
                    && c.scope == r.scope
                    && c.due_at == r.at
                    && r.interaction != 0
                    && (c.kind == EngagementKind::ActivityInvite) == r.activity.is_some()
            })
    }
}
pub fn invitation_kind(kind: EngagementKind) -> bool {
    matches!(
        kind,
        EngagementKind::ActivityInvite | EngagementKind::VoiceInvite
    )
}
pub fn voice_invitation() -> &'static str {
    "Would you like to talk with Abbey in Discord voice? In the server where you want to talk, review `/voice consent` and `/voice status`. A manager in the voice channel uses `/voice join consent:true` or `/voice resume consent:true` after everyone present has agreed. This invitation saves no voice agreement and starts no audio processing. You can withdraw with `/voice consent` or stop the call with `/voice leave`."
}
