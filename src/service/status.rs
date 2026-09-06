//! Typed readiness authority shared by root and observed Discord setup.
use super::telemetry::TelemetryRequests;
use crate::{
    observability::ManagedFailure,
    persist::{PersistOverall, PersistReport},
    readiness::*,
};
use std::sync::Mutex;
struct State {
    readiness: ReadinessState,
    checkpoints: ReadyCheckpoints,
}
pub struct ManagedStatus {
    identity: RunIdentity,
    state: Mutex<State>,
    output: TelemetryRequests,
}
impl ManagedStatus {
    pub fn new(
        identity: RunIdentity,
        output: TelemetryRequests,
        privacy: PersistReport,
        telegram: bool,
        slack: bool,
    ) -> Self {
        Self {
            identity,
            output,
            state: Mutex::new(State {
                readiness: ReadinessState {
                    phase: ReadinessPhase::Starting,
                    discord: DiscordState::Connecting,
                    scheduler: SchedulerState::Starting,
                    telegram: if telegram {
                        ConnectorState::Starting
                    } else {
                        ConnectorState::Disabled
                    },
                    slack: if slack {
                        ConnectorState::Starting
                    } else {
                        ConnectorState::Disabled
                    },
                    last_persistence: persistence(privacy),
                },
                checkpoints: ReadyCheckpoints {
                    canonical_privacy_committed: privacy.canonical_state
                        == crate::persist::PersistComponentOutcome::Committed,
                    ..ReadyCheckpoints::default()
                },
            }),
        }
    }
    pub fn scheduler_running(&self) {
        let mut state = crate::runtime::AppState::lock(&self.state);
        state.checkpoints.scheduler_running = true;
        state.readiness.scheduler = SchedulerState::Running;
    }
    /// Called only after actual Ready setup registered commands and applied presence.
    pub fn discord_ready(&self) {
        let mut state = crate::runtime::AppState::lock(&self.state);
        state.checkpoints.discord_ready = true;
        state.checkpoints.commands_registered = true;
        state.checkpoints.presence_applied = true;
        state.readiness.discord = DiscordState::Ready;
    }
    pub fn discord_connecting(&self) {
        let mut state = crate::runtime::AppState::lock(&self.state);
        state.readiness.discord = DiscordState::Connecting;
        state.checkpoints.discord_ready = false;
        if state.readiness.phase != ReadinessPhase::Draining {
            state.readiness.phase = ReadinessPhase::Starting;
        }
    }
    pub fn discord_resumed(&self) {
        let mut state = crate::runtime::AppState::lock(&self.state);
        state.readiness.discord = DiscordState::Ready;
        state.checkpoints.discord_ready = true;
    }
    pub fn draining(&self) {
        crate::runtime::AppState::lock(&self.state).readiness.phase = ReadinessPhase::Draining;
    }
    pub fn telegram(&self, connector: ConnectorState) {
        crate::runtime::AppState::lock(&self.state)
            .readiness
            .telegram = connector;
    }
    pub fn slack(&self, connector: ConnectorState) {
        crate::runtime::AppState::lock(&self.state).readiness.slack = connector;
    }
    pub fn persisted(&self, report: PersistReport) {
        crate::runtime::AppState::lock(&self.state)
            .readiness
            .last_persistence = persistence(report);
    }
    pub fn refresh(
        &self,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), ManagedFailure>>, ManagedFailure> {
        let mut state = crate::runtime::AppState::lock(&self.state);
        if state.readiness.phase != ReadinessPhase::Draining {
            state.readiness.phase = if state.checkpoints.complete() {
                ReadinessPhase::Ready
            } else {
                ReadinessPhase::Starting
            };
        }
        let document = ReadinessDocument::new(
            &self.identity,
            state.readiness,
            crate::runtime::now_millis(),
            state.checkpoints,
        )?;
        self.output.readiness(document)
    }
}
fn persistence(report: PersistReport) -> LastPersistence {
    match report.overall {
        PersistOverall::MemoryOnly => LastPersistence::MemoryOnly,
        PersistOverall::Complete => LastPersistence::Complete,
        PersistOverall::Partial => LastPersistence::Partial,
        PersistOverall::Failed => LastPersistence::Failed,
    }
}
