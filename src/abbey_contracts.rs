//! Data-only qualification for the pinned Abbey Program 1 corpus.

use regex::Regex;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize as DeriveDeserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const PINNED_REPOSITORY: &str = "https://github.com/donaldfilimon/abi";
const PINNED_REVISION: &str = "348754bdaaf59a40fbb858380f925e0aba95a23b";
const PINNED_DIGEST: &str = "72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e";
const PINNED_ARTIFACT_COUNT: usize = 81;
const PINNED_TOTAL_BYTES: usize = 88_328;
const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;
const MAX_CORPUS_BYTES: usize = 16 * 1024 * 1024;
const AGGREGATE_DOMAIN: &[u8] = b"abbey-contract-corpus-v1\0";
const LOCK_BYTES: &[u8] = include_bytes!("../contracts/abbey/abbey-contracts.lock.json");
const MANIFEST_BYTES: &[u8] = include_bytes!("../contracts/abbey/corpus/manifest.json");

macro_rules! artifact {
    ($path:literal) => {
        EmbeddedArtifact {
            path: $path,
            bytes: include_bytes!(concat!("../contracts/abbey/corpus/", $path)),
        }
    };
}

const ARTIFACTS: &[EmbeddedArtifact] = &[
    artifact!("README.md"),
    artifact!("compatibility.md"),
    artifact!("v1/fixtures/boundary/identity-delegation-nine-hops.json"),
    artifact!("v1/fixtures/boundary/jcs-number-domain.json"),
    artifact!("v1/fixtures/cancellation/consent-barge-in.json"),
    artifact!("v1/fixtures/cancellation/consent-participant-change-close.json"),
    artifact!("v1/fixtures/cancellation/execution-actuator-race.json"),
    artifact!("v1/fixtures/cancellation/execution-before-start.json"),
    artifact!("v1/fixtures/cancellation/execution-stale-cancellation.json"),
    artifact!("v1/fixtures/degraded/authorization-dependency-allow.json"),
    artifact!("v1/fixtures/degraded/consent-connection-loss.json"),
    artifact!("v1/fixtures/degraded/execution-partial-rollback.json"),
    artifact!("v1/fixtures/degraded/learning-quiet-override.json"),
    artifact!("v1/fixtures/invalid/authorization-prohibited-grant.json"),
    artifact!("v1/fixtures/invalid/authorization-self-approval.json"),
    artifact!("v1/fixtures/invalid/authorization-unknown-field.json"),
    artifact!("v1/fixtures/invalid/consent-open-without-manager.json"),
    artifact!("v1/fixtures/invalid/consent-participant-change-stays-open.json"),
    artifact!("v1/fixtures/invalid/episode-adapter-digest.json"),
    artifact!("v1/fixtures/invalid/episode-mandatory-unbounded.json"),
    artifact!("v1/fixtures/invalid/episode-overstated-claim.json"),
    artifact!("v1/fixtures/invalid/execution-missing-idempotency.json"),
    artifact!("v1/fixtures/invalid/execution-nonterminal-response.json"),
    artifact!("v1/fixtures/invalid/identity-delegation-cycle.json"),
    artifact!("v1/fixtures/invalid/identity-wildcard-scope.json"),
    artifact!("v1/fixtures/invalid/jcs-duplicate-member.json"),
    artifact!("v1/fixtures/invalid/learning-disabled-update.json"),
    artifact!("v1/fixtures/invalid/learning-unset-update.json"),
    artifact!("v1/fixtures/privacy/authorization-error-cause.json"),
    artifact!("v1/fixtures/privacy/consent-report-content.json"),
    artifact!("v1/fixtures/privacy/execution-receipt-content.json"),
    artifact!("v1/fixtures/privacy/identity-raw-snowflake.json"),
    artifact!("v1/fixtures/privacy/learning-authority-payload.json"),
    artifact!("v1/fixtures/unknown-field/execution-metadata-extension.json"),
    artifact!("v1/fixtures/valid/authorization-approval.json"),
    artifact!("v1/fixtures/valid/authorization-error.json"),
    artifact!("v1/fixtures/valid/authorization-grant.json"),
    artifact!("v1/fixtures/valid/authorization-prohibited-package.json"),
    artifact!("v1/fixtures/valid/consent-open-transition.json"),
    artifact!("v1/fixtures/valid/consent-operator-flow.json"),
    artifact!("v1/fixtures/valid/corpus-digest-vector.json"),
    artifact!("v1/fixtures/valid/episode-claim.json"),
    artifact!("v1/fixtures/valid/episode-evidence.json"),
    artifact!("v1/fixtures/valid/episode-mandatory-incident.json"),
    artifact!("v1/fixtures/valid/episode-proposal.json"),
    artifact!("v1/fixtures/valid/episode-tombstone.json"),
    artifact!("v1/fixtures/valid/execution-action-proposal.json"),
    artifact!("v1/fixtures/valid/execution-approved-proposal.json"),
    artifact!("v1/fixtures/valid/execution-complete-response.json"),
    artifact!("v1/fixtures/valid/execution-recommendation.json"),
    artifact!("v1/fixtures/valid/execution-request-envelope.json"),
    artifact!("v1/fixtures/valid/identity-delegation-chain.json"),
    artifact!("v1/fixtures/valid/identity-principal.json"),
    artifact!("v1/fixtures/valid/jcs-vector.json"),
    artifact!("v1/schemas/authorization/approval.schema.json"),
    artifact!("v1/schemas/authorization/grant.schema.json"),
    artifact!("v1/schemas/authorization/policy-decision.schema.json"),
    artifact!("v1/schemas/authorization/scope.schema.json"),
    artifact!("v1/schemas/capability/action-proposal.schema.json"),
    artifact!("v1/schemas/capability/execution-request.schema.json"),
    artifact!("v1/schemas/capability/package.schema.json"),
    artifact!("v1/schemas/capability/recommendation.schema.json"),
    artifact!("v1/schemas/cognition/request.schema.json"),
    artifact!("v1/schemas/cognition/response.schema.json"),
    artifact!("v1/schemas/common/definitions.schema.json"),
    artifact!("v1/schemas/consent/barge-in.schema.json"),
    artifact!("v1/schemas/consent/epoch.schema.json"),
    artifact!("v1/schemas/consent/operator-verification-report.schema.json"),
    artifact!("v1/schemas/consent/transition.schema.json"),
    artifact!("v1/schemas/episode/claim.schema.json"),
    artifact!("v1/schemas/episode/evidence.schema.json"),
    artifact!("v1/schemas/episode/proposal.schema.json"),
    artifact!("v1/schemas/episode/tombstone.schema.json"),
    artifact!("v1/schemas/error/error.schema.json"),
    artifact!("v1/schemas/event/cancellation.schema.json"),
    artifact!("v1/schemas/event/metadata-event.schema.json"),
    artifact!("v1/schemas/identity/delegation-chain.schema.json"),
    artifact!("v1/schemas/identity/principal.schema.json"),
    artifact!("v1/schemas/learning/guild-learning-policy.schema.json"),
    artifact!("v1/schemas/learning/promotion-candidate.schema.json"),
    artifact!("v1/schemas/receipt/outcome-receipt.schema.json"),
];

#[derive(Clone, Copy)]
struct EmbeddedArtifact {
    path: &'static str,
    bytes: &'static [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContractErrorCode {
    LockInvalid,
    ManifestInvalid,
    InventoryMismatch,
    ArtifactLengthMismatch,
    ArtifactDigestMismatch,
    AggregateDigestMismatch,
    SchemaInvalid,
    FixtureInvalid,
    FixtureMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContractError {
    code: ContractErrorCode,
    path: Option<&'static str>,
}

impl ContractError {
    const fn new(code: ContractErrorCode, path: Option<&'static str>) -> Self {
        Self { code, path }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QualifiedDigest {
    aggregate_digest: &'static str,
    artifact_count: usize,
    total_bytes: usize,
    contract_major: u64,
    contract_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureDisposition {
    Valid,
    SchemaInvalid,
    NumericDomain,
    CancellationMismatch,
    DegradedAuthority,
    SelfApproval,
    ConsentOpenDenied,
    ConsentCloseRequired,
    ConsentTransitionInvalid,
    ConsentCancellationIncomplete,
    MandatoryControlsMissing,
    EvidenceOverclaim,
    IdempotencyRequired,
    DelegationCycle,
    DelegationChainBroken,
    DuplicateMember,
    LearningDisabled,
    QuietOverride,
    ForbiddenContent,
    LearningAuthorityForbidden,
    PayloadReferenceRequired,
    RedactedSummaryRequired,
}

impl FixtureDisposition {
    fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "valid" => Self::Valid,
            "schema_invalid" => Self::SchemaInvalid,
            "numeric_domain" => Self::NumericDomain,
            "cancellation_mismatch" => Self::CancellationMismatch,
            "degraded_authority" => Self::DegradedAuthority,
            "self_approval" => Self::SelfApproval,
            "consent_open_denied" => Self::ConsentOpenDenied,
            "consent_close_required" => Self::ConsentCloseRequired,
            "consent_transition_invalid" => Self::ConsentTransitionInvalid,
            "consent_cancellation_incomplete" => Self::ConsentCancellationIncomplete,
            "mandatory_controls_missing" => Self::MandatoryControlsMissing,
            "evidence_overclaim" => Self::EvidenceOverclaim,
            "idempotency_required" => Self::IdempotencyRequired,
            "delegation_cycle" => Self::DelegationCycle,
            "delegation_chain_broken" => Self::DelegationChainBroken,
            "duplicate_member" => Self::DuplicateMember,
            "learning_disabled" => Self::LearningDisabled,
            "quiet_override" => Self::QuietOverride,
            "forbidden_content" => Self::ForbiddenContent,
            "learning_authority_forbidden" => Self::LearningAuthorityForbidden,
            "payload_reference_required" => Self::PayloadReferenceRequired,
            "redacted_summary_required" => Self::RedactedSummaryRequired,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy)]
struct FixtureCheck {
    taxonomy: &'static str,
    expected: FixtureDisposition,
    actual: FixtureDisposition,
}

struct DecodedFixture {
    expected: FixtureDisposition,
    actual: FixtureDisposition,
    document: Value,
}

impl fmt::Debug for DecodedFixture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedFixture")
            .field("expected", &self.expected)
            .field("actual", &self.actual)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, DeriveDeserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceClassification {
    LocalTest,
    InstalledArtifact,
    LiveDiscord,
}

#[derive(Debug, DeriveDeserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AuthorizationResult {
    Authorized,
    Denied,
    Lost,
}

#[derive(Debug, DeriveDeserialize)]
#[serde(deny_unknown_fields)]
struct OperatorVerificationReport {
    verifier_build_id: String,
    contract_revision: u64,
    authorization_result: AuthorizationResult,
    epoch_open_count: u64,
    epoch_close_count: u64,
    participant_change_count: u64,
    decoded_receive_count: u64,
    stt_completion_count: u64,
    synthesis_completion_count: u64,
    playback_completion_count: u64,
    barge_in_cancellation_count: u64,
    pause_count: u64,
    resume_count: u64,
    final_leave_count: u64,
    duration_ms: String,
    terminal_status: String,
    evidence_classification: EvidenceClassification,
    redacted: bool,
    truncated: bool,
}

#[derive(DeriveDeserialize)]
#[serde(deny_unknown_fields)]
struct ContractLock {
    source_repository: String,
    source_revision: String,
    contract_major: u64,
    contract_revision: u64,
    aggregate_digest: String,
}

#[derive(DeriveDeserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    contract_major: u64,
    contract_revision: u64,
    algorithm: String,
    redaction_profile: String,
    artifacts: Vec<ManifestArtifact>,
    aggregate_digest: String,
}

#[derive(DeriveDeserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestArtifact {
    path: String,
    bytes: usize,
    media_type: String,
    sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_id: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(deny_unknown_fields)]
struct FixtureWrapper {
    case_id: String,
    schema: String,
    expect: String,
    document: Value,
}

struct ContractCorpus;

impl ContractCorpus {
    fn embedded_lock() -> &'static [u8] {
        LOCK_BYTES
    }

    fn embedded_manifest() -> &'static [u8] {
        MANIFEST_BYTES
    }

    fn embedded_artifacts() -> &'static [EmbeddedArtifact] {
        ARTIFACTS
    }

    fn artifact_bytes(path: &str) -> Option<&'static [u8]> {
        ARTIFACTS
            .iter()
            .find(|artifact| artifact.path == path)
            .map(|artifact| artifact.bytes)
    }

    fn verify_embedded() -> Result<QualifiedDigest, ContractError> {
        let qualified = Self::verify_bytes(LOCK_BYTES, MANIFEST_BYTES, ARTIFACTS)?;
        let checks = Self::fixture_checks()?;
        if let Some(check) = checks.iter().find(|check| check.expected != check.actual) {
            return Err(ContractError::new(
                ContractErrorCode::FixtureMismatch,
                ARTIFACTS
                    .iter()
                    .find(|artifact| taxonomy(artifact.path) == Some(check.taxonomy))
                    .map(|artifact| artifact.path),
            ));
        }
        Ok(qualified)
    }

    fn verify_bytes(
        lock_bytes: &[u8],
        manifest_bytes: &[u8],
        artifacts: &[EmbeddedArtifact],
    ) -> Result<QualifiedDigest, ContractError> {
        let lock: ContractLock = serde_json::from_slice(lock_bytes)
            .map_err(|_| ContractError::new(ContractErrorCode::LockInvalid, None))?;
        if lock.source_repository != PINNED_REPOSITORY
            || lock.source_revision != PINNED_REVISION
            || lock.contract_major != 1
            || lock.contract_revision != 1
            || lock.aggregate_digest != PINNED_DIGEST
        {
            return Err(ContractError::new(ContractErrorCode::LockInvalid, None));
        }
        let manifest: Manifest = serde_json::from_slice(manifest_bytes)
            .map_err(|_| ContractError::new(ContractErrorCode::ManifestInvalid, None))?;
        if manifest.contract_major != 1
            || manifest.contract_revision != 1
            || manifest.algorithm != "abbey-contract-corpus-sha256-v1"
            || manifest.redaction_profile != "abbey-contract-redaction-v1"
            || manifest.aggregate_digest != PINNED_DIGEST
            || manifest.artifacts.len() != artifacts.len()
        {
            return Err(ContractError::new(ContractErrorCode::ManifestInvalid, None));
        }

        let actual = artifacts
            .iter()
            .map(|artifact| (artifact.path, artifact.bytes))
            .collect::<BTreeMap<_, _>>();
        if actual.len() != artifacts.len() {
            return Err(ContractError::new(
                ContractErrorCode::InventoryMismatch,
                None,
            ));
        }
        let mut total_bytes = 0usize;
        for row in &manifest.artifacts {
            let Some(bytes) = actual.get(row.path.as_str()) else {
                return Err(ContractError::new(
                    ContractErrorCode::InventoryMismatch,
                    None,
                ));
            };
            let static_path = artifacts
                .iter()
                .find(|artifact| artifact.path == row.path)
                .map(|artifact| artifact.path);
            if bytes.len() != row.bytes {
                return Err(ContractError::new(
                    ContractErrorCode::ArtifactLengthMismatch,
                    static_path,
                ));
            }
            total_bytes = total_bytes.checked_add(bytes.len()).ok_or_else(|| {
                ContractError::new(ContractErrorCode::ManifestInvalid, static_path)
            })?;
            if bytes.len() > MAX_ARTIFACT_BYTES || hex_digest(bytes) != row.sha256 {
                return Err(ContractError::new(
                    ContractErrorCode::ArtifactDigestMismatch,
                    static_path,
                ));
            }
        }
        if total_bytes > MAX_CORPUS_BYTES
            || total_bytes != PINNED_TOTAL_BYTES
            || artifacts.len() != PINNED_ARTIFACT_COUNT
        {
            return Err(ContractError::new(
                ContractErrorCode::InventoryMismatch,
                None,
            ));
        }
        if aggregate_digest(&manifest)? != PINNED_DIGEST {
            return Err(ContractError::new(
                ContractErrorCode::AggregateDigestMismatch,
                None,
            ));
        }
        Ok(QualifiedDigest {
            aggregate_digest: PINNED_DIGEST,
            artifact_count: artifacts.len(),
            total_bytes,
            contract_major: manifest.contract_major,
            contract_revision: manifest.contract_revision,
        })
    }

    fn fixture_checks() -> Result<Vec<FixtureCheck>, ContractError> {
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

    fn decode_fixture(path: &'static str, bytes: &[u8]) -> Result<DecodedFixture, ContractError> {
        decode_fixture_with_registry(path, bytes, &schema_registry()?)
    }

    fn decode_embedded_fixture(path: &'static str) -> Result<DecodedFixture, ContractError> {
        let bytes = Self::artifact_bytes(path)
            .ok_or_else(|| ContractError::new(ContractErrorCode::InventoryMismatch, Some(path)))?;
        Self::decode_fixture(path, bytes)
    }

    fn synthetic_operator_report() -> Result<OperatorVerificationReport, ContractError> {
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

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    lower_hex(&digest)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn aggregate_digest(manifest: &Manifest) -> Result<String, ContractError> {
    let mut zeroed = Manifest {
        contract_major: manifest.contract_major,
        contract_revision: manifest.contract_revision,
        algorithm: manifest.algorithm.clone(),
        redaction_profile: manifest.redaction_profile.clone(),
        artifacts: manifest
            .artifacts
            .iter()
            .map(|row| ManifestArtifact {
                path: row.path.clone(),
                bytes: row.bytes,
                media_type: row.media_type.clone(),
                sha256: row.sha256.clone(),
                schema_id: row.schema_id.clone(),
            })
            .collect(),
        aggregate_digest: "0".repeat(64),
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&zeroed)
        .map_err(|_| ContractError::new(ContractErrorCode::ManifestInvalid, None))?;
    manifest_bytes.push(b'\n');
    let mut entries = zeroed
        .artifacts
        .drain(..)
        .map(|row| (row.path, row.bytes, row.sha256))
        .collect::<Vec<_>>();
    entries.push((
        "manifest.json".to_owned(),
        manifest_bytes.len(),
        hex_digest(&manifest_bytes),
    ));
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut digest = Sha256::new();
    digest.update(AGGREGATE_DOMAIN);
    for (path, bytes, sha256) in entries {
        digest.update(path.as_bytes());
        digest.update(b"\0");
        digest.update(bytes.to_string().as_bytes());
        digest.update(b"\0");
        digest.update(sha256.as_bytes());
        digest.update(b"\n");
    }
    Ok(lower_hex(&digest.finalize()))
}

fn taxonomy(path: &'static str) -> Option<&'static str> {
    path.strip_prefix("v1/fixtures/")?.split('/').next()
}

#[derive(Debug)]
enum StrictJsonError {
    DuplicateMember,
    Invalid,
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a finite JSON value without duplicate members")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("non_finite_number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(StrictValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some((key, StrictValue(value))) = map.next_entry::<String, StrictValue>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate_member"));
            }
            values.insert(key, value);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

fn strict_value(bytes: &[u8]) -> Result<Value, StrictJsonError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    match StrictValue::deserialize(&mut deserializer) {
        Ok(StrictValue(value)) if deserializer.end().is_ok() => Ok(value),
        Ok(_) => Err(StrictJsonError::Invalid),
        Err(error) if error.to_string().starts_with("duplicate_member") => {
            Err(StrictJsonError::DuplicateMember)
        }
        Err(_) => Err(StrictJsonError::Invalid),
    }
}

type SchemaRegistry = BTreeMap<String, Value>;

fn schema_registry() -> Result<SchemaRegistry, ContractError> {
    let mut registry = BTreeMap::new();
    for artifact in ARTIFACTS
        .iter()
        .filter(|artifact| artifact.path.ends_with(".schema.json"))
    {
        let schema = strict_value(artifact.bytes).map_err(|_| {
            ContractError::new(ContractErrorCode::SchemaInvalid, Some(artifact.path))
        })?;
        let object = schema.as_object().ok_or_else(|| {
            ContractError::new(ContractErrorCode::SchemaInvalid, Some(artifact.path))
        })?;
        let schema_id = object
            .get("$id")
            .and_then(Value::as_str)
            .filter(|value| value.starts_with("https://abbey.local/contracts/abbey/v1/schemas/"))
            .ok_or_else(|| {
                ContractError::new(ContractErrorCode::SchemaInvalid, Some(artifact.path))
            })?;
        if object.get("$schema").and_then(Value::as_str)
            != Some("https://json-schema.org/draft/2020-12/schema")
            || !object.contains_key("x-abbey-data-class")
            || !object
                .get("x-abbey-max-bytes")
                .and_then(Value::as_u64)
                .is_some_and(|value| value > 0 && value <= MAX_ARTIFACT_BYTES as u64)
            || !matches!(
                object.get("x-abbey-unknown-fields").and_then(Value::as_str),
                Some("reject" | "extensions-only")
            )
            || registry.insert(schema_id.to_owned(), schema).is_some()
        {
            return Err(ContractError::new(
                ContractErrorCode::SchemaInvalid,
                Some(artifact.path),
            ));
        }
    }
    for (schema_id, schema) in &registry {
        if !references_are_local(schema, schema_id, &registry) {
            return Err(ContractError::new(ContractErrorCode::SchemaInvalid, None));
        }
    }
    Ok(registry)
}

fn references_are_local(value: &Value, owner_id: &str, registry: &SchemaRegistry) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref") {
                let Some(reference) = reference.as_str() else {
                    return false;
                };
                let base = reference.split('#').next().unwrap_or_default();
                let target = if base.is_empty() { owner_id } else { base };
                if !registry.contains_key(target) {
                    return false;
                }
            }
            object
                .values()
                .all(|nested| references_are_local(nested, owner_id, registry))
        }
        Value::Array(values) => values
            .iter()
            .all(|nested| references_are_local(nested, owner_id, registry)),
        _ => true,
    }
}

fn decode_fixture_with_registry(
    path: &'static str,
    bytes: &[u8],
    registry: &SchemaRegistry,
) -> Result<DecodedFixture, ContractError> {
    let wrapper: FixtureWrapper = serde_json::from_slice(bytes)
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

fn resolve_reference<'a>(
    reference: &str,
    owner_id: &str,
    registry: &'a SchemaRegistry,
) -> Option<(&'a Value, String)> {
    let (base, fragment) = reference
        .split_once('#')
        .map_or((reference, ""), |(base, fragment)| (base, fragment));
    let target_id = if base.is_empty() { owner_id } else { base };
    let mut target = registry.get(target_id)?;
    if !fragment.is_empty() {
        if !fragment.starts_with('/') {
            return None;
        }
        for token in fragment[1..].split('/') {
            let token = token.replace("~1", "/").replace("~0", "~");
            target = target.as_object()?.get(&token)?;
        }
    }
    Some((target, target_id.to_owned()))
}

fn validate_schema(
    value: &Value,
    schema: &Value,
    owner_id: &str,
    registry: &SchemaRegistry,
) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    if let Some(reference) = schema.get("$ref") {
        let Some(reference) = reference.as_str() else {
            return false;
        };
        let Some((target, target_id)) = resolve_reference(reference, owner_id, registry) else {
            return false;
        };
        return validate_schema(value, target, &target_id, registry);
    }
    if schema
        .get("const")
        .is_some_and(|constant| constant != value)
    {
        return false;
    }
    if schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|variants| !variants.contains(value))
    {
        return false;
    }
    if schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            schemas
                .iter()
                .any(|nested| !validate_schema(value, nested, owner_id, registry))
        })
    {
        return false;
    }
    if schema
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            schemas
                .iter()
                .filter(|nested| validate_schema(value, nested, owner_id, registry))
                .count()
                != 1
        })
    {
        return false;
    }
    if schema
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            !schemas
                .iter()
                .any(|nested| validate_schema(value, nested, owner_id, registry))
        })
    {
        return false;
    }
    if let Some(expected) = schema.get("type") {
        let matches = match expected {
            Value::String(expected) => json_type_matches(value, expected),
            Value::Array(expected) => expected
                .iter()
                .filter_map(Value::as_str)
                .any(|expected| json_type_matches(value, expected)),
            _ => false,
        };
        if !matches {
            return false;
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            return false;
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let Ok(expression) = Regex::new(&format!("^(?:{pattern})$")) else {
                return false;
            };
            if !expression.is_match(text) {
                return false;
            }
        }
    }
    if let Some(values) = value.as_array() {
        let length = values.len() as u64;
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            return false;
        }
        if schema.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
            let unique = values
                .iter()
                .filter_map(|item| serde_json::to_string(item).ok())
                .collect::<BTreeSet<_>>();
            if unique.len() != values.len() {
                return false;
            }
        }
        if let Some(item_schema) = schema.get("items")
            && values
                .iter()
                .any(|item| !validate_schema(item, item_schema, owner_id, registry))
        {
            return false;
        }
    }
    if let Some(object) = value.as_object() {
        if schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|key| !object.contains_key(key))
            })
        {
            return false;
        }
        let length = object.len() as u64;
        if schema
            .get("minProperties")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxProperties")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            return false;
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, item) in object {
            if let Some(property_schema) = properties.and_then(|known| known.get(key)) {
                if !validate_schema(item, property_schema, owner_id, registry) {
                    return false;
                }
                continue;
            }
            match schema.get("additionalProperties") {
                Some(Value::Bool(false)) => return false,
                Some(additional @ Value::Object(_))
                    if !validate_schema(item, additional, owner_id, registry) =>
                {
                    return false;
                }
                _ => {}
            }
        }
    }
    if let Some(number) = value.as_f64()
        && (schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|minimum| number < minimum)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|maximum| number > maximum))
    {
        return false;
    }
    true
}

fn json_type_matches(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn privacy_safe(value: &Value) -> bool {
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

fn numeric_domain_invalid(value: &Value) -> bool {
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

fn pre_schema_disposition(schema_id: &str, document: &Value) -> Option<FixtureDisposition> {
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

fn semantic_disposition(schema_id: &str, document: &Value) -> Option<FixtureDisposition> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    const DIGEST: &str = "72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e";

    #[test]
    fn embedded_corpus_has_the_exact_qualified_identity() {
        let qualified = ContractCorpus::verify_embedded().expect("qualified embedded corpus");

        assert_eq!(qualified.aggregate_digest, DIGEST);
        assert_eq!(qualified.artifact_count, 81);
        assert_eq!(qualified.total_bytes, 88_328);
        assert_eq!(qualified.contract_major, 1);
        assert_eq!(qualified.contract_revision, 1);
    }

    #[test]
    fn every_fixture_family_matches_its_independently_computed_disposition() {
        let checks = ContractCorpus::fixture_checks().expect("fixture checks");
        let families = checks
            .iter()
            .map(|check| check.taxonomy)
            .collect::<BTreeSet<_>>();

        assert_eq!(checks.len(), 52);
        assert_eq!(
            families,
            BTreeSet::from([
                "boundary",
                "cancellation",
                "degraded",
                "invalid",
                "privacy",
                "unknown-field",
                "valid",
            ])
        );
        assert!(checks.iter().all(|check| check.expected == check.actual));
    }

    #[test]
    fn authority_unknown_field_is_rejected_even_when_fixture_claims_valid() {
        let mut fixture: Value = serde_json::from_slice(
            ContractCorpus::artifact_bytes("v1/fixtures/valid/authorization-grant.json")
                .expect("fixture bytes"),
        )
        .expect("fixture JSON");
        fixture["document"]["unexpected_authority"] = json!(true);
        fixture["expect"] = json!("valid");
        let bytes = serde_json::to_vec(&fixture).expect("fixture bytes");

        let decoded =
            ContractCorpus::decode_fixture("v1/fixtures/valid/authorization-grant.json", &bytes)
                .expect("closed fixture outcome");

        assert_eq!(decoded.expected, FixtureDisposition::Valid);
        assert_eq!(decoded.actual, FixtureDisposition::SchemaInvalid);
    }

    #[test]
    fn tolerant_metadata_extensions_are_preserved_as_data() {
        let decoded = ContractCorpus::decode_embedded_fixture(
            "v1/fixtures/unknown-field/execution-metadata-extension.json",
        )
        .expect("tolerant fixture");

        assert_eq!(decoded.actual, FixtureDisposition::Valid);
        assert_eq!(
            decoded.document["extensions"],
            json!({"future_counter": 7, "future_flag": true})
        );
    }

    #[test]
    fn synthetic_operator_flow_decodes_as_redacted_local_test_evidence() {
        let report = ContractCorpus::synthetic_operator_report().expect("operator report");

        assert_eq!(report.verifier_build_id, "verifier_alpha");
        assert_eq!(report.contract_revision, 1);
        assert_eq!(
            report.evidence_classification,
            EvidenceClassification::LocalTest
        );
        assert_eq!(report.authorization_result, AuthorizationResult::Authorized);
        assert_eq!(report.epoch_open_count, 2);
        assert_eq!(report.epoch_close_count, 2);
        assert_eq!(report.participant_change_count, 1);
        assert_eq!(report.decoded_receive_count, 2);
        assert_eq!(report.stt_completion_count, 2);
        assert_eq!(report.synthesis_completion_count, 2);
        assert_eq!(report.playback_completion_count, 1);
        assert_eq!(report.barge_in_cancellation_count, 1);
        assert_eq!(report.pause_count, 1);
        assert_eq!(report.resume_count, 1);
        assert_eq!(report.final_leave_count, 1);
        assert_eq!(report.duration_ms, "120000");
        assert_eq!(report.terminal_status, "complete");
        assert!(report.redacted);
        assert!(!report.truncated);
    }

    #[test]
    fn native_verifier_rejects_a_changed_embedded_artifact() {
        let mut artifacts = ContractCorpus::embedded_artifacts().to_vec();
        let readme = artifacts
            .iter_mut()
            .find(|artifact| artifact.path == "README.md")
            .expect("README artifact");
        readme.bytes = b"changed without updating its manifest";

        let error = ContractCorpus::verify_bytes(
            ContractCorpus::embedded_lock(),
            ContractCorpus::embedded_manifest(),
            &artifacts,
        )
        .expect_err("changed artifact must fail");

        assert_eq!(error.code, ContractErrorCode::ArtifactLengthMismatch);
        assert_eq!(error.path, Some("README.md"));
    }

    #[test]
    fn fixture_errors_never_embed_private_values_or_json() {
        let private = "DO-NOT-ECHO-CONTRACT-PRIVATE-VALUE";
        let bytes = serde_json::to_vec(&json!({
            "case_id": "private_probe",
            "schema": "https://abbey.local/contracts/abbey/v1/schemas/authorization/grant.schema.json",
            "expect": "valid",
            "document": {"token": private},
        }))
        .expect("fixture bytes");

        let decoded = ContractCorpus::decode_fixture("synthetic/private.json", &bytes)
            .expect("privacy disposition");
        let rendered = format!("{decoded:?}");

        assert_eq!(decoded.actual, FixtureDisposition::ForbiddenContent);
        assert!(!rendered.contains(private));
        assert!(!rendered.contains("token"));
    }
}
