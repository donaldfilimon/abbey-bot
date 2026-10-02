//! Pure explicit control transitions; transport supplies the invoking identity.
use crate::engagement::*;
use crate::work::WorkError;
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum StopScope {
    #[name = "global"]
    Global,
    #[name = "current_server"]
    CurrentServer,
    #[name = "current_conversation"]
    CurrentConversation,
}
pub fn blocked_scope(p: &MemberPolicy, scope: &EngagementScope) -> bool {
    p.global_stop
        || p.stopped_scopes.contains(scope)
        || matches!(scope, EngagementScope::Guild {guild,..} if p.stopped_guilds.contains(guild))
}
pub fn set_stop(
    p: &mut MemberPolicy,
    origin: &EngagementScope,
    scope: StopScope,
    stop: bool,
) -> Result<(), WorkError> {
    match scope {
        StopScope::Global => p.global_stop = stop,
        StopScope::CurrentServer => {
            let EngagementScope::Guild { guild, .. } = origin else {
                return Err(WorkError::Invalid);
            };
            if stop {
                p.stopped_guilds.insert(*guild);
            } else {
                p.stopped_guilds.remove(guild);
            }
        }
        StopScope::CurrentConversation => {
            if stop {
                p.stopped_scopes.insert(origin.clone());
            } else {
                p.stopped_scopes.remove(origin);
            }
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub fn configure(
    s: &mut EngagementStore,
    actor: u64,
    origin: &EngagementScope,
    daily: u8,
    timezone: String,
    weekly: Option<u8>,
    start: Option<u8>,
    end: Option<u8>,
    destination: Option<DestinationPreference>,
) -> Result<(), WorkError> {
    let mut p = s.member_policies.get(&actor).cloned().unwrap_or_default();
    p.daily_limit = Some(daily);
    p.timezone = Some(timezone);
    p.weekly_limit = weekly;
    if let Some(v) = start {
        p.quiet_start = v;
    }
    if let Some(v) = end {
        p.quiet_end = v;
    }
    if let Some(v) = destination {
        p.destinations.insert(origin.clone(), v);
    }
    save_policy(s, actor, p)
}
pub fn save_policy(
    s: &mut EngagementStore,
    actor: u64,
    mut p: MemberPolicy,
) -> Result<(), WorkError> {
    if actor == 0 || (!s.member_policies.contains_key(&actor) && s.member_policies.len() >= 10000) {
        return Err(WorkError::Invalid);
    }
    p.validate()?;
    p.revision = p.revision.checked_add(1).ok_or(WorkError::Invalid)?;
    s.member_policies.insert(actor, p);
    Ok(())
}
pub fn set_weekly(
    p: &mut MemberPolicy,
    origin: EngagementScope,
    enabled: bool,
    weekday: Option<u8>,
    hour: Option<u8>,
) -> Result<(), WorkError> {
    let subscription = if enabled {
        let weekday = weekday.ok_or(WorkError::Invalid)?;
        let hour = hour.ok_or(WorkError::Invalid)?;
        if weekday > 6 || hour > 23 {
            return Err(WorkError::Invalid);
        }
        Some(WeeklySubscription {
            weekday,
            hour,
            destination: p
                .destinations
                .get(&origin)
                .copied()
                .unwrap_or(DestinationPreference::Origin),
            scope: origin,
        })
    } else {
        None
    };
    p.weekly_subscription = subscription;
    Ok(())
}
pub fn set_community(
    s: &mut EngagementStore,
    guild: u64,
    manager: bool,
    feature: CommunityFeature,
    enabled: bool,
    channel: Option<u64>,
) -> Result<(), WorkError> {
    if !manager || guild == 0 {
        return Err(WorkError::Invalid);
    }
    if enabled && channel.is_none_or(|c| c == 0) {
        return Err(WorkError::Invalid);
    }
    if !s.guild_features.contains_key(&guild) && s.guild_features.len() >= 10000 {
        return Err(WorkError::Invalid);
    }
    let p = s.guild_features.entry(guild).or_default();
    p.revision = p.revision.checked_add(1).ok_or(WorkError::Invalid)?;
    if enabled {
        p.enabled.insert(feature);
        p.channels.insert(feature, [channel.unwrap()].into());
    } else {
        p.enabled.remove(&feature);
        p.channels.remove(&feature);
    }
    Ok(())
}
pub fn dismiss(s: &mut EngagementStore, actor: u64, id: u64) -> Result<(), WorkError> {
    let c = s.candidates.get_mut(&id).ok_or(WorkError::Invalid)?;
    if c.member != Some(actor)
        || !matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
    {
        return Err(WorkError::Invalid);
    }
    let revision = c.revision.checked_add(1).ok_or(WorkError::Invalid)?;
    c.state = CandidateState::Cancelled;
    c.revision = revision;
    Ok(())
}
pub fn feedback(
    s: &mut EngagementStore,
    actor: u64,
    id: u64,
    kind: FeedbackKind,
    at: u64,
) -> Result<(), WorkError> {
    let c = s.candidates.get(&id).ok_or(WorkError::Invalid)?;
    let permitted = c.member == Some(actor)
        || c.introduction_id
            .and_then(|i| s.introductions.get(&i))
            .is_some_and(|i| i.members.contains(&actor));
    if actor == 0
        || !permitted
        || c.state != CandidateState::Sent
        || s.feedback.get(&id).is_some_and(|m| m.contains_key(&actor))
    {
        return Err(WorkError::Invalid);
    }
    s.feedback
        .entry(id)
        .or_default()
        .insert(actor, ExplicitFeedback { actor, at, kind });
    Ok(())
}
pub fn render_status(
    s: &EngagementStore,
    actor: u64,
    origin: &EngagementScope,
    now: u64,
) -> String {
    let p = s.member_policies.get(&actor).cloned().unwrap_or_default();
    let eligibility =
        s.eligibility.get(&actor).is_some_and(|v| !v.is_empty()) || p.weekly_subscription.is_some();
    let counts = receipt_counts(s, |c| owns_receipt(s, c, actor));
    format!(
        "Your engagement settings\nPersonalized contact: {}.\nDaily limit: {}; weekly limit: {}.\nTimezone: {}; quiet hours: {:02}:00–{:02}:00.\nEligibility: {}. Global stop: {}. This origin stopped: {}. Snoozed now: {}.\nWeekly subscription: {}. This origin destination: {:?}.\nReceipts: {}.\nSave a positive daily limit and IANA timezone with `/engage configure`. Scoped resume preserves a global stop; inbound messages never resume contact.",
        if p.personalized_enabled() {
            "configured"
        } else {
            "disabled"
        },
        p.daily_limit.map_or("unset".into(), |v| v.to_string()),
        p.weekly_limit
            .map_or("unrestricted".into(), |v| v.to_string()),
        p.timezone.as_deref().unwrap_or("unset"),
        p.quiet_start,
        p.quiet_end,
        if eligibility {
            "established"
        } else {
            "not established"
        },
        p.global_stop,
        blocked_scope(&p, origin),
        p.snoozed_until.is_some_and(|t| t > now),
        if p.weekly_subscription.is_some() {
            "subscribed"
        } else {
            "off"
        },
        p.destinations
            .get(origin)
            .copied()
            .unwrap_or(DestinationPreference::Origin),
        counts
    )
}

/// Stops consume matching unfinished work; resume never restores source identities.
pub fn stop_store(
    s: &mut EngagementStore,
    actor: u64,
    origin: &EngagementScope,
    scope: StopScope,
) -> Result<(), WorkError> {
    let mut next = s.clone();
    let mut policy = next
        .member_policies
        .get(&actor)
        .cloned()
        .unwrap_or_default();
    set_stop(&mut policy, origin, scope, true)?;
    save_policy(&mut next, actor, policy)?;
    let matches = |candidate_scope: &EngagementScope| match scope {
        StopScope::Global => true,
        StopScope::CurrentConversation => candidate_scope == origin,
        StopScope::CurrentServer => {
            matches!((origin,candidate_scope), (EngagementScope::Guild {guild:a,..},EngagementScope::Guild {guild:b,..}) if a==b)
        }
    };
    let mut introductions = std::collections::BTreeSet::new();
    for (id, intro) in &next.introductions {
        if intro.members.contains(&actor)
            && matches(&intro.scope)
            && matches!(
                intro.state,
                IntroductionState::Pending | IntroductionState::Ready | IntroductionState::Consumed
            )
            && next.candidates.values().any(|c| {
                c.introduction_id == Some(*id)
                    && matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
            })
        {
            introductions.insert(*id);
        }
    }
    for c in next.candidates.values_mut() {
        if matches!(c.state, CandidateState::Pending | CandidateState::Reserved)
            && matches(&c.scope)
            && (c.member == Some(actor)
                || c.introduction_id
                    .is_some_and(|id| introductions.contains(&id)))
        {
            c.revision = c.revision.checked_add(1).ok_or(WorkError::Invalid)?;
            c.state = CandidateState::Cancelled;
        }
    }
    for id in introductions {
        let intro = next.introductions.get_mut(&id).ok_or(WorkError::Invalid)?;
        intro.revision = intro.revision.checked_add(1).ok_or(WorkError::Invalid)?;
        intro.state = IntroductionState::Cancelled;
        intro.approvals = [None; 2];
    }
    next.validate()?;
    *s = next;
    Ok(())
}

/// A linked introduction belongs privately to both participants.
pub(crate) fn owns_receipt(s: &EngagementStore, c: &Candidate, actor: u64) -> bool {
    actor != 0
        && (c.member == Some(actor)
            || c.introduction_id
                .and_then(|id| s.introductions.get(&id))
                .is_some_and(|i| i.members.contains(&actor)))
}
/// Closed state reasons expose counts only; no native identity or content.
pub(crate) fn receipt_counts(s: &EngagementStore, include: impl Fn(&Candidate) -> bool) -> String {
    let states = [
        CandidateState::Pending,
        CandidateState::Reserved,
        CandidateState::Sent,
        CandidateState::Cancelled,
        CandidateState::Rejected,
        CandidateState::ReviewRequired,
    ];
    states
        .into_iter()
        .map(|state| {
            let count = s
                .candidates
                .values()
                .filter(|c| include(c) && c.state == state)
                .count();
            let reason = match state {
                CandidateState::Pending => "pending (awaiting policy/access admission)",
                CandidateState::Reserved => "reserved (delivery in progress)",
                CandidateState::Sent => "sent (confirmed Discord receipt)",
                CandidateState::Cancelled => "cancelled (withdrawn or invalidated)",
                CandidateState::Rejected => "rejected (policy/access/context or delivery refused)",
                CandidateState::ReviewRequired => "review required (delivery uncertain; no retry)",
            };
            format!("{reason}: {count}")
        })
        .collect::<Vec<_>>()
        .join("; ")
}
