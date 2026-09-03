//! Bounded, configuration-directed provider executable discovery.
//!
//! Discovery accepts exact candidate paths from validated configuration. It
//! never searches `PATH`, scans ports, reads provider configuration or
//! credential stores, inspects sessions, loads a model, or runs inference.
//! The only child operation is the fixed `--version` probe.

use std::fmt;
use std::fs::File;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt as _};

use super::ProviderCapabilities;
use super::config::ProviderSettings;
use super::domain::{
    BlockedReason, DetectionState, DiscoveryBoundary, Eligibility, IsolationCapabilities,
    ProviderClass, ProviderDescriptor, ProviderId, ProviderProvenance,
};

const DEFAULT_VERSION_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_VERSION_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Hard limits for the fixed version probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryLimits {
    version_output_bytes: usize,
    version_timeout: Duration,
}

impl DiscoveryLimits {
    /// Construct limits while retaining hard upper bounds even when a caller
    /// supplies overly permissive values.
    #[must_use]
    pub fn new(version_output_bytes: usize, version_timeout: Duration) -> Self {
        let version_output_bytes = version_output_bytes.clamp(1, MAX_VERSION_OUTPUT_BYTES);
        let version_timeout = if version_timeout.is_zero() {
            Duration::from_millis(1)
        } else {
            version_timeout.min(MAX_VERSION_TIMEOUT)
        };
        Self {
            version_output_bytes,
            version_timeout,
        }
    }

    #[must_use]
    pub const fn version_output_bytes(self) -> usize {
        self.version_output_bytes
    }

    #[must_use]
    pub const fn version_timeout(self) -> Duration {
        self.version_timeout
    }
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            version_output_bytes: DEFAULT_VERSION_OUTPUT_BYTES,
            version_timeout: DEFAULT_VERSION_TIMEOUT,
        }
    }
}

/// One provider's exact executable candidates and safe descriptor template.
///
/// Candidate paths deliberately have no public `Debug` representation. The
/// count is safe metadata; the paths themselves are runtime-private.
#[derive(Clone)]
pub struct DiscoveryRequest {
    pub id: ProviderId,
    pub class: ProviderClass,
    pub discovery: DiscoveryBoundary,
    pub candidate_paths: Vec<PathBuf>,
    pub declared_capabilities: ProviderCapabilities,
    pub isolation: IsolationCapabilities,
}

impl DiscoveryRequest {
    #[must_use]
    pub fn new(
        id: ProviderId,
        class: ProviderClass,
        candidate_paths: Vec<PathBuf>,
        declared_capabilities: ProviderCapabilities,
        isolation: IsolationCapabilities,
    ) -> Self {
        let discovery = if candidate_paths.is_empty() {
            DiscoveryBoundary::Unconfigured
        } else if matches!(class, ProviderClass::OsManagedLocal) {
            DiscoveryBoundary::OsManaged
        } else {
            DiscoveryBoundary::ExactBinary
        };
        Self {
            id,
            class,
            discovery,
            candidate_paths,
            declared_capabilities,
            isolation,
        }
    }

    /// Build a value-free discovery request from validated provider settings.
    /// Endpoint and model values are used only to select the safe boundary
    /// shape and are never copied into this request.
    #[must_use]
    pub fn from_settings(
        settings: &ProviderSettings,
        class: ProviderClass,
        declared_capabilities: ProviderCapabilities,
        isolation: IsolationCapabilities,
    ) -> Self {
        let candidate_paths = settings.binary.iter().cloned().collect::<Vec<_>>();
        let discovery =
            if matches!(class, ProviderClass::OsManagedLocal) && settings.binary.is_some() {
                DiscoveryBoundary::OsManaged
            } else {
                match (settings.binary.is_some(), settings.endpoint.is_some()) {
                    (false, false) => DiscoveryBoundary::Unconfigured,
                    (true, false) => DiscoveryBoundary::ExactBinary,
                    (false, true) => DiscoveryBoundary::ExactEndpoint,
                    (true, true) => DiscoveryBoundary::ExactBinaryAndEndpoint,
                }
            };
        Self {
            id: settings.id.clone(),
            class,
            discovery,
            candidate_paths,
            declared_capabilities,
            isolation,
        }
    }

    /// Override the safe boundary shape when the provider also has an exact
    /// endpoint or is the explicitly approved OS-managed exception.
    #[must_use]
    pub fn with_discovery_boundary(mut self, discovery: DiscoveryBoundary) -> Self {
        self.discovery = discovery;
        self
    }
}

impl fmt::Debug for DiscoveryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryRequest")
            .field("id", &self.id)
            .field("class", &self.class)
            .field("discovery", &self.discovery)
            .field("candidate_count", &self.candidate_paths.len())
            .field("declared_capabilities", &self.declared_capabilities)
            .field("isolation", &self.isolation)
            .finish()
    }
}

/// Exact detected executable identity.
///
/// The executable path remains available to crate-internal adapter assembly,
/// but is intentionally omitted from `Debug` and provider inspection.
#[derive(Clone, PartialEq, Eq)]
pub struct ExecutableIdentity {
    path: PathBuf,
    binary_sha256: String,
    version_sha256: String,
}

impl ExecutableIdentity {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn binary_sha256(&self) -> &str {
        &self.binary_sha256
    }

    #[must_use]
    pub fn version_sha256(&self) -> &str {
        &self.version_sha256
    }
}

impl fmt::Debug for ExecutableIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutableIdentity")
            .field("binary_sha256", &self.binary_sha256)
            .field("version_sha256", &self.version_sha256)
            .finish()
    }
}

/// Safe descriptor plus an optional crate-private resolved executable.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    descriptor: ProviderDescriptor,
    executable: Option<ExecutableIdentity>,
}

impl DiscoveryResult {
    pub(crate) fn not_configured(mut request: DiscoveryRequest) -> Self {
        request.discovery = DiscoveryBoundary::Unconfigured;
        result(request, DetectionState::NotDetected, None)
    }

    #[must_use]
    pub fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn executable_identity(&self) -> Option<&ExecutableIdentity> {
        self.executable.as_ref()
    }

    pub(crate) fn into_parts(self) -> (ProviderDescriptor, Option<ExecutableIdentity>) {
        (self.descriptor, self.executable)
    }
}

impl fmt::Debug for DiscoveryResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryResult")
            .field("descriptor", &self.descriptor)
            .field("executable", &self.executable)
            .finish()
    }
}

/// Inspect one exact candidate set and run at most one fixed version probe.
pub async fn discover(request: DiscoveryRequest, limits: DiscoveryLimits) -> DiscoveryResult {
    if request.candidate_paths.is_empty() {
        let detection = match request.discovery {
            DiscoveryBoundary::Unconfigured => DetectionState::NotDetected,
            // An exact endpoint is configuration evidence only. Discovery does
            // not contact it; qualification later proves the transport.
            DiscoveryBoundary::ExactEndpoint
                if matches!(
                    request.class,
                    ProviderClass::LocalServer | ProviderClass::Cloud
                ) =>
            {
                DetectionState::Detected
            }
            DiscoveryBoundary::ExactEndpoint => DetectionState::InvalidConfiguration,
            DiscoveryBoundary::ExactBinary
            | DiscoveryBoundary::ExactBinaryAndEndpoint
            | DiscoveryBoundary::OsManaged => DetectionState::InvalidConfiguration,
        };
        return result(request, detection, None);
    }
    // Configuration must resolve ambiguity before discovery. Do not inspect or
    // execute any candidate when more than one exact path was supplied.
    if request.candidate_paths.len() != 1 {
        return result(request, DetectionState::Ambiguous, None);
    }

    if !matches!(
        request.discovery,
        DiscoveryBoundary::ExactBinary
            | DiscoveryBoundary::ExactBinaryAndEndpoint
            | DiscoveryBoundary::OsManaged
    ) {
        return result(request, DetectionState::InvalidConfiguration, None);
    }
    if matches!(request.class, ProviderClass::AgentCli)
        && !matches!(
            request.discovery,
            DiscoveryBoundary::ExactBinary | DiscoveryBoundary::ExactBinaryAndEndpoint
        )
        || matches!(request.class, ProviderClass::OsManagedLocal)
            && !matches!(request.discovery, DiscoveryBoundary::OsManaged)
    {
        return result(request, DetectionState::InvalidConfiguration, None);
    }

    let path = request.candidate_paths[0].clone();
    match validate_exact_candidate(&path) {
        CandidateState::Missing => result(request, DetectionState::NotDetected, None),
        CandidateState::Invalid => result(request, DetectionState::InvalidConfiguration, None),
        CandidateState::Ready => {
            let before = match hash_file(&path) {
                Ok(hash) => hash,
                Err(()) => {
                    return result(request, DetectionState::InvalidConfiguration, None);
                }
            };
            let version_sha256 = match probe_version(&path, limits).await {
                Ok(hash) => hash,
                Err(()) => {
                    return result(request, DetectionState::InvalidConfiguration, None);
                }
            };
            // Fail closed if the exact path changed type or content while its
            // version was being inspected.
            if validate_exact_candidate(&path) != CandidateState::Ready
                || hash_file(&path).as_deref() != Ok(before.as_str())
            {
                return result(request, DetectionState::InvalidConfiguration, None);
            }
            result(
                request,
                DetectionState::Detected,
                Some(ExecutableIdentity {
                    path,
                    binary_sha256: before,
                    version_sha256,
                }),
            )
        }
    }
}

fn result(
    request: DiscoveryRequest,
    detection: DetectionState,
    executable: Option<ExecutableIdentity>,
) -> DiscoveryResult {
    let eligibility = match detection {
        DetectionState::NotDetected => Eligibility::Blocked(BlockedReason::NotDetected),
        DetectionState::Detected => Eligibility::Blocked(BlockedReason::Unqualified),
        DetectionState::Ambiguous => Eligibility::Blocked(BlockedReason::Ambiguous),
        DetectionState::InvalidConfiguration => {
            Eligibility::Blocked(BlockedReason::InvalidConfiguration)
        }
    };
    DiscoveryResult {
        descriptor: ProviderDescriptor {
            id: request.id,
            class: request.class,
            discovery: request.discovery,
            detection,
            eligibility,
            declared_capabilities: request.declared_capabilities,
            isolation: request.isolation,
            provenance: ProviderProvenance::Configuration,
        },
        executable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateState {
    Missing,
    Ready,
    Invalid,
}

fn validate_exact_candidate(path: &Path) -> CandidateState {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return CandidateState::Invalid;
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CandidateState::Missing;
        }
        Err(_) => return CandidateState::Invalid,
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return CandidateState::Invalid;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return CandidateState::Invalid;
        }
    }
    CandidateState::Ready
}

fn hash_file(path: &Path) -> Result<String, ()> {
    let mut file = File::open(path).map_err(|_| ())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| ())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(lower_hex(&digest.finalize()))
}

async fn probe_version(path: &Path, limits: DiscoveryLimits) -> Result<String, ()> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| ())?;
    let stdout = child.stdout.take().ok_or(())?;
    let stderr = child.stderr.take().ok_or(())?;
    let output_limit = limits.version_output_bytes();
    let operation = async move {
        let (stdout, stderr, status) = tokio::try_join!(
            read_capped(stdout, output_limit),
            read_capped(stderr, output_limit),
            async { child.wait().await.map_err(|_| ()) },
        )?;
        if !status.success()
            || stdout.len().saturating_add(stderr.len()) == 0
            || stdout.len().saturating_add(stderr.len()) > output_limit
        {
            return Err(());
        }
        let mut digest = Sha256::new();
        digest.update(&stdout);
        digest.update([0]);
        digest.update(&stderr);
        Ok(lower_hex(&digest.finalize()))
    };
    tokio::time::timeout(limits.version_timeout(), operation)
        .await
        .map_err(|_| ())?
}

async fn read_capped(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, ()> {
    let mut output = Vec::with_capacity(limit.min(4096));
    let mut buffer = [0_u8; 4096];
    loop {
        let count = reader.read(&mut buffer).await.map_err(|_| ())?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            return Err(());
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
