//! Content-free provider qualification manifests.
//!
//! The transient provider self-test report remains the real legacy-v1 object
//! format. Runtime/provider qualification state is published only as a strict
//! v2 array whose variable values are normalized provider IDs and SHA-256
//! identities. Dynamic routing eligibility never belongs in this file.

use std::collections::HashSet;
use std::fmt;
use std::io::Read as _;
#[cfg(unix)]
use std::io::Write as _;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest as _, Sha256};

use super::qualification::{FIXTURE_VERSION, QUALIFICATION_VERSION, QualificationReport};
use super::{IsolationCapabilities, ProviderCapabilities, ProviderClass, ProviderId};

pub const PROVIDER_MANIFEST_VERSION: u32 = 2;
pub const MAX_PROVIDER_MANIFEST_BYTES: u64 = 256 * 1024;
pub const FOUNDATION_MODELS_PROVIDER_ID: &str = "foundation-models";

#[cfg(unix)]
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationStatus {
    Qualified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderIdentityHashes {
    pub abbey_binary_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_binary_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_sha256: Option<String>,
    pub tool_schema_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_sha256: Option<String>,
}

impl ProviderIdentityHashes {
    fn validate(&self) -> Result<(), ManifestError> {
        validate_sha256(&self.abbey_binary_sha256)?;
        validate_optional_sha256(self.provider_binary_sha256.as_deref())?;
        validate_optional_sha256(self.model_sha256.as_deref())?;
        validate_optional_sha256(self.os_sha256.as_deref())?;
        validate_sha256(&self.tool_schema_sha256)?;
        validate_optional_sha256(self.sandbox_sha256.as_deref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCapabilities {
    pub text: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub tools: bool,
    pub vision: bool,
    pub ocr: bool,
}

impl DeclaredCapabilities {
    #[must_use]
    pub const fn any(self) -> bool {
        self.text
            || self.streaming
            || self.structured_output
            || self.tools
            || self.vision
            || self.ocr
    }

    #[must_use]
    pub const fn as_provider_capabilities(self) -> ProviderCapabilities {
        ProviderCapabilities {
            text: self.text,
            streaming: self.streaming,
            structured_output: self.structured_output,
            tools: self.tools,
            vision: self.vision,
            ocr: self.ocr,
        }
    }

    #[must_use]
    pub const fn satisfies(self, required: ProviderCapabilities) -> bool {
        self.as_provider_capabilities().satisfies(required)
    }
}

impl From<ProviderCapabilities> for DeclaredCapabilities {
    fn from(capabilities: ProviderCapabilities) -> Self {
        Self {
            text: capabilities.text,
            streaming: capabilities.streaming,
            structured_output: capabilities.structured_output,
            tools: capabilities.tools,
            vision: capabilities.vision,
            ocr: capabilities.ocr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualifiedIsolation {
    pub environment_cleared: bool,
    pub absolute_no_shell_execution: bool,
    pub process_tree_contained: bool,
    pub private_runtime_state: bool,
    pub loopback_only: bool,
    pub sandbox_attested: bool,
}

impl From<QualifiedIsolation> for IsolationCapabilities {
    fn from(capabilities: QualifiedIsolation) -> Self {
        Self {
            environment_cleared: capabilities.environment_cleared,
            absolute_no_shell_execution: capabilities.absolute_no_shell_execution,
            process_tree_contained: capabilities.process_tree_contained,
            private_runtime_state: capabilities.private_runtime_state,
            loopback_only: capabilities.loopback_only,
            sandbox_attested: capabilities.sandbox_attested,
        }
    }
}

impl From<IsolationCapabilities> for QualifiedIsolation {
    fn from(capabilities: IsolationCapabilities) -> Self {
        Self {
            environment_cleared: capabilities.environment_cleared,
            absolute_no_shell_execution: capabilities.absolute_no_shell_execution,
            process_tree_contained: capabilities.process_tree_contained,
            private_runtime_state: capabilities.private_runtime_state,
            loopback_only: capabilities.loopback_only,
            sandbox_attested: capabilities.sandbox_attested,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRecord {
    pub version: u32,
    pub fixture_version: String,
    #[serde(deserialize_with = "deserialize_canonical_provider_id")]
    pub provider_id: ProviderId,
    pub provider_class: ProviderClass,
    pub identity: ProviderIdentityHashes,
    pub declared_capabilities: DeclaredCapabilities,
    pub isolation_capabilities: QualifiedIsolation,
    pub qualification_status: QualificationStatus,
}

impl ProviderRecord {
    fn validate(&self) -> Result<(), ManifestError> {
        if self.version != PROVIDER_MANIFEST_VERSION {
            return Err(ManifestError::SchemaMismatch);
        }
        if self.fixture_version != FIXTURE_VERSION {
            return Err(ManifestError::FixtureMismatch);
        }
        self.identity.validate()?;
        if matches!(self.qualification_status, QualificationStatus::Qualified)
            && !self.declared_capabilities.any()
        {
            return Err(ManifestError::InvalidQualification);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderManifest {
    records: Vec<ProviderRecord>,
}

impl ProviderManifest {
    pub fn new(records: Vec<ProviderRecord>) -> Result<Self, ManifestError> {
        validate_records(&records)?;
        Ok(Self { records })
    }

    #[must_use]
    pub fn records(&self) -> &[ProviderRecord] {
        &self.records
    }

    #[must_use]
    pub fn record(&self, provider_id: &ProviderId) -> Option<&ProviderRecord> {
        self.records
            .iter()
            .find(|record| &record.provider_id == provider_id)
    }

    pub fn exact_qualified_record(
        &self,
        provider_id: &ProviderId,
        provider_class: ProviderClass,
        identity: &ProviderIdentityHashes,
        required: ProviderCapabilities,
    ) -> Result<&ProviderRecord, ManifestError> {
        let record = self
            .record(provider_id)
            .ok_or(ManifestError::QualificationMissing)?;
        if record.provider_class != provider_class || &record.identity != identity {
            return Err(ManifestError::IdentityMismatch);
        }
        if !matches!(record.qualification_status, QualificationStatus::Qualified) {
            return Err(ManifestError::NotQualified);
        }
        if !record.declared_capabilities.satisfies(required) {
            return Err(ManifestError::CapabilityMismatch);
        }
        Ok(record)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestDocument {
    LegacyV1(Box<QualificationReport>),
    V2(ProviderManifest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    MissingOrUnreadable,
    #[cfg(not(unix))]
    SecurityUnsupported,
    #[cfg(unix)]
    Symlink,
    #[cfg(unix)]
    NotRegularFile,
    Oversized,
    #[cfg(unix)]
    WrongOwner,
    #[cfg(unix)]
    WrongMode,
    #[cfg(unix)]
    MissingParent,
    #[cfg(unix)]
    ParentSymlink,
    #[cfg(unix)]
    ParentNotDirectory,
    #[cfg(unix)]
    ParentWrongOwner,
    #[cfg(unix)]
    ParentWrongMode,
    Malformed,
    SchemaMismatch,
    FixtureMismatch,
    DuplicateProvider,
    InvalidIdentity,
    InvalidQualification,
    QualificationMissing,
    IdentityMismatch,
    NotQualified,
    CapabilityMismatch,
    #[cfg(unix)]
    PublishFailed,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingOrUnreadable => "provider manifest is missing or unreadable",
            #[cfg(not(unix))]
            Self::SecurityUnsupported => {
                "owner-only provider manifest validation is unsupported on this platform"
            }
            #[cfg(unix)]
            Self::Symlink => "provider manifest must be a regular file, not a symlink",
            #[cfg(unix)]
            Self::NotRegularFile => "provider manifest must be a regular file",
            Self::Oversized => "provider manifest exceeds the 256 KiB limit",
            #[cfg(unix)]
            Self::WrongOwner => "provider manifest must be owned by the running user",
            #[cfg(unix)]
            Self::WrongMode => "provider manifest must have mode 0600",
            #[cfg(unix)]
            Self::MissingParent => "provider manifest state directory is missing",
            #[cfg(unix)]
            Self::ParentSymlink => "provider manifest state directory must not be a symlink",
            #[cfg(unix)]
            Self::ParentNotDirectory => "provider manifest parent must be a directory",
            #[cfg(unix)]
            Self::ParentWrongOwner => {
                "provider manifest state directory must be owned by the running user"
            }
            #[cfg(unix)]
            Self::ParentWrongMode => "provider manifest state directory must have mode 0700",
            Self::Malformed => "provider manifest is malformed",
            Self::SchemaMismatch => "provider manifest uses an unsupported schema version",
            Self::FixtureMismatch => "provider manifest uses a stale fixture version",
            Self::DuplicateProvider => "provider manifest contains duplicate provider identities",
            Self::InvalidIdentity => "provider manifest contains an invalid identity hash",
            Self::InvalidQualification => {
                "provider manifest contains an invalid qualification record"
            }
            Self::QualificationMissing => "provider manifest has no record for this provider",
            Self::IdentityMismatch => "provider manifest identity does not match this provider",
            Self::NotQualified => "provider manifest does not qualify this provider",
            Self::CapabilityMismatch => "provider manifest lacks a required provider capability",
            #[cfg(unix)]
            Self::PublishFailed => "provider manifest publication failed",
        })
    }
}

impl std::error::Error for ManifestError {}

pub fn decode_manifest(bytes: &[u8]) -> Result<ManifestDocument, ManifestError> {
    if bytes.len() as u64 > MAX_PROVIDER_MANIFEST_BYTES {
        return Err(ManifestError::Oversized);
    }
    match bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
    {
        Some(b'{') => {
            let report: QualificationReport =
                serde_json::from_slice(bytes).map_err(|_| ManifestError::Malformed)?;
            validate_legacy_report(&report)?;
            Ok(ManifestDocument::LegacyV1(Box::new(report)))
        }
        Some(b'[') => {
            let records: Vec<ProviderRecord> =
                serde_json::from_slice(bytes).map_err(|_| ManifestError::Malformed)?;
            Ok(ManifestDocument::V2(ProviderManifest::new(records)?))
        }
        _ => Err(ManifestError::Malformed),
    }
}

pub fn read_manifest(path: &Path) -> Result<ManifestDocument, ManifestError> {
    // Compatibility is about the serialized schema, not a weaker filesystem
    // boundary: legacy-v1 objects and v2 arrays share the same private state
    // directory and regular-file checks.
    let mut file = open_owner_only_manifest(path)?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_PROVIDER_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ManifestError::MissingOrUnreadable)?;
    if bytes.len() as u64 > MAX_PROVIDER_MANIFEST_BYTES {
        return Err(ManifestError::Oversized);
    }
    decode_manifest(&bytes)
}

pub fn publish_v2(path: &Path, records: &[ProviderRecord]) -> Result<(), ManifestError> {
    #[cfg(not(unix))]
    {
        let _ = (path, records);
        Err(ManifestError::SecurityUnsupported)
    }

    #[cfg(unix)]
    {
        let parent = path.parent().ok_or(ManifestError::MissingParent)?;
        ensure_state_directory(parent)?;
        validate_state_directory(parent, effective_user_id())?;
        validate_manifest_filename(path)?;
        if path.exists() || std::fs::symlink_metadata(path).is_ok() {
            validate_manifest_path(path, effective_user_id())?;
        }

        let mut ordered = records.to_vec();
        ordered.sort_by(|left, right| left.provider_id.as_str().cmp(right.provider_id.as_str()));
        validate_records(&ordered)?;
        let mut encoded =
            serde_json::to_vec_pretty(&ordered).map_err(|_| ManifestError::PublishFailed)?;
        encoded.push(b'\n');
        if encoded.len() as u64 > MAX_PROVIDER_MANIFEST_BYTES {
            return Err(ManifestError::Oversized);
        }

        let (temporary_path, mut temporary) = create_temporary_manifest(parent)?;
        let mut cleanup = TemporaryFile::new(temporary_path.clone());
        temporary
            .write_all(&encoded)
            .and_then(|()| temporary.sync_all())
            .map_err(|_| ManifestError::PublishFailed)?;
        drop(temporary);
        validate_manifest_path(&temporary_path, effective_user_id())?;
        // This rename is the commit point and the last fallible operation.
        // Keeping every validation/write failure before it guarantees that an
        // `Err` leaves the previous manifest untouched. A crash can retain the
        // old entry, which fails closed as stale rather than exposing a partial
        // new file.
        std::fs::rename(&temporary_path, path).map_err(|_| ManifestError::PublishFailed)?;
        cleanup.disarm();
        Ok(())
    }
}

#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> String {
    lower_hex(&Sha256::digest(bytes))
}

pub fn production_tool_schema_sha256() -> Result<String, ManifestError> {
    #[derive(Serialize)]
    struct ToolContract<'a> {
        name: &'a str,
        description: &'a str,
        parameters: &'a serde_json::Value,
    }

    let tools = crate::tools::production_tools();
    let contract: Vec<_> = tools
        .iter()
        .map(|tool| ToolContract {
            name: tool.name,
            description: tool.description,
            parameters: &tool.parameters,
        })
        .collect();
    serde_json::to_vec(&contract)
        .map(|encoded| sha256_bytes(&encoded))
        .map_err(|_| ManifestError::InvalidIdentity)
}

fn validate_records(records: &[ProviderRecord]) -> Result<(), ManifestError> {
    let mut provider_ids = HashSet::with_capacity(records.len());
    for record in records {
        record.validate()?;
        if !provider_ids.insert(record.provider_id.as_str()) {
            return Err(ManifestError::DuplicateProvider);
        }
    }
    Ok(())
}

fn deserialize_canonical_provider_id<'de, D>(deserializer: D) -> Result<ProviderId, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    let provider_id = ProviderId::parse(&raw).map_err(serde::de::Error::custom)?;
    if raw != provider_id.as_str() {
        return Err(serde::de::Error::custom(
            "provider manifest identity is not canonical",
        ));
    }
    Ok(provider_id)
}

fn validate_legacy_report(report: &QualificationReport) -> Result<(), ManifestError> {
    if report.version != QUALIFICATION_VERSION {
        return Err(ManifestError::SchemaMismatch);
    }
    if report.fixture_version != FIXTURE_VERSION {
        return Err(ManifestError::FixtureMismatch);
    }
    let providers = [&report.primary, &report.fm_server, &report.fm_cli];
    for provider in providers {
        for identity in [
            provider.identity.as_ref(),
            provider.vision_identity.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if identity.fixture_version != FIXTURE_VERSION {
                return Err(ManifestError::FixtureMismatch);
            }
        }
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), ManifestError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ManifestError::InvalidIdentity)
    }
}

fn validate_optional_sha256(value: Option<&str>) -> Result<(), ManifestError> {
    value.map_or(Ok(()), validate_sha256)
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

fn open_owner_only_manifest(path: &Path) -> Result<std::fs::File, ManifestError> {
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(ManifestError::SecurityUnsupported)
    }

    #[cfg(unix)]
    {
        let parent = path.parent().ok_or(ManifestError::MissingParent)?;
        validate_state_directory(parent, effective_user_id())?;
        validate_manifest_filename(path)?;
        validate_manifest_path(path, effective_user_id())?;
        let file = std::fs::File::open(path).map_err(|_| ManifestError::MissingOrUnreadable)?;
        validate_open_file(&file, effective_user_id())?;
        Ok(file)
    }
}

#[cfg(unix)]
fn validate_manifest_filename(path: &Path) -> Result<(), ManifestError> {
    let file_name = path.file_name().ok_or(ManifestError::NotRegularFile)?;
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return Err(ManifestError::NotRegularFile);
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_state_directory(path: &Path) -> Result<(), ManifestError> {
    use std::os::unix::fs::DirBuilderExt as _;

    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(path)
                .map_err(|_| ManifestError::PublishFailed)
        }
        Err(_) => Err(ManifestError::PublishFailed),
    }
}

#[cfg(unix)]
fn validate_state_directory(path: &Path, expected_uid: u32) -> Result<(), ManifestError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ManifestError::MissingParent
        } else {
            ManifestError::MissingOrUnreadable
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ManifestError::ParentSymlink);
    }
    if !metadata.is_dir() {
        return Err(ManifestError::ParentNotDirectory);
    }
    if metadata.uid() != expected_uid {
        return Err(ManifestError::ParentWrongOwner);
    }
    if metadata.mode() & 0o777 != 0o700 {
        return Err(ManifestError::ParentWrongMode);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_manifest_path(path: &Path, expected_uid: u32) -> Result<(), ManifestError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| ManifestError::MissingOrUnreadable)?;
    validate_manifest_metadata(&metadata, expected_uid)
}

#[cfg(unix)]
fn validate_open_file(file: &std::fs::File, expected_uid: u32) -> Result<(), ManifestError> {
    let metadata = file
        .metadata()
        .map_err(|_| ManifestError::MissingOrUnreadable)?;
    validate_manifest_metadata(&metadata, expected_uid)
}

#[cfg(unix)]
fn validate_manifest_metadata(
    metadata: &std::fs::Metadata,
    expected_uid: u32,
) -> Result<(), ManifestError> {
    use std::os::unix::fs::MetadataExt as _;

    if metadata.file_type().is_symlink() {
        return Err(ManifestError::Symlink);
    }
    if !metadata.is_file() {
        return Err(ManifestError::NotRegularFile);
    }
    if metadata.len() > MAX_PROVIDER_MANIFEST_BYTES {
        return Err(ManifestError::Oversized);
    }
    if metadata.uid() != expected_uid {
        return Err(ManifestError::WrongOwner);
    }
    if metadata.mode() & 0o777 != 0o600 {
        return Err(ManifestError::WrongMode);
    }
    Ok(())
}

#[cfg(unix)]
fn create_temporary_manifest(parent: &Path) -> Result<(PathBuf, std::fs::File), ManifestError> {
    use std::os::unix::fs::OpenOptionsExt as _;

    for _ in 0..32 {
        let serial = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".abbey-provider-manifest-{}-{serial}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(ManifestError::PublishFailed),
        }
    }
    Err(ManifestError::PublishFailed)
}

#[cfg(unix)]
fn effective_user_id() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` takes no arguments and has no preconditions.
    unsafe { geteuid() }
}

#[cfg(unix)]
struct TemporaryFile {
    path: Option<PathBuf>,
}

#[cfg(unix)]
impl TemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

#[cfg(unix)]
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
