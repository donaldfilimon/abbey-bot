//! Attempted capacity is durable and never refunded. Call inside the owned commit.
use super::*;
use crate::calendar::utc;
use crate::work::follow_up::FollowUpDecision;
use chrono::{Datelike, Duration, Timelike};
use chrono_tz::Tz;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntroductionReservation {
    pub introduction: Introduction,
    pub policy_revisions: [u64; 2],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngagementReservation {
    pub introduction: Option<IntroductionReservation>,
    pub candidate_id: u64,
    pub revision: u64,
    pub policy_revision: u64,
    pub scope: EngagementScope,
    pub member: Option<u64>,
    pub destination: DestinationPreference,
}
#[derive(Debug, Clone, Copy)]
pub enum DeliveryOutcome {
    Sent { message_id: u64 },
    Rejected,
    Cancelled,
    ReviewRequired,
}
fn buckets(at: u64, tz: Tz) -> Result<(String, String), WorkError> {
    let local = utc(at)?.with_timezone(&tz);
    let day = local.date_naive();
    let monday = day
        .checked_sub_signed(Duration::days(i64::from(
            day.weekday().num_days_from_monday(),
        )))
        .ok_or(WorkError::Invalid)?;
    Ok((
        day.format("%Y-%m-%d").to_string(),
        monday.format("%Y-%m-%d").to_string(),
    ))
}
impl EngagementStore {
    pub(super) fn allowed(
        &self,
        c: &Candidate,
        member: u64,
        now: u64,
    ) -> Result<(String, String), WorkError> {
        self.allowed_with_reason(c, member, now)
            .map_err(|(error, _)| error)
    }
    // One eligibility calculation owns both the existing error contract and
    // the closed private explanations used by optional task follow-ups.
    fn allowed_with_reason(
        &self,
        c: &Candidate,
        member: u64,
        now: u64,
    ) -> Result<(String, String), (WorkError, FollowUpDecision)> {
        let p = self
            .member_policies
            .get(&member)
            .ok_or((WorkError::Denied, FollowUpDecision::Disabled))?;
        p.validate()
            .map_err(|error| (error, FollowUpDecision::Disabled))?;
        let guild_stopped =
            matches!(c.scope,EngagementScope::Guild{guild,..} if p.stopped_guilds.contains(&guild));
        if p.global_stop || guild_stopped || p.stopped_scopes.contains(&c.scope) {
            return Err((WorkError::Denied, FollowUpDecision::OptedOut));
        }
        if !p.personalized_enabled() {
            return Err((WorkError::Denied, FollowUpDecision::Disabled));
        }
        if p.snoozed_until.is_some_and(|t| t > now) {
            return Err((WorkError::Denied, FollowUpDecision::Quiet));
        }
        let tz = p
            .timezone
            .as_ref()
            .ok_or((WorkError::Denied, FollowUpDecision::Disabled))?
            .parse::<Tz>()
            .map_err(|_| (WorkError::Invalid, FollowUpDecision::Disabled))?;
        let hour = utc(now)
            .map_err(|error| (error, FollowUpDecision::Disabled))?
            .with_timezone(&tz)
            .hour();
        let start = u32::from(p.quiet_start);
        let end = u32::from(p.quiet_end);
        if (start < end && hour >= start && hour < end)
            || (start > end && (hour >= start || hour < end))
        {
            return Err((WorkError::Denied, FollowUpDecision::Quiet));
        }
        if c.kind == EngagementKind::WeeklyCheckIn
            && p.weekly_subscription
                .as_ref()
                .is_none_or(|w| w.scope != c.scope)
        {
            return Err((WorkError::Denied, FollowUpDecision::OptedOut));
        }
        buckets(now, tz).map_err(|error| (error, FollowUpDecision::Disabled))
    }
    fn task_follow_up_member_allowed(
        &self,
        c: &Candidate,
        now: u64,
    ) -> Result<(String, String), (WorkError, FollowUpDecision)> {
        if c.kind != EngagementKind::FollowUp || c.work_ref.is_none() {
            return Err((WorkError::Invalid, FollowUpDecision::StaleTask));
        }
        let member = c
            .member
            .filter(|member| *member != 0)
            .ok_or((WorkError::Invalid, FollowUpDecision::AccessDenied))?;
        c.scope
            .validate()
            .map_err(|error| (error, FollowUpDecision::AccessDenied))?;
        if matches!(c.scope, EngagementScope::Dm { member: owner, .. } if owner != member) {
            return Err((WorkError::Denied, FollowUpDecision::AccessDenied));
        }
        let b = self.allowed_with_reason(c, member, now)?;
        // Task requests require a positive saved choice for this exact origin.
        // Existing conversation candidates retain their original fallback.
        if self.member_policies[&member]
            .destinations
            .get(&c.scope)
            .copied()
            != Some(c.destination)
        {
            return Err((WorkError::Denied, FollowUpDecision::OptedOut));
        }
        Ok(b)
    }
    /// Member policy only: Work task/access/source authority remains with WorkStore.
    pub(crate) fn task_follow_up_member_decision(
        &self,
        c: &Candidate,
        now: u64,
    ) -> Result<FollowUpDecision, WorkError> {
        let b = match self.task_follow_up_member_allowed(c, now) {
            Ok(b) => b,
            Err((WorkError::Denied, reason)) => return Ok(reason),
            Err((error, _)) => return Err(error),
        };
        let member = c.member.ok_or(WorkError::Invalid)?;
        match self.capacity(member, now, &b) {
            Ok(()) => Ok(FollowUpDecision::Allowed),
            Err(WorkError::Denied) => Ok(FollowUpDecision::Budget),
            Err(error) => Err(error),
        }
    }
    pub(super) fn capacity(
        &self,
        member: u64,
        now: u64,
        b: &(String, String),
    ) -> Result<(), WorkError> {
        if now < self.safety_pruned_through {
            return Err(WorkError::Denied);
        }
        let p = self.member_policies.get(&member).ok_or(WorkError::Denied)?;
        let tz = p
            .timezone
            .as_ref()
            .ok_or(WorkError::Denied)?
            .parse::<Tz>()
            .map_err(|_| WorkError::Invalid)?;
        let rows: Vec<_> = self
            .charges
            .iter()
            .filter(|c| c.member == member)
            .map(|c| (&c.local_day, &c.local_week, c.at))
            .chain(
                self.erased_contact_charges
                    .iter()
                    .filter(|c| c.member == member)
                    .map(|c| (&c.local_day, &c.local_week, c.at)),
            )
            .collect();
        let saved_day = rows.iter().filter(|c| *c.0 == b.0).count();
        let saved_week = rows.iter().filter(|c| *c.1 == b.1).count();
        let mut day = 0;
        let mut week = 0;
        for c in rows {
            let current = buckets(c.2, tz)?;
            day += usize::from(current.0 == b.0);
            week += usize::from(current.1 == b.1);
        }
        utc(now)?;
        if saved_day.max(day) >= usize::from(p.daily_limit.ok_or(WorkError::Denied)?)
            || p.weekly_limit
                .is_some_and(|limit| saved_week.max(week) >= usize::from(limit))
        {
            return Err(WorkError::Denied);
        }
        Ok(())
    }
    pub fn reserve(
        &mut self,
        id: u64,
        revision: u64,
        now: u64,
    ) -> Result<EngagementReservation, WorkError> {
        utc(now)?;
        let c = self.candidates.get(&id).ok_or(WorkError::Missing)?;
        if c.revision != revision
            || c.state != CandidateState::Pending
            || c.due_at > now
            || !c.weekly_window(now)?
            || !c.task_follow_up_window(now)?
        {
            return Err(WorkError::Stale);
        }
        if c.kind == EngagementKind::Introduction {
            return self.reserve_introduction(id, revision, now);
        }
        if super::community::feature(c.kind).is_some() {
            if !self.community_receipt_current(c) || !self.community_capacity(c, now) {
                return Err(WorkError::Denied);
            }
            if c.member.is_none() {
                let r = EngagementReservation {
                    introduction: None,
                    candidate_id: id,
                    revision,
                    policy_revision: self.community_receipts[&id].policy_revision,
                    scope: c.scope.clone(),
                    member: None,
                    destination: DestinationPreference::Origin,
                };
                self.community_receipts
                    .get_mut(&id)
                    .ok_or(WorkError::Missing)?
                    .attempted_at = Some(now);
                let c = self.candidates.get_mut(&id).ok_or(WorkError::Missing)?;
                c.state = CandidateState::Reserved;
                c.policy_revision = r.policy_revision;
                c.destination = r.destination;
                return Ok(r);
            }
        }
        let member = c.member.ok_or(WorkError::Denied)?;
        if super::invitations::invitation_kind(c.kind)
            && c.source.is_none()
            && !self.invitation_receipt_current(c)
        {
            return Err(WorkError::Denied);
        }
        let eligible = if c.kind == EngagementKind::ProjectCheckIn {
            self.community_receipt_current(c)
        } else if super::invitations::invitation_kind(c.kind) && c.source.is_none() {
            self.invitation_eligible(member)
        } else {
            self.eligibility.get(&member).is_some_and(|sources| {
                c.source
                    .as_ref()
                    .is_none_or(|source| sources.contains(source))
            })
        };
        if !eligible {
            return Err(WorkError::Denied);
        }
        let b = if c.work_ref.is_some() {
            self.task_follow_up_member_allowed(c, now)
                .map_err(|(error, _)| error)?
        } else {
            self.allowed(c, member, now)?
        };
        self.capacity(member, now, &b)?;
        if self.charges.len() + self.erased_contact_charges.len() >= 20_000 {
            return Err(WorkError::Full);
        }
        let p = &self.member_policies[&member];
        let destination = if c.work_ref.is_some() {
            // Consent was checked against this stored destination before the
            // existing reserve path could overwrite it from current policy.
            c.destination
        } else if c.kind == EngagementKind::ProjectCheckIn {
            DestinationPreference::Origin
        } else if c.kind == EngagementKind::WeeklyCheckIn {
            p.weekly_subscription
                .as_ref()
                .ok_or(WorkError::Denied)?
                .destination
        } else {
            p.destinations
                .get(&c.scope)
                .copied()
                .unwrap_or(DestinationPreference::Origin)
        };
        let r = EngagementReservation {
            introduction: None,
            candidate_id: id,
            revision,
            policy_revision: p.revision,
            scope: c.scope.clone(),
            member: c.member,
            destination,
        };
        if let Some(receipt) = self.community_receipts.get_mut(&id) {
            receipt.attempted_at = Some(now);
        }
        self.charges.push(ContactCharge {
            candidate_id: id,
            member,
            local_day: b.0,
            local_week: b.1,
            at: now,
        });
        let c = self.candidates.get_mut(&id).ok_or(WorkError::Missing)?;
        c.state = CandidateState::Reserved;
        c.policy_revision = r.policy_revision;
        c.destination = destination;
        Ok(r)
    }
    pub fn validate_reserved(&self, r: &EngagementReservation, now: u64) -> Result<(), WorkError> {
        let c = self
            .candidates
            .get(&r.candidate_id)
            .ok_or(WorkError::Missing)?;
        if c.state != CandidateState::Reserved
            || c.revision != r.revision
            || c.policy_revision != r.policy_revision
            || c.scope != r.scope
            || c.member != r.member
            || c.destination != r.destination
            || !c.weekly_window(now)?
            || !c.task_follow_up_window(now)?
        {
            return Err(WorkError::Stale);
        }
        if c.kind == EngagementKind::Introduction {
            return self.validate_introduction_reservation(r, now);
        }
        if r.introduction.is_some() {
            return Err(WorkError::Invalid);
        }
        if super::community::feature(c.kind).is_some() {
            if !self.community_receipt_current(c) {
                return Err(WorkError::Stale);
            }
            if r.member.is_none() {
                return Ok(());
            }
        }
        let member = r.member.ok_or(WorkError::Denied)?;
        if super::invitations::invitation_kind(c.kind)
            && c.source.is_none()
            && !self.invitation_receipt_current(c)
        {
            return Err(WorkError::Denied);
        }
        if self
            .member_policies
            .get(&member)
            .is_none_or(|p| p.revision != r.policy_revision)
        {
            return Err(WorkError::Stale);
        }
        if c.work_ref.is_some() {
            if now < self.safety_pruned_through {
                return Err(WorkError::Denied);
            }
            self.task_follow_up_member_allowed(c, now)
                .map_err(|(error, _)| error)?;
            // Its own durable charge may have exhausted the budget. Validate
            // consent again, but never admit or charge this reservation twice.
        } else {
            self.allowed(c, member, now)?;
        }
        let eligible = if c.kind == EngagementKind::ProjectCheckIn {
            self.community_receipt_current(c)
        } else if super::invitations::invitation_kind(c.kind) && c.source.is_none() {
            self.invitation_eligible(member)
        } else {
            self.eligibility.get(&member).is_some_and(|sources| {
                c.source
                    .as_ref()
                    .is_none_or(|source| sources.contains(source))
            })
        };
        if !eligible {
            return Err(WorkError::Stale);
        }
        Ok(())
    }
    pub fn settle(&mut self, id: u64, outcome: DeliveryOutcome) -> Result<(), WorkError> {
        let charged = self.charges.iter().any(|charge| charge.candidate_id == id)
            || self.community_receipts.contains_key(&id);
        let c = self.candidates.get_mut(&id).ok_or(WorkError::Missing)?;
        let reconcile = c.state == CandidateState::Cancelled
            && charged
            && matches!(
                outcome,
                DeliveryOutcome::Sent { .. } | DeliveryOutcome::ReviewRequired
            );
        if c.state != CandidateState::Reserved && !reconcile {
            return Err(WorkError::Stale);
        }
        let (state, message) = match outcome {
            DeliveryOutcome::Sent { message_id: 0 } => return Err(WorkError::Invalid),
            DeliveryOutcome::Sent { message_id } => (CandidateState::Sent, Some(message_id)),
            DeliveryOutcome::Rejected => (CandidateState::Rejected, None),
            DeliveryOutcome::Cancelled => (CandidateState::Cancelled, None),
            DeliveryOutcome::ReviewRequired => (CandidateState::ReviewRequired, None),
        };
        c.state = state;
        c.message_id = message;
        if c.work_ref.is_some()
            && matches!(state, CandidateState::Sent | CandidateState::ReviewRequired)
        {
            // Observed send uncertainty supersedes an earlier cancellation
            // explanation; only closed cancelled/rejected rows retain one.
            c.follow_up_reason = None;
        }
        if let Some(id) = c.introduction_id
            && matches!(state, CandidateState::Rejected | CandidateState::Cancelled)
        {
            let i = self.introductions.get_mut(&id).ok_or(WorkError::Missing)?;
            i.state = IntroductionState::Cancelled;
            i.approvals = [None; 2];
        }
        Ok(())
    }
    pub fn cancel_source(&mut self, scope: &EngagementScope, message: u64) -> usize {
        self.cancel_matching(|c| {
            &c.scope == scope && c.source.as_ref().is_some_and(|s| s.message == message)
        })
    }
    pub fn cancel_member_origin(
        &mut self,
        member: u64,
        scope: &EngagementScope,
        after: u64,
    ) -> usize {
        self.cancel_matching(|c| {
            c.member == Some(member)
                && &c.scope == scope
                && c.source.as_ref().is_some_and(|s| s.at < after)
        })
    }
    fn cancel_matching(&mut self, matches: impl Fn(&Candidate) -> bool) -> usize {
        let mut n = 0;
        for c in self.candidates.values_mut() {
            if matches(c) && matches!(c.state, CandidateState::Pending | CandidateState::Reserved) {
                c.state = CandidateState::Cancelled;
                n += 1;
            }
        }
        n
    }
    pub fn recover_reserved(&mut self) -> usize {
        let mut n = 0;
        for c in self.candidates.values_mut() {
            if c.state == CandidateState::Reserved {
                c.state = CandidateState::ReviewRequired;
                if c.work_ref.is_some() {
                    c.follow_up_reason = None;
                }
                n += 1;
            }
        }
        n
    }
}

impl Candidate {
    fn task_follow_up_window(&self, now: u64) -> Result<bool, WorkError> {
        if self.work_ref.is_none() {
            return Ok(true);
        }
        let expires = self.expires_at.ok_or(WorkError::Invalid)?;
        if self.kind != EngagementKind::FollowUp || expires <= self.due_at {
            return Err(WorkError::Invalid);
        }
        utc(expires)?;
        Ok(now < expires)
    }
}
#[cfg(test)]
mod task_follow_up_tests;
