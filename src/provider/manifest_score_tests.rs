use super::super::score_fixtures::{assert_components, fixtures};
use super::super::scoring::{ExecutionLocality, QualificationAttempt};
use super::*;
fn old_record() -> ProviderRecord {
    let ManifestDocument::V2(manifest) = decode_manifest(include_bytes!(
        "../../tests/fixtures/provider-capability-only-v2.json"
    ))
    .unwrap() else {
        panic!()
    };
    manifest.records()[0].clone()
}
fn all_record() -> ProviderRecord {
    let mut r = old_record();
    r.declared_capabilities = super::super::ProviderCapabilities {
        text: true,
        streaming: true,
        tools: true,
        structured_output: true,
        vision: true,
        ocr: true,
    }
    .into();
    r
}
fn decode_profiles(profiles: &str) -> ProviderRecord {
    let mut value = serde_json::to_value(all_record()).unwrap();
    value["score_policy"] = 1.into();
    let mut object = serde_json::to_string(&value).unwrap();
    object.pop();
    object.push_str(",\"score_profiles\":");
    object.push_str(profiles);
    object.push('}');
    let ManifestDocument::V2(manifest) = decode_manifest(format!("[{object}]").as_bytes()).unwrap()
    else {
        panic!()
    };
    manifest.records()[0].clone()
}
#[test]
fn frozen_old_v2_and_v1_schema_have_conservative_shared_projection() {
    let record = old_record();
    assert!(record.score_profiles.is_none());
    let document = decode_manifest(include_bytes!(
        "../../tests/fixtures/provider-legacy-v1.json"
    ))
    .unwrap();
    let ManifestDocument::LegacyV1(report) = &document else {
        panic!()
    };
    let expected = report.primary.identity.as_ref().unwrap();
    for case in fixtures().compatibility {
        assert_components(
            record
                .score_profile(RequestClass::TextReadOnly, case.locality)
                .unwrap()
                .components(),
            case.components,
        );
        assert_components(
            document
                .legacy_score_profile(
                    LegacyScoreRoute::Primary,
                    expected,
                    RequestClass::TextReadOnly,
                    case.locality,
                    0,
                )
                .unwrap()
                .components(),
            case.components,
        );
        for class in [
            RequestClass::TextWithTools,
            RequestClass::VisionDescribe,
            RequestClass::VisionOcr,
        ] {
            assert_eq!(
                record.score_profile(class, case.locality).unwrap_err(),
                ManifestError::CapabilityMismatch
            );
            assert!(
                document
                    .legacy_score_profile(
                        LegacyScoreRoute::Primary,
                        expected,
                        class,
                        case.locality,
                        0
                    )
                    .is_err()
            );
        }
    }
    let mut wrong = expected.clone();
    wrong.os_build = "other".into();
    assert_eq!(
        document
            .legacy_score_profile(
                LegacyScoreRoute::Primary,
                &wrong,
                RequestClass::TextReadOnly,
                ExecutionLocality::SameHost,
                0,
            )
            .unwrap_err(),
        ManifestError::IdentityMismatch
    );
}
#[test]
fn shared_five_attempt_writer_and_score_reader_use_the_same_producer() {
    for case in fixtures().profiles {
        let e = case.evidence;
        let attempts = std::array::from_fn(|i| QualificationAttempt {
            succeeded: i < usize::from(e.successful_attempts),
            duration_ms: e.successful_duration_ms.get(i).copied(),
            mandatory_check_mask: e.mandatory_check_mask,
        });
        let evidence = ScoreProducerPolicy::V1
            .qualify(e.request_class, &attempts, e.locality)
            .unwrap();
        let mut record = all_record();
        record.score_policy = Some(1);
        record.score_profiles = Some(vec![evidence]);
        let ManifestDocument::V2(manifest) =
            decode_manifest(&serde_json::to_vec(&vec![record]).unwrap()).unwrap()
        else {
            panic!()
        };
        assert_components(
            manifest.records()[0]
                .score_profile(e.request_class, ExecutionLocality::PublicRemote)
                .unwrap()
                .components(),
            case.components,
        );
    }
}
#[test]
fn absent_null_partial_wrong_type_and_duplicate_envelope_fields_are_distinct() {
    let base = serde_json::to_value(old_record()).unwrap();
    for fields in [
        serde_json::json!({"score_policy":null}),
        serde_json::json!({"score_profiles":null}),
        serde_json::json!({"score_policy":1}),
        serde_json::json!({"score_profiles":[]}),
        serde_json::json!({"score_policy":null,"score_profiles":[]}),
        serde_json::json!({"score_policy":1,"score_profiles":null}),
        serde_json::json!({"score_policy":2,"score_profiles":[]}),
        serde_json::json!({"score_policy":1.0,"score_profiles":[]}),
        serde_json::json!({"score_policy":"1","score_profiles":[]}),
        serde_json::json!({"score_policy":1,"score_profiles":{}}),
    ] {
        let mut v = base.clone();
        for (k, x) in fields.as_object().unwrap() {
            v[k] = x.clone();
        }
        assert!(
            decode_manifest(&serde_json::to_vec(&vec![v]).unwrap()).is_err(),
            "{fields}"
        );
    }
    let mut encoded = serde_json::to_string(&base).unwrap();
    encoded.pop();
    encoded.push_str(",\"score_policy\":1,\"score_policy\":1,\"score_profiles\":[]}");
    assert!(decode_manifest(format!("[{encoded}]").as_bytes()).is_err());
}
#[test]
fn malformed_classes_preserve_unrelated_valid_classes_and_never_grant_scores() {
    let evidence = fixtures()
        .profiles
        .into_iter()
        .map(|c| c.evidence)
        .find(|e| e.request_class == RequestClass::TextReadOnly)
        .unwrap();
    let good = serde_json::to_value(&evidence).unwrap();
    let neighbor = fixtures()
        .profiles
        .into_iter()
        .map(|c| c.evidence)
        .find(|e| e.request_class == RequestClass::VisionOcr)
        .unwrap();
    let neighbor = serde_json::to_value(&neighbor).unwrap();
    let mut bads = vec![];
    for key in [
        "request_class",
        "successful_attempts",
        "mandatory_check_mask",
        "successful_duration_ms",
        "locality",
    ] {
        let mut b = good.clone();
        b.as_object_mut().unwrap().remove(key);
        bads.push(b);
        let mut b = good.clone();
        b[key] = Value::Null;
        bads.push(b);
    }
    for (key, value) in [
        ("request_class", serde_json::json!("unknown")),
        ("successful_attempts", serde_json::json!(3)),
        ("successful_attempts", serde_json::json!(6)),
        ("successful_attempts", serde_json::json!(4.0)),
        ("mandatory_check_mask", serde_json::json!(6)),
        ("mandatory_check_mask", serde_json::json!(15)),
        ("mandatory_check_mask", serde_json::json!(7.0)),
        ("successful_duration_ms", serde_json::json!([1, 2, 3])),
        (
            "successful_duration_ms",
            serde_json::json!([1, 2, 3, 900001]),
        ),
        ("successful_duration_ms", serde_json::json!([1, 2, 3, -1])),
        ("successful_duration_ms", serde_json::json!([1, 2, 3, 1.0])),
        ("locality", serde_json::json!("near")),
        ("extra", serde_json::json!(true)),
    ] {
        let mut b = good.clone();
        b[key] = value;
        bads.push(b);
    }
    for bad in bads {
        let record = decode_profiles(&serde_json::to_string(&vec![bad, neighbor.clone()]).unwrap());
        assert!(
            record
                .score_profile(RequestClass::TextReadOnly, ExecutionLocality::SameHost)
                .is_err()
        );
        assert!(
            record
                .score_profile(RequestClass::VisionOcr, ExecutionLocality::SameHost)
                .is_ok()
        );
    }
    let record = decode_profiles(
        &serde_json::to_string(&vec![good.clone(), good, neighbor.clone()]).unwrap(),
    );
    assert!(
        record
            .score_profile(RequestClass::TextReadOnly, ExecutionLocality::SameHost)
            .is_err()
    );
    assert!(
        record
            .score_profile(RequestClass::VisionOcr, ExecutionLocality::SameHost)
            .is_ok()
    );
    let mut dup = serde_json::to_string(&evidence).unwrap();
    dup.pop();
    dup.push_str(",\"successful_attempts\":4}");
    let record = decode_profiles(&format!(
        "[{dup},{}]",
        serde_json::to_string(&neighbor).unwrap()
    ));
    assert!(
        record
            .score_profile(RequestClass::TextReadOnly, ExecutionLocality::SameHost)
            .is_err()
    );
    assert!(
        record
            .score_profile(RequestClass::VisionOcr, ExecutionLocality::SameHost)
            .is_ok()
    );
}

#[test]
fn canonical_writer_sorts_classes_and_rejects_every_invalid_programmatic_pair() {
    let mut record = all_record();
    record.score_policy = Some(1);
    let mut evidence: Vec<_> = fixtures()
        .profiles
        .into_iter()
        .map(|c| c.evidence)
        .filter(|e| e.locality == ExecutionLocality::SameHost)
        .collect();
    evidence.reverse();
    record.score_profiles = Some(evidence);
    let encoded = encode_v2(&[record.clone()]).unwrap();
    let value: Value = serde_json::from_slice(&encoded).unwrap();
    let classes: Vec<RequestClass> = value[0]["score_profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| serde_json::from_value(p["request_class"].clone()).unwrap())
        .collect();
    assert_eq!(classes, RequestClass::ALL);
    let ManifestDocument::V2(decoded) = decode_manifest(&encoded).unwrap() else {
        panic!()
    };
    for class in RequestClass::ALL {
        assert!(
            decoded.records()[0]
                .score_profile(class, ExecutionLocality::PublicRemote)
                .is_ok()
        );
    }
    let mut invalid = record.clone();
    invalid.score_policy = None;
    assert!(encode_v2(&[invalid]).is_err());
    let mut invalid = record.clone();
    invalid.score_profiles = None;
    assert!(encode_v2(&[invalid]).is_err());
    let mut invalid = record.clone();
    invalid.score_policy = Some(2);
    assert!(encode_v2(&[invalid]).is_err());
    let mut invalid = record.clone();
    invalid.score_profiles.as_mut().unwrap()[0].mandatory_check_mask = 255;
    assert!(encode_v2(&[invalid]).is_err());
    let mut invalid = record.clone();
    let duplicate = invalid.score_profiles.as_ref().unwrap()[0].clone();
    invalid.score_profiles.as_mut().unwrap().push(duplicate);
    assert!(encode_v2(&[invalid]).is_err());
    let mut invalid = record;
    invalid.declared_capabilities.ocr = false;
    assert!(encode_v2(&[invalid]).is_err());
}
#[test]
fn score_bearing_empty_or_incompatible_classes_never_fall_back_to_compatibility() {
    let record = decode_profiles("[]");
    for class in RequestClass::ALL {
        assert!(
            record
                .score_profile(class, ExecutionLocality::SameHost)
                .is_err()
        );
    }
    let evidence = fixtures()
        .profiles
        .into_iter()
        .map(|c| c.evidence)
        .find(|e| e.request_class == RequestClass::TextWithTools)
        .unwrap();
    let mut record = old_record();
    record.score_policy = Some(1);
    record.score_profiles = Some(vec![evidence]);
    let bytes = serde_json::to_vec(&vec![record]).unwrap();
    let ManifestDocument::V2(decoded) = decode_manifest(&bytes).unwrap() else {
        panic!()
    };
    for class in RequestClass::ALL {
        assert!(
            decoded.records()[0]
                .score_profile(class, ExecutionLocality::SameHost)
                .is_err()
        );
    }
}
#[test]
fn duplicate_request_class_key_poisoning_is_local_and_order_independent() {
    let all = fixtures();
    let text = all.profiles[0].evidence.clone();
    let vision = all
        .profiles
        .iter()
        .find(|p| p.evidence.request_class == RequestClass::VisionDescribe)
        .unwrap()
        .evidence
        .clone();
    let good = all
        .profiles
        .iter()
        .find(|p| p.evidence.request_class == RequestClass::VisionOcr)
        .unwrap()
        .evidence
        .clone();
    let mut duplicate = serde_json::to_string(&text).unwrap();
    duplicate.pop();
    duplicate.push_str(",\"request_class\":\"vision_describe\"}");
    for raw in [
        format!(
            "[{duplicate},{},{}]",
            serde_json::to_string(&vision).unwrap(),
            serde_json::to_string(&good).unwrap()
        ),
        format!(
            "[{},{},{duplicate}]",
            serde_json::to_string(&good).unwrap(),
            serde_json::to_string(&vision).unwrap()
        ),
    ] {
        let record = decode_profiles(&raw);
        assert!(
            record
                .score_profile(RequestClass::TextReadOnly, ExecutionLocality::SameHost)
                .is_err()
        );
        assert!(
            record
                .score_profile(RequestClass::VisionDescribe, ExecutionLocality::SameHost)
                .is_err()
        );
        assert!(
            record
                .score_profile(RequestClass::VisionOcr, ExecutionLocality::SameHost)
                .is_ok()
        );
    }
}

#[test]
fn legacy_projection_preserves_overall_target_time_and_fm_envelope_rejections() {
    use super::super::qualification::{
        CapabilityEvidence, QualificationReport, QualificationTarget,
    };
    let ManifestDocument::LegacyV1(report) = decode_manifest(include_bytes!(
        "../../tests/fixtures/provider-legacy-v1.json"
    ))
    .unwrap() else {
        panic!()
    };
    let expected = report.primary.identity.clone().unwrap();
    for mutate in [
        |r: &mut QualificationReport| r.overall_pass = false,
        |r: &mut QualificationReport| r.target = QualificationTarget::Fm,
        |r: &mut QualificationReport| r.generated_unix_secs = 301,
        |r: &mut QualificationReport| r.primary.configured = false,
    ] {
        let mut bad = report.clone();
        mutate(&mut bad);
        assert!(
            ManifestDocument::LegacyV1(bad)
                .legacy_score_profile(
                    LegacyScoreRoute::Primary,
                    &expected,
                    RequestClass::TextReadOnly,
                    ExecutionLocality::SameHost,
                    0
                )
                .is_err()
        );
    }
    let mut future = report.clone();
    future.generated_unix_secs = 300;
    assert!(
        ManifestDocument::LegacyV1(future)
            .legacy_score_profile(
                LegacyScoreRoute::Primary,
                &expected,
                RequestClass::TextReadOnly,
                ExecutionLocality::SameHost,
                0
            )
            .is_ok()
    );
    let mut fm = report;
    fm.target = QualificationTarget::Fm;
    fm.fm_cli = fm.primary.clone();
    let mut expected = expected;
    expected.mode = Some("system".into());
    fm.fm_cli.identity = Some(expected.clone());
    fm.fm_cli.capabilities.tools = CapabilityEvidence::pass();
    fm.fm_cli.capabilities.structured_output = CapabilityEvidence::pass();
    let valid = ManifestDocument::LegacyV1(fm.clone());
    assert!(
        valid
            .legacy_score_profile(
                LegacyScoreRoute::FmCli,
                &expected,
                RequestClass::TextReadOnly,
                ExecutionLocality::SameHost,
                0
            )
            .is_ok()
    );
    for mutate in [
        |r: &mut QualificationReport| {
            r.fm_cli.capabilities.tools = CapabilityEvidence::unsupported()
        },
        |r: &mut QualificationReport| r.fm_cli.configured = false,
        |r: &mut QualificationReport| r.fm_cli.capabilities.vision = CapabilityEvidence::pass(),
    ] {
        let mut bad = fm.clone();
        mutate(&mut bad);
        assert!(
            ManifestDocument::LegacyV1(bad)
                .legacy_score_profile(
                    LegacyScoreRoute::FmCli,
                    &expected,
                    RequestClass::TextReadOnly,
                    ExecutionLocality::SameHost,
                    0
                )
                .is_err()
        );
    }
    fm.fm_cli.capabilities.vision = CapabilityEvidence::pass();
    fm.fm_cli.vision_identity = Some(expected.clone());
    assert!(
        ManifestDocument::LegacyV1(fm.clone())
            .legacy_score_profile(
                LegacyScoreRoute::FmCli,
                &expected,
                RequestClass::VisionDescribe,
                ExecutionLocality::SameHost,
                0
            )
            .is_ok()
    );
    let mut pcc = expected;
    pcc.mode = Some("pcc".into());
    fm.fm_cli.identity = Some(pcc.clone());
    fm.fm_cli.vision_identity = Some(pcc.clone());
    assert!(
        ManifestDocument::LegacyV1(fm)
            .legacy_score_profile(
                LegacyScoreRoute::FmCli,
                &pcc,
                RequestClass::TextReadOnly,
                ExecutionLocality::SameHost,
                0
            )
            .is_err()
    );
}

#[test]
fn shared_rejected_evidence_is_not_publishable_and_never_projects_a_class() {
    for case in fixtures().rejections {
        let bad = case.apply(&fixtures().profiles[0].evidence);
        let parsed = decode_profiles(&format!("[{bad}]"));
        assert!(
            parsed
                .score_profile(RequestClass::TextReadOnly, ExecutionLocality::SameHost)
                .is_err()
        );
        if let Ok(evidence) = serde_json::from_value::<QualificationScoreEvidence>(bad) {
            let mut record = all_record();
            record.score_policy = Some(1);
            record.score_profiles = Some(vec![evidence]);
            assert!(encode_v2(&[record]).is_err());
        }
    }
}

#[test]
fn qualification_run_nonce_is_optional_but_strict_when_present() {
    let record = old_record();
    assert!(record.qualification_run_nonce.is_none());
    for invalid in [
        serde_json::Value::Null,
        serde_json::json!(true),
        serde_json::json!(7),
        serde_json::json!("AB".repeat(32)),
        serde_json::json!("ab".repeat(31)),
        serde_json::json!("gh".repeat(32)),
    ] {
        let mut value = serde_json::to_value(&record).unwrap();
        value["qualification_run_nonce"] = invalid;
        assert!(decode_manifest(&serde_json::to_vec(&vec![value]).unwrap()).is_err());
    }
    let mut witnessed = record;
    witnessed.qualification_run_nonce = Some("ab".repeat(32));
    witnessed.qualification_generation = Some(1);
    witnessed.qualification_completed_unix_secs = Some(0);
    assert!(decode_manifest(&encode_v2(&[witnessed]).unwrap()).is_ok());
}

#[test]
fn qualification_witness_group_requires_bounded_complete_ordering_evidence() {
    let mut record = old_record();
    record.qualification_run_nonce = Some("ab".repeat(32));
    record.qualification_generation = Some(1);
    record.qualification_completed_unix_secs = Some(10);
    let complete = serde_json::to_value(record).unwrap();
    for key in [
        "qualification_run_nonce",
        "qualification_generation",
        "qualification_completed_unix_secs",
    ] {
        let mut missing = complete.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(decode_manifest(&serde_json::to_vec(&vec![missing]).unwrap()).is_err());
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!(true),
            serde_json::json!(1.5),
            serde_json::json!("1"),
            serde_json::json!(u64::MAX),
        ] {
            let mut wrong = complete.clone();
            wrong[key] = invalid;
            assert!(decode_manifest(&serde_json::to_vec(&vec![wrong]).unwrap()).is_err());
        }
    }
    let mut zero_generation = complete;
    zero_generation["qualification_generation"] = serde_json::json!(0);
    assert!(decode_manifest(&serde_json::to_vec(&vec![zero_generation]).unwrap()).is_err());
}
