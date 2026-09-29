use super::*;
#[cfg(unix)]
use std::io::Write as _;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
struct TestFiles {
    root: PathBuf,
    cli: PathBuf,
    manifest: PathBuf,
}

#[cfg(unix)]
impl TestFiles {
    fn new() -> Self {
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};

        let serial = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            ".abbey-qualification-test-{}-{serial}",
            std::process::id()
        ));
        let mut root_builder = std::fs::DirBuilder::new();
        root_builder.mode(0o700);
        root_builder.create(&root).unwrap();
        let cli = root.join(format!("fm-{}-{serial}", std::process::id(),));
        let manifest = root.join(format!("manifest-{}-{serial}.json", std::process::id(),));
        let mut cli_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&cli)
            .unwrap();
        cli_file.write_all(b"synthetic fm executable").unwrap();
        Self {
            root,
            cli,
            manifest,
        }
    }

    fn config(&self) -> FmConfig {
        FmConfig {
            mode: super::super::FmMode::System,
            endpoint: None,
            cli: self.cli.clone(),
            fallback: true,
            primary: false,
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
        let _ = std::fs::remove_dir_all(&self.root);
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
        fm_cli_modes: Vec::new(),
        fm_manifest_identity: None,
    }
}

#[cfg(unix)]
fn successful_v2_fm_record(config: &FmConfig) -> super::super::ProviderRecord {
    super::super::ProviderRecord {
        qualification_run_nonce: None,
        qualification_generation: None,
        qualification_completed_unix_secs: None,
        version: super::super::PROVIDER_MANIFEST_VERSION,
        fixture_version: FIXTURE_VERSION.to_string(),
        provider_id: super::super::ProviderId::parse(FOUNDATION_MODELS_PROVIDER_ID).unwrap(),
        provider_class: super::super::ProviderClass::OsManagedLocal,
        identity: fm_manifest_identity(config).unwrap(),
        declared_capabilities: super::super::DeclaredCapabilities {
            text: true,
            streaming: false,
            structured_output: true,
            tools: true,
            vision: true,
            ocr: true,
        },
        isolation_capabilities: super::super::QualifiedIsolation {
            environment_cleared: true,
            absolute_no_shell_execution: true,
            process_tree_contained: false,
            private_runtime_state: true,
            loopback_only: false,
            sandbox_attested: false,
        },
        qualification_status: QualificationStatus::Qualified,
        score_policy: None,
        score_profiles: None,
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
fn tools_pass_records_marker_other_constructors_do_not() {
    let evidence = CapabilityEvidence::tools_pass("ABBEY_PROVIDER_CONTINUATION_V1");
    assert!(evidence.passed());
    assert_eq!(
        evidence.tool_result_marker.as_deref(),
        Some("ABBEY_PROVIDER_CONTINUATION_V1")
    );
    let round_tripped: CapabilityEvidence =
        serde_json::from_value(serde_json::to_value(&evidence).unwrap()).unwrap();
    assert_eq!(round_tripped, evidence);

    for evidence in [
        CapabilityEvidence::pass(),
        CapabilityEvidence::fail("tool_protocol"),
        CapabilityEvidence::unsupported(),
        CapabilityEvidence::skipped(),
    ] {
        assert_eq!(evidence.tool_result_marker, None);
    }
}

#[test]
fn tool_result_marker_defaults_to_none_when_absent_from_the_wire() {
    let decoded: CapabilityEvidence = serde_json::from_str(r#"{"status":"pass"}"#).unwrap();
    assert_eq!(decoded.tool_result_marker, None);
    assert!(decoded.passed());
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
        fm_cli_modes: Vec::new(),
        fm_manifest_identity: None,
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
fn v2_manifest_qualifies_the_same_exact_fm_identity_without_dynamic_eligibility() {
    let files = TestFiles::new();
    let config = files.config();
    let record = successful_v2_fm_record(&config);
    super::super::publish_v2(&files.manifest, std::slice::from_ref(&record)).unwrap();

    let verified = verify_fm_manifest(&files.manifest, &config).expect("exact v2 record");
    assert!(verified.cli.text);
    assert!(verified.cli.structured_output);
    assert!(verified.cli.tools);
    assert!(verified.cli.vision);
    assert!(verified.cli.ocr);
    assert!(!verified.cli.streaming);
    assert!(verified.server.is_none());

    let mut mismatched = record;
    mismatched.identity.os_sha256 = Some("0".repeat(64));
    super::super::publish_v2(&files.manifest, &[mismatched]).unwrap();
    let error = verify_fm_manifest(&files.manifest, &config).unwrap_err();
    assert!(error.contains("identity does not match"), "{error}");
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

#[cfg(unix)]
fn pcc(files: &TestFiles) -> FmConfig {
    FmConfig {
        mode: FmMode::Pcc,
        ..files.config()
    }
}

#[test]
#[cfg(unix)]
fn pcc_manifest_record_verifies() {
    let files = TestFiles::new();
    let system = files.config();
    let pcc = pcc(&files);
    let system_record = successful_v2_fm_record(&system);
    super::super::publish_v2(&files.manifest, std::slice::from_ref(&system_record)).unwrap();
    // A system record never qualifies PCC: its record is simply missing.
    assert_eq!(
        qualify_fm(Some(&files.manifest), &pcc),
        (None, FmQualificationState::Missing)
    );

    let mut pcc_record = successful_v2_fm_record(&pcc);
    pcc_record.provider_id =
        super::super::ProviderId::parse(FOUNDATION_MODELS_PCC_PROVIDER_ID).unwrap();
    super::super::publish_v2(&files.manifest, &[system_record, pcc_record]).unwrap();
    let verified = verify_fm_manifest(&files.manifest, &pcc).expect("exact PCC record");
    assert!(verified.cli.text && verified.cli.tools && verified.cli.structured_output);
    assert_eq!(
        qualify_fm(Some(&files.manifest), &pcc).1,
        FmQualificationState::Qualified
    );
    assert_eq!(
        qualify_fm(Some(&files.manifest), &system).1,
        FmQualificationState::Qualified
    );
    assert_eq!(fm_record_id(FmMode::Pcc), "foundation-models-pcc");
    assert_eq!(fm_record_id(FmMode::System), "foundation-models");
}

#[test]
#[cfg(unix)]
fn stale_manifest_degrades_not_errors() {
    let files = TestFiles::new();
    let config = files.config();
    assert_eq!(
        qualify_fm(None, &config),
        (None, FmQualificationState::Missing)
    );
    assert_eq!(
        qualify_fm(Some(&files.manifest), &config),
        (None, FmQualificationState::Missing),
        "absent file"
    );

    let report = successful_fm_report(&config);
    let mut stale = report.clone();
    stale.fixture_version = "old-fixture".into();
    let mut changed = report.clone();
    changed.fm_cli.identity.as_mut().unwrap().cli_sha256 = Some("1".repeat(64));
    let mut os_update = report.clone();
    os_update.fm_cli.identity.as_mut().unwrap().os_build = "different-build".into();
    let mut failed = report.clone();
    failed.overall_pass = false;
    for (case, expected) in [
        (stale, FmQualificationState::Stale),
        (changed, FmQualificationState::IdentityChanged),
        (os_update, FmQualificationState::IdentityChanged),
    ] {
        files.write_report(&case, 0o600);
        assert_eq!(qualify_fm(Some(&files.manifest), &config), (None, expected));
    }
    files.write_report(&failed, 0o600);
    assert!(matches!(
        qualify_fm(Some(&files.manifest), &config),
        (None, FmQualificationState::Refused(reason)) if reason.contains("successful FM qualification")
    ));

    let mut record = successful_v2_fm_record(&config);
    record.identity.os_sha256 = Some("0".repeat(64));
    super::super::publish_v2(&files.manifest, &[record]).unwrap();
    assert_eq!(
        qualify_fm(Some(&files.manifest), &config),
        (None, FmQualificationState::IdentityChanged)
    );
    assert_eq!(
        FmQualificationState::IdentityChanged.as_str(),
        "identity_changed"
    );
}

#[test]
#[cfg(unix)]
fn fm_vision_still_requires_manifest() {
    // Vision uses the strict verifier: every degraded state is still an error.
    let files = TestFiles::new();
    let config = files.config();
    assert!(verify_fm_manifest(&files.manifest, &config).is_err());
    let mut stale = successful_fm_report(&config);
    stale.fixture_version = "old-fixture".into();
    files.write_report(&stale, 0o600);
    let error = verify_fm_manifest(&files.manifest, &config).unwrap_err();
    assert!(error.contains("stale fixture"), "{error}");
    let mut changed = successful_fm_report(&config);
    changed.fm_cli.identity.as_mut().unwrap().cli_sha256 = Some("1".repeat(64));
    files.write_report(&changed, 0o600);
    let error = verify_fm_manifest(&files.manifest, &config).unwrap_err();
    assert!(error.contains("does not match"), "{error}");
}
