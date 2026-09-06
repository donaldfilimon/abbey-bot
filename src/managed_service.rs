//! Synchronous managed preflight. Call before runtime or credential construction.
use crate::{
    bootstrap::{BootstrapCode, BootstrapDocument, BootstrapPhase},
    managed_env::ManagedEnvironment,
    managed_log::ManagedLog,
    observability::{EventCode, EventComponent, EventOutcome, ManagedFailure, OperationalEvent},
    persist::{
        self, PersistComponentOutcome, PersistErrorCategory, PersistReport, PersistenceSink, Stores,
    },
    readiness::{ReadinessPublisher, RunIdentity},
    wdbx::{Recall, WdbxStore},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedStartupFailure {
    Identity,
    Bootstrap,
    Readiness,
    Logging,
    State,
    PrivacyRewrite,
    Environment,
    Clock,
}
/// One-way fatal latch. Notify is constructed without starting a runtime/thread;
/// notify_one retains a permit if a failure precedes root's first poll.
pub struct ManagedFatalSignal {
    failed: AtomicBool,
    notify: tokio::sync::Notify,
}
impl ManagedFatalSignal {
    pub fn pending(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub async fn notified(&self) {
        if !self.pending() {
            self.notify.notified().await;
        }
    }
    pub(crate) fn trigger(&self) {
        self.failed.store(true, Ordering::Release);
        self.notify.notify_one();
    }
}
pub struct ManagedService {
    pub publisher: ReadinessPublisher,
    pub log: Arc<ManagedLog>,
    pub privacy_report: PersistReport,
    pub environment: ManagedEnvironment,
    pub fatal: Arc<ManagedFatalSignal>,
}
fn wall_ms() -> Result<u64, ManagedStartupFailure> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ManagedStartupFailure::Clock)?
        .as_millis();
    if millis > i64::MAX as u128 {
        return Err(ManagedStartupFailure::Clock);
    }
    Ok(millis as u64)
}
fn event(
    log: &ManagedLog,
    publisher: &ReadinessPublisher,
    component: EventComponent,
    code: EventCode,
    outcome: EventOutcome,
) -> Result<(), ManagedStartupFailure> {
    let event = OperationalEvent::new(wall_ms()?, component, code, outcome)
        .map_err(|_| ManagedStartupFailure::Logging)?;
    log.write(&event).map_err(|_| {
        bootstrap_failure(publisher, BootstrapCode::LogWriter);
        ManagedStartupFailure::Logging
    })
}
fn bootstrap_failure(publisher: &ReadinessPublisher, code: BootstrapCode) {
    let _ = publisher.publish_bootstrap(&BootstrapDocument::new(
        publisher.identity(),
        BootstrapPhase::Failed,
        code,
    ));
}
struct PrivateSink<'a>(&'a crate::readiness::private::PrivateDirectory);
impl PersistenceSink for PrivateSink<'_> {
    fn publish(
        &self,
        _directory: &Path,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<(), PersistErrorCategory> {
        let leaf = match destination.file_name().and_then(|v| v.to_str()) {
            Some(persist::STATE_FILE) => persist::STATE_FILE,
            Some(persist::WDBX_FILE) => persist::WDBX_FILE,
            _ => return Err(PersistErrorCategory::UnsafeFileType),
        };
        self.0.publish(leaf, bytes).map_err(|error| match error {
            ManagedFailure::UnsafeFileType => PersistErrorCategory::UnsafeFileType,
            ManagedFailure::Directory => PersistErrorCategory::CreateDirectory,
            ManagedFailure::File | ManagedFailure::Identity => {
                PersistErrorCategory::CreateTemporary
            }
            ManagedFailure::Sync => PersistErrorCategory::SyncTemporary,
            ManagedFailure::DirectorySync => PersistErrorCategory::SyncDirectory,
            ManagedFailure::Rename => PersistErrorCategory::PublishRename,
            _ => PersistErrorCategory::WriteTemporary,
        })
    }
}
fn rewrite_privacy(
    publisher: &ReadinessPublisher,
    home: &Path,
) -> Result<PersistReport, ManagedStartupFailure> {
    // These are canonical persistence inputs, never credentials. Preserve existing
    // facts and projection migration authority; only InteractionEntry's decoder
    // changes the private legacy rows here.
    const MAX_STATE_BYTES: usize = 128 * 1024 * 1024;
    let directory = publisher.directory();
    let stores = match directory
        .read_optional(persist::STATE_FILE, MAX_STATE_BYTES)
        .map_err(|_| ManagedStartupFailure::State)?
    {
        Some(bytes) => {
            serde_json::from_slice::<Stores>(&bytes).map_err(|_| ManagedStartupFailure::State)?
        }
        None => Stores::default(),
    };
    let recall = match directory
        .read_optional(persist::WDBX_FILE, MAX_STATE_BYTES)
        .map_err(|_| ManagedStartupFailure::State)?
    {
        Some(bytes) => Recall::from_store(
            WdbxStore::parse(
                std::str::from_utf8(&bytes).map_err(|_| ManagedStartupFailure::State)?,
            )
            .map_err(|_| ManagedStartupFailure::State)?,
        ),
        None => Recall::new(),
    };
    let sink = PrivateSink(directory);
    let path = home.join(".local/share/abbey-bot");
    let canonical = match persist::persist_canonical(&sink, &path, &stores) {
        Ok(()) => PersistComponentOutcome::Committed,
        Err(error) => PersistComponentOutcome::Failed(error),
    };
    let projection = if canonical == PersistComponentOutcome::Committed {
        match persist::persist_projection(&sink, &path, &recall) {
            Ok(()) => PersistComponentOutcome::Committed,
            Err(error) => PersistComponentOutcome::Failed(error),
        }
    } else {
        PersistComponentOutcome::SkippedCanonicalFailure
    };
    Ok(PersistReport::from_components(canonical, projection))
}
pub fn begin(home: &Path) -> Result<ManagedService, ManagedStartupFailure> {
    let identity = RunIdentity::current().map_err(|_| ManagedStartupFailure::Identity)?;
    let publisher =
        ReadinessPublisher::open(home, identity).map_err(|_| ManagedStartupFailure::Bootstrap)?;
    publisher
        .publish_bootstrap(&BootstrapDocument::new(
            publisher.identity(),
            BootstrapPhase::Starting,
            BootstrapCode::None,
        ))
        .map_err(|_| ManagedStartupFailure::Bootstrap)?;
    if publisher.validate_readiness_target().is_err() {
        bootstrap_failure(&publisher, BootstrapCode::ReadinessFile);
        return Err(ManagedStartupFailure::Readiness);
    }
    let fatal = Arc::new(ManagedFatalSignal {
        failed: AtomicBool::new(false),
        notify: tokio::sync::Notify::new(),
    });
    let signal = fatal.clone();
    let log = match ManagedLog::open(home, Arc::new(move |_| signal.trigger())) {
        Ok(log) => Arc::new(log),
        Err(error) => {
            let code = match error {
                ManagedFailure::Directory => BootstrapCode::LogDirectory,
                ManagedFailure::UnsafeFileType | ManagedFailure::File => BootstrapCode::LogFile,
                _ => BootstrapCode::LogWriter,
            };
            bootstrap_failure(&publisher, code);
            return Err(ManagedStartupFailure::Logging);
        }
    };
    event(
        &log,
        &publisher,
        EventComponent::Process,
        EventCode::Starting,
        EventOutcome::Started,
    )?;
    let report = rewrite_privacy(&publisher, home)?;
    event(
        &log,
        &publisher,
        EventComponent::State,
        EventCode::StateLoaded,
        EventOutcome::Succeeded,
    )?;
    let outcome = match report.overall {
        persist::PersistOverall::Complete => EventOutcome::Succeeded,
        persist::PersistOverall::Partial => EventOutcome::Degraded,
        persist::PersistOverall::MemoryOnly | persist::PersistOverall::Failed => {
            EventOutcome::Failed
        }
    };
    event(
        &log,
        &publisher,
        EventComponent::State,
        EventCode::PrivacyRewrite,
        outcome,
    )?;
    if report.canonical_state != PersistComponentOutcome::Committed {
        return Err(ManagedStartupFailure::PrivacyRewrite);
    }
    let environment = ManagedEnvironment::load_after_privacy(home, &report)
        .map_err(|_| ManagedStartupFailure::Environment)?;
    Ok(ManagedService {
        publisher,
        log,
        privacy_report: report,
        environment,
        fatal,
    })
}
#[cfg(test)]
mod tests;
