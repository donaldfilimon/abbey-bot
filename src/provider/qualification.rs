//! Provider qualification evidence and the owner-only runtime manifest gate.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::manifest::{
    FOUNDATION_MODELS_PCC_PROVIDER_ID, FOUNDATION_MODELS_PROVIDER_ID, ManifestDocument,
    ManifestError, ProviderIdentityHashes, QualificationStatus, production_tool_schema_sha256,
    read_manifest, sha256_bytes,
};
use super::{FmConfig, FmMode, ProviderCapabilities};

pub const QUALIFICATION_VERSION: u32 = 1;
pub const FIXTURE_VERSION: &str = "abbey-provider-fixtures-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationTarget {
    Primary,
    Fm,
    All,
}

impl QualificationTarget {
    pub const fn includes_primary(self) -> bool {
        matches!(self, Self::Primary | Self::All)
    }

    pub const fn includes_fm(self) -> bool {
        matches!(self, Self::Fm | Self::All)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    Pass,
    Fail,
    Unsupported,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    pub status: ProbeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Set only on a passing `tools` capability, to the marker the
    /// continuation turn was required to return exactly, after a synthetic
    /// tool RESULT (never after the tool CALL alone). `tools: pass` on its
    /// own only proves a tool call streamed correctly; a manifest consumer
    /// that requires this field too
    /// (`deploy/configure-mlx-primary.py`, `deploy/publish-provider-qualification.py`)
    /// cannot be satisfied by a tool-call-only probe. `#[serde(default)]` so
    /// a manifest written before this field existed still deserializes,
    /// with the field read back as `None` (docs/superpowers/specs/2026-09-04-mlx-vlm-tool-continuation-diagnosis.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result_marker: Option<String>,
}

impl CapabilityEvidence {
    pub fn pass() -> Self {
        Self {
            status: ProbeStatus::Pass,
            category: None,
            tool_result_marker: None,
        }
    }

    /// A passing `tools` capability, additionally recording the exact
    /// marker the model returned for the tool-result continuation turn.
    pub fn tools_pass(marker: impl Into<String>) -> Self {
        Self {
            status: ProbeStatus::Pass,
            category: None,
            tool_result_marker: Some(marker.into()),
        }
    }

    pub fn fail(category: &'static str) -> Self {
        Self {
            status: ProbeStatus::Fail,
            category: Some(category.to_string()),
            tool_result_marker: None,
        }
    }

    pub fn unsupported() -> Self {
        Self {
            status: ProbeStatus::Unsupported,
            category: None,
            tool_result_marker: None,
        }
    }

    pub fn skipped() -> Self {
        Self {
            status: ProbeStatus::Skipped,
            category: None,
            tool_result_marker: None,
        }
    }

    pub const fn passed(&self) -> bool {
        matches!(self.status, ProbeStatus::Pass)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidenceSet {
    pub text: CapabilityEvidence,
    pub streaming: CapabilityEvidence,
    pub structured_output: CapabilityEvidence,
    pub tools: CapabilityEvidence,
    pub vision: CapabilityEvidence,
    pub ocr: CapabilityEvidence,
}

impl CapabilityEvidenceSet {
    pub fn skipped() -> Self {
        Self {
            text: CapabilityEvidence::skipped(),
            streaming: CapabilityEvidence::skipped(),
            structured_output: CapabilityEvidence::skipped(),
            tools: CapabilityEvidence::skipped(),
            vision: CapabilityEvidence::skipped(),
            ocr: CapabilityEvidence::skipped(),
        }
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text: self.text.passed(),
            streaming: self.streaming.passed(),
            structured_output: self.structured_output.passed(),
            tools: self.tools.passed(),
            vision: self.vision.passed(),
            ocr: self.ocr.passed(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub abbey_binary_sha256: String,
    pub os_build: String,
    pub fixture_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEvidence {
    pub configured: bool,
    /// Identity of the text/tool route. Vision may be served by a separately
    /// configured endpoint, so it must never be inferred from this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<ProviderIdentity>,
    /// Identity of the route that actually received the vision/OCR fixtures.
    /// Absent when image capabilities were unsupported or skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision_identity: Option<ProviderIdentity>,
    pub capabilities: CapabilityEvidenceSet,
}

impl ProviderEvidence {
    pub fn skipped() -> Self {
        Self {
            configured: false,
            identity: None,
            vision_identity: None,
            capabilities: CapabilityEvidenceSet::skipped(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationReport {
    pub version: u32,
    pub fixture_version: String,
    pub generated_unix_secs: u64,
    pub target: QualificationTarget,
    pub overall_pass: bool,
    pub primary: ProviderEvidence,
    pub fm_server: ProviderEvidence,
    /// Evidence for the first configured FM mode; drives the exit status.
    pub fm_cli: ProviderEvidence,
    /// Evidence for every configured FM mode in route order (each identity
    /// carries its `mode`), so a publisher can emit one record per mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fm_cli_modes: Vec<ProviderEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedFmCapabilities {
    pub server: Option<ProviderCapabilities>,
    pub cli: ProviderCapabilities,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("could not open a qualification-bound executable: {error}"))?;
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("could not hash a qualification-bound executable: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(lower_hex(&digest.finalize()))
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

pub fn current_binary_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("could not identify the running Abbey binary: {error}"))
}

pub fn current_binary_sha256() -> Result<String, String> {
    file_sha256(&current_binary_path()?)
}

pub fn current_os_build() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/sw_vers")
            .args(["-buildVersion"])
            .env_clear()
            .output()
            .map_err(|error| format!("could not read the macOS build identity: {error}"))?;
        if !output.status.success() {
            return Err("could not read the macOS build identity".into());
        }
        String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "the macOS build identity was empty".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    }
}

pub fn fm_identity(config: &FmConfig) -> Result<ProviderIdentity, String> {
    Ok(ProviderIdentity {
        endpoint: config.endpoint.clone(),
        model: None,
        cli_path: Some(config.cli.clone()),
        cli_sha256: Some(file_sha256(&config.cli)?),
        mode: Some(config.mode.as_str().to_string()),
        abbey_binary_sha256: current_binary_sha256()?,
        os_build: current_os_build()?,
        fixture_version: FIXTURE_VERSION.to_string(),
    })
}

pub fn primary_identity(endpoint: String, model: String) -> Result<ProviderIdentity, String> {
    Ok(ProviderIdentity {
        endpoint: Some(endpoint),
        model: Some(model),
        cli_path: None,
        cli_sha256: None,
        mode: None,
        abbey_binary_sha256: current_binary_sha256()?,
        os_build: current_os_build()?,
        fixture_version: FIXTURE_VERSION.to_string(),
    })
}

pub fn fm_manifest_identity(config: &FmConfig) -> Result<ProviderIdentityHashes, String> {
    Ok(ProviderIdentityHashes {
        abbey_binary_sha256: current_binary_sha256()?,
        provider_binary_sha256: Some(file_sha256(&config.cli)?),
        model_sha256: None,
        os_sha256: Some(sha256_bytes(current_os_build()?.as_bytes())),
        tool_schema_sha256: production_tool_schema_sha256().map_err(|error| error.to_string())?,
        sandbox_sha256: None,
    })
}

/// Per-mode qualification outcome for text routing. Only `Qualified` admits
/// the mode; every other state leaves it registered but unadmitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FmQualificationState {
    Qualified,
    Missing,
    Stale,
    IdentityChanged,
    Refused(String),
}

impl FmQualificationState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Qualified => "qualified",
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::IdentityChanged => "identity_changed",
            Self::Refused(_) => "refused",
        }
    }
}

type FmRejection = (FmQualificationState, String);

fn refused(message: impl Into<String>) -> FmRejection {
    let message = message.into();
    (FmQualificationState::Refused(message.clone()), message)
}

fn classify_manifest_error(error: ManifestError) -> FmRejection {
    let state = match error {
        ManifestError::MissingOrUnreadable | ManifestError::QualificationMissing => {
            FmQualificationState::Missing
        }
        ManifestError::SchemaMismatch | ManifestError::FixtureMismatch => {
            FmQualificationState::Stale
        }
        ManifestError::IdentityMismatch => FmQualificationState::IdentityChanged,
        _ => return refused(render_fm_manifest_error(error)),
    };
    (state, render_fm_manifest_error(error))
}

/// The V2 manifest record that qualifies one FM mode.
pub const fn fm_record_id(mode: FmMode) -> &'static str {
    match mode {
        FmMode::System => FOUNDATION_MODELS_PROVIDER_ID,
        FmMode::Pcc => FOUNDATION_MODELS_PCC_PROVIDER_ID,
    }
}

/// Strict verification: FM vision uses this and fails startup on any error.
pub fn verify_fm_manifest(
    path: &Path,
    config: &FmConfig,
) -> Result<VerifiedFmCapabilities, String> {
    verify_fm_manifest_typed(path, config).map_err(|(_, message)| message)
}

/// Degrading verification for text routing: never an error, always a state.
/// `None` is an unset `ABBEY_FM_CAPABILITY_MANIFEST`.
pub fn qualify_fm(
    path: Option<&Path>,
    config: &FmConfig,
) -> (Option<VerifiedFmCapabilities>, FmQualificationState) {
    let Some(path) = path else {
        return (None, FmQualificationState::Missing);
    };
    match verify_fm_manifest_typed(path, config) {
        Ok(verified) => (Some(verified), FmQualificationState::Qualified),
        Err((state, _)) => (None, state),
    }
}

fn verify_fm_manifest_typed(
    path: &Path,
    config: &FmConfig,
) -> Result<VerifiedFmCapabilities, FmRejection> {
    match read_manifest(path).map_err(classify_manifest_error)? {
        ManifestDocument::LegacyV1(report) => verify_legacy_fm_report(&report, config),
        ManifestDocument::V2(manifest) => verify_v2_fm_manifest(&manifest, config),
    }
}

fn verify_legacy_fm_report(
    report: &QualificationReport,
    config: &FmConfig,
) -> Result<VerifiedFmCapabilities, FmRejection> {
    if report.version != QUALIFICATION_VERSION
        || report.fixture_version != FIXTURE_VERSION
        || report
            .fm_cli
            .identity
            .as_ref()
            .is_some_and(|identity| identity.fixture_version != FIXTURE_VERSION)
        || report
            .fm_cli
            .vision_identity
            .as_ref()
            .is_some_and(|identity| identity.fixture_version != FIXTURE_VERSION)
    {
        return Err((
            FmQualificationState::Stale,
            "ABBEY_FM_CAPABILITY_MANIFEST uses a stale fixture or format version".into(),
        ));
    }
    let now = unix_now();
    if report.generated_unix_secs > now.saturating_add(300) {
        return Err(refused(
            "ABBEY_FM_CAPABILITY_MANIFEST has an invalid future timestamp",
        ));
    }
    if !report.overall_pass || !report.target.includes_fm() || !report.fm_cli.configured {
        return Err(refused(
            "ABBEY_FM_CAPABILITY_MANIFEST does not record a successful FM qualification",
        ));
    }
    let expected = fm_identity(config).map_err(refused)?;
    if report.fm_cli.identity.as_ref() != Some(&expected) {
        return Err((
            FmQualificationState::IdentityChanged,
            "ABBEY_FM_CAPABILITY_MANIFEST does not match this binary, FM executable, mode, or OS build"
                .into(),
        ));
    }
    let cli = report.fm_cli.capabilities.capabilities();
    if !(cli.text && cli.structured_output && cli.tools) {
        return Err(refused(
            "ABBEY_FM_CAPABILITY_MANIFEST lacks required FM CLI text/tool qualification",
        ));
    }
    if (cli.vision || cli.ocr) && report.fm_cli.vision_identity.as_ref() != Some(&expected) {
        return Err((
            FmQualificationState::IdentityChanged,
            "ABBEY_FM_CAPABILITY_MANIFEST does not bind its FM image qualification to this executable"
                .into(),
        ));
    }
    let server = match config.endpoint.as_ref() {
        Some(_) => {
            if !report.fm_server.configured || report.fm_server.identity.as_ref() != Some(&expected)
            {
                return Err((
                    FmQualificationState::IdentityChanged,
                    "ABBEY_FM_CAPABILITY_MANIFEST does not bind the configured FM server".into(),
                ));
            }
            let capabilities = report.fm_server.capabilities.capabilities();
            if !(capabilities.text && capabilities.streaming) {
                return Err(refused(
                    "ABBEY_FM_CAPABILITY_MANIFEST lacks required FM server qualification",
                ));
            }
            Some(capabilities)
        }
        None => None,
    };
    Ok(VerifiedFmCapabilities { server, cli })
}

fn verify_v2_fm_manifest(
    manifest: &super::manifest::ProviderManifest,
    config: &FmConfig,
) -> Result<VerifiedFmCapabilities, FmRejection> {
    let provider_id = super::ProviderId::parse(fm_record_id(config.mode))
        .map_err(|_| refused("the Foundation Models provider identity is invalid"))?;
    let required = ProviderCapabilities {
        text: true,
        streaming: config.endpoint.is_some(),
        structured_output: true,
        tools: true,
        vision: false,
        ocr: false,
    };
    let record = manifest
        .exact_qualified_record(
            &provider_id,
            super::ProviderClass::OsManagedLocal,
            &fm_manifest_identity(config).map_err(refused)?,
            required,
        )
        .map_err(classify_manifest_error)?;
    if !matches!(record.qualification_status, QualificationStatus::Qualified) {
        return Err(refused(
            "ABBEY_FM_CAPABILITY_MANIFEST does not record a successful FM qualification",
        ));
    }

    let qualified = record.declared_capabilities.as_provider_capabilities();
    let cli = ProviderCapabilities {
        streaming: false,
        ..qualified
    };
    let server = config.endpoint.as_ref().map(|_| ProviderCapabilities {
        text: true,
        streaming: true,
        structured_output: false,
        tools: false,
        vision: false,
        ocr: false,
    });
    Ok(VerifiedFmCapabilities { server, cli })
}

fn render_fm_manifest_error(error: ManifestError) -> String {
    match error {
        #[cfg(unix)]
        ManifestError::WrongMode => {
            "ABBEY_FM_CAPABILITY_MANIFEST must not be group- or world-readable and must have mode 0600"
                .to_string()
        }
        #[cfg(unix)]
        ManifestError::WrongOwner => {
            "ABBEY_FM_CAPABILITY_MANIFEST must be owned by the running user".to_string()
        }
        #[cfg(unix)]
        ManifestError::Symlink | ManifestError::NotRegularFile => {
            "ABBEY_FM_CAPABILITY_MANIFEST must be a regular file, not a symlink".to_string()
        }
        ManifestError::Oversized => {
            "ABBEY_FM_CAPABILITY_MANIFEST exceeds the 256 KiB limit".to_string()
        }
        ManifestError::Malformed => "ABBEY_FM_CAPABILITY_MANIFEST is malformed".to_string(),
        ManifestError::SchemaMismatch | ManifestError::FixtureMismatch => {
            "ABBEY_FM_CAPABILITY_MANIFEST uses a stale fixture or format version".to_string()
        }
        other => format!("ABBEY_FM_CAPABILITY_MANIFEST rejected: {other}"),
    }
}

#[cfg(test)]
mod tests;
