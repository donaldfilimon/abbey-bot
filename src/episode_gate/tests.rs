use super::*;

const DIGEST_HEX: &str = "01080f161d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3da";

/// Absolute on every platform (a POSIX literal is relative on Windows).
fn abi_path() -> String {
    std::env::temp_dir().join("abi").display().to_string()
}

fn token_path() -> String {
    std::env::temp_dir()
        .join("abbey-episode-token")
        .display()
        .to_string()
}

/// Built with serde_json so Windows path separators are escaped correctly.
fn config_json_with(abi_cli: &str, token_file: &str, timeout_secs: u64) -> String {
    serde_json::json!({
        "abi_cli": abi_cli,
        "endpoint": "http://127.0.0.1:50051",
        "token_file": token_file,
        "policy_version": "policy_v1",
        "contract_revision": 2,
        "contract_digest": DIGEST_HEX,
        "timeout_secs": timeout_secs,
    })
    .to_string()
}

fn config_json(abi_cli: &str, timeout_secs: u64) -> String {
    config_json_with(abi_cli, &token_path(), timeout_secs)
}

fn config() -> EpisodeGateConfig {
    EpisodeGateConfig::from_json(&config_json(&abi_path(), 5)).unwrap()
}

fn request() -> LearningToggleRequest {
    LearningToggleRequest {
        scoped_guild: "discord:123456789012345678".into(),
        scoped_user: "discord:42".into(),
        now: 1_700_000_000,
        nonce: 0,
    }
}

#[test]
fn write_json_matches_the_canonical_fixture() {
    // Generated from `abi-wdbx::v3::episode` on 2026-09-06; the transcription
    // above must serialize byte-for-byte to what the gateway deserializes.
    let fixture = include_str!("../../tests/fixtures/episode_write_proposal.json");
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::try_from(index).unwrap().wrapping_mul(7).wrapping_add(1);
    }
    let write = EpisodeWrite {
        request_id: "req-00000000deadbeef".into(),
        operation_id: "learning-toggle-0123456789abcdef".into(),
        contract_revision: 2,
        contract_digest: digest,
        guild_ref: "discord-123456789012345678".into(),
        consent_epoch: None,
        source_type: EpisodeSource::DiscordGuild,
        policy_version: "policy_v1".into(),
        evidence_level: EvidenceLevel::C0,
        event: EpisodeEvent::Proposal {
            requested_by: ActorRef {
                principal_id: "admin-fedcba9876543210".into(),
                kind: ActorKind::GuildAdministrator,
            },
            proposed_by: ActorRef {
                principal_id: "abbey-service".into(),
                kind: ActorKind::Service,
            },
        },
        token_cost: 1,
        expected_commitment: None,
        quiet: false,
    };
    assert_eq!(serde_json::to_string(&write).unwrap(), fixture.trim_end());
    let round_trip: EpisodeWrite = serde_json::from_str(fixture).unwrap();
    assert_eq!(round_trip, write);
}

#[test]
fn config_parses_and_validates() {
    let parsed = config();
    assert_eq!(parsed.endpoint(), "http://127.0.0.1:50051");
    assert_eq!(parsed.timeout_secs(), 5);
    assert_eq!(parsed.evidence_level, EvidenceLevel::C0);
    assert_eq!(parsed.service_principal, DEFAULT_SERVICE_PRINCIPAL);
    assert_eq!(parse_digest(DIGEST_HEX).unwrap(), parsed.contract_digest);

    let relative = config_json("abi", 5);
    assert!(
        EpisodeGateConfig::from_json(&relative)
            .unwrap_err()
            .contains("abi_cli")
    );
    let dotdot = config_json(
        &std::env::temp_dir()
            .join("..")
            .join("abi")
            .display()
            .to_string(),
        5,
    );
    assert!(
        EpisodeGateConfig::from_json(&dotdot)
            .unwrap_err()
            .contains("abi_cli")
    );
    let slow = config_json(&abi_path(), 0);
    assert!(
        EpisodeGateConfig::from_json(&slow)
            .unwrap_err()
            .contains("timeout_secs")
    );
    let unknown =
        config_json(&abi_path(), 5).replace("\"timeout_secs\"", "\"token\":\"x\",\"timeout_secs\"");
    assert!(
        EpisodeGateConfig::from_json(&unknown)
            .unwrap_err()
            .contains("invalid JSON")
    );
    let zero_digest = config_json(&abi_path(), 5).replace(DIGEST_HEX, &"0".repeat(64));
    assert!(
        EpisodeGateConfig::from_json(&zero_digest)
            .unwrap_err()
            .contains("contract_digest")
    );
    let bad_scheme = config_json(&abi_path(), 5).replace("http://", "grpc://");
    assert!(
        EpisodeGateConfig::from_json(&bad_scheme)
            .unwrap_err()
            .contains("endpoint")
    );
    let bad_level = config_json(&abi_path(), 5).replace(
        "\"timeout_secs\"",
        "\"evidence_level\":\"c9\",\"timeout_secs\"",
    );
    assert!(
        EpisodeGateConfig::from_json(&bad_level)
            .unwrap_err()
            .contains("evidence_level")
    );
}

#[test]
fn guild_coverage_is_optional_and_validated() {
    let everywhere = config();
    assert!(everywhere.covers("discord:123456789012345678"));
    assert!(everywhere.covers("discord:dm:42"));
    assert_eq!(everywhere.coverage(), None);

    let scoped = config_json(&abi_path(), 5).replace(
        "\"timeout_secs\"",
        "\"guilds\":[\" discord:123456789012345678 \",\"discord:123456789012345678\"],\"timeout_secs\"",
    );
    let scoped = EpisodeGateConfig::from_json(&scoped).unwrap();
    assert!(scoped.covers("discord:123456789012345678"));
    assert!(!scoped.covers("discord:999"));
    assert!(!scoped.covers("discord:dm:42"));
    assert_eq!(scoped.coverage(), Some(1), "trimmed and deduplicated");
    assert_eq!(EpisodeGate::new(scoped).counters().covered_guilds, Some(1));

    let empty =
        config_json(&abi_path(), 5).replace("\"timeout_secs\"", "\"guilds\":[],\"timeout_secs\"");
    assert!(
        EpisodeGateConfig::from_json(&empty)
            .unwrap_err()
            .contains("guilds must name at least one")
    );
    let unmappable = config_json(&abi_path(), 5).replace(
        "\"timeout_secs\"",
        "\"guilds\":[\"discord:has space\"],\"timeout_secs\"",
    );
    assert!(
        EpisodeGateConfig::from_json(&unmappable)
            .unwrap_err()
            .contains("guilds entries")
    );
}

#[test]
fn endpoint_transport_mirrors_the_abi_cli_rule() {
    let with = |endpoint: &str, ca: bool| {
        let mut text = config_json(&abi_path(), 5).replace("http://127.0.0.1:50051", endpoint);
        if ca {
            let ca_cert = serde_json::to_string(
                &std::env::temp_dir()
                    .join("gateway-ca.pem")
                    .display()
                    .to_string(),
            )
            .unwrap();
            text = text.replace(
                "\"timeout_secs\"",
                &format!("\"ca_cert\":{ca_cert},\"timeout_secs\""),
            );
        }
        EpisodeGateConfig::from_json(&text)
    };
    assert!(with("http://localhost:50051", false).is_ok());
    assert!(with("http://[::1]:50051", false).is_ok());
    assert!(
        with("http://10.0.0.1:50051", false)
            .unwrap_err()
            .contains("require https and ca_cert")
    );
    assert!(
        with("https://gateway.internal:50051", false)
            .unwrap_err()
            .contains("ca_cert is required")
    );
    assert!(with("https://gateway.internal:50051", true).is_ok());
    assert!(
        with("gateway.internal:50051", false)
            .unwrap_err()
            .contains("http://")
    );
}

#[test]
fn from_path_requires_the_named_files_to_exist() {
    let dir = std::env::temp_dir().join(format!(
        "abbey-episode-gate-config-{}-{}",
        std::process::id(),
        NEXT_WRITE_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let abi = dir.join("abi");
    let token = dir.join("token");
    std::fs::write(&abi, "").unwrap();
    let text = config_json_with(&abi.display().to_string(), &token.display().to_string(), 5);
    let config_file = dir.join("gate.json");
    std::fs::write(&config_file, &text).unwrap();
    let error = EpisodeGateConfig::from_path(&config_file).unwrap_err();
    assert!(error.starts_with("token_file:"), "{error}");
    std::fs::write(&token, "").unwrap();
    assert!(EpisodeGateConfig::from_path(&config_file).is_ok());
    assert!(
        EpisodeGateConfig::from_path(Path::new("gate.json"))
            .unwrap_err()
            .contains(CONFIG_ENV)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_errors_never_echo_values() {
    let text = config_json(&abi_path(), 5).replace("policy_v1", "Policy V1 SECRET");
    let error = EpisodeGateConfig::from_json(&text).unwrap_err();
    assert!(error.contains("policy_version"));
    assert!(!error.contains("SECRET"));
}

#[test]
fn guild_refs_and_principals_are_ledger_safe_and_content_free() {
    assert_eq!(
        guild_ref_for("discord:123456789012345678").as_deref(),
        Some("discord-123456789012345678")
    );
    assert_eq!(
        guild_ref_for("Discord:DM:7").as_deref(),
        Some("discord-dm-7")
    );
    assert!(guild_ref_for("").is_none());
    assert!(guild_ref_for("discord:a b").is_none());
    assert!(guild_ref_for(&"x".repeat(129)).is_none());

    let principal = requester_principal("discord:1", "discord:42");
    assert!(principal.starts_with("admin-"));
    assert_eq!(principal.len(), 6 + 16);
    assert!(bounded_identifier(&principal, MAX_IDENTIFIER_LEN));
    assert!(!principal.contains("42"));
    assert_eq!(principal, requester_principal("discord:1", "discord:42"));
    assert_ne!(principal, requester_principal("discord:2", "discord:42"));
    assert_ne!(principal, requester_principal("discord:1", "discord:43"));
}

#[test]
fn learning_toggle_proposal_is_bound_to_the_configured_policy() {
    let write = learning_toggle_proposal(&config(), &request()).unwrap();
    assert_eq!(write.guild_ref, "discord-123456789012345678");
    assert_eq!(write.contract_revision, 2);
    assert_eq!(write.policy_version, "policy_v1");
    assert_eq!(write.source_type, EpisodeSource::DiscordGuild);
    assert_eq!(write.consent_epoch, None);
    assert_eq!(write.token_cost, TOKEN_COST);
    assert!(!write.quiet);
    assert!(bounded_identifier(&write.request_id, MAX_IDENTIFIER_LEN));
    assert!(bounded_identifier(&write.operation_id, MAX_IDENTIFIER_LEN));
    let EpisodeEvent::Proposal {
        requested_by,
        proposed_by,
    } = &write.event
    else {
        panic!("a learning toggle is a proposal");
    };
    assert_eq!(requested_by.kind, ActorKind::GuildAdministrator);
    assert_eq!(proposed_by.kind, ActorKind::Service);
    assert_ne!(requested_by.principal_id, proposed_by.principal_id);

    let mut later = request();
    later.nonce = 1;
    let second = learning_toggle_proposal(&config(), &later).unwrap();
    assert_ne!(second.request_id, write.request_id);
    assert_ne!(second.operation_id, write.operation_id);

    let mut bad = request();
    bad.scoped_guild = "discord:no spaces".into();
    assert!(learning_toggle_proposal(&config(), &bad).is_err());
}

#[test]
fn classify_reads_only_the_shapes_the_cli_promises() {
    let appended =
        br#"{"decision":"appended","episode_digest":"abababababababababababababababababababababababababababababababab","sequence":"3","request_id":"r"}"#;
    assert_eq!(
        classify(Some(0), appended, b""),
        GateOutcome::Appended {
            digest_hex: "ab".repeat(32),
            sequence: "3".into(),
        }
    );
    assert!(matches!(
        classify(Some(0), br#"{"decision":"preview","episode_digest":"ab","sequence":"0"}"#, b""),
        GateOutcome::Unavailable { detail } if detail.contains("preview")
    ));
    assert!(matches!(
        classify(Some(0), b"not json", b""),
        GateOutcome::Unavailable { .. }
    ));
    assert_eq!(
        classify(
            Some(1),
            b"",
            b"episode propose: FailedPrecondition: learning_disabled\n"
        ),
        GateOutcome::Rejected {
            detail: "episode propose: FailedPrecondition: learning_disabled".into(),
        }
    );
    assert!(matches!(
        classify(Some(2), b"", b"usage: abi wdbx episode"),
        GateOutcome::Unavailable { detail } if detail.contains("status 2")
    ));
    assert!(matches!(
        classify(None, b"", b""),
        GateOutcome::Unavailable { .. }
    ));
    let long = "x".repeat(1000);
    if let GateOutcome::Rejected { detail } = classify(Some(1), b"", long.as_bytes()) {
        assert_eq!(detail.len(), MAX_DETAIL_CHARS);
    } else {
        panic!("expected a rejection");
    }
}

#[cfg(unix)]
mod with_a_fake_abi {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "abbey-episode-gate-{label}-{}-{}",
                std::process::id(),
                NEXT_WRITE_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn script(&self, body: &str) -> PathBuf {
            let path = self.0.join("abi");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        }

        fn gate(&self, script: &Path, timeout_secs: u64) -> EpisodeGate {
            EpisodeGate::new(
                EpisodeGateConfig::from_json(&config_json(
                    &script.display().to_string(),
                    timeout_secs,
                ))
                .unwrap(),
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn an_append_is_reported_and_the_write_file_is_removed() {
        let scratch = Scratch::new("append");
        let recorded = scratch.0.join("argv");
        // Echo the arguments and the write file's contents, then answer as the CLI does.
        let script = scratch.script(&format!(
            "printf '%s\\n' \"$@\" > {argv}\ncat \"$4\" >> {argv}\nprintf '%s\\n' '{{\"decision\":\"appended\",\"episode_digest\":\"abababababababababababababababababababababababababababababababab\",\"sequence\":\"1\"}}'",
            argv = recorded.display()
        ));
        let gate = scratch.gate(&script, 5);
        let outcome = gate.record_learning_toggle(request()).await;
        assert_eq!(
            outcome,
            GateOutcome::Appended {
                digest_hex: "ab".repeat(32),
                sequence: "1".into(),
            }
        );
        let argv = std::fs::read_to_string(&recorded).unwrap();
        let mut lines = argv.lines();
        assert_eq!(lines.next(), Some("wdbx"));
        assert_eq!(lines.next(), Some("episode"));
        assert_eq!(lines.next(), Some("propose"));
        let write_file = PathBuf::from(lines.next().unwrap());
        assert!(
            !write_file.exists(),
            "the write file must be removed after the call"
        );
        assert_eq!(lines.next(), Some("--json"));
        assert_eq!(lines.next(), Some("--endpoint"));
        assert_eq!(lines.next(), Some("http://127.0.0.1:50051"));
        assert_eq!(lines.next(), Some("--token-file"));
        assert_eq!(lines.next(), Some(token_path().as_str()));
        let body = lines.next().unwrap();
        let write: EpisodeWrite = serde_json::from_str(body).unwrap();
        assert_eq!(write.guild_ref, "discord-123456789012345678");
        assert!(
            !body.contains("discord:"),
            "no colon-form scoped id may reach the ledger"
        );
        assert!(
            !body.contains("42\""),
            "the requester's user id must not appear verbatim"
        );
    }

    #[tokio::test]
    async fn a_refusal_is_a_rejection_with_the_gateway_label() {
        let scratch = Scratch::new("reject");
        let script = scratch
            .script("echo 'episode propose: FailedPrecondition: learning_disabled' >&2\nexit 1");
        let gate = scratch.gate(&script, 5);
        assert_eq!(
            gate.record_learning_toggle(request()).await,
            GateOutcome::Rejected {
                detail: "episode propose: FailedPrecondition: learning_disabled".into(),
            }
        );
    }

    #[tokio::test]
    async fn a_hung_or_missing_binary_is_unavailable() {
        let scratch = Scratch::new("hang");
        let script = scratch.script("sleep 30");
        let gate = scratch.gate(&script, 1);
        assert!(matches!(
            gate.record_learning_toggle(request()).await,
            GateOutcome::Unavailable { detail } if detail.contains("within 1s")
        ));
        let missing = scratch.0.join("absent");
        let gate = scratch.gate(&missing, 1);
        assert!(matches!(
            gate.record_learning_toggle(request()).await,
            GateOutcome::Unavailable { detail } if detail.contains("could not start")
        ));
    }

    #[tokio::test]
    async fn the_child_does_not_inherit_the_bots_environment() {
        let scratch = Scratch::new("env");
        let recorded = scratch.0.join("env");
        let script = scratch.script(&format!("env > {}\nexit 1", recorded.display()));
        let gate = scratch.gate(&script, 5);
        // Cargo always sets CARGO_MANIFEST_DIR for a test process and every
        // parent has PATH; neither is on the allowlist, so neither may reach
        // the child. No process-environment mutation: sibling tests read env
        // concurrently and `set_var` would race them.
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        assert!(std::env::var_os("PATH").is_some());
        let _ = gate.record_learning_toggle(request()).await;
        let seen = std::fs::read_to_string(&recorded).unwrap();
        assert!(!seen.contains("CARGO_MANIFEST_DIR="));
        assert!(!seen.lines().any(|line| line.starts_with("PATH=")));
    }
}

// ---------------------------------------------------------------------------
// Memory candidates (amendment 2026-09-06).
// ---------------------------------------------------------------------------

fn pattern(mul: u8, add: u8) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::try_from(index)
            .unwrap()
            .wrapping_mul(mul)
            .wrapping_add(add);
    }
    out
}

fn memory_request(class: MemoryClass) -> MemoryCandidateRequest {
    MemoryCandidateRequest {
        scoped_guild: "discord:123456789012345678".into(),
        class,
        retention: RetentionClass::Durable,
        payload: b"uses rust".to_vec(),
        member_scoped: true,
        supersedes: None,
        forgets: None,
        now: 1_700_000_000,
        nonce: 3,
    }
}

fn candidate_of(write: &EpisodeWrite) -> (&ActorRef, &MemoryCandidate) {
    match &write.event {
        EpisodeEvent::MemoryCandidate {
            recorded_by,
            candidate,
        } => (recorded_by, candidate),
        EpisodeEvent::Proposal { .. } => panic!("a memory request builds a memory candidate"),
    }
}

#[test]
fn memory_candidate_json_matches_the_canonical_fixture() {
    // Copied byte-for-byte from wdbx `crates/abi-wdbx/tests/golden/` on
    // 2026-09-06 (generated there from `abi-wdbx::v3::episode` with
    // `WDBX_WRITE_GOLDEN=1`); the store pins its digest as
    // 3c19a479a23077d95238b710876e5c03d2300dd77e9ccfedbbe6c11b0fc768bc.
    let fixture = include_str!("../../tests/fixtures/episode_write_memory_candidate.json");
    let write = EpisodeWrite {
        request_id: "req-memory-00000000deadbeef".into(),
        operation_id: "memory-embedding-0123456789abcdef".into(),
        contract_revision: 2,
        contract_digest: pattern(7, 1),
        guild_ref: "discord-123456789012345678".into(),
        consent_epoch: None,
        source_type: EpisodeSource::DiscordGuild,
        policy_version: "policy_v1".into(),
        evidence_level: EvidenceLevel::C0,
        event: EpisodeEvent::MemoryCandidate {
            recorded_by: ActorRef {
                principal_id: "abbey-service".into(),
                kind: ActorKind::Service,
            },
            candidate: MemoryCandidate {
                class: MemoryClass::Embedding,
                retention: RetentionClass::Durable,
                payload_commitment: pattern(5, 2),
                payload_bytes: 1_536,
                dimension: Some(384),
                embedding_version: Some("abbey-embedding-v1".into()),
                member_scoped: true,
                supersedes: None,
                forgets: None,
            },
        },
        token_cost: 1,
        expected_commitment: None,
        quiet: false,
    };
    assert_eq!(serde_json::to_string(&write).unwrap(), fixture.trim_end());
    let round_trip: EpisodeWrite = serde_json::from_str(fixture).unwrap();
    assert_eq!(round_trip, write);
}

#[test]
fn a_fact_candidate_commits_to_the_payload_and_never_carries_it() {
    use sha2::{Digest as _, Sha256};

    let config = config();
    let memory = memory_request(MemoryClass::Fact);
    let write = memory_candidate_write(&config, &memory).unwrap();
    let (recorded_by, candidate) = candidate_of(&write);
    assert_eq!(recorded_by.kind, ActorKind::Service);
    assert_eq!(recorded_by.principal_id, DEFAULT_SERVICE_PRINCIPAL);
    let expected: [u8; 32] = Sha256::digest(b"uses rust").into();
    assert_eq!(candidate.payload_commitment, expected);
    assert_eq!(candidate.payload_bytes, 9);
    assert_eq!(candidate.class, MemoryClass::Fact);
    assert_eq!(candidate.retention, RetentionClass::Durable);
    assert_eq!(candidate.dimension, None);
    assert_eq!(candidate.embedding_version, None);
    assert!(candidate.member_scoped);
    assert_eq!(write.guild_ref, "discord-123456789012345678");
    assert_eq!(write.source_type, EpisodeSource::DiscordGuild);
    assert!(write.operation_id.starts_with("memory-fact-"));
    assert!(bounded_identifier(&write.operation_id, MAX_IDENTIFIER_LEN));
    assert!(bounded_identifier(&write.request_id, MAX_IDENTIFIER_LEN));
    let json = serde_json::to_string(&write).unwrap();
    assert!(!json.contains("uses rust"));
    assert!(json.contains("\"kind\":\"memory_candidate\""));

    // Same clock and nonce as a learning toggle still yields distinct ids.
    let mut toggle = request();
    toggle.now = 1_700_000_000;
    toggle.nonce = 3;
    let toggle = learning_toggle_proposal(&config, &toggle).unwrap();
    assert_ne!(toggle.request_id, write.request_id);
    assert_ne!(toggle.operation_id, write.operation_id);
}

#[test]
fn forget_and_supersede_shapes_follow_the_store_rules() {
    let config = config();

    let mut forget = memory_request(MemoryClass::Fact);
    forget.payload.clear();
    forget.forgets = Some([7; 32]);
    let write = memory_candidate_write(&config, &forget).unwrap();
    let (_, candidate) = candidate_of(&write);
    assert_eq!(candidate.payload_commitment, [0; 32]);
    assert_eq!(candidate.payload_bytes, 0);
    assert_eq!(candidate.forgets, Some([7; 32]));
    assert_eq!(candidate.supersedes, None);

    let mut forget_with_payload = forget.clone();
    forget_with_payload.payload = b"x".to_vec();
    assert!(memory_candidate_write(&config, &forget_with_payload).is_err());
    let mut forget_and_supersede = forget.clone();
    forget_and_supersede.supersedes = Some([8; 32]);
    assert!(memory_candidate_write(&config, &forget_and_supersede).is_err());

    let mut supersede = memory_request(MemoryClass::Experience);
    supersede.retention = RetentionClass::Operational;
    supersede.member_scoped = false;
    supersede.supersedes = Some([9; 32]);
    let write = memory_candidate_write(&config, &supersede).unwrap();
    let (_, candidate) = candidate_of(&write);
    assert_eq!(candidate.supersedes, Some([9; 32]));
    assert_eq!(candidate.class, MemoryClass::Experience);
    assert!(!candidate.member_scoped);
    assert!(write.operation_id.starts_with("memory-experience-"));

    let mut empty = memory_request(MemoryClass::Summary);
    empty.payload.clear();
    assert!(memory_candidate_write(&config, &empty).is_err());
    assert!(memory_candidate_write(&config, &memory_request(MemoryClass::Embedding)).is_err());
    let mut bad_guild = memory_request(MemoryClass::Fact);
    bad_guild.scoped_guild = "discord:not a guild".into();
    assert!(memory_candidate_write(&config, &bad_guild).is_err());
}

#[test]
fn counters_start_at_zero_and_count_every_outcome() {
    let gate = EpisodeGate::new(config());
    assert_eq!(gate.counters(), GateCounters::default());
    gate.count(&GateOutcome::Appended {
        digest_hex: "ab".repeat(32),
        sequence: "1".into(),
    });
    gate.count(&GateOutcome::Rejected {
        detail: "FailedPrecondition: episode_learning_disabled".into(),
    });
    gate.count(&GateOutcome::Rejected {
        detail: "FailedPrecondition: episode_storage_budget_exhausted".into(),
    });
    gate.count(&GateOutcome::Unavailable {
        detail: "timed out".into(),
    });
    gate.note_ungated_forget();
    assert_eq!(
        gate.counters(),
        GateCounters {
            appended: 1,
            rejected: 2,
            unavailable: 1,
            ungated_forgets: 1,
            covered_guilds: None,
        }
    );
}

#[tokio::test]
async fn pre_cancelled_abi_never_reaches_executable_launch() {
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let outcome = run_abi_owned(Path::new(""), &[], 5, Some(cancel)).await;
    assert_eq!(
        outcome,
        GateOutcome::Unavailable {
            detail: "the abi operation was cancelled during shutdown".into(),
        }
    );
}

#[test]
fn malformed_append_receipts_never_authorize_local_memory() {
    for (digest, sequence) in [
        ("ab".to_string(), "1"),
        ("gg".repeat(32), "1"),
        ("AB".repeat(32), "1"),
        ("00".repeat(32), "1"),
        ("ab".repeat(32), "-1"),
        ("ab".repeat(32), "01"),
        ("ab".repeat(32), "18446744073709551616"),
    ] {
        let json = serde_json::json!({"decision":"appended", "episode_digest":digest, "sequence":sequence});
        assert!(
            matches!(
                classify(Some(0), json.to_string().as_bytes(), b""),
                GateOutcome::Unavailable { .. }
            ),
            "{json}"
        );
    }
}
