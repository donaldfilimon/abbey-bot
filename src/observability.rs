//! Closed managed operational evidence. Never accepts tracing fields or diagnostic text.
use serde::{Deserialize, Serialize};

macro_rules! closed {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
    };
}
closed!(EventComponent {
    Process,
    State,
    Discord,
    Scheduler,
    Persistence,
    Provider,
    Voice,
    Telegram,
    Slack,
    Shutdown
});
closed!(EventCode {
    Starting,
    StateLoaded,
    PrivacyRewrite,
    TaskStarted,
    TaskExit,
    DiscordReady,
    CommandsRegistered,
    PresenceApplied,
    ConnectorState,
    PersistenceAttempt,
    ProviderAttempt,
    VoiceState,
    ReadinessPublished,
    ShutdownStarted,
    ShutdownCompleted,
    ShutdownFinalizing
});
closed!(EventOutcome {
    Started,
    Succeeded,
    Ready,
    Degraded,
    Failed,
    Cancelled,
    TimedOut,
    Skipped,
    Draining,
    Stopped
});
closed!(OperationalErrorCategory {
    Configuration,
    Authentication,
    Authorization,
    Unavailable,
    Timeout,
    Protocol,
    Capacity,
    Persistence,
    UnsafeFileType,
    UnexpectedReturn,
    Panic,
    Internal
});
closed!(EventTask {
    Scheduler,
    Telegram,
    Slack
});
closed!(ManagedFailure {
    UnsupportedPlatform,
    Identity,
    UnsafeFileType,
    Directory,
    File,
    Encode,
    Write,
    Sync,
    DirectorySync,
    Rename,
    Remove,
    WriterPoisoned,
    LineLimit
});

/// The type contains no dynamic payload or private run identity. Provider attempts
/// are categorized without copying backend labels or configuration.
#[derive(Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalEvent {
    schema_version: u8,
    occurred_at_unix_ms: u64,
    component: EventComponent,
    code: EventCode,
    outcome: EventOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_category: Option<OperationalErrorCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aggregate_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<EventTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_id: Option<crate::provider::ProviderId>,
}
impl OperationalEvent {
    pub fn new(
        at: u64,
        component: EventComponent,
        code: EventCode,
        outcome: EventOutcome,
    ) -> Result<Self, ManagedFailure> {
        if at > i64::MAX as u64 {
            return Err(ManagedFailure::Encode);
        }
        Ok(Self {
            schema_version: 1,
            occurred_at_unix_ms: at,
            component,
            code,
            outcome,
            error_category: None,
            duration_ms: None,
            aggregate_count: None,
            task: None,
            provider_id: None,
        })
    }
    pub fn with_error(mut self, category: OperationalErrorCategory) -> Self {
        self.error_category = Some(category);
        self
    }
    pub fn with_duration(mut self, duration: std::time::Duration) -> Self {
        self.duration_ms = Some(duration.as_millis().min(i64::MAX as u128) as u64);
        self
    }
    pub fn with_count(mut self, count: u32) -> Self {
        self.aggregate_count = Some(count);
        self
    }
    pub fn with_task(mut self, task: EventTask) -> Self {
        self.task = Some(task);
        self
    }
    pub fn with_provider(mut self, provider: crate::provider::ProviderId) -> Self {
        self.provider_id = Some(provider);
        self
    }
    pub fn encode(&self) -> Result<Vec<u8>, ManagedFailure> {
        let mut line = serde_json::to_vec(self).map_err(|_| ManagedFailure::Encode)?;
        line.push(b'\n');
        if line.len() > 16 * 1024 {
            return Err(ManagedFailure::LineLimit);
        }
        Ok(line)
    }
}

/// Called after a writer failure, outside its mutex. The integration must close
/// readiness/admission and wake root; this callback must not recursively log.
pub type FatalHook = std::sync::Arc<dyn Fn(ManagedFailure) + Send + Sync>;

#[cfg(test)]
mod tests;
