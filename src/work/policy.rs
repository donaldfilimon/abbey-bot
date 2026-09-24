//! Deterministic preference policy; scope scheduling lives in schedule.rs.
use super::*;

impl WorkPreferenceProfile {
    #[cfg(test)]
    pub fn effective_hour(&self, default_hour: u8) -> u8 {
        self.explicit_hour
            .or_else(|| self.learning_enabled.then_some(self.learned_hour).flatten())
            .unwrap_or(default_hour)
    }

    #[cfg(test)]
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
