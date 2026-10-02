//! One immutable generation admission, rechecked at every dispatch/output boundary.
use super::{Ask, SessionMode};
use crate::{llm, personal_memory::MemoryUsePermitSet, runtime::AppState};

#[derive(Debug, Clone)]
pub(crate) struct GenerationGuard {
    permits: MemoryUsePermitSet,
    exposure_epoch: u64,
    barrier_open: bool,
}

pub(crate) const WITHDRAWN_REPLY: &str =
    "Personal context changed while this answer was being prepared. Please ask again.";

impl GenerationGuard {
    pub(crate) fn capture(state: &AppState, ask: &Ask<'_>) -> Result<Self, llm::LlmError> {
        let context = ask.context;
        let permits = &context.personal_memory_permits;
        match ask.subject {
            Some((guild, user)) => {
                if !crate::personal_memory::valid_scope(guild)
                    || !crate::personal_memory::valid_scope(user)
                    || permits.scope_identity()
                        != Some(crate::personal_memory::subject_key(guild, user).as_str())
                    || !permits.is_context_sealed()
                    || !permits.validates_context(&context.user_facts, &context.channel_summary)
                    || (!context.user_facts.is_empty() && !permits.authorizes_personal_memory())
                {
                    return Err(denied());
                }
            }
            None => {
                if !matches!(ask.session_mode, SessionMode::SourceOnly)
                    || !context.user_facts.is_empty()
                    || !context.channel_summary.is_empty()
                    || permits.is_context_sealed()
                    || permits.authorizes_personal_memory()
                {
                    return Err(denied());
                }
            }
        }
        let exposure_epoch = state.personal_memory_exposure_epoch();
        if ask.subject.is_some() && permits.exposure_epoch() != exposure_epoch {
            return Err(denied());
        }
        let value = Self {
            permits: permits.clone(),
            exposure_epoch,
            barrier_open: state
                .validate_personal_memory_permits(&MemoryUsePermitSet::empty(exposure_epoch)),
        };
        value.check(state)?;
        Ok(value)
    }

    pub(crate) fn check(&self, state: &AppState) -> Result<(), llm::LlmError> {
        if state.personal_memory_exposure_epoch() != self.exposure_epoch
            || state
                .validate_personal_memory_permits(&MemoryUsePermitSet::empty(self.exposure_epoch))
                != self.barrier_open
            || (self.permits.authorizes_personal_memory()
                && !state.validate_personal_memory_permits(&self.permits))
        {
            Err(denied())
        } else {
            Ok(())
        }
    }

    pub(crate) fn fresh(state: &AppState) -> Self {
        let exposure_epoch = state.personal_memory_exposure_epoch();
        Self {
            permits: MemoryUsePermitSet::empty(exposure_epoch),
            exposure_epoch,
            barrier_open: state
                .validate_personal_memory_permits(&MemoryUsePermitSet::empty(exposure_epoch)),
        }
    }

    pub(super) fn validates_prepared(&self, prepared: &crate::engine::PreparedTurn) -> bool {
        prepared.personal_memory_permits == self.permits
    }

    pub(super) fn authorizes_personal_memory(&self) -> bool {
        self.permits.authorizes_personal_memory()
    }

    pub(super) async fn while_current<T>(
        &self,
        state: &AppState,
        work: impl std::future::Future<Output = Result<T, llm::LlmError>>,
    ) -> Result<T, llm::LlmError> {
        self.check(state)?;
        let work = std::pin::pin!(work);
        let mut work = work;
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                biased;
                _ = tick.tick() => self.check(state)?,
                result = &mut work => {
                    self.check(state)?;
                    return result;
                }
            }
        }
    }

    pub(super) fn matches_host(&self, host: &crate::runtime::ToolScope<'_>) -> bool {
        self.permits.scope_identity()
            == Some(
                crate::personal_memory::subject_key(&host.scoped_guild, &host.scoped_user).as_str(),
            )
    }
}

fn denied() -> llm::LlmError {
    llm::LlmError::classified(
        WITHDRAWN_REPLY,
        crate::provider::ProviderFailureKind::Cancelled,
    )
}

#[cfg(test)]
#[path = "consent_tests.rs"]
pub(super) mod tests;
