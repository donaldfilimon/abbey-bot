use super::*;
use crate::provider::{
    DeclaredCapabilities, DiscoveryBoundary, PROVIDER_MANIFEST_VERSION, ProviderIdentityHashes,
    ProviderManifest, QualifiedIsolation,
};

#[cfg(unix)]
const SANDBOX_RUNNER: &str = "/usr/bin/sandbox-exec";
#[cfg(windows)]
const SANDBOX_RUNNER: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

fn id(raw: &str) -> ProviderId {
    ProviderId::parse(raw).expect("valid provider id")
}

fn config(values: &[(&str, &str)]) -> ProviderConfig {
    ProviderConfig::from_iter(values.iter().copied()).expect("valid provider config")
}

fn endpoint_request(raw: &str, class: ProviderClass, sandbox_attested: bool) -> DiscoveryRequest {
    DiscoveryRequest::new(
        id(raw),
        class,
        Vec::new(),
        ProviderCapabilities::text_with_tools(),
        IsolationCapabilities {
            environment_cleared: true,
            absolute_no_shell_execution: true,
            process_tree_contained: true,
            private_runtime_state: true,
            loopback_only: matches!(class, ProviderClass::LocalServer),
            sandbox_attested,
        },
    )
    .with_discovery_boundary(DiscoveryBoundary::ExactEndpoint)
}

fn qualified_record(raw: &str, class: ProviderClass, sandbox_attested: bool) -> ProviderRecord {
    ProviderRecord {
        version: PROVIDER_MANIFEST_VERSION,
        fixture_version: crate::provider::FIXTURE_VERSION.to_string(),
        provider_id: id(raw),
        provider_class: class,
        identity: ProviderIdentityHashes {
            abbey_binary_sha256: "a".repeat(64),
            provider_binary_sha256: None,
            model_sha256: Some("b".repeat(64)),
            os_sha256: None,
            tool_schema_sha256: "c".repeat(64),
            sandbox_sha256: sandbox_attested.then(|| "d".repeat(64)),
        },
        declared_capabilities: DeclaredCapabilities {
            text: true,
            streaming: false,
            structured_output: true,
            tools: true,
            vision: false,
            ocr: false,
        },
        isolation_capabilities: QualifiedIsolation {
            environment_cleared: true,
            absolute_no_shell_execution: true,
            process_tree_contained: true,
            private_runtime_state: true,
            loopback_only: matches!(class, ProviderClass::LocalServer),
            sandbox_attested,
        },
        qualification_status: QualificationStatus::Qualified,
    }
}

fn apply_record(catalog: &mut ProviderCatalog, record: ProviderRecord) -> bool {
    let provider_id = record.provider_id.clone();
    let identity = record.identity.clone();
    let manifest = ProviderManifest::new(vec![record]).expect("valid provider manifest");
    catalog.apply_manifest(
        &provider_id,
        &manifest,
        &identity,
        ProviderCapabilities::text(),
    )
}

#[tokio::test]
async fn only_explicit_discovery_ids_are_registered_as_detected() {
    let config = config(&[("ABBEY_PROVIDER_DISCOVERY", "selected")]);
    let catalog = ProviderCatalog::discover_configured(
        &config,
        [
            endpoint_request("selected", ProviderClass::LocalServer, false),
            endpoint_request("not-selected", ProviderClass::LocalServer, false),
        ],
        DiscoveryLimits::default(),
    )
    .await;

    assert_eq!(
        catalog.descriptor(&id("selected")).unwrap().detection,
        DetectionState::Detected
    );
    let skipped = catalog.descriptor(&id("not-selected")).unwrap();
    assert_eq!(skipped.detection, DetectionState::NotDetected);
    assert_eq!(skipped.discovery, DiscoveryBoundary::Unconfigured);
    assert_eq!(
        skipped.eligibility,
        Eligibility::Blocked(BlockedReason::NotDetected)
    );
}

#[tokio::test]
async fn detected_providers_auto_register_but_remain_unqualified() {
    let config = config(&[("ABBEY_PROVIDER_DISCOVERY", "local")]);
    assert!(ProviderCatalog::new(&config).is_empty());
    let catalog = ProviderCatalog::discover_configured(
        &config,
        [endpoint_request("local", ProviderClass::LocalServer, false)],
        DiscoveryLimits::default(),
    )
    .await;
    let descriptor = catalog.descriptor(&id("local")).unwrap();
    assert_eq!(descriptor.detection, DetectionState::Detected);
    assert_eq!(
        descriptor.eligibility,
        Eligibility::Blocked(BlockedReason::Unqualified)
    );
    assert_eq!(descriptor.provenance, ProviderProvenance::Configuration);
    assert!(catalog.executable_identity(&id("local")).is_none());
    assert_eq!(catalog.routable().count(), 0);
}

#[tokio::test]
async fn empty_cloud_allowlist_denies_even_a_qualified_record() {
    let config = config(&[("ABBEY_PROVIDER_DISCOVERY", "cloud-one")]);
    let mut catalog = ProviderCatalog::discover_configured(
        &config,
        [endpoint_request("cloud-one", ProviderClass::Cloud, false)],
        DiscoveryLimits::default(),
    )
    .await;
    let record = qualified_record("cloud-one", ProviderClass::Cloud, false);
    assert!(apply_record(&mut catalog, record));
    let descriptor = catalog.descriptor(&id("cloud-one")).unwrap();
    assert_eq!(
        descriptor.eligibility,
        Eligibility::Blocked(BlockedReason::CloudNotAllowed)
    );
    assert_eq!(descriptor.provenance, ProviderProvenance::QualifiedManifest);
}

#[tokio::test]
async fn exact_qualification_and_allowlist_make_cloud_provider_routable() {
    let base = config(&[
        ("ABBEY_PROVIDER_DISCOVERY", "cloud-one"),
        ("ABBEY_PROVIDER_CLOUD_ALLOW", "cloud-one"),
    ]);
    let mut catalog = ProviderCatalog::discover_configured(
        &base,
        [endpoint_request("cloud-one", ProviderClass::Cloud, false)],
        DiscoveryLimits::default(),
    )
    .await;
    let record = qualified_record("cloud-one", ProviderClass::Cloud, false);
    apply_record(&mut catalog, record);
    assert_eq!(catalog.routable().count(), 1);
    assert_eq!(
        catalog.descriptor(&id("cloud-one")).unwrap().eligibility,
        Eligibility::Routable
    );

    let deny_again = config(&[("ABBEY_PROVIDER_DISCOVERY", "cloud-one")]);
    catalog.reapply_policy(&deny_again);
    assert_eq!(
        catalog.descriptor(&id("cloud-one")).unwrap().eligibility,
        Eligibility::Blocked(BlockedReason::CloudNotAllowed)
    );
}

#[tokio::test]
async fn agent_cli_requires_allowlist_configured_sandbox_and_attestation() {
    let no_allow = config(&[("ABBEY_PROVIDER_DISCOVERY", "strict-cli")]);
    let allowed_without_sandbox = config(&[
        ("ABBEY_PROVIDER_DISCOVERY", "strict-cli"),
        ("ABBEY_PROVIDER_AGENT_CLI_ALLOW", "strict-cli"),
    ]);
    let allowed_with_sandbox = config(&[
        ("ABBEY_PROVIDER_DISCOVERY", "strict-cli"),
        ("ABBEY_PROVIDER_AGENT_CLI_ALLOW", "strict-cli"),
        ("ABBEY_PROVIDER_SANDBOX_RUNNER", SANDBOX_RUNNER),
        ("ABBEY_PROVIDER_SANDBOX_PROFILE", "abbey-provider"),
    ]);

    for (config, attested, expected) in [
        (
            no_allow,
            true,
            Eligibility::Blocked(BlockedReason::InvalidConfiguration),
        ),
        (
            allowed_without_sandbox,
            true,
            Eligibility::Blocked(BlockedReason::InvalidConfiguration),
        ),
        (
            allowed_with_sandbox.clone(),
            false,
            Eligibility::Blocked(BlockedReason::InvalidConfiguration),
        ),
        (
            allowed_with_sandbox,
            true,
            Eligibility::Blocked(BlockedReason::InvalidConfiguration),
        ),
    ] {
        let mut catalog = ProviderCatalog::discover_configured(
            &config,
            [endpoint_request(
                "strict-cli",
                ProviderClass::AgentCli,
                attested,
            )],
            DiscoveryLimits::default(),
        )
        .await;
        let record = qualified_record("strict-cli", ProviderClass::AgentCli, attested);
        apply_record(&mut catalog, record);
        assert_eq!(
            catalog.descriptor(&id("strict-cli")).unwrap().eligibility,
            expected
        );
    }
}

#[tokio::test]
async fn operator_disablement_and_detection_state_cannot_be_bypassed() {
    let disabled_config = config(&[
        ("ABBEY_PROVIDER_DISCOVERY", "disabled"),
        ("ABBEY_PROVIDER_DISABLED", "disabled"),
    ]);
    let mut catalog = ProviderCatalog::discover_configured(
        &disabled_config,
        [endpoint_request(
            "disabled",
            ProviderClass::LocalServer,
            false,
        )],
        DiscoveryLimits::default(),
    )
    .await;
    let record = qualified_record("disabled", ProviderClass::LocalServer, false);
    apply_record(&mut catalog, record);
    assert_eq!(
        catalog.descriptor(&id("disabled")).unwrap().eligibility,
        Eligibility::Blocked(BlockedReason::OperatorDisabled)
    );

    let absent_config = config(&[]);
    let mut absent = ProviderCatalog::discover_configured(
        &absent_config,
        [endpoint_request(
            "absent",
            ProviderClass::LocalServer,
            false,
        )],
        DiscoveryLimits::default(),
    )
    .await;
    let absent_record = qualified_record("absent", ProviderClass::LocalServer, false);
    apply_record(&mut absent, absent_record);
    assert_eq!(
        absent.descriptor(&id("absent")).unwrap().eligibility,
        Eligibility::Blocked(BlockedReason::NotDetected)
    );
}

#[tokio::test]
async fn qualification_errors_map_to_fixed_ineligible_reasons() {
    let config = config(&[("ABBEY_PROVIDER_DISCOVERY", "local")]);
    let mut catalog = ProviderCatalog::discover_configured(
        &config,
        [endpoint_request("local", ProviderClass::LocalServer, false)],
        DiscoveryLimits::default(),
    )
    .await;
    for (error, reason) in [
        (
            ManifestError::QualificationMissing,
            BlockedReason::Unqualified,
        ),
        (ManifestError::NotQualified, BlockedReason::Unqualified),
        (
            ManifestError::IdentityMismatch,
            BlockedReason::IdentityMismatch,
        ),
        (
            ManifestError::CapabilityMismatch,
            BlockedReason::CapabilityUnavailable,
        ),
        (
            ManifestError::Malformed,
            BlockedReason::RequalificationRequired,
        ),
    ] {
        catalog.apply_manifest_result(&id("local"), Err(error));
        assert_eq!(
            catalog.descriptor(&id("local")).unwrap().eligibility,
            Eligibility::Blocked(reason)
        );
        assert_eq!(
            catalog.descriptor(&id("local")).unwrap().provenance,
            ProviderProvenance::Configuration
        );
    }
}

#[tokio::test]
async fn duplicate_provider_ids_become_one_ambiguous_entry() {
    let config = config(&[("ABBEY_PROVIDER_DISCOVERY", "duplicate")]);
    let catalog = ProviderCatalog::discover_configured(
        &config,
        [
            endpoint_request("duplicate", ProviderClass::LocalServer, false),
            endpoint_request("duplicate", ProviderClass::LocalServer, false),
        ],
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(catalog.len(), 1);
    assert_eq!(
        catalog.descriptor(&id("duplicate")).unwrap().detection,
        DetectionState::Ambiguous
    );
    assert_eq!(
        catalog.descriptor(&id("duplicate")).unwrap().eligibility,
        Eligibility::Blocked(BlockedReason::Ambiguous)
    );
}

#[tokio::test]
async fn descriptor_iteration_is_stable_and_debug_is_path_free() {
    let config = config(&[("ABBEY_PROVIDER_DISCOVERY", "zeta,alpha")]);
    let catalog = ProviderCatalog::discover_configured(
        &config,
        [
            endpoint_request("zeta", ProviderClass::LocalServer, false),
            endpoint_request("alpha", ProviderClass::LocalServer, false),
        ],
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(
        catalog
            .descriptors()
            .map(|descriptor| descriptor.id.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    let rendered = format!("{catalog:?}");
    assert!(!rendered.contains("ENDPOINT"));
    assert!(!rendered.contains("CREDENTIAL"));
}
