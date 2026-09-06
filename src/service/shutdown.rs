//! Terminal resources remain owned until the explicit Tokio runtime boundary.
use super::{ServiceSupervisor, ShutdownBudget, persistence::PersistenceWriter};
use crate::persist::PersistReport;
use std::{future::Future, pin::Pin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageOutcome {
    Completed,
    Failed,
    TimedOut,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotStartedReason {
    NotQuiescent,
    DeadlineExpired,
    WriterUnavailable,
    SnapshotIncomplete,
}
#[derive(Debug)]
pub enum FinalPersistOutcome {
    NotStarted(NotStartedReason),
    Completed(PersistReport),
    Incomplete {
        progress: super::persistence::ComponentProgress,
    },
}
#[derive(Debug)]
pub enum ResourceCategory {
    SnapshotWriter,
    ProviderBlockWriter,
    TelemetryWriter,
    ReadinessRefresh,
    VoiceCleanup,
    ShardCleanup,
    SnapshotPreparation,
}
#[derive(Debug)]
pub struct ShutdownReport {
    pub reason: super::ShutdownReason,
    pub stages: [StageOutcome; 4],
    pub aborted: Vec<super::OwnedTaskKind>,
    pub reaped: Vec<super::OwnedTaskKind>,
    pub outstanding: Vec<super::OwnedTaskKind>,
    pub outstanding_resources: Vec<ResourceCategory>,
    pub total_duration: std::time::Duration,
    pub final_persist: FinalPersistOutcome,
}
impl FinalPersistOutcome {
    pub fn successful_completion(&self) -> bool {
        matches!(self, Self::Completed(report) if matches!(report.overall, crate::persist::PersistOverall::Complete | crate::persist::PersistOverall::MemoryOnly))
    }
}
impl ShutdownReport {
    pub fn clean(&self) -> bool {
        self.stages
            .iter()
            .all(|stage| *stage == StageOutcome::Completed)
            && self.outstanding.is_empty()
            && self.outstanding_resources.is_empty()
            && self.final_persist.successful_completion()
    }
    pub fn log(&self) {
        tracing::info!(reason = ?self.reason, stages = ?self.stages, aborted = ?self.aborted, reaped = ?self.reaped, outstanding = ?self.outstanding, resources = ?self.outstanding_resources, duration_ms = self.total_duration.as_millis(), final_persist = ?self.final_persist, "shutdown ownership report");
    }
}
pub type VoiceCleanup = Pin<Box<dyn Future<Output = Result<(), ()>> + Send>>;
#[derive(Default)]
pub struct TerminalBoundary {
    pub report: Option<ShutdownReport>,
    pub budget: Option<ShutdownBudget>,
    pub incomplete: bool,
    pub supervisor: Option<ServiceSupervisor>,
    pub writer: Option<PersistenceWriter>,
    pub telemetry: Option<super::telemetry::TelemetryWriter>,
    pub telemetry_joined: bool,
    pub refresh: Option<super::refresh::RefreshOwner>,
    pub refresh_joined: bool,
    pub initialization: Option<tokio::task::JoinHandle<Result<crate::Data, crate::Error>>>,
    pub final_snapshot: Option<tokio::task::JoinHandle<super::persistence::Snapshot>>,
    pub provider_writer: Option<crate::provider::BlockWriter>,
    pub voice_cleanup: Option<VoiceCleanup>,
    pub shard_cleanup: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

/// Stage one completes only after both actor teardown and physical gateway leave.
pub async fn close_voice(
    disconnect: impl Future<Output = ()>,
    remove_call: impl Future<Output = Result<(), ()>>,
) -> Result<(), ()> {
    disconnect.await;
    remove_call.await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_physical_cleanup_or_partial_persist_never_reports_clean_shutdown() {
        let mut report = ShutdownReport {
            reason: super::super::ShutdownReason::Signal,
            stages: [StageOutcome::Completed; 4],
            aborted: Vec::new(),
            reaped: Vec::new(),
            outstanding: Vec::new(),
            outstanding_resources: Vec::new(),
            total_duration: std::time::Duration::ZERO,
            final_persist: FinalPersistOutcome::Completed(PersistReport::memory_only()),
        };
        assert!(report.clean());
        report.stages[0] = StageOutcome::Failed;
        assert!(
            !report.clean(),
            "observed Songbird failure cannot masquerade as a clean exit"
        );
        report.stages[0] = StageOutcome::Completed;
        report.final_persist = FinalPersistOutcome::Completed(PersistReport::from_components(
            crate::persist::PersistComponentOutcome::Committed,
            crate::persist::PersistComponentOutcome::Failed(
                crate::persist::PersistErrorCategory::SyncDirectory,
            ),
        ));
        assert!(
            !report.clean(),
            "completed but partial durability remains a failed shutdown"
        );
    }
    #[tokio::test]
    async fn physical_leave_is_part_of_actual_voice_stage_completion() {
        let (release_actor, actor) = tokio::sync::oneshot::channel();
        let (remove_started, started) = tokio::sync::oneshot::channel();
        let (release_remove, remove) = tokio::sync::oneshot::channel();
        let cleanup = tokio::spawn(close_voice(
            async {
                let _ = actor.await;
            },
            async {
                let _ = remove_started.send(());
                let _ = remove.await;
                Err(())
            },
        ));
        tokio::task::yield_now().await;
        assert!(!cleanup.is_finished());
        release_actor.send(()).unwrap();
        started.await.unwrap();
        assert!(
            !cleanup.is_finished(),
            "virtual teardown alone is not physical leave"
        );
        release_remove.send(()).unwrap();
        assert_eq!(cleanup.await.unwrap(), Err(()));
    }
}
