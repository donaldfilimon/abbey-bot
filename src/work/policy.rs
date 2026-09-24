//! Deterministic notification and preference policy. Calendar conversion is
//! supplied by the runtime so the same decision code covers every timezone.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryDecision {
    Disabled,
    Quiet,
    Limited,
    Eligible,
}

/// A local day/hour already resolved in the configured timezone by the caller.
#[derive(Debug, Clone, Copy)]
pub struct LocalDeliveryTime<'a> {
    pub day: &'a str,
    pub hour: u8,
}

impl WorkAutomationPolicy {
    pub fn validate(&self) -> Result<(), WorkError> {
        if self.quiet_start >= 24
            || self.quiet_end >= 24
            || self.briefing_hour >= 24
            || self.daily_limit == 0
            || self.daily_limit > 20
            || (self.enabled && (self.destination.is_none() || self.timezone.is_empty()))
        {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }

    pub fn permits(
        &self,
        receipts: impl IntoIterator<Item = WorkDeliveryReceipt>,
        project_id: u64,
        local: LocalDeliveryTime<'_>,
    ) -> DeliveryDecision {
        if !self.enabled {
            return DeliveryDecision::Disabled;
        }
        if local.hour >= 24
            || if self.quiet_start < self.quiet_end {
                local.hour >= self.quiet_start && local.hour < self.quiet_end
            } else {
                local.hour >= self.quiet_start || local.hour < self.quiet_end
            }
        {
            return DeliveryDecision::Quiet;
        }
        let attempts = receipts
            .into_iter()
            .filter(|receipt| receipt.project_id == project_id && receipt.local_day == local.day)
            .count();
        if attempts >= usize::from(self.daily_limit) {
            DeliveryDecision::Limited
        } else {
            DeliveryDecision::Eligible
        }
    }
}

impl WorkPreferenceProfile {
    pub fn effective_hour(&self, default_hour: u8) -> u8 {
        self.explicit_hour
            .or_else(|| self.learning_enabled.then_some(self.learned_hour).flatten())
            .unwrap_or(default_hour)
    }

    pub fn observe(&mut self, evidence: PreferenceEvidence) -> Result<(), WorkError> {
        if matches!(evidence.feedback, WorkFeedback::Snoozed { hour: 24.. }) {
            return Err(WorkError::Invalid);
        }
        if self
            .evidence
            .iter()
            .any(|prior| prior.delivery_id == evidence.delivery_id)
        {
            return Ok(());
        }
        self.evidence.push(evidence);
        if self.evidence.len() > 100 {
            self.evidence.remove(0);
        }
        if !self.learning_enabled || self.explicit_hour.is_some() || self.evidence.len() < 5 {
            return Ok(());
        }
        let mut hours = [0u8; 24];
        for observation in &self.evidence {
            if let WorkFeedback::Snoozed { hour } = observation.feedback {
                hours[usize::from(hour)] = hours[usize::from(hour)].saturating_add(1);
            }
        }
        let Some((hour, count)) = hours
            .iter()
            .enumerate()
            .max_by_key(|(hour, count)| (*count, std::cmp::Reverse(*hour)))
        else {
            return Ok(());
        };
        if *count >= 3 {
            self.learned_hour = Some(u8::try_from(hour).map_err(|_| WorkError::Invalid)?);
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.evidence.clear();
        self.learned_hour = None;
    }
}

impl WorkStore {
    /// Reserve a delivery before making the network call. An interrupted or
    /// ambiguous attempt counts toward the ceiling and is never retried blindly.
    pub fn reserve_delivery(
        &mut self,
        project_id: u64,
        recipient: u64,
        at: u64,
        local: LocalDeliveryTime<'_>,
    ) -> Result<u64, DeliveryDecision> {
        let policy = self
            .automation
            .get(&project_id)
            .cloned()
            .unwrap_or_default();
        let decision = policy.permits(self.deliveries.values().cloned(), project_id, local);
        if decision != DeliveryDecision::Eligible {
            return Err(decision);
        }
        let id = self.next_id().map_err(|_| DeliveryDecision::Limited)?;
        self.deliveries.insert(
            id,
            WorkDeliveryReceipt {
                id,
                project_id,
                recipient,
                local_day: local.day.to_string(),
                at,
                state: DeliveryState::Attempting,
                message_id: None,
            },
        );
        Ok(id)
    }
}
