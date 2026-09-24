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
        if self
            .evidence
            .iter()
            .any(|e| e.delivery_id == evidence.delivery_id && e.actor == evidence.actor)
        {
            return Ok(());
        }
        if self.evidence.len() >= 100 {
            return Err(WorkError::Full);
        }
        self.evidence.push(evidence);
        self.recompute();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.evidence.clear();
        self.recompute();
    }
}

impl WorkStore {
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
            || receipt.recipient != access.channel
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
            if !profile
                .evidence
                .iter()
                .any(|e| e.delivery_id == delivery_id && e.actor == Some(access.actor))
            {
                return Err(WorkError::Missing);
            }
            profile
                .evidence
                .retain(|e| e.delivery_id != delivery_id || e.actor != Some(access.actor));
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
pub fn initial_timezone(
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
