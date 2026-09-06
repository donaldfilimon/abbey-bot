//! Private process identity, strict v1 readiness wire format and owned publication.
use crate::observability::ManagedFailure;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, io::Read, path::Path};
pub(crate) mod private;
/// The lifecycle owner must schedule refreshes at this interval or sooner.
pub const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, PartialEq, Eq)]
pub struct RunIdentity {
    pid: u32,
    nonce: String,
    executable_sha256: String,
}
impl fmt::Debug for RunIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunIdentity([private])")
    }
}
impl RunIdentity {
    pub fn current() -> Result<Self, ManagedFailure> {
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy).map_err(|_| ManagedFailure::Identity)?;
        let path = std::env::current_exe().map_err(|_| ManagedFailure::Identity)?;
        let mut file = std::fs::File::open(path).map_err(|_| ManagedFailure::Identity)?;
        let before = file.metadata().map_err(|_| ManagedFailure::Identity)?;
        if !before.is_file() {
            return Err(ManagedFailure::Identity);
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|_| ManagedFailure::Identity)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        let after = file.metadata().map_err(|_| ManagedFailure::Identity)?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(ManagedFailure::Identity);
        }
        Self::from_parts(std::process::id(), hex(&entropy), hex(&hash.finalize()))
    }
    fn from_parts(
        pid: u32,
        nonce: String,
        executable_sha256: String,
    ) -> Result<Self, ManagedFailure> {
        if pid == 0 || pid > i32::MAX as u32 || !valid_hex(&nonce) || !valid_hex(&executable_sha256)
        {
            return Err(ManagedFailure::Identity);
        }
        Ok(Self {
            pid,
            nonce,
            executable_sha256,
        })
    }
    pub(crate) fn fields(&self) -> (u32, &str, &str) {
        (self.pid, &self.nonce, &self.executable_sha256)
    }
    pub(crate) fn matches(&self, pid: u32, nonce: &str) -> bool {
        self.pid == pid && self.nonce == nonce
    }
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn valid_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
macro_rules! state_enum {
    ($name:ident { $($v:ident),+ }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($v),+ }
    };
}
state_enum!(ReadinessPhase {
    Starting,
    Ready,
    Draining
});
state_enum!(DiscordState {
    Connecting,
    Ready,
    Stopped
});
state_enum!(SchedulerState {
    Starting,
    Running,
    Stopped
});
state_enum!(ConnectorState {
    Disabled,
    Starting,
    Connected,
    Degraded,
    Stopped
});
state_enum!(LastPersistence {
    NotAttempted,
    MemoryOnly,
    Complete,
    Partial,
    Failed
});

#[derive(Debug, Clone, Copy)]
pub struct ReadinessState {
    pub phase: ReadinessPhase,
    pub discord: DiscordState,
    pub scheduler: SchedulerState,
    pub telegram: ConnectorState,
    pub slack: ConnectorState,
    pub last_persistence: LastPersistence,
}
/// Required startup checkpoints are explicit. Readiness cannot be inferred from a PID.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadyCheckpoints {
    pub canonical_privacy_committed: bool,
    pub scheduler_running: bool,
    pub discord_ready: bool,
    pub commands_registered: bool,
    pub presence_applied: bool,
}
impl ReadyCheckpoints {
    pub fn complete(self) -> bool {
        self.canonical_privacy_committed
            && self.scheduler_running
            && self.discord_ready
            && self.commands_registered
            && self.presence_applied
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessDocument {
    schema_version: u8,
    pid: u32,
    run_nonce: String,
    executable_sha256: String,
    phase: ReadinessPhase,
    published_at_unix_ms: u64,
    discord: DiscordState,
    scheduler: SchedulerState,
    telegram: ConnectorState,
    slack: ConnectorState,
    last_persistence: LastPersistence,
}
impl ReadinessDocument {
    pub fn is_ready(&self) -> bool {
        self.phase == ReadinessPhase::Ready
    }
    pub fn new(
        identity: &RunIdentity,
        state: ReadinessState,
        at: u64,
        checkpoints: ReadyCheckpoints,
    ) -> Result<Self, ManagedFailure> {
        if at > i64::MAX as u64
            || (state.phase == ReadinessPhase::Ready
                && (!checkpoints.complete()
                    || state.discord != DiscordState::Ready
                    || state.scheduler != SchedulerState::Running))
        {
            return Err(ManagedFailure::Encode);
        }
        let (pid, nonce, sha) = identity.fields();
        Ok(Self {
            schema_version: 1,
            pid,
            run_nonce: nonce.into(),
            executable_sha256: sha.into(),
            phase: state.phase,
            published_at_unix_ms: at,
            discord: state.discord,
            scheduler: state.scheduler,
            telegram: state.telegram,
            slack: state.slack,
            last_persistence: state.last_persistence,
        })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ManagedFailure> {
        if bytes.len() > 4096 {
            return Err(ManagedFailure::Encode);
        }
        // Deserialize directly: derived struct visitors reject duplicate keys before
        // a Value/map can silently overwrite them.
        let doc: Self = serde_json::from_slice(bytes).map_err(|_| ManagedFailure::Encode)?;
        if doc.schema_version != 1 || doc.published_at_unix_ms > i64::MAX as u64 {
            return Err(ManagedFailure::Encode);
        }
        RunIdentity::from_parts(
            doc.pid,
            doc.run_nonce.clone(),
            doc.executable_sha256.clone(),
        )?;
        Ok(doc)
    }
    pub fn encode(&self) -> Result<Vec<u8>, ManagedFailure> {
        let mut bytes = serde_json::to_vec(self).map_err(|_| ManagedFailure::Encode)?;
        bytes.push(b'\n');
        Self::decode(&bytes)?;
        Ok(bytes)
    }
    pub fn fresh(&self, start: u64, now: u64) -> bool {
        fresh(self.published_at_unix_ms, start, now)
    }
}
pub fn fresh(published: u64, start: u64, now: u64) -> bool {
    [published, start, now]
        .iter()
        .all(|v| *v <= i64::MAX as u64)
        && published >= start
        && if published > now {
            published.checked_sub(now).is_some_and(|v| v <= 2000)
        } else {
            now.checked_sub(published).is_some_and(|v| v <= 30_000)
        }
}

/// Synchronous filesystem owner. The root must retain the actual blocking handle
/// when a publication exceeds its cooperative allowance.
pub struct ReadinessPublisher {
    directory: private::PrivateDirectory,
    identity: RunIdentity,
}
impl ReadinessPublisher {
    pub fn open(home: &Path, identity: RunIdentity) -> Result<Self, ManagedFailure> {
        let directory = private::PrivateDirectory::open(home, &[".local", "share", "abbey-bot"])?;
        directory.validate_optional("bootstrap-status.json")?;
        Ok(Self {
            directory,
            identity,
        })
    }
    pub fn validate_readiness_target(&self) -> Result<(), ManagedFailure> {
        self.directory.validate_optional("readiness.json")
    }
    pub(crate) fn directory(&self) -> &private::PrivateDirectory {
        &self.directory
    }
    pub fn identity(&self) -> &RunIdentity {
        &self.identity
    }
    pub fn publish(&self, document: &ReadinessDocument) -> Result<(), ManagedFailure> {
        if !self.identity.matches(document.pid, &document.run_nonce)
            || self.identity.executable_sha256 != document.executable_sha256
        {
            return Err(ManagedFailure::Identity);
        }
        self.directory
            .publish("readiness.json", &document.encode()?)
    }
    pub fn remove_readiness(&self) -> Result<bool, ManagedFailure> {
        self.directory
            .remove_matching("readiness.json", 4096, |bytes| {
                let doc = ReadinessDocument::decode(bytes)?;
                Ok(self.identity.matches(doc.pid, &doc.run_nonce))
            })
    }
    pub fn publish_bootstrap(
        &self,
        document: &crate::bootstrap::BootstrapDocument,
    ) -> Result<(), ManagedFailure> {
        if !document.matches(&self.identity) {
            return Err(ManagedFailure::Identity);
        }
        self.directory
            .publish("bootstrap-status.json", &document.encode()?)
    }
    pub fn remove_bootstrap(&self) -> Result<bool, ManagedFailure> {
        self.directory
            .remove_matching("bootstrap-status.json", 512, |bytes| {
                Ok(crate::bootstrap::BootstrapDocument::decode(bytes)?.matches(&self.identity))
            })
    }
}
#[cfg(test)]
pub(crate) mod tests;
