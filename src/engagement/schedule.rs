//! Pure candidate scheduling; transport access is independently checked by callers.
use super::*;
use crate::calendar::{local_hour, utc};
use chrono::{Datelike, Duration};
use chrono_tz::Tz;
#[derive(Debug, Clone)]
pub struct CandidateProposal {
    pub kind: EngagementKind,
    pub source: Option<SourceRef>,
    pub member: Option<u64>,
    pub scope: EngagementScope,
    pub due_at: u64,
    pub introduction_id: Option<u64>,
}
impl Candidate {
    pub(super) fn weekly_deadline(&self) -> Result<Option<u64>, WorkError> {
        if !matches!(
            self.kind,
            EngagementKind::WeeklyCheckIn
                | EngagementKind::Welcome
                | EngagementKind::ProjectCheckIn
        ) {
            return Ok(None);
        }
        let deadline = self.due_at.checked_add(3600).ok_or(WorkError::Invalid)?;
        utc(deadline)?;
        Ok(Some(deadline))
    }
    pub(super) fn weekly_window(&self, now: u64) -> Result<bool, WorkError> {
        Ok(self
            .weekly_deadline()?
            .is_none_or(|end| self.due_at <= now && now < end))
    }
}
impl EngagementStore {
    pub fn propose(&mut self, p: CandidateProposal, now: u64) -> Result<Option<u64>, WorkError> {
        if self
            .erased_identities
            .contains(&erasure_identity::proposal(&p))
        {
            return Err(WorkError::Stale);
        }
        utc(now)?;
        utc(p.due_at)?;
        if let Some(source) = &p.source {
            utc(source.at)?;
        }
        p.scope.validate()?;
        if self.candidates.values().any(|c| {
            c.kind == p.kind
                && c.member == p.member
                && c.scope == p.scope
                && match (&c.source, &p.source) {
                    (Some(a), Some(b)) => {
                        a.message == b.message
                            && (p.kind != EngagementKind::WeeklyCheckIn || c.due_at == p.due_at)
                    }
                    (None, None) => {
                        c.introduction_id == p.introduction_id
                            && (p.introduction_id.is_some() || c.due_at == p.due_at)
                    }
                    _ => false,
                }
        }) {
            return Ok(None);
        }
        if self.candidates.len() + self.introductions.len() >= 10_000
            || self
                .candidates
                .values()
                .filter(|c| matches!(c.state, CandidateState::Pending | CandidateState::Reserved))
                .count()
                >= 1000
        {
            return Err(WorkError::Full);
        }
        if let Some(member) = p.member
            && p.kind != EngagementKind::ProjectCheckIn
        {
            let eligible = self.eligibility.get(&member).ok_or(WorkError::Denied)?;
            if let Some(source) = &p.source {
                if !eligible.contains(source) {
                    return Err(WorkError::Denied);
                }
            } else if p.kind == EngagementKind::WeeklyCheckIn
                && self
                    .member_policies
                    .get(&member)
                    .and_then(|p| p.weekly_subscription.as_ref())
                    .is_none_or(|w| w.scope != p.scope)
            {
                return Err(WorkError::Denied);
            }
        }
        let id = self.sequence.checked_add(1).ok_or(WorkError::Full)?;
        let policy = p.member.and_then(|m| self.member_policies.get(&m));
        let candidate = Candidate {
            id,
            kind: p.kind,
            source: p.source,
            member: p.member,
            scope: p.scope.clone(),
            due_at: p.due_at,
            revision: 1,
            state: CandidateState::Pending,
            dedupe_key: format!("engagement:{id}"),
            policy_revision: policy.map_or(0, |p| p.revision),
            destination: if super::community::feature(p.kind).is_some() {
                DestinationPreference::Origin
            } else {
                policy
                    .and_then(|policy| policy.destinations.get(&p.scope))
                    .copied()
                    .unwrap_or(DestinationPreference::Origin)
            },
            message_id: None,
            introduction_id: p.introduction_id,
            work_ref: None,
            expires_at: None,
            follow_up_reason: None,
        };
        candidate.weekly_deadline()?;
        // Validate the complete prospective record without partially inserting it.
        let mut next = self.clone();
        next.sequence = id;
        next.candidates.insert(id, candidate);
        next.validate()?;
        *self = next;
        Ok(Some(id))
    }
    pub fn due(&self, now: u64) -> Vec<u64> {
        if utc(now).is_err() {
            return Vec::new();
        }
        let mut rows: Vec<_> = self
            .candidates
            .values()
            .filter(|c| {
                c.state == CandidateState::Pending
                    && c.due_at <= now
                    && c.weekly_window(now).unwrap_or(false)
            })
            .collect();
        rows.sort_by_key(|c| (c.due_at, c.id));
        rows.into_iter().map(|c| c.id).collect()
    }
    /// Owned scheduler transactions retire missed weekly occurrences permanently.
    /// Preflight every deadline before mutation; attempted charges and dedupe survive.
    pub fn expire_missed_weekly(&mut self, now: u64) -> Result<usize, WorkError> {
        utc(now)?;
        let mut expired = Vec::new();
        for c in self.candidates.values() {
            if matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
                && c.weekly_deadline()?.is_some_and(|end| now >= end)
            {
                expired.push(c.id);
            }
        }
        for id in &expired {
            self.candidates.get_mut(id).ok_or(WorkError::Missing)?.state =
                CandidateState::Cancelled;
        }
        Ok(expired.len())
    }
    /// Return the next future occurrence; missed weeks never become backlog.
    pub fn next_weekly(&self, member: u64, now: u64) -> Result<Option<u64>, WorkError> {
        let policy = self
            .member_policies
            .get(&member)
            .ok_or(WorkError::Missing)?;
        policy.validate()?;
        let Some(w) = &policy.weekly_subscription else {
            return Ok(None);
        };
        let tz = policy
            .timezone
            .as_ref()
            .ok_or(WorkError::Denied)?
            .parse::<Tz>()
            .map_err(|_| WorkError::Invalid)?;
        let at = utc(now)?;
        let day = at.with_timezone(&tz).date_naive();
        for offset in 0..=7 {
            let d = day
                .checked_add_signed(Duration::days(offset))
                .ok_or(WorkError::Invalid)?;
            if d.weekday().num_days_from_monday() == u32::from(w.weekday) {
                let occurrence = local_hour(tz, d, w.hour)?;
                if occurrence > at {
                    return Ok(Some(
                        u64::try_from(occurrence.timestamp()).map_err(|_| WorkError::Invalid)?,
                    ));
                }
            }
        }
        Err(WorkError::Invalid)
    }
}
#[cfg(test)]
mod tests;
