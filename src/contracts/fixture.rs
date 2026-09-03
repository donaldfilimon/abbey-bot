use super::schema::{StrictJsonError, strict_value, validate_schema};
use super::{
    ARTIFACTS, ContractCorpus, ContractError, ContractErrorCode, DecodedFixture, FixtureCheck,
    FixtureDisposition, OperatorVerificationReport, SchemaRegistry, registry::schema_registry,
};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn taxonomy(path: &'static str) -> Option<&'static str> {
    path.strip_prefix("v1/fixtures/")?.split('/').next()
}

pub(crate) fn privacy_safe(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().all(|(key, value)| {
            !matches!(
                key.to_ascii_lowercase().as_str(),
                "audio"
                    | "transcript"
                    | "message"
                    | "prompt"
                    | "response_text"
                    | "credential"
                    | "token"
                    | "password"
                    | "username"
                    | "display_name"
                    | "filesystem_path"
                    | "participant_identity"
            ) && privacy_safe(value)
        }),
        Value::Array(values) => values.iter().all(privacy_safe),
        Value::String(text) => {
            !(text.bytes().all(|byte| byte.is_ascii_digit()) && (17..=20).contains(&text.len()))
                && !["/Users/", "/home/", "C:\\", "sk-", "ghp_"]
                    .iter()
                    .any(|prefix| text.starts_with(prefix))
        }
        _ => true,
    }
}

pub(crate) fn numeric_domain_invalid(value: &Value) -> bool {
    const MAX_SAFE: u64 = 9_007_199_254_740_991;
    match value {
        Value::Object(object) => object.values().any(numeric_domain_invalid),
        Value::Array(values) => values.iter().any(numeric_domain_invalid),
        Value::Number(number) => {
            number.as_u64().is_some_and(|number| number > MAX_SAFE)
                || number
                    .as_i64()
                    .is_some_and(|number| number.unsigned_abs() > MAX_SAFE)
                || number
                    .as_f64()
                    .is_some_and(|number| number.abs() > MAX_SAFE as f64)
        }
        _ => false,
    }
}

pub(crate) fn pre_schema_disposition(
    schema_id: &str,
    document: &Value,
) -> Option<FixtureDisposition> {
    let object = document.as_object()?;
    if schema_id.ends_with("/learning/promotion-candidate.schema.json") {
        let forbidden = [
            "grant",
            "approval",
            "safety_policy_mutation",
            "command_registration",
            "platform_write",
            "direct_platform_write",
        ];
        if forbidden.iter().any(|key| object.contains_key(*key)) {
            return Some(FixtureDisposition::LearningAuthorityForbidden);
        }
    }
    if schema_id.ends_with("/episode/proposal.schema.json")
        && object.get("priority_class").and_then(Value::as_str) == Some("MandatoryIncident")
        && (object.get("minimized").and_then(Value::as_bool) != Some(true)
            || object.get("redacted").and_then(Value::as_bool) != Some(true)
            || object.get("deletion_required").and_then(Value::as_bool) != Some(true)
            || object.get("deletion_key").and_then(Value::as_str).is_none()
            || object.get("retention_class").and_then(Value::as_str) != Some("mandatory_incident")
            || !matches!(
                object.get("hold_state").and_then(Value::as_str),
                Some("active" | "released")
            ))
    {
        return Some(FixtureDisposition::MandatoryControlsMissing);
    }
    None
}

pub(crate) fn semantic_disposition(
    schema_id: &str,
    document: &Value,
) -> Option<FixtureDisposition> {
    let object = document.as_object()?;
    if schema_id.ends_with("/identity/delegation-chain.schema.json") {
        let hops = object.get("hops").and_then(Value::as_array)?;
        for pair in hops.windows(2) {
            if pair[0].get("delegatee_principal_id") != pair[1].get("delegator_principal_id") {
                return Some(FixtureDisposition::DelegationChainBroken);
            }
        }
        let mut seen = BTreeSet::new();
        if let Some(first) = hops
            .first()
            .and_then(|hop| hop.get("delegator_principal_id"))
            .and_then(Value::as_str)
        {
            seen.insert(first);
        }
        for hop in hops {
            if let Some(delegatee) = hop.get("delegatee_principal_id").and_then(Value::as_str)
                && !seen.insert(delegatee)
            {
                return Some(FixtureDisposition::DelegationCycle);
            }
        }
    }
    if schema_id.ends_with("/authorization/approval.schema.json")
        && object.get("approver_principal_id") == object.get("request_subject_principal_id")
    {
        return Some(FixtureDisposition::SelfApproval);
    }
    if schema_id.ends_with("/authorization/policy-decision.schema.json")
        && object.get("reason_code").and_then(Value::as_str) == Some("dependency_unavailable")
        && object.get("decision").and_then(Value::as_str) != Some("deny")
    {
        return Some(FixtureDisposition::DegradedAuthority);
    }
    if schema_id.ends_with("/cognition/request.schema.json")
        && matches!(
            object.get("effect_class").and_then(Value::as_str),
            Some("durable_write" | "platform_effect")
        )
        && object.get("idempotency_key").is_none_or(Value::is_null)
    {
        return Some(FixtureDisposition::IdempotencyRequired);
    }
    if schema_id.ends_with("/event/cancellation.schema.json")
        && object.get("cancellation_reference") != object.get("target_cancellation_reference")
    {
        return Some(FixtureDisposition::CancellationMismatch);
    }
    if schema_id.ends_with("/consent/transition.schema.json") {
        let from = object.get("from_state").and_then(Value::as_str);
        let to = object.get("to_state").and_then(Value::as_str);
        let allowed = matches!(
            (from, to),
            (Some("Closed"), Some("PendingAttestation"))
                | (Some("PendingAttestation"), Some("Open"))
                | (Some("Open"), Some("Closing"))
                | (Some("Closing"), Some("Closed"))
        );
        if !allowed {
            return Some(
                if matches!(
                    object.get("reason_code").and_then(Value::as_str),
                    Some(
                        "participant_change"
                            | "unidentified_participant"
                            | "attestation_lost"
                            | "manager_deauthorized"
                            | "connection_lost"
                            | "explicit_stop"
                    )
                ) {
                    FixtureDisposition::ConsentCloseRequired
                } else {
                    FixtureDisposition::ConsentTransitionInvalid
                },
            );
        }
        if to == Some("Open")
            && (object.get("manager_authorized").and_then(Value::as_bool) != Some(true)
                || object
                    .get("all_current_participants_consented")
                    .and_then(Value::as_bool)
                    != Some(true)
                || object
                    .get("participant_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    == 0)
        {
            return Some(FixtureDisposition::ConsentOpenDenied);
        }
        if to == Some("Closing") {
            let required = BTreeSet::from([
                "decoded_receive",
                "stt",
                "reasoning",
                "synthesis",
                "provider",
                "playback",
            ]);
            let actual = object
                .get("cancelled_stages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>();
            if actual != required {
                return Some(FixtureDisposition::ConsentCancellationIncomplete);
            }
        }
    }
    if schema_id.ends_with("/episode/claim.schema.json") {
        let level = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .and_then(|value| value.strip_prefix('C'))
                .and_then(|value| value.parse::<u8>().ok())
        };
        if level("display_evidence_level").unwrap_or(8) > level("evidence_level").unwrap_or(0) {
            return Some(FixtureDisposition::EvidenceOverclaim);
        }
    }
    if schema_id.ends_with("/episode/proposal.schema.json") {
        if object.get("payload_mode").and_then(Value::as_str) == Some("reference")
            && object.get("payload_reference").is_none_or(Value::is_null)
        {
            return Some(FixtureDisposition::PayloadReferenceRequired);
        }
        if object.get("payload_mode").and_then(Value::as_str) == Some("redacted_empty")
            && object
                .get("redacted_summary_code")
                .is_none_or(Value::is_null)
        {
            return Some(FixtureDisposition::RedactedSummaryRequired);
        }
    }
    if schema_id.ends_with("/learning/guild-learning-policy.schema.json") {
        if matches!(
            object.get("state").and_then(Value::as_str),
            Some("Unset" | "ExplicitDisabled")
        ) && object
            .get("adaptive_update_allowed")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Some(FixtureDisposition::LearningDisabled);
        }
        if object.get("quiet_override").and_then(Value::as_bool) == Some(true)
            && object
                .get("unsolicited_action_allowed")
                .and_then(Value::as_bool)
                == Some(true)
        {
            return Some(FixtureDisposition::QuietOverride);
        }
    }
    None
}

pub(crate) fn decode_fixture_with_registry(
    path: &'static str,
    bytes: &[u8],
    registry: &SchemaRegistry,
) -> Result<DecodedFixture, ContractError> {
    let wrapper: super::FixtureWrapper = serde_json::from_slice(bytes)
        .map_err(|_| ContractError::new(ContractErrorCode::FixtureInvalid, Some(path)))?;
    if wrapper.case_id.is_empty() {
        return Err(ContractError::new(
            ContractErrorCode::FixtureInvalid,
            Some(path),
        ));
    }
    let expected = FixtureDisposition::from_code(&wrapper.expect)
        .ok_or_else(|| ContractError::new(ContractErrorCode::FixtureInvalid, Some(path)))?;
    let strict = strict_value(bytes);
    let actual = match strict {
        Err(StrictJsonError::DuplicateMember) => FixtureDisposition::DuplicateMember,
        Err(StrictJsonError::Invalid) => {
            return Err(ContractError::new(
                ContractErrorCode::FixtureInvalid,
                Some(path),
            ));
        }
        Ok(_) if !privacy_safe(&wrapper.document) => FixtureDisposition::ForbiddenContent,
        Ok(_) if numeric_domain_invalid(&wrapper.document) => FixtureDisposition::NumericDomain,
        Ok(_) => {
            let schema = registry
                .get(&wrapper.schema)
                .ok_or_else(|| ContractError::new(ContractErrorCode::FixtureInvalid, Some(path)))?;
            if let Some(disposition) = pre_schema_disposition(&wrapper.schema, &wrapper.document) {
                disposition
            } else {
                let max_bytes = schema
                    .get("x-abbey-max-bytes")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        ContractError::new(ContractErrorCode::SchemaInvalid, Some(path))
                    })? as usize;
                let mut encoded = serde_json::to_vec_pretty(&wrapper.document).map_err(|_| {
                    ContractError::new(ContractErrorCode::FixtureInvalid, Some(path))
                })?;
                encoded.push(b'\n');
                if encoded.len() > max_bytes
                    || !validate_schema(&wrapper.document, schema, &wrapper.schema, registry)
                {
                    FixtureDisposition::SchemaInvalid
                } else {
                    semantic_disposition(&wrapper.schema, &wrapper.document)
                        .unwrap_or(FixtureDisposition::Valid)
                }
            }
        }
    };
    Ok(DecodedFixture {
        expected,
        actual,
        document: wrapper.document,
    })
}

impl ContractCorpus {
    pub(crate) fn fixture_checks() -> Result<Vec<FixtureCheck>, ContractError> {
        let registry = schema_registry()?;
        ARTIFACTS
            .iter()
            .filter_map(|artifact| taxonomy(artifact.path).map(|family| (artifact, family)))
            .map(|(artifact, family)| {
                let decoded =
                    decode_fixture_with_registry(artifact.path, artifact.bytes, &registry)?;
                Ok(FixtureCheck {
                    taxonomy: family,
                    expected: decoded.expected,
                    actual: decoded.actual,
                })
            })
            .collect()
    }

    pub(crate) fn decode_fixture(
        path: &'static str,
        bytes: &[u8],
    ) -> Result<DecodedFixture, ContractError> {
        decode_fixture_with_registry(path, bytes, &schema_registry()?)
    }

    pub(crate) fn decode_embedded_fixture(
        path: &'static str,
    ) -> Result<DecodedFixture, ContractError> {
        let bytes = Self::artifact_bytes(path)
            .ok_or_else(|| ContractError::new(ContractErrorCode::InventoryMismatch, Some(path)))?;
        Self::decode_fixture(path, bytes)
    }

    pub(crate) fn synthetic_operator_report() -> Result<OperatorVerificationReport, ContractError> {
        let decoded =
            Self::decode_embedded_fixture("v1/fixtures/valid/consent-operator-flow.json")?;
        if decoded.actual != FixtureDisposition::Valid {
            return Err(ContractError::new(
                ContractErrorCode::FixtureMismatch,
                Some("v1/fixtures/valid/consent-operator-flow.json"),
            ));
        }
        serde_json::from_value(decoded.document).map_err(|_| {
            ContractError::new(
                ContractErrorCode::FixtureInvalid,
                Some("v1/fixtures/valid/consent-operator-flow.json"),
            )
        })
    }
}
