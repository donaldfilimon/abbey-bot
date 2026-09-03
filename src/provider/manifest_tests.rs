use super::*;
use crate::provider::qualification::{
    ProviderEvidence, QualificationReport, QualificationTarget, unix_now,
};

fn hash(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn record(id: &str, class: ProviderClass) -> ProviderRecord {
    ProviderRecord {
        version: PROVIDER_MANIFEST_VERSION,
        fixture_version: FIXTURE_VERSION.to_string(),
        provider_id: ProviderId::parse(id).expect("valid provider id"),
        provider_class: class,
        identity: ProviderIdentityHashes {
            abbey_binary_sha256: hash(0x11),
            provider_binary_sha256: Some(hash(0x22)),
            model_sha256: Some(hash(0x33)),
            os_sha256: Some(hash(0x44)),
            tool_schema_sha256: hash(0x55),
            sandbox_sha256: Some(hash(0x66)),
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
            loopback_only: false,
            sandbox_attested: true,
        },
        qualification_status: QualificationStatus::Qualified,
    }
}

fn legacy_report() -> QualificationReport {
    QualificationReport {
        version: QUALIFICATION_VERSION,
        fixture_version: FIXTURE_VERSION.to_string(),
        generated_unix_secs: unix_now(),
        target: QualificationTarget::Fm,
        overall_pass: true,
        primary: ProviderEvidence::skipped(),
        fm_server: ProviderEvidence::skipped(),
        fm_cli: ProviderEvidence::skipped(),
    }
}

#[test]
fn legacy_v1_object_and_v2_array_are_distinct_compatible_documents() {
    let legacy = serde_json::to_vec(&legacy_report()).expect("serialize v1 report");
    assert!(matches!(
        decode_manifest(&legacy).expect("read real v1 object"),
        ManifestDocument::LegacyV1(_)
    ));

    let v2 = serde_json::to_vec(&vec![record("mlx", ProviderClass::LocalServer)])
        .expect("serialize v2 records");
    let ManifestDocument::V2(manifest) = decode_manifest(&v2).expect("read v2 array") else {
        panic!("v2 array decoded as legacy object");
    };
    assert_eq!(manifest.records().len(), 1);
    assert_eq!(manifest.records()[0].provider_id.as_str(), "mlx");
}

#[test]
fn exact_qualification_requires_identity_class_status_and_capabilities() {
    let provider = record("foundation-models", ProviderClass::OsManagedLocal);
    let id = provider.provider_id.clone();
    let identity = provider.identity.clone();
    let manifest = ProviderManifest::new(vec![provider.clone()]).expect("valid manifest");
    let required = ProviderCapabilities {
        text: true,
        streaming: false,
        structured_output: true,
        tools: true,
        vision: false,
        ocr: false,
    };
    assert_eq!(
        manifest
            .exact_qualified_record(&id, ProviderClass::OsManagedLocal, &identity, required,)
            .expect("exact qualification"),
        &provider
    );

    let mut wrong_identity = identity.clone();
    wrong_identity.os_sha256 = Some(hash(0xaa));
    assert_eq!(
        manifest
            .exact_qualified_record(
                &id,
                ProviderClass::OsManagedLocal,
                &wrong_identity,
                required,
            )
            .unwrap_err(),
        ManifestError::IdentityMismatch
    );
    assert_eq!(
        manifest
            .exact_qualified_record(&id, ProviderClass::Cloud, &identity, required)
            .unwrap_err(),
        ManifestError::IdentityMismatch
    );

    let mut failed = provider.clone();
    failed.qualification_status = QualificationStatus::Failed;
    let failed_manifest = ProviderManifest::new(vec![failed]).expect("valid failed evidence");
    assert_eq!(
        failed_manifest
            .exact_qualified_record(&id, ProviderClass::OsManagedLocal, &identity, required,)
            .unwrap_err(),
        ManifestError::NotQualified
    );

    let missing = ProviderCapabilities {
        vision: true,
        ..required
    };
    assert_eq!(
        manifest
            .exact_qualified_record(&id, ProviderClass::OsManagedLocal, &identity, missing,)
            .unwrap_err(),
        ManifestError::CapabilityMismatch
    );
}

#[test]
fn v2_rejects_schema_fixture_identity_duplicate_and_unknown_content() {
    let base = record("mlx", ProviderClass::LocalServer);

    let mut wrong_schema = base.clone();
    wrong_schema.version = 1;
    assert_eq!(
        ProviderManifest::new(vec![wrong_schema]).unwrap_err(),
        ManifestError::SchemaMismatch
    );

    let mut wrong_fixture = base.clone();
    wrong_fixture.fixture_version = "stale-fixture".to_string();
    assert_eq!(
        ProviderManifest::new(vec![wrong_fixture]).unwrap_err(),
        ManifestError::FixtureMismatch
    );

    let mut wrong_hash = base.clone();
    wrong_hash.identity.model_sha256 = Some("A".repeat(64));
    assert_eq!(
        ProviderManifest::new(vec![wrong_hash]).unwrap_err(),
        ManifestError::InvalidIdentity
    );

    assert_eq!(
        ProviderManifest::new(vec![base.clone(), base.clone()]).unwrap_err(),
        ManifestError::DuplicateProvider
    );

    let mut encoded = serde_json::to_value(vec![base]).expect("serialize records");
    encoded[0].as_object_mut().expect("record object").insert(
        "provider_error".to_string(),
        serde_json::Value::String("PRIVATE_PROVIDER_RESPONSE".to_string()),
    );
    assert_eq!(
        decode_manifest(&serde_json::to_vec(&encoded).expect("encode mutated record")).unwrap_err(),
        ManifestError::Malformed
    );

    let mut noncanonical = encoded;
    noncanonical[0]["provider_id"] = serde_json::Value::String("MLX_LOCAL".to_string());
    noncanonical[0]
        .as_object_mut()
        .expect("record object")
        .remove("provider_error");
    assert_eq!(
        decode_manifest(&serde_json::to_vec(&noncanonical).expect("encode noncanonical identity"))
            .unwrap_err(),
        ManifestError::Malformed
    );
}

#[test]
fn v2_serialization_is_content_free() {
    let encoded = serde_json::to_string(&vec![record("mlx", ProviderClass::LocalServer)])
        .expect("serialize records");
    for forbidden_field in [
        "prompt",
        "response",
        "credential",
        "environment",
        "endpoint",
        "executable_path",
        "model_path",
        "provider_error",
        "eligibility",
    ] {
        assert!(
            !encoded.contains(&format!("\"{forbidden_field}\"")),
            "leaked field {forbidden_field}: {encoded}"
        );
    }
    for forbidden_canary in [
        "PRIVATE_PROMPT_CANARY",
        "PRIVATE_RESPONSE_CANARY",
        "PRIVATE_CREDENTIAL_CANARY",
    ] {
        assert!(
            !encoded.contains(forbidden_canary),
            "leaked {forbidden_canary}: {encoded}"
        );
    }
}

#[test]
fn tool_schema_hash_is_stable_lowercase_sha256() {
    let first = production_tool_schema_sha256().expect("hash production tools");
    let second = production_tool_schema_sha256().expect("hash production tools again");
    assert_eq!(first, second);
    validate_sha256(&first).expect("valid schema hash");
}

#[cfg(not(unix))]
#[test]
fn owner_only_file_publication_fails_closed_without_posix_mode_semantics() {
    let path = std::path::Path::new("provider-manifest-not-created.json");
    assert_eq!(
        publish_v2(path, &[record("mlx", ProviderClass::LocalServer)]).unwrap_err(),
        ManifestError::SecurityUnsupported
    );
    assert_eq!(
        read_manifest(path).unwrap_err(),
        ManifestError::SecurityUnsupported
    );
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot {
        root: PathBuf,
        state: PathBuf,
        manifest: PathBuf,
    }

    impl TestRoot {
        fn new(create_state: bool) -> Self {
            let serial = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "abbey-provider-manifest-test-{}-{serial}",
                std::process::id()
            ));
            std::fs::create_dir(&root).expect("create test root");
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .expect("make root private");
            let state = root.join("state");
            if create_state {
                std::fs::create_dir(&state).expect("create state directory");
                std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))
                    .expect("make state private");
            }
            let manifest = state.join("providers.json");
            Self {
                root,
                state,
                manifest,
            }
        }

        fn write_raw(&self, bytes: &[u8], mode: u32) {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(&self.manifest)
                .expect("create manifest");
            file.write_all(bytes).expect("write manifest");
            file.sync_all().expect("sync manifest");
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.state, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn publication_creates_private_state_and_sorted_v2_array() {
        let files = TestRoot::new(false);
        let z = record("zeta", ProviderClass::Cloud);
        let a = record("alpha", ProviderClass::LocalServer);
        publish_v2(&files.manifest, &[z, a]).expect("publish v2 manifest");

        let state_mode = std::fs::metadata(&files.state)
            .expect("state metadata")
            .permissions()
            .mode()
            & 0o777;
        let manifest_mode = std::fs::metadata(&files.manifest)
            .expect("manifest metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(state_mode, 0o700);
        assert_eq!(manifest_mode, 0o600);

        let bytes = std::fs::read(&files.manifest).expect("read published manifest");
        assert_eq!(
            bytes.iter().copied().find(|b| !b.is_ascii_whitespace()),
            Some(b'[')
        );
        let ManifestDocument::V2(manifest) =
            read_manifest(&files.manifest).expect("secure v2 read")
        else {
            panic!("published v2 array decoded as legacy v1");
        };
        let ids: Vec<_> = manifest
            .records()
            .iter()
            .map(|entry| entry.provider_id.as_str())
            .collect();
        assert_eq!(ids, ["alpha", "zeta"]);
    }

    #[test]
    fn failed_publication_preserves_previous_manifest_byte_for_byte() {
        let files = TestRoot::new(true);
        let original = record("mlx", ProviderClass::LocalServer);
        publish_v2(&files.manifest, &[original]).expect("publish original manifest");
        let before = std::fs::read(&files.manifest).expect("read original manifest");

        let mut invalid = record("ollama", ProviderClass::LocalServer);
        invalid.identity.sandbox_sha256 = Some("not-a-hash".to_string());
        assert_eq!(
            publish_v2(&files.manifest, &[invalid]).unwrap_err(),
            ManifestError::InvalidIdentity
        );
        assert_eq!(
            std::fs::read(&files.manifest).expect("read preserved manifest"),
            before
        );
        assert!(
            std::fs::read_dir(&files.state)
                .expect("read state directory")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn reads_fail_closed_for_symlinks_modes_owners_and_oversize() {
        let files = TestRoot::new(true);
        let encoded = serde_json::to_vec(&vec![record("mlx", ProviderClass::LocalServer)])
            .expect("serialize manifest");
        files.write_raw(&encoded, 0o600);

        assert_eq!(
            validate_manifest_path(&files.manifest, effective_user_id().wrapping_add(1))
                .unwrap_err(),
            ManifestError::WrongOwner
        );
        assert_eq!(
            validate_state_directory(&files.state, effective_user_id().wrapping_add(1))
                .unwrap_err(),
            ManifestError::ParentWrongOwner
        );

        std::fs::set_permissions(&files.manifest, std::fs::Permissions::from_mode(0o640))
            .expect("set unsafe manifest mode");
        assert_eq!(
            read_manifest(&files.manifest).unwrap_err(),
            ManifestError::WrongMode
        );
        std::fs::set_permissions(&files.manifest, std::fs::Permissions::from_mode(0o600))
            .expect("restore manifest mode");

        std::fs::set_permissions(&files.state, std::fs::Permissions::from_mode(0o750))
            .expect("set unsafe state mode");
        assert_eq!(
            read_manifest(&files.manifest).unwrap_err(),
            ManifestError::ParentWrongMode
        );
        std::fs::set_permissions(&files.state, std::fs::Permissions::from_mode(0o700))
            .expect("restore state mode");

        std::fs::remove_file(&files.manifest).expect("remove manifest");
        std::os::unix::fs::symlink(files.root.join("missing"), &files.manifest)
            .expect("create manifest symlink");
        assert_eq!(
            read_manifest(&files.manifest).unwrap_err(),
            ManifestError::Symlink
        );
        std::fs::remove_file(&files.manifest).expect("remove symlink");

        let oversized = vec![b' '; (MAX_PROVIDER_MANIFEST_BYTES + 1) as usize];
        files.write_raw(&oversized, 0o600);
        assert_eq!(
            read_manifest(&files.manifest).unwrap_err(),
            ManifestError::Oversized
        );
    }

    #[test]
    fn a_symlinked_state_directory_is_never_followed() {
        let files = TestRoot::new(true);
        let linked_state = files.root.join("linked-state");
        std::os::unix::fs::symlink(&files.state, &linked_state).expect("create state symlink");
        let linked_manifest = linked_state.join("providers.json");
        assert_eq!(
            publish_v2(
                &linked_manifest,
                &[record("mlx", ProviderClass::LocalServer)]
            )
            .unwrap_err(),
            ManifestError::ParentSymlink
        );
    }
}
