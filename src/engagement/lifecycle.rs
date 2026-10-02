//! Attempted capacity is durable and never refunded. Call inside the owned commit.
use super::*;
use crate::calendar::utc;
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
        let p = self.member_policies.get(&member).ok_or(WorkError::Denied)?;
        p.validate()?;
        let guild_stopped =
            matches!(c.scope,EngagementScope::Guild{guild,..} if p.stopped_guilds.contains(&guild));
        if !p.personalized_enabled()
            || guild_stopped
            || p.stopped_scopes.contains(&c.scope)
            || p.snoozed_until.is_some_and(|t| t > now)
        {
            return Err(WorkError::Denied);
        }
        let tz = p
            .timezone
            .as_ref()
            .ok_or(WorkError::Denied)?
            .parse::<Tz>()
            .map_err(|_| WorkError::Invalid)?;
        let hour = utc(now)?.with_timezone(&tz).hour();
        let start = u32::from(p.quiet_start);
        let end = u32::from(p.quiet_end);
        if (start < end && hour >= start && hour < end)
            || (start > end && (hour >= start || hour < end))
        {
            return Err(WorkError::Denied);
        }
        if c.kind == EngagementKind::WeeklyCheckIn
            && p.weekly_subscription
                .as_ref()
                .is_none_or(|w| w.scope != c.scope)
        {
            return Err(WorkError::Denied);
        }
        buckets(now, tz)
    }
    pub(super) fn capacity(
        &self,
        member: u64,
        now: u64,
        b: &(String, String),
    ) -> Result<(), WorkError> {
        let p = self.member_policies.get(&member).ok_or(WorkError::Denied)?;
        let tz = p
            .timezone
            .as_ref()
            .ok_or(WorkError::Denied)?
            .parse::<Tz>()
            .map_err(|_| WorkError::Invalid)?;
        let rows: Vec<_> = self.charges.iter().filter(|c| c.member == member).collect();
        let saved_day = rows.iter().filter(|c| c.local_day == b.0).count();
        let saved_week = rows.iter().filter(|c| c.local_week == b.1).count();
        let mut day = 0;
        let mut week = 0;
        for c in rows {
            let current = buckets(c.at, tz)?;
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
        let b = self.allowed(c, member, now)?;
        self.capacity(member, now, &b)?;
        if self.charges.len() >= 20_000 {
            return Err(WorkError::Full);
        }
        let p = &self.member_policies[&member];
        let destination = if c.kind == EngagementKind::ProjectCheckIn {
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
        self.allowed(c, member, now)?;
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
                n += 1;
            }
        }
        n
    }
}
