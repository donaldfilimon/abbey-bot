//! Deterministic validation of explicit member settings and bounded records.
use super::*;

impl EngagementScope {
    pub(super) fn validate(&self) -> Result<(), WorkError> {
        let (first, channel) = match self {
            Self::Guild { guild, channel } => (*guild, *channel),
            Self::Dm { member, channel } => (*member, *channel),
        };
        if first == 0 || channel == 0 {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }
    fn matches_member(&self, actor: u64) -> bool {
        !matches!(self,Self::Dm {member,..} if *member != actor)
    }
}
impl From<EngagementScope> for String {
    fn from(scope: EngagementScope) -> Self {
        match scope {
            EngagementScope::Guild { guild, channel } => {
                format!("guild\u{1f}{guild}\u{1f}{channel}")
            }
            EngagementScope::Dm { member, channel } => format!("dm\u{1f}{member}\u{1f}{channel}"),
        }
    }
}
impl TryFrom<String> for EngagementScope {
    type Error = WorkError;
    fn try_from(key: String) -> Result<Self, Self::Error> {
        let parts: Vec<_> = key.split('\u{1f}').collect();
        let [kind, first, channel] = parts.as_slice() else {
            return Err(WorkError::Invalid);
        };
        let first = first.parse().map_err(|_| WorkError::Invalid)?;
        let channel = channel.parse().map_err(|_| WorkError::Invalid)?;
        let scope = match *kind {
            "guild" => Self::Guild {
                guild: first,
                channel,
            },
            "dm" => Self::Dm {
                member: first,
                channel,
            },
            _ => return Err(WorkError::Invalid),
        };
        scope.validate()?;
        if String::from(scope.clone()) != key {
            return Err(WorkError::Invalid);
        }
        Ok(scope)
    }
}
impl MemberPolicy {
    pub fn personalized_enabled(&self) -> bool {
        self.daily_limit.is_some_and(|v| (1..=4).contains(&v))
            && self
                .timezone
                .as_ref()
                .is_some_and(|v| v.parse::<chrono_tz::Tz>().is_ok())
            && !self.global_stop
    }
    pub fn validate(&self) -> Result<(), WorkError> {
        if self.stopped_guilds.contains(&0)
            || self.daily_limit.is_some_and(|v| !(1..=4).contains(&v))
            || self.weekly_limit.is_some_and(|v| {
                !(1..=28).contains(&v)
                    || self
                        .daily_limit
                        .is_none_or(|d| u16::from(v) > u16::from(d) * 7)
            })
            || self
                .timezone
                .as_ref()
                .is_some_and(|v| v.parse::<chrono_tz::Tz>().is_err())
            || self.quiet_start > 23
            || self.quiet_end > 23
        {
            return Err(WorkError::Invalid);
        }
        for scope in self.stopped_scopes.iter().chain(self.destinations.keys()) {
            scope.validate()?;
        }
        if let Some(weekly) = &self.weekly_subscription {
            weekly.scope.validate()?;
            if weekly.weekday > 6 || weekly.hour > 23 {
                return Err(WorkError::Invalid);
            }
        }
        Ok(())
    }
}
impl SourceRef {
    pub(crate) fn validate(&self) -> Result<(), WorkError> {
        self.scope.validate()?;
        if self.message == 0
            || self.author == 0
            || self.revision == 0
            || !self.scope.matches_member(self.author)
        {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }
}
impl EngagementStore {
    pub fn validate(&self) -> Result<(), WorkError> {
        self.validate_erased_charges()?;
        self.validate_invitation_requests()?;
        self.validate_community()?;
        for (id, members) in &self.feedback {
            let c = self.candidates.get(id).ok_or(WorkError::Invalid)?;
            if *id == 0
                || c.state != CandidateState::Sent
                || members.is_empty()
                || members.len() > 2
            {
                return Err(WorkError::Invalid);
            }
            for (actor, feedback) in members {
                let permitted = c.member == Some(*actor)
                    || c.introduction_id
                        .and_then(|i| self.introductions.get(&i))
                        .is_some_and(|i| i.members.contains(actor));
                if *actor == 0 || feedback.actor != *actor || !permitted {
                    return Err(WorkError::Invalid);
                }
            }
        }
        if self.feedback.len() > 10_000 {
            return Err(WorkError::Invalid);
        }
        if self.candidates.len() + self.introductions.len() > 10_000
            || self
                .candidates
                .values()
                .filter(|c| matches!(c.state, CandidateState::Pending | CandidateState::Reserved))
                .count()
                > 1000
            || self.member_policies.len() > 10_000
            || self.eligibility.len() > 10_000
            || self.eligibility.values().map(BTreeSet::len).sum::<usize>() > 10_000
            || self.guild_features.len() > 10_000
            || self.charges.len() + self.erased_contact_charges.len() > 20_000
        {
            return Err(WorkError::Full);
        }
        if self.weekly_assessments.len() > 10_000
            || self
                .weekly_assessments
                .iter()
                .any(|(m, t)| *m == 0 || crate::calendar::utc(*t).is_err())
        {
            return Err(WorkError::Invalid);
        }
        if self.responses.len() > 10_000 || self.responses.iter().any(|(a, b)| *a == 0 || *b == 0) {
            return Err(WorkError::Invalid);
        }
        if self.observations.len() > 10_000
            || self.observations.values().map(BTreeMap::len).sum::<usize>() > 10_000
        {
            return Err(WorkError::Full);
        }
        for (scope, rows) in &self.observations {
            scope.validate()?;
            for (member, source) in rows {
                source.validate()?;
                if source.scope != *scope || source.author != *member {
                    return Err(WorkError::Invalid);
                }
            }
        }
        for (member, policy) in &self.member_policies {
            if *member == 0 {
                return Err(WorkError::Invalid);
            }
            policy.validate()?;
            if policy
                .stopped_scopes
                .iter()
                .chain(policy.destinations.keys())
                .chain(policy.weekly_subscription.iter().map(|w| &w.scope))
                .any(|s| !s.matches_member(*member))
            {
                return Err(WorkError::Invalid);
            }
        }
        for (member, sources) in &self.eligibility {
            if *member == 0 {
                return Err(WorkError::Invalid);
            }
            for source in sources {
                source.validate()?;
                if source.author != *member {
                    return Err(WorkError::Invalid);
                }
            }
        }
        for (guild, policy) in &self.guild_features {
            if *guild == 0
                || policy
                    .channels
                    .values()
                    .any(|channels| channels.contains(&0))
                || policy
                    .enabled
                    .iter()
                    .any(|f| policy.channels.get(f).is_none_or(BTreeSet::is_empty))
            {
                return Err(WorkError::Invalid);
            }
        }
        let mut keys = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for (id, c) in &self.candidates {
            c.scope.validate()?;
            c.validate_task_follow_up()?;
            if *id == 0
                || c.id != *id
                || *id > self.sequence
                || c.revision == 0
                || c.dedupe_key.is_empty()
                || c.dedupe_key.len() > 256
                || !keys.insert(&c.dedupe_key)
                || c.member
                    .is_some_and(|m| m == 0 || !c.scope.matches_member(m))
                || (c.state == CandidateState::Sent) != c.message_id.is_some()
                || c.message_id == Some(0)
                || matches!(c.scope, EngagementScope::Dm {member,..} if c.member != Some(member))
                || matches!(
                    c.kind,
                    EngagementKind::FollowUp
                        | EngagementKind::WeeklyCheckIn
                        | EngagementKind::ActivityInvite
                        | EngagementKind::VoiceInvite
                ) && c.member.is_none()
                || matches!(
                    c.kind,
                    EngagementKind::FollowUp | EngagementKind::UnansweredQuestion
                ) && c.source.is_none()
                || c.kind == EngagementKind::Introduction
                    && (c.member.is_some()
                        || c.source.is_some()
                        || c.destination != DestinationPreference::Origin)
            {
                return Err(WorkError::Invalid);
            }
            if let Some(source) = &c.source {
                source.validate()?;
                if source.scope != c.scope
                    || c.member.is_some_and(|m| m != source.author)
                    || !identities.insert((
                        source.scope.clone(),
                        source.message,
                        c.member,
                        c.kind,
                        if c.kind == EngagementKind::WeeklyCheckIn {
                            c.due_at
                        } else {
                            0
                        },
                    ))
                {
                    return Err(WorkError::Invalid);
                }
            }
            if let Some(intro) = c.introduction_id {
                if c.kind != EngagementKind::Introduction
                    || self
                        .introductions
                        .get(&intro)
                        .is_none_or(|i| i.scope != c.scope || i.revision != c.revision)
                {
                    return Err(WorkError::Invalid);
                }
            } else if c.kind == EngagementKind::Introduction {
                return Err(WorkError::Invalid);
            }
        }
        let mut introduction_candidates = BTreeSet::new();
        for c in self
            .candidates
            .values()
            .filter(|c| c.introduction_id.is_some())
        {
            if !introduction_candidates.insert(c.introduction_id) {
                return Err(WorkError::Invalid);
            }
        }
        for (id, i) in &self.introductions {
            i.scope.validate()?;
            if *id == 0
                || i.id != *id
                || *id > self.sequence
                || i.revision == 0
                || i.destination == 0
                || !matches!(i.scope, EngagementScope::Guild{channel,..} if channel == i.destination)
                || i.members.contains(&0)
                || i.members[0] == i.members[1]
                || !matches!(i.scope, EngagementScope::Guild { .. })
                || i.approved_self_descriptions
                    .iter()
                    .flatten()
                    .any(|s| !super::introductions::description_valid(s))
                || i.approvals.iter().flatten().any(|r| *r != i.revision)
                || matches!(
                    i.state,
                    IntroductionState::Ready | IntroductionState::Consumed
                ) && (i.approvals != [Some(i.revision); 2]
                    || i.approved_self_descriptions.iter().any(Option::is_none))
            {
                return Err(WorkError::Invalid);
            }
        }
        for i in self.introductions.values() {
            let c = self
                .candidates
                .values()
                .find(|c| c.introduction_id == Some(i.id))
                .ok_or(WorkError::Invalid)?;
            let valid = match i.state {
                IntroductionState::Pending | IntroductionState::Ready => {
                    c.state == CandidateState::Pending
                }
                IntroductionState::Consumed => matches!(
                    c.state,
                    CandidateState::Reserved
                        | CandidateState::Sent
                        | CandidateState::ReviewRequired
                ),
                IntroductionState::Cancelled => matches!(
                    c.state,
                    CandidateState::Cancelled
                        | CandidateState::Rejected
                        | CandidateState::Sent
                        | CandidateState::ReviewRequired
                ),
            };
            if !valid {
                return Err(WorkError::Invalid);
            }
        }
        let mut charged = BTreeSet::new();
        for row in &self.charges {
            let candidate = self
                .candidates
                .get(&row.candidate_id)
                .ok_or(WorkError::Invalid)?;
            let permitted = candidate.member == Some(row.member)
                || candidate
                    .introduction_id
                    .and_then(|id| self.introductions.get(&id))
                    .is_some_and(|i| i.members.contains(&row.member));
            if row.member == 0
                || !permitted
                || !charged.insert((row.candidate_id, row.member))
                || chrono::NaiveDate::parse_from_str(&row.local_day, "%Y-%m-%d").is_err()
                || chrono::NaiveDate::parse_from_str(&row.local_week, "%Y-%m-%d").is_err()
            {
                return Err(WorkError::Invalid);
            }
        }
        for c in self.candidates.values() {
            if matches!(
                c.state,
                CandidateState::Reserved | CandidateState::Sent | CandidateState::ReviewRequired
            ) && c
                .member
                .is_some_and(|member| !charged.contains(&(c.id, member)))
            {
                return Err(WorkError::Invalid);
            }
        }
        for i in self.introductions.values() {
            for c in self
                .candidates
                .values()
                .filter(|c| c.introduction_id == Some(i.id))
            {
                let n = i
                    .members
                    .iter()
                    .filter(|m| charged.contains(&(c.id, **m)))
                    .count();
                if n == 1
                    || (matches!(
                        c.state,
                        CandidateState::Reserved
                            | CandidateState::Sent
                            | CandidateState::ReviewRequired
                    ) && n != 2)
                {
                    return Err(WorkError::Invalid);
                }
            }
        }
        Ok(())
    }
}
