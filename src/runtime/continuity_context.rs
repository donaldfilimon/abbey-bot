//! Fresh native access and immutable continuity authority. Transport is injected;
//! no Stores or safety lock may cross its await. Public team output is excluded.
use super::AppState;
use crate::work::{WorkAccess, WorkError, WorkScope, continuity::AuthorizedContinuity};
use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

/// Authorization is GET-only and cancellation-safe; dropping it ends read IO.
pub(crate) trait ContinuityAccessProvider: Send + Sync {
    fn authorize<'a>(
        &'a self,
        scope: &'a WorkScope,
        actor: u64,
        channel: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContinuityAudience {
    OwnerDm,
    PrivateInteraction,
}

pub(super) struct ContinuitySafety {
    pub(super) generation: Option<u64>,
    pub(super) blocked: BTreeSet<WorkScope>,
    pub(super) pending:
        std::collections::BTreeMap<WorkScope, super::continuity_commit::PendingPublication>,
    pub(super) orphans:
        std::collections::BTreeMap<WorkScope, crate::work::continuity::ContinuityCard>,
    pub(super) forgets:
        std::collections::BTreeMap<String, Option<crate::episode_gate::GateOutcome>>,
}
impl ContinuitySafety {
    pub(super) fn new() -> Self {
        Self {
            generation: Some(1),
            blocked: BTreeSet::new(),
            pending: std::collections::BTreeMap::new(),
            orphans: std::collections::BTreeMap::new(),
            forgets: std::collections::BTreeMap::new(),
        }
    }
}

/// Nonserializable and privately minted after fresh access and canonical recheck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdmittedContinuity {
    value: AuthorizedContinuity,
    generation: u64,
    audience: ContinuityAudience,
}
impl AdmittedContinuity {
    pub(crate) fn text(&self) -> &str {
        self.value.text()
    }

    pub(crate) fn current(&self, state: &AppState) -> bool {
        if state.continuity_generation(&self.value.card().scope) != Some(self.generation) {
            return false;
        }
        let current = {
            let stores = AppState::lock(&state.stores);
            self.value
                .current(&stores.continuity, &stores.work, super::now())
        };
        current && state.continuity_generation(&self.value.card().scope) == Some(self.generation)
    }

    pub(crate) async fn fresh(&self, state: &AppState) -> bool {
        self.current(state)
            && state
                .prepare_continuity_context(
                    self.value.card().scope.clone(),
                    self.value.actor(),
                    self.value.channel(),
                    self.audience,
                )
                .await
                .as_ref()
                == Some(self)
            && self.current(state)
    }

    pub(crate) fn binds(&self, scope: &str, subject: Option<(&str, &str)>, private: bool) -> bool {
        let guild = match self.value.card().scope {
            WorkScope::Personal { owner } => format!("discord:dm:{owner}"),
            WorkScope::Team { guild, .. } => format!("discord:{guild}"),
        };
        let user = format!("discord:{}", self.value.actor());
        scope == format!("discord:{}", self.value.channel())
            && subject == Some((guild.as_str(), user.as_str()))
            && (self.audience != ContinuityAudience::PrivateInteraction || private)
    }

    pub(crate) fn permits_delivery(&self, native: Option<&str>, spoken: bool) -> bool {
        !spoken
            && match self.audience {
                ContinuityAudience::OwnerDm => {
                    native.is_none_or(|id| id == self.value.channel().to_string())
                }
                ContinuityAudience::PrivateInteraction => native.is_none(),
            }
    }

    pub(crate) fn permits_public(&self) -> bool {
        self.audience == ContinuityAudience::OwnerDm
    }

    pub(crate) fn permits_private(&self, actor: u64, channel: u64) -> bool {
        self.audience == ContinuityAudience::PrivateInteraction
            && self.value.actor() == actor
            && self.value.channel() == channel
    }
}

impl AppState {
    pub(crate) fn attach_continuity_access(
        &self,
        access: Arc<dyn ContinuityAccessProvider>,
    ) -> Result<(), WorkError> {
        self.continuity_access
            .set(access)
            .map_err(|_| WorkError::Invalid)
    }
    pub(crate) async fn prepare_continuity_context(
        &self,
        scope: WorkScope,
        actor: u64,
        channel: u64,
        audience: ContinuityAudience,
    ) -> Option<AdmittedContinuity> {
        if actor == 0 || channel == 0 || !audience.allows(&scope, actor, channel) {
            return None;
        }
        let generation = self.continuity_generation(&scope)?;
        // Capture only the requested identity before REST. This provisional
        // access cannot escape as admission; the native adapter must prove it.
        let requested = WorkAccess {
            actor,
            channel,
            guild: match scope {
                WorkScope::Personal { .. } => None,
                WorkScope::Team { guild, .. } => Some(guild),
            },
            can_view: true,
            can_manage: false,
        };
        let expected = {
            let stores = Self::lock(&self.stores);
            stores
                .continuity
                .context(&scope, &requested, &stores.work, super::now())?
        };
        if let Some(receipt) = &expected.card().episode_receipt {
            let native = super::continuity_commit::continuity_scope(&scope);
            if self
                .gate_for(&native)?
                .verify_memory(&native, receipt)
                .await
                != crate::episode_gate::continuity::MemoryCandidateState::Live
            {
                return None;
            }
        }
        let access = self
            .fresh_continuity_access(&scope, actor, channel)
            .await
            .ok()?;
        let value = {
            let stores = Self::lock(&self.stores);
            stores
                .continuity
                .context(&scope, &access, &stores.work, super::now())?
        };
        if value != expected || self.continuity_generation(&scope)? != generation {
            return None;
        }
        Some(AdmittedContinuity {
            value,
            generation,
            audience,
        })
    }

    pub(crate) async fn fresh_continuity_access(
        &self,
        scope: &WorkScope,
        actor: u64,
        channel: u64,
    ) -> Result<WorkAccess, WorkError> {
        let provider = self.continuity_access.get().ok_or(WorkError::Denied)?;
        let proof = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            provider.authorize(scope, actor, channel),
        );
        let access = if let Some(service) = self.service_registry() {
            if !service.is_running() {
                return Err(WorkError::Stale);
            }
            let cancelled = service.cancellation();
            tokio::select! { biased; _=cancelled.cancelled()=>return Err(WorkError::Stale), result=proof=>result.map_err(|_|WorkError::Denied)?? }
        } else {
            proof.await.map_err(|_| WorkError::Denied)??
        };
        if actor == 0
            || channel == 0
            || access.actor != actor
            || access.channel != channel
            || access.scope() != *scope
            || !access.can_view
        {
            return Err(WorkError::Denied);
        }
        Ok(access)
    }
    pub(crate) fn continuity_generation(&self, scope: &WorkScope) -> Option<u64> {
        let safety = Self::lock(&self.continuity_safety);
        (!safety.blocked.contains(scope))
            .then_some(safety.generation)
            .flatten()
    }
}

impl ContinuityAudience {
    fn allows(self, scope: &WorkScope, actor: u64, channel: u64) -> bool {
        match scope {
            WorkScope::Personal { owner } => *owner == actor,
            WorkScope::Team {
                channel: native, ..
            } => self == Self::PrivateInteraction && *native == channel,
        }
    }
}

#[cfg(test)]
mod tests;
