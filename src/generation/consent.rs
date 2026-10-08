//! One immutable generation admission, rechecked at every dispatch/output boundary.
use super::{Ask, SessionMode};
use crate::{llm, personal_memory::MemoryUsePermitSet, runtime::AppState};

#[derive(Debug, Clone)]
pub(crate) struct GenerationGuard {
    continuity: Option<crate::runtime::continuity_context::AdmittedContinuity>,
    permits: MemoryUsePermitSet,
    exposure_epoch: u64,
    barrier_open: bool,
    correction: Option<std::sync::Arc<crate::brain::correction::CorrectionSource>>,
}

pub(crate) const WITHDRAWN_REPLY: &str = llm::CONTEXT_CHANGED_REPLY;

impl GenerationGuard {
    pub(crate) fn capture(state: &AppState, ask: &Ask<'_>) -> Result<Self, llm::LlmError> {
        let context = ask.context;
        if context.continuity.as_ref().is_some_and(|card| {
            !card.binds(
                ask.scope,
                ask.subject,
                matches!(ask.session_mode, SessionMode::Ephemeral),
            ) || !card.current(state)
        }) {
            return Err(denied());
        }
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
            continuity: context.continuity.clone(),
            correction: match ask.session_mode {
                SessionMode::Repair(source) => {
                    if source.scope != ask.scope
                        || ask.subject.is_none_or(|(guild, _)| guild != source.guild)
                    {
                        return Err(denied());
                    }
                    Some(std::sync::Arc::new(source.clone()))
                }
                _ => None,
            },
            permits: permits.clone(),
            exposure_epoch,
            barrier_open: state
                .validate_personal_memory_permits(&MemoryUsePermitSet::empty(exposure_epoch)),
        };
        value.check(state)?;
        Ok(value)
    }

    pub(crate) fn check(&self, state: &AppState) -> Result<(), llm::LlmError> {
        if self
            .continuity
            .as_ref()
            .is_some_and(|card| !card.current(state))
        {
            return Err(denied());
        }
        if let Some(source) = &self.correction
            && !correction_current(state, source)
        {
            return Err(denied());
        }
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
            continuity: None,
            correction: None,
            permits: MemoryUsePermitSet::empty(exposure_epoch),
            exposure_epoch,
            barrier_open: state
                .validate_personal_memory_permits(&MemoryUsePermitSet::empty(exposure_epoch)),
        }
    }

    pub(super) fn validates_prepared(&self, prepared: &crate::engine::PreparedTurn) -> bool {
        prepared.personal_memory_permits == self.permits && prepared.continuity == self.continuity
    }

    /// Native access is refreshed at dispatch/output boundaries, while the
    /// inexpensive canonical monitor continues to run without repeated REST.
    pub(crate) async fn check_fresh(&self, state: &AppState) -> Result<(), llm::LlmError> {
        self.check(state)?;
        // Keep the receipt/process authorization state machine off callers'
        // inline frames, including ordinary turns with no continuity card.
        if let Some(card) = &self.continuity
            && !Box::pin(card.fresh(state)).await
        {
            return Err(denied());
        }
        self.check(state)
    }

    pub(super) fn permits_delivery(&self, native: Option<&str>, spoken: bool) -> bool {
        self.continuity
            .as_ref()
            .is_none_or(|card| card.permits_delivery(native, spoken))
    }

    pub(crate) fn permits_public(&self) -> bool {
        self.continuity
            .as_ref()
            .is_none_or(|card| card.permits_public())
    }

    pub(crate) fn permits_private(&self, actor: u64, channel: u64) -> bool {
        self.continuity
            .as_ref()
            .is_none_or(|card| card.permits_private(actor, channel))
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

/// Check source admission before taking a new evidence snapshot and throughout
/// retained generation. Each lock is released before any asynchronous work.
pub(crate) fn correction_current(
    state: &AppState,
    source: &crate::brain::correction::CorrectionSource,
) -> bool {
    let enabled = {
        let stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds)
            .lookup(&source.guild, &*stores)
            .is_some_and(|settings| settings.enabled && settings.learning_enabled)
    };
    enabled && AppState::lock(&state.rewards).correction_current(source, crate::runtime::now())
}

fn denied() -> llm::LlmError {
    llm::LlmError::context_changed()
}

#[cfg(test)]
#[path = "consent_tests.rs"]
pub(super) mod tests;
