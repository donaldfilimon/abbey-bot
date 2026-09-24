//! Attributable, deterministic scope-local preferences. Explicit controls always
//! win. Only validated sent receipts enter evidence; silence supplies no signal.
use super::*;

impl WorkAccess {
    pub fn scope(self) -> WorkScope {
        self.guild
            .map_or(WorkScope::Personal { owner: self.actor }, |guild| {
                WorkScope::Team {
                    guild,
                    channel: self.channel,
                }
            })
    }
}

impl WorkPreferenceProfile {
    pub fn effective_hour(&self, default_hour: u8) -> u8 {
        self.explicit_hour
            .or_else(|| self.learning_enabled.then_some(self.learned_hour).flatten())
            .unwrap_or(default_hour)
    }

    fn recompute(&mut self) {
        self.learned_hour = None;
        self.reduce_followups = false;
        self.briefing_rank = 0;
        let evidence: Vec<_> = self
            .evidence
            .iter()
            .filter(|e| e.actor.is_some_and(|a| a != 0) && e.scope.is_some() && e.kind.is_some())
            .collect();
        if !self.learning_enabled || evidence.len() < 5 {
            return;
        }
        let mut hours = [0usize; 24];
        let mut dismissed = 0;
        let mut useful_briefings = 0usize;
        let mut dismissed_briefings = 0usize;
        for e in evidence {
            match e.feedback {
                WorkFeedback::Snoozed { hour: 0..=23 } => {
                    if let WorkFeedback::Snoozed { hour } = e.feedback {
                        hours[usize::from(hour)] += 1;
                    }
                }
                WorkFeedback::Dismissed => {
                    dismissed += 1;
                    if e.kind == Some(WorkDeliveryKind::Briefing) {
                        dismissed_briefings += 1;
                    }
                }
                WorkFeedback::Useful if e.kind == Some(WorkDeliveryKind::Briefing) => {
                    useful_briefings += 1;
                }
                _ => {}
            }
        }
        if let Some((hour, count)) = hours
            .iter()
            .enumerate()
            .max_by_key(|(hour, count)| (**count, std::cmp::Reverse(*hour)))
            && *count >= 5
        {
            self.learned_hour = Some(hour as u8);
        }
        self.reduce_followups = dismissed >= 5;
        self.briefing_rank = if useful_briefings >= 5 && useful_briefings > dismissed_briefings {
            1
        } else if dismissed_briefings >= 5 && dismissed_briefings > useful_briefings {
            -1
        } else {
            0
        };
    }

    pub(super) fn observe(&mut self, evidence: PreferenceEvidence) -> Result<(), WorkError> {
        if matches!(evidence.feedback, WorkFeedback::Snoozed { hour: 24.. }) {
            return Err(WorkError::Invalid);
        }
        self.remember_observations();
        let Some(actor) = evidence.actor.filter(|a| *a != 0) else {
            return Ok(());
        };
        if evidence.scope.is_none() || evidence.kind.is_none() {
            return Ok(());
        }
        if !self
            .observed_deliveries
            .insert((evidence.delivery_id, actor))
        {
            return Ok(());
        }
        // Legacy unattributed rows are inert and never consume window capacity.
        self.evidence.retain(attributed);
        if self.evidence.len() >= 100 {
            self.evidence.drain(..self.evidence.len() - 99);
        }
        self.evidence.push(evidence);
        self.recompute();
        Ok(())
    }

    fn remember_observations(&mut self) {
        self.observed_deliveries.extend(
            self.evidence
                .iter()
                .filter(|e| attributed(e))
                .filter_map(|e| e.actor.map(|actor| (e.delivery_id, actor))),
        );
    }

    pub fn reset(&mut self) {
        self.remember_observations();
        self.evidence.clear();
        self.recompute();
    }
}

fn attributed(e: &PreferenceEvidence) -> bool {
    e.actor.is_some_and(|a| a != 0) && e.scope.is_some() && e.kind.is_some()
}

#[derive(Debug, Default)]
pub struct WorkAutomationUpdate {
    pub enabled: bool,
    /// None preserves the existing explicit choice; true is a self-only opt-in.
    pub private_to_me: Option<bool>,
    pub timezone: Option<String>,
    pub briefing_hour: Option<u8>,
    pub quiet_start: Option<u8>,
    pub quiet_end: Option<u8>,
    pub daily_limit: Option<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkPreferenceSnapshot {
    pub profile: WorkPreferenceProfile,
    pub timezone: Option<String>,
    pub effective_hour: u8,
}

impl WorkStore {
    pub fn preference_snapshot(
        &self,
        access: WorkAccess,
    ) -> Result<WorkPreferenceSnapshot, WorkError> {
        let profile = self.preference_profile(access)?;
        let policy = self.scope_automation.get(&access.scope().key());
        Ok(WorkPreferenceSnapshot {
            effective_hour: profile.effective_hour(policy.map_or(9, |p| p.briefing_hour)),
            timezone: policy.map(|p| p.timezone.clone()),
            profile,
        })
    }

    pub fn update_automation(
        &mut self,
        access: WorkAccess,
        update: WorkAutomationUpdate,
        configured_owner: Option<&str>,
    ) -> Result<WorkAutomationPolicy, WorkError> {
        let scope = access.scope();
        self.scope_projects(&scope, access, true)?;
        let mut policy = match self.scope_automation.get(&scope.key()) {
            Some(policy) => policy.clone(),
            None => WorkAutomationPolicy {
                timezone: initial_timezone(access, update.timezone.clone(), configured_owner)?,
                ..Default::default()
            },
        };
        policy.enabled = update.enabled;
        policy.destination = Some(access.channel);
        if let Some(private) = update.private_to_me {
            policy.delivery_target = Some(if private {
                if access.guild.is_none() {
                    return Err(WorkError::Invalid);
                }
                WorkDestination::TeamPrivate {
                    principal: access.actor,
                }
            } else if access.guild.is_some() {
                WorkDestination::TeamChannel {
                    channel: access.channel,
                }
            } else {
                WorkDestination::Personal {
                    principal: access.actor,
                }
            });
        }
        if let Some(timezone) = update.timezone {
            policy.timezone = timezone;
        }
        if let Some(hour) = update.briefing_hour {
            policy.briefing_hour = hour;
        }
        if let Some(hour) = update.quiet_start {
            policy.quiet_start = hour;
        }
        if let Some(hour) = update.quiet_end {
            policy.quiet_end = hour;
        }
        if let Some(limit) = update.daily_limit {
            policy.daily_limit = limit;
        }
        self.configure_automation(&scope, access, policy.clone())?;
        if let Some(hour) = update.briefing_hour {
            self.control_preferences(access, None, Some(Some(hour)), false)?;
        }
        Ok(self.scope_automation[&scope.key()].clone())
    }

    /// All scope projects must authorize access: private evidence cannot leak
    /// through a second project in the same channel. Shared controls need every
    /// project's manager grant, not merely Discord MANAGE_GUILD.
    pub fn preference_profile(
        &self,
        access: WorkAccess,
    ) -> Result<WorkPreferenceProfile, WorkError> {
        let scope = access.scope();
        self.scope_projects(&scope, access, false)?;
        let mut profile = self
            .preferences
            .get(&scope.key())
            .cloned()
            .unwrap_or_default();
        profile.recompute();
        Ok(profile)
    }

    pub fn control_preferences(
        &mut self,
        access: WorkAccess,
        enabled: Option<bool>,
        explicit_hour: Option<Option<u8>>,
        reset: bool,
    ) -> Result<(), WorkError> {
        let scope = access.scope();
        self.scope_projects(&scope, access, true)?;
        if explicit_hour.flatten().is_some_and(|h| h >= 24) {
            return Err(WorkError::Invalid);
        }
        let profile = self.preferences.entry(scope.key()).or_default();
        if let Some(enabled) = enabled {
            profile.learning_enabled = enabled;
        }
        if let Some(hour) = explicit_hour {
            profile.explicit_hour = hour;
        }
        if reset {
            profile.reset();
        }
        profile.recompute();
        Ok(())
    }

    /// Feedback actor is always the freshly authorized caller, never an input
    /// principal. Replacement/removal may touch only that actor's observation.
    pub fn feedback(
        &mut self,
        access: WorkAccess,
        delivery_id: u64,
        feedback: Option<WorkFeedback>,
        correction: bool,
        now: u64,
    ) -> Result<(), WorkError> {
        let scope = access.scope();
        self.scope_projects(&scope, access, false)?;
        let receipt = self
            .deliveries
            .get(&delivery_id)
            .ok_or(WorkError::Missing)?;
        if receipt.state != DeliveryState::Sent
            || receipt.message_id.is_none()
            || receipt.scope.as_ref() != Some(&scope)
            || receipt.kind.is_none()
            || receipt.at > now
            || !receipt.feedback_destination_matches(access)
        {
            return Err(WorkError::Denied);
        }
        self.project(receipt.project_id, access)?;
        if matches!(feedback, Some(WorkFeedback::Snoozed { hour: 24.. })) {
            return Err(WorkError::Invalid);
        }
        let kind = receipt.kind;
        let profile = self.preferences.entry(scope.key()).or_default();
        if correction {
            let index = profile
                .evidence
                .iter()
                .position(|e| {
                    attributed(e) && e.delivery_id == delivery_id && e.actor == Some(access.actor)
                })
                .ok_or(WorkError::Missing)?;
            profile.remember_observations();
            if let Some(feedback) = feedback {
                // Correct in place: an old observation does not become fresh evidence.
                profile.evidence[index].feedback = feedback;
                profile.evidence[index].at = now;
            } else {
                profile.evidence.remove(index);
            }
            profile.recompute();
            return Ok(());
        } else if feedback.is_none() {
            return Err(WorkError::Invalid);
        }
        if let Some(feedback) = feedback {
            profile.observe(PreferenceEvidence {
                actor: Some(access.actor),
                scope: Some(scope),
                kind,
                delivery_id,
                feedback,
                at: now,
            })?;
        }
        profile.recompute();
        Ok(())
    }
}

/// A provisioned identity supplies only a timezone convenience, never access.
fn initial_timezone(
    access: WorkAccess,
    explicit: Option<String>,
    configured_owner: Option<&str>,
) -> Result<String, WorkError> {
    if let Some(zone) = explicit {
        return Ok(zone);
    }
    let owner = configured_owner
        .map(|value| value.parse::<u64>().map_err(|_| WorkError::Invalid))
        .transpose()?;
    if owner == Some(0) {
        return Err(WorkError::Invalid);
    }
    if access.guild.is_none() && owner == Some(access.actor) {
        Ok("America/New_York".into())
    } else {
        Err(WorkError::Invalid)
    }
}

#[cfg(test)]
mod tests;
