use super::*;
use crate::bootstrap::BootstrapDocument;
#[derive(Deserialize)]
struct Case {
    name: String,
    kind: String,
    document: String,
    valid: bool,
}
#[test]
fn shared_literal_document_corpus_and_canonical_encoding() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../tests/fixtures/service-protocol/documents-v1.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 215);
    for case in cases {
        let valid = match case.kind.as_str() {
            "readiness" => ReadinessDocument::decode(case.document.as_bytes()).is_ok(),
            "bootstrap" => BootstrapDocument::decode(case.document.as_bytes()).is_ok(),
            _ => panic!("unknown fixture kind"),
        };
        assert_eq!(valid, case.valid, "{}", case.name);
    }
    let ready = include_bytes!("../../tests/fixtures/service-protocol/readiness-v1.json");
    assert_eq!(
        ReadinessDocument::decode(ready).unwrap().encode().unwrap(),
        ready
    );
    let bootstrap = include_bytes!("../../tests/fixtures/service-protocol/bootstrap-v1.json");
    assert_eq!(
        BootstrapDocument::decode(bootstrap)
            .unwrap()
            .encode()
            .unwrap(),
        bootstrap
    );
}
#[test]
fn shared_freshness_corpus() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/service-protocol/freshness-v1.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 28);
    for case in cases {
        let result = ["published", "start", "now"].map(|field| case[field].as_u64());
        let actual = match result {
            [Some(p), Some(s), Some(n)] => fresh(p, s, n),
            _ => false,
        };
        assert_eq!(actual, case["fresh"].as_bool().unwrap(), "{}", case["name"]);
    }
}
fn identity() -> RunIdentity {
    RunIdentity::from_parts(4242, "a".repeat(64), "b".repeat(64)).unwrap()
}
fn state() -> ReadinessState {
    ReadinessState {
        phase: ReadinessPhase::Ready,
        discord: DiscordState::Ready,
        scheduler: SchedulerState::Running,
        telegram: ConnectorState::Disabled,
        slack: ConnectorState::Disabled,
        last_persistence: LastPersistence::Complete,
    }
}
fn checkpoints() -> ReadyCheckpoints {
    ReadyCheckpoints {
        canonical_privacy_committed: true,
        scheduler_running: true,
        discord_ready: true,
        commands_registered: true,
        presence_applied: true,
    }
}
#[test]
fn every_required_checkpoint_gates_ready_and_identity_debug_is_private() {
    for index in 0..5 {
        let mut check = checkpoints();
        match index {
            0 => check.canonical_privacy_committed = false,
            1 => check.scheduler_running = false,
            2 => check.discord_ready = false,
            3 => check.commands_registered = false,
            _ => check.presence_applied = false,
        };
        assert!(ReadinessDocument::new(&identity(), state(), 42, check).is_err());
    }
    assert!(ReadinessDocument::new(&identity(), state(), 42, checkpoints()).is_ok());
    assert_eq!(format!("{:?}", identity()), "RunIdentity([private])");
    assert!(RunIdentity::from_parts(0, "a".repeat(64), "b".repeat(64)).is_err());
}
#[cfg(unix)]
pub(crate) fn temporary_home() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).unwrap();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("abbey-observability-{}", hex(&random)));
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
#[cfg(unix)]
#[test]
fn publications_are_private_atomic_and_preserve_successor_identity() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let home = temporary_home();
    let publisher = ReadinessPublisher::open(&home, identity()).unwrap();
    let doc = ReadinessDocument::new(&identity(), state(), 42, checkpoints()).unwrap();
    publisher.publish(&doc).unwrap();
    let path = home.join(".local/share/abbey-bot/readiness.json");
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
    let mut next = doc.clone();
    next.run_nonce = "c".repeat(64);
    publisher
        .directory
        .publish("readiness.json", &next.encode().unwrap())
        .unwrap();
    assert!(!publisher.remove_readiness().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), next.encode().unwrap());
    std::fs::remove_file(&path).unwrap();
    let outside = home.join("sentinel");
    std::fs::write(&outside, b"PRIVATE_CANARY").unwrap();
    symlink(&outside, &path).unwrap();
    assert!(publisher.publish(&doc).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"PRIVATE_CANARY");
    std::fs::remove_dir_all(home).unwrap();
}
#[cfg(unix)]
#[test]
fn bootstrap_survives_unsafe_readiness_and_missing_log_directory() {
    use crate::bootstrap::{BootstrapCode, BootstrapPhase};
    let home = temporary_home();
    let publisher = ReadinessPublisher::open(&home, identity()).unwrap();
    let bootstrap =
        BootstrapDocument::new(&identity(), BootstrapPhase::Starting, BootstrapCode::None);
    publisher.publish_bootstrap(&bootstrap).unwrap();
    std::fs::create_dir(home.join(".local/share/abbey-bot/readiness.json")).unwrap();
    assert!(publisher.validate_readiness_target().is_err());
    publisher
        .publish_bootstrap(&BootstrapDocument::new(
            &identity(),
            BootstrapPhase::Failed,
            BootstrapCode::ReadinessFile,
        ))
        .unwrap();
    assert!(!home.join("Library").exists());
    assert!(publisher.remove_bootstrap().unwrap());
    assert!(!publisher.remove_bootstrap().unwrap());
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn real_os_identity_is_fresh_and_hashes_this_test_executable() {
    let first = RunIdentity::current().unwrap();
    let second = RunIdentity::current().unwrap();
    assert_eq!(first.pid, std::process::id());
    assert_ne!(first.nonce, second.nonce);
    assert!(valid_hex(&first.nonce));
    assert_eq!(first.executable_sha256, second.executable_sha256);
    let mut executable = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    let mut expected = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = executable.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        expected.update(&buffer[..count]);
    }
    assert_eq!(first.executable_sha256, hex(&expected.finalize()));
}
