//! Provider qualification evidence and the owner-only runtime manifest gate.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{FmConfig, ProviderCapabilities};

pub const QUALIFICATION_VERSION: u32 = 1;
pub const FIXTURE_VERSION: &str = "abbey-provider-fixtures-v1";
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

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
}

impl CapabilityEvidence {
    pub fn pass() -> Self {
        Self {
            status: ProbeStatus::Pass,
            category: None,
        }
    }

    pub fn fail(category: &'static str) -> Self {
        Self {
            status: ProbeStatus::Fail,
            category: Some(category.to_string()),
        }
    }

    pub fn unsupported() -> Self {
        Self {
            status: ProbeStatus::Unsupported,
            category: None,
        }
    }

    pub fn skipped() -> Self {
        Self {
            status: ProbeStatus::Skipped,
            category: None,
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
    pub fm_cli: ProviderEvidence,
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
    Ok(format!("{:x}", digest.finalize()))
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

fn validate_owner_only_regular_file(path: &Path) -> Result<std::fs::Metadata, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "ABBEY_FM_CAPABILITY_MANIFEST is missing or unreadable".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("ABBEY_FM_CAPABILITY_MANIFEST must be a regular file, not a symlink".into());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("ABBEY_FM_CAPABILITY_MANIFEST exceeds the 256 KiB limit".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        if metadata.mode() & 0o077 != 0 {
            return Err("ABBEY_FM_CAPABILITY_MANIFEST must not be group- or world-readable".into());
        }
        // SAFETY: `geteuid` takes no arguments and has no preconditions.
        if metadata.uid() != unsafe { geteuid() } {
            return Err("ABBEY_FM_CAPABILITY_MANIFEST must be owned by the running user".into());
        }
    }
    Ok(metadata)
}

pub fn verify_fm_manifest(
    path: &Path,
    config: &FmConfig,
) -> Result<VerifiedFmCapabilities, String> {
    if matches!(config.mode, super::FmMode::Pcc) {
        return Err(
            "PCC remains intentionally unqualified; use ABBEY_FM_MODE=system or disable FM".into(),
        );
    }
    validate_owner_only_regular_file(path)?;
    let bytes = std::fs::read(path)
        .map_err(|_| "ABBEY_FM_CAPABILITY_MANIFEST is unreadable".to_string())?;
    let report: QualificationReport = serde_json::from_slice(&bytes)
        .map_err(|_| "ABBEY_FM_CAPABILITY_MANIFEST is malformed".to_string())?;
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
        return Err("ABBEY_FM_CAPABILITY_MANIFEST uses a stale fixture or format version".into());
    }
    let now = unix_now();
    if report.generated_unix_secs > now.saturating_add(300) {
        return Err("ABBEY_FM_CAPABILITY_MANIFEST has an invalid future timestamp".into());
    }
    if !report.overall_pass || !report.target.includes_fm() || !report.fm_cli.configured {
        return Err(
            "ABBEY_FM_CAPABILITY_MANIFEST does not record a successful FM qualification".into(),
        );
    }
    let expected = fm_identity(config)?;
    if report.fm_cli.identity.as_ref() != Some(&expected) {
        return Err(
            "ABBEY_FM_CAPABILITY_MANIFEST does not match this binary, FM executable, mode, or OS build"
                .into(),
        );
    }
    let cli = report.fm_cli.capabilities.capabilities();
    if !(cli.text && cli.structured_output && cli.tools) {
        return Err(
            "ABBEY_FM_CAPABILITY_MANIFEST lacks required FM CLI text/tool qualification".into(),
        );
    }
    if (cli.vision || cli.ocr) && report.fm_cli.vision_identity.as_ref() != Some(&expected) {
        return Err(
            "ABBEY_FM_CAPABILITY_MANIFEST does not bind its FM image qualification to this executable"
                .into(),
        );
    }
    let server = match config.endpoint.as_ref() {
        Some(_) => {
            if !report.fm_server.configured || report.fm_server.identity.as_ref() != Some(&expected)
            {
                return Err(
                    "ABBEY_FM_CAPABILITY_MANIFEST does not bind the configured FM server".into(),
                );
            }
            let capabilities = report.fm_server.capabilities.capabilities();
            if !(capabilities.text && capabilities.streaming) {
                return Err(
                    "ABBEY_FM_CAPABILITY_MANIFEST lacks required FM server qualification".into(),
                );
            }
            Some(capabilities)
        }
        None => None,
    };
    Ok(VerifiedFmCapabilities { server, cli })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);

    #[cfg(unix)]
    struct TestFiles {
        cli: PathBuf,
        manifest: PathBuf,
    }

    #[cfg(unix)]
    impl TestFiles {
        fn new() -> Self {
            use std::os::unix::fs::OpenOptionsExt as _;

            let serial = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir();
            let cli = root.join(format!(
                ".abbey-qualification-test-{}-{serial}-fm",
                std::process::id()
            ));
            let manifest = root.join(format!(
                ".abbey-qualification-test-{}-{serial}.json",
                std::process::id()
            ));
            let mut cli_file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&cli)
                .unwrap();
            cli_file.write_all(b"synthetic fm executable").unwrap();
            Self { cli, manifest }
        }

        fn config(&self) -> FmConfig {
            FmConfig {
                mode: super::super::FmMode::System,
                endpoint: None,
                cli: self.cli.clone(),
                fallback: true,
                timeout_secs: 30,
            }
        }

        fn write_report(&self, report: &QualificationReport, mode: u32) {
            use std::os::unix::fs::OpenOptionsExt as _;

            let _ = std::fs::remove_file(&self.manifest);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(&self.manifest)
                .unwrap();
            serde_json::to_writer(&mut file, report).unwrap();
            file.flush().unwrap();
        }

        fn write_raw(&self, bytes: &[u8], mode: u32) {
            use std::os::unix::fs::OpenOptionsExt as _;

            let _ = std::fs::remove_file(&self.manifest);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(&self.manifest)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.flush().unwrap();
        }
    }

    #[cfg(unix)]
    impl Drop for TestFiles {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.cli);
            let _ = std::fs::remove_file(&self.manifest);
        }
    }

    #[cfg(unix)]
    fn successful_fm_report(config: &FmConfig) -> QualificationReport {
        let passing = CapabilityEvidenceSet {
            text: CapabilityEvidence::pass(),
            streaming: CapabilityEvidence::unsupported(),
            structured_output: CapabilityEvidence::pass(),
            tools: CapabilityEvidence::pass(),
            vision: CapabilityEvidence::pass(),
            ocr: CapabilityEvidence::pass(),
        };
        QualificationReport {
            version: QUALIFICATION_VERSION,
            fixture_version: FIXTURE_VERSION.into(),
            generated_unix_secs: unix_now(),
            target: QualificationTarget::Fm,
            overall_pass: true,
            primary: ProviderEvidence::skipped(),
            fm_server: ProviderEvidence::skipped(),
            fm_cli: ProviderEvidence {
                configured: true,
                identity: Some(fm_identity(config).unwrap()),
                vision_identity: Some(fm_identity(config).unwrap()),
                capabilities: passing,
            },
        }
    }

    #[test]
    fn evidence_maps_only_pass_to_runtime_capability() {
        let set = CapabilityEvidenceSet {
            text: CapabilityEvidence::pass(),
            streaming: CapabilityEvidence::unsupported(),
            structured_output: CapabilityEvidence::fail("schema_mismatch"),
            tools: CapabilityEvidence::skipped(),
            vision: CapabilityEvidence::pass(),
            ocr: CapabilityEvidence::pass(),
        };
        assert_eq!(
            set.capabilities(),
            ProviderCapabilities {
                text: true,
                streaming: false,
                structured_output: false,
                tools: false,
                vision: true,
                ocr: true,
            }
        );
    }

    #[test]
    fn report_serialization_contains_no_provider_payload_fields() {
        let encoded = serde_json::to_string(&QualificationReport {
            version: QUALIFICATION_VERSION,
            fixture_version: FIXTURE_VERSION.into(),
            generated_unix_secs: 1,
            target: QualificationTarget::Fm,
            overall_pass: false,
            primary: ProviderEvidence::skipped(),
            fm_server: ProviderEvidence::skipped(),
            fm_cli: ProviderEvidence::skipped(),
        })
        .unwrap();
        assert!(!encoded.contains("vision_identity"), "{encoded}");
        for forbidden in ["prompt", "response_body", "image_bytes", "environment"] {
            assert!(
                !encoded.contains(forbidden),
                "leaked field {forbidden}: {encoded}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn manifest_requires_owner_only_exact_identity() {
        use std::os::unix::fs::PermissionsExt as _;

        let files = TestFiles::new();
        let config = files.config();
        let report = successful_fm_report(&config);
        files.write_report(&report, 0o600);
        let verified = verify_fm_manifest(&files.manifest, &config).expect("exact report");
        assert!(verified.cli.vision && verified.cli.ocr && verified.cli.tools);

        let mut mismatched = report.clone();
        mismatched
            .fm_cli
            .identity
            .as_mut()
            .unwrap()
            .abbey_binary_sha256 = "0".repeat(64);
        files.write_report(&mismatched, 0o600);
        assert!(verify_fm_manifest(&files.manifest, &config).is_err());

        files.write_report(&report, 0o600);
        std::fs::set_permissions(&files.manifest, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = verify_fm_manifest(&files.manifest, &config).unwrap_err();
        assert!(error.contains("group- or world-readable"), "{error}");
    }

    #[test]
    #[cfg(unix)]
    fn manifest_fails_closed_for_every_stale_or_incomplete_shape() {
        use std::os::unix::fs::symlink;

        let files = TestFiles::new();
        let config = files.config();
        let report = successful_fm_report(&config);

        assert!(verify_fm_manifest(&files.manifest, &config).is_err());

        symlink(&files.cli, &files.manifest).unwrap();
        let error = verify_fm_manifest(&files.manifest, &config).unwrap_err();
        assert!(error.contains("symlink"), "{error}");
        std::fs::remove_file(&files.manifest).unwrap();

        files.write_raw(b"{not json", 0o600);
        assert!(
            verify_fm_manifest(&files.manifest, &config)
                .unwrap_err()
                .contains("malformed")
        );

        let mut cases = Vec::new();
        let mut wrong_version = report.clone();
        wrong_version.version += 1;
        cases.push(wrong_version);
        let mut wrong_fixture = report.clone();
        wrong_fixture.fixture_version = "old-fixture".into();
        cases.push(wrong_fixture);
        let mut wrong_binary = report.clone();
        wrong_binary
            .fm_cli
            .identity
            .as_mut()
            .unwrap()
            .abbey_binary_sha256 = "0".repeat(64);
        cases.push(wrong_binary);
        let mut wrong_cli_hash = report.clone();
        wrong_cli_hash.fm_cli.identity.as_mut().unwrap().cli_sha256 = Some("1".repeat(64));
        cases.push(wrong_cli_hash);
        let mut wrong_vision_cli_hash = report.clone();
        wrong_vision_cli_hash
            .fm_cli
            .vision_identity
            .as_mut()
            .unwrap()
            .cli_sha256 = Some("2".repeat(64));
        cases.push(wrong_vision_cli_hash);
        let mut missing_vision_identity = report.clone();
        missing_vision_identity.fm_cli.vision_identity = None;
        cases.push(missing_vision_identity);
        let mut wrong_cli_path = report.clone();
        wrong_cli_path.fm_cli.identity.as_mut().unwrap().cli_path =
            Some(PathBuf::from("/different/fm"));
        cases.push(wrong_cli_path);
        let mut wrong_mode = report.clone();
        wrong_mode.fm_cli.identity.as_mut().unwrap().mode = Some("pcc".into());
        cases.push(wrong_mode);
        let mut wrong_os = report.clone();
        wrong_os.fm_cli.identity.as_mut().unwrap().os_build = "different-build".into();
        cases.push(wrong_os);
        let mut failed_report = report.clone();
        failed_report.overall_pass = false;
        cases.push(failed_report);
        let mut wrong_target = report.clone();
        wrong_target.target = QualificationTarget::Primary;
        cases.push(wrong_target);
        let mut missing_tool_evidence = report.clone();
        missing_tool_evidence.fm_cli.capabilities.tools = CapabilityEvidence::fail("tool_protocol");
        cases.push(missing_tool_evidence);
        let mut future = report.clone();
        future.generated_unix_secs = unix_now().saturating_add(3_600);
        cases.push(future);

        for case in cases {
            files.write_report(&case, 0o600);
            assert!(
                verify_fm_manifest(&files.manifest, &config).is_err(),
                "unsafe manifest unexpectedly qualified: {case:?}"
            );
        }
    }
}
