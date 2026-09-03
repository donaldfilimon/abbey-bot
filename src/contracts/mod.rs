//! Data-only qualification for the pinned Abbey Program 1 corpus.

use serde::{Deserialize as DeriveDeserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub(crate) mod aggregate;
pub(crate) mod fixture;
pub(crate) mod registry;
pub(crate) mod schema;
pub(crate) mod verify;

pub(crate) use aggregate::{aggregate_digest, hex_digest};
pub(crate) use fixture::taxonomy;
pub(crate) use schema::SchemaRegistry;
pub(crate) use verify::{ARTIFACTS, LOCK_BYTES, MANIFEST_BYTES};

pub(crate) const PINNED_REPOSITORY: &str = "https://github.com/donaldfilimon/abi";
pub(crate) const PINNED_REVISION: &str = "348754bdaaf59a40fbb858380f925e0aba95a23b";
pub(crate) const PINNED_DIGEST: &str =
    "72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e";
pub(crate) const PINNED_ARTIFACT_COUNT: usize = 81;
pub(crate) const PINNED_TOTAL_BYTES: usize = 88_328;
pub(crate) const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CORPUS_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const AGGREGATE_DOMAIN: &[u8] = b"abbey-contract-corpus-v1\0";

#[derive(Clone, Copy)]
pub(crate) struct EmbeddedArtifact {
    pub(crate) path: &'static str,
    pub(crate) bytes: &'static [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContractErrorCode {
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
pub(crate) struct ContractError {
    pub(crate) code: ContractErrorCode,
    pub(crate) path: Option<&'static str>,
}

impl ContractError {
    pub(crate) const fn new(code: ContractErrorCode, path: Option<&'static str>) -> Self {
        Self { code, path }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QualifiedDigest {
    pub(crate) aggregate_digest: &'static str,
    pub(crate) artifact_count: usize,
    pub(crate) total_bytes: usize,
    pub(crate) contract_major: u64,
    pub(crate) contract_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixtureDisposition {
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
    pub(crate) fn from_code(code: &str) -> Option<Self> {
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
pub(crate) struct FixtureCheck {
    pub(crate) taxonomy: &'static str,
    pub(crate) expected: FixtureDisposition,
    pub(crate) actual: FixtureDisposition,
}

pub(crate) struct DecodedFixture {
    pub(crate) expected: FixtureDisposition,
    pub(crate) actual: FixtureDisposition,
    pub(crate) document: Value,
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
pub(crate) enum EvidenceClassification {
    LocalTest,
    InstalledArtifact,
    LiveDiscord,
}

#[derive(Debug, DeriveDeserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthorizationResult {
    Authorized,
    Denied,
    Lost,
}

#[derive(Debug, DeriveDeserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperatorVerificationReport {
    pub(crate) verifier_build_id: String,
    pub(crate) contract_revision: u64,
    pub(crate) authorization_result: AuthorizationResult,
    pub(crate) epoch_open_count: u64,
    pub(crate) epoch_close_count: u64,
    pub(crate) participant_change_count: u64,
    pub(crate) decoded_receive_count: u64,
    pub(crate) stt_completion_count: u64,
    pub(crate) synthesis_completion_count: u64,
    pub(crate) playback_completion_count: u64,
    pub(crate) barge_in_cancellation_count: u64,
    pub(crate) pause_count: u64,
    pub(crate) resume_count: u64,
    pub(crate) final_leave_count: u64,
    pub(crate) duration_ms: String,
    pub(crate) terminal_status: String,
    pub(crate) evidence_classification: EvidenceClassification,
    pub(crate) redacted: bool,
    pub(crate) truncated: bool,
}

#[derive(DeriveDeserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContractLock {
    pub(crate) source_repository: String,
    pub(crate) source_revision: String,
    pub(crate) contract_major: u64,
    pub(crate) contract_revision: u64,
    pub(crate) aggregate_digest: String,
}

#[derive(DeriveDeserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) contract_major: u64,
    pub(crate) contract_revision: u64,
    pub(crate) algorithm: String,
    pub(crate) redaction_profile: String,
    pub(crate) artifacts: Vec<ManifestArtifact>,
    pub(crate) aggregate_digest: String,
}

#[derive(DeriveDeserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestArtifact {
    pub(crate) path: String,
    pub(crate) bytes: usize,
    pub(crate) media_type: String,
    pub(crate) sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) schema_id: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureWrapper {
    pub(crate) case_id: String,
    pub(crate) schema: String,
    pub(crate) expect: String,
    pub(crate) document: Value,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContractCorpus;

impl ContractCorpus {
    pub(crate) fn verify_embedded() -> Result<QualifiedDigest, ContractError> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ContractCorpus, ContractErrorCode, FixtureDisposition};
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
