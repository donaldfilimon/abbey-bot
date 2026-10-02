use super::*;
fn fixture() -> ActivityReadiness {
    let digest = "a".repeat(64);
    let receipt = |participants| {
        serde_json::json!({"digest":digest,"origin":"https://court.example.org","record":"docs/acceptance/synthetic-offline.md","verified_at":1,"participants":participants}).to_string()
    };
    ActivityReadiness {
        https_origin: "https://court.example.org".into(),
        deployed_digest: digest.clone(),
        iframe_receipt: receipt(1),
        shared_receipt: receipt(2),
        verified_at: 1,
    }
}
#[test]
fn activity_readiness_rejects_local_credentials_query_paths_and_malformed_digest() {
    assert!(validate_activity_readiness(&fixture()).is_ok());
    for origin in [
        "http://court.example.org",
        "https://localhost",
        "https://127.0.0.1",
        "https://[::1]",
        "https://host.local",
        "https://user:secret@court.example.org",
        "https://court.example.org?token=a",
        "https://court.example.org/path",
        "invalid",
    ] {
        let mut r = fixture();
        r.https_origin = origin.into();
        assert!(validate_activity_readiness(&r).is_err(), "{origin}");
    }
    for digest in ["z".repeat(64), "a".repeat(63)] {
        let mut r = fixture();
        r.deployed_digest = digest;
        assert!(validate_activity_readiness(&r).is_err());
    }
}
#[test]
fn activity_readiness_missing_stale_version_and_single_participant_fail() {
    for receipt in ["".into(),serde_json::json!({"digest":"b".repeat(64),"origin":"https://court.example.org","record":"docs/acceptance/synthetic-offline.md","verified_at":1,"participants":2}).to_string(),serde_json::json!({"digest":"a".repeat(64),"origin":"https://court.example.org","record":"docs/acceptance/synthetic-offline.md","verified_at":1,"participants":1}).to_string()] {
        let mut r=fixture();r.shared_receipt=receipt;assert!(validate_activity_readiness(&r).is_err());
    }
}

#[test]
fn activity_readiness_actual_evidence_requires_version_and_shared_recovery() {
    let r = fixture();
    let mut evidence = serde_json::json!({"digest":r.deployed_digest,"origin":r.https_origin,"verified_at":1,"participants":2,"discord_iframe":true,"shared_room_case_votes":true,"disconnection_recovery":true,"no_solo_vote_upload":true});
    assert!(
        validate_acceptance_evidence(&r, &serde_json::to_vec(&evidence).unwrap(), true).is_ok()
    );
    for flag in [
        "discord_iframe",
        "shared_room_case_votes",
        "disconnection_recovery",
        "no_solo_vote_upload",
    ] {
        evidence[flag] = false.into();
        assert!(
            validate_acceptance_evidence(&r, &serde_json::to_vec(&evidence).unwrap(), true)
                .is_err()
        );
        evidence[flag] = true.into();
    }
    evidence["digest"] = "b".repeat(64).into();
    assert!(
        validate_acceptance_evidence(&r, &serde_json::to_vec(&evidence).unwrap(), true).is_err()
    );
}
