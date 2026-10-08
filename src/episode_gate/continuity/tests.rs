//! Strict transcribed ABI verify JSON; no external process is called here.
use super::*;
fn wire(forgotten: &str) -> serde_json::Value {
    serde_json::json!({
        "found":"true", "guild_ref":"discord-123", "event_kind":"memory_candidate",
        "signature_status":"unsigned", "memory_forgotten":forgotten,
    })
}
#[test]
fn continuity_receipt_liveness_decodes_only_positive_current_identity() {
    for (flag, expected) in [
        ("false", MemoryCandidateState::Live),
        ("true", MemoryCandidateState::Forgotten),
    ] {
        let json = serde_json::to_vec(&wire(flag)).unwrap();
        assert_eq!(decode(Some(0), &json, "discord-123"), expected);
    }
}
#[test]
fn continuity_receipt_liveness_refuses_unknown_duplicate_unsigned_shape_errors() {
    for (field, bad) in [
        ("found", serde_json::json!(true)),
        ("found", serde_json::json!("false")),
        ("guild_ref", serde_json::json!("discord-456")),
        ("event_kind", serde_json::json!("proposal")),
        ("memory_forgotten", serde_json::json!(false)),
        ("memory_forgotten", serde_json::json!("FALSE")),
        ("signature_status", serde_json::json!("invalid")),
        ("signature_status", serde_json::json!("unknown_key")),
    ] {
        let mut json = wire("false");
        json[field] = bad;
        assert_eq!(
            decode(Some(0), &serde_json::to_vec(&json).unwrap(), "discord-123"),
            MemoryCandidateState::Unknown,
            "{field}"
        );
    }
    for field in [
        "found",
        "guild_ref",
        "event_kind",
        "memory_forgotten",
        "signature_status",
    ] {
        let mut json = wire("false");
        json.as_object_mut().unwrap().remove(field);
        assert_eq!(
            decode(Some(0), &serde_json::to_vec(&json).unwrap(), "discord-123"),
            MemoryCandidateState::Unknown
        );
    }
    let json = serde_json::to_vec(&wire("true")).unwrap();
    for code in [None, Some(1), Some(2)] {
        assert_eq!(
            decode(code, &json, "discord-123"),
            MemoryCandidateState::Unknown
        );
    }
    let mut trailing = json.clone();
    trailing.extend_from_slice(b"{}");
    assert_eq!(
        decode(Some(0), &trailing, "discord-123"),
        MemoryCandidateState::Unknown
    );
    let duplicate=br#"{"found":"true","guild_ref":"discord-123","event_kind":"memory_candidate","signature_status":"unsigned","memory_forgotten":"false","memory_forgotten":"true"}"#;
    assert_eq!(
        decode(Some(0), duplicate, "discord-123"),
        MemoryCandidateState::Unknown
    );
}

#[cfg(unix)]
mod cli {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(body: &str) -> (Self, EpisodeGate) {
            let dir = std::env::temp_dir().join(format!(
                "abbey-continuity-verify-{}-{}",
                std::process::id(),
                NEXT_WRITE_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join("abi");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let cfg=EpisodeGateConfig::from_json(&serde_json::json!({
                "abi_cli":path,"endpoint":"http://127.0.0.1:50051","token_file":dir.join("token"),
                "policy_version":"v1","contract_revision":2,"contract_digest":"ab".repeat(32),"timeout_secs":1
            }).to_string()).unwrap();
            (Self(dir), EpisodeGate::new(cfg))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[tokio::test]
    async fn continuity_verify_runs_exact_read_only_argv_and_decodes_live() {
        let body = format!(
            "[ \"$1\" = wdbx ] && [ \"$2\" = episode ] && [ \"$3\" = verify ] && [ \"$4\" = discord-123 ] && [ \"$5\" = '{}' ] && [ \"$6\" = --json ] && [ \"$7\" = --endpoint ] && [ \"$9\" = --token-file ] || exit 1\nprintf '%s\\n' '{}'",
            "ab".repeat(32),
            wire("false")
        );
        let (_scratch, gate) = Scratch::new(&body);
        assert_eq!(
            gate.verify_memory("discord:123", &"ab".repeat(32)).await,
            MemoryCandidateState::Live
        );
    }
    #[tokio::test]
    async fn continuity_verify_reports_forgotten_and_refuses_nonzero_exit() {
        for (exit, expected) in [
            (0, MemoryCandidateState::Forgotten),
            (1, MemoryCandidateState::Unknown),
        ] {
            let (_scratch, gate) =
                Scratch::new(&format!("printf '%s\\n' '{}'\nexit {exit}", wire("true")));
            assert_eq!(
                gate.verify_memory("discord:123", &"ab".repeat(32)).await,
                expected
            );
        }
    }
    #[tokio::test]
    async fn continuity_verify_closed_service_never_starts_a_child() {
        let (scratch, gate) = Scratch::new("touch \"$0.started\"\nprintf '%s\\n' '{}'");
        let mut supervisor = crate::service::ServiceSupervisor::new();
        let service = supervisor.operations();
        supervisor.begin_draining(
            crate::service::ShutdownReason::Signal,
            tokio::time::Instant::now(),
        );
        gate.attach_service(service);
        assert_eq!(
            gate.verify_memory("discord:123", &"ab".repeat(32)).await,
            MemoryCandidateState::Unknown
        );
        assert!(!scratch.0.join("abi.started").exists());
    }
}
