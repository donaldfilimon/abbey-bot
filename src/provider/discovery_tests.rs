use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::*;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "abbey-provider-discovery-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create discovery test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn provider_id() -> ProviderId {
    ProviderId::parse("fixture-provider").expect("valid provider id")
}

fn request(candidate_paths: Vec<PathBuf>) -> DiscoveryRequest {
    DiscoveryRequest::new(
        provider_id(),
        ProviderClass::AgentCli,
        candidate_paths,
        ProviderCapabilities::text(),
        IsolationCapabilities::default(),
    )
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write fake provider");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .expect("make fake provider executable");
}

#[tokio::test]
async fn absent_and_multiple_candidates_are_never_probed() {
    let directory = TestDirectory::new();
    let absent = discover(
        request(vec![directory.path().join("missing-provider")]),
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(absent.descriptor().detection, DetectionState::NotDetected);
    assert_eq!(
        absent.descriptor().eligibility,
        Eligibility::Blocked(BlockedReason::NotDetected)
    );
    assert!(absent.executable_identity().is_none());

    let multiple = discover(
        request(vec![
            directory.path().join("candidate-a"),
            directory.path().join("candidate-b"),
        ]),
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(multiple.descriptor().detection, DetectionState::Ambiguous);
    assert_eq!(
        multiple.descriptor().eligibility,
        Eligibility::Blocked(BlockedReason::Ambiguous)
    );
    assert!(multiple.executable_identity().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn multiple_real_candidates_are_marked_ambiguous_without_execution() {
    let directory = TestDirectory::new();
    let sentinel = directory.path().join("must-not-exist");
    let first = directory.path().join("provider-a");
    let second = directory.path().join("provider-b");
    let body = format!("printf 'executed' > '{}'", sentinel.display());
    write_executable(&first, &body);
    write_executable(&second, &body);

    let result = discover(
        request(vec![first, second]),
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(result.descriptor().detection, DetectionState::Ambiguous);
    assert!(!sentinel.exists(), "ambiguous candidates must not execute");
}

#[tokio::test]
async fn an_exact_endpoint_registers_without_network_discovery() {
    let request = request(Vec::new()).with_discovery_boundary(DiscoveryBoundary::ExactEndpoint);
    let result = discover(request, DiscoveryLimits::default()).await;
    assert_eq!(result.descriptor().detection, DetectionState::Detected);
    assert_eq!(
        result.descriptor().eligibility,
        Eligibility::Blocked(BlockedReason::Unqualified)
    );
    assert_eq!(
        result.descriptor().discovery,
        DiscoveryBoundary::ExactEndpoint
    );
    assert!(result.executable_identity().is_none());
}

#[tokio::test]
async fn discovery_boundary_and_candidate_shape_must_agree() {
    let directory = TestDirectory::new();
    let missing = directory.path().join("provider");
    for request in [
        request(Vec::new()).with_discovery_boundary(DiscoveryBoundary::ExactBinary),
        request(vec![missing]).with_discovery_boundary(DiscoveryBoundary::ExactEndpoint),
        request(Vec::new()).with_discovery_boundary(DiscoveryBoundary::ExactEndpoint),
    ] {
        let result = discover(request, DiscoveryLimits::default()).await;
        assert_eq!(
            result.descriptor().detection,
            DetectionState::InvalidConfiguration
        );
    }
}

#[cfg(unix)]
#[test]
fn validated_settings_map_to_safe_boundary_shapes_without_copying_values() {
    let config = super::super::config::ProviderConfig::from_iter([
        ("ABBEY_PROVIDER_FM_BINARY", "/usr/bin/fm"),
        ("ABBEY_PROVIDER_HYBRID_BINARY", "/usr/bin/hybrid"),
        ("ABBEY_PROVIDER_HYBRID_ENDPOINT", "https://provider.invalid"),
        ("ABBEY_PROVIDER_REMOTE_ENDPOINT", "https://provider.invalid"),
    ])
    .expect("valid provider settings");

    let fm = DiscoveryRequest::from_settings(
        config.provider(&ProviderId::parse("fm").unwrap()).unwrap(),
        ProviderClass::OsManagedLocal,
        ProviderCapabilities::text(),
        IsolationCapabilities::default(),
    );
    assert_eq!(fm.discovery, DiscoveryBoundary::OsManaged);
    assert_eq!(fm.candidate_paths, [PathBuf::from("/usr/bin/fm")]);

    let hybrid = DiscoveryRequest::from_settings(
        config
            .provider(&ProviderId::parse("hybrid").unwrap())
            .unwrap(),
        ProviderClass::AgentCli,
        ProviderCapabilities::text(),
        IsolationCapabilities::default(),
    );
    assert_eq!(
        hybrid.discovery,
        DiscoveryBoundary::ExactBinaryAndEndpoint
    );

    let remote = DiscoveryRequest::from_settings(
        config
            .provider(&ProviderId::parse("remote").unwrap())
            .unwrap(),
        ProviderClass::Cloud,
        ProviderCapabilities::text(),
        IsolationCapabilities::default(),
    );
    assert_eq!(remote.discovery, DiscoveryBoundary::ExactEndpoint);
    let safe_debug = format!("{remote:?}");
    assert!(!safe_debug.contains("provider.invalid"));
}

#[tokio::test]
async fn non_exact_paths_fail_as_fixed_invalid_configuration() {
    let result = discover(
        request(vec![PathBuf::from("relative/provider")]),
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(
        result.descriptor().detection,
        DetectionState::InvalidConfiguration
    );
    assert_eq!(
        result.descriptor().eligibility,
        Eligibility::Blocked(BlockedReason::InvalidConfiguration)
    );
    assert!(result.executable_identity().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn one_exact_executable_is_hashed_and_version_probed_with_no_environment() {
    let directory = TestDirectory::new();
    let executable = directory.path().join("private-provider-canary");
    assert!(
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        "Cargo supplies the environment canary to the test process"
    );
    write_executable(
        &executable,
        r#"
if [ "${CARGO_MANIFEST_DIR+x}" = x ]; then
  exit 17
fi
printf 'private-version-canary\n'
"#,
    );

    let result = discover(
        request(vec![executable.clone()]),
        DiscoveryLimits::default(),
    )
    .await;
    assert_eq!(result.descriptor().detection, DetectionState::Detected);
    assert_eq!(
        result.descriptor().eligibility,
        Eligibility::Blocked(BlockedReason::Unqualified)
    );
    let identity = result
        .executable_identity()
        .expect("detected executable identity");
    assert_eq!(identity.path(), executable);
    assert_eq!(identity.binary_sha256(), hash_file(&executable).unwrap());
    assert_eq!(identity.binary_sha256().len(), 64);
    assert_eq!(identity.version_sha256().len(), 64);

    let safe_debug = format!("{result:?}");
    assert!(!safe_debug.contains("private-provider-canary"));
    assert!(!safe_debug.contains("private-version-canary"));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_directories_and_non_executables_are_invalid() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let executable = directory.path().join("provider");
    write_executable(&executable, "printf 'v1\\n'");
    let link = directory.path().join("provider-link");
    symlink(&executable, &link).expect("create provider symlink");
    let plain = directory.path().join("plain-provider");
    std::fs::write(&plain, "not executable").expect("write plain provider");

    for candidate in [link, directory.path().to_path_buf(), plain] {
        let result = discover(request(vec![candidate]), DiscoveryLimits::default()).await;
        assert_eq!(
            result.descriptor().detection,
            DetectionState::InvalidConfiguration
        );
        assert!(result.executable_identity().is_none());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn failed_oversized_and_timed_out_version_probes_fail_closed() {
    let directory = TestDirectory::new();
    let failed = directory.path().join("failed");
    write_executable(&failed, "exit 23");
    let oversized = directory.path().join("oversized");
    write_executable(
        &oversized,
        "i=0; while [ \"$i\" -lt 200 ]; do printf 'xxxxxxxx'; i=$((i + 1)); done",
    );
    let timed_out = directory.path().join("timed-out");
    write_executable(&timed_out, "while :; do :; done");

    for (candidate, limits) in [
        (failed, DiscoveryLimits::default()),
        (oversized, DiscoveryLimits::new(128, Duration::from_secs(1))),
        (
            timed_out,
            DiscoveryLimits::new(1024, Duration::from_millis(25)),
        ),
    ] {
        let result = discover(request(vec![candidate]), limits).await;
        assert_eq!(
            result.descriptor().detection,
            DetectionState::InvalidConfiguration
        );
        assert_eq!(
            result.descriptor().eligibility,
            Eligibility::Blocked(BlockedReason::InvalidConfiguration)
        );
        assert!(result.executable_identity().is_none());
    }
}

#[test]
fn caller_limits_cannot_remove_the_hard_bounds() {
    let limits = DiscoveryLimits::new(usize::MAX, Duration::from_secs(600));
    assert_eq!(limits.version_output_bytes(), MAX_VERSION_OUTPUT_BYTES);
    assert_eq!(limits.version_timeout(), MAX_VERSION_TIMEOUT);

    let minimum = DiscoveryLimits::new(0, Duration::ZERO);
    assert_eq!(minimum.version_output_bytes(), 1);
    assert_eq!(minimum.version_timeout(), Duration::from_millis(1));
}
