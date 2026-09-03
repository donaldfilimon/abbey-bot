use super::{
    ContractCorpus, ContractError, ContractErrorCode, EmbeddedArtifact, MAX_ARTIFACT_BYTES,
    MAX_CORPUS_BYTES, Manifest, PINNED_ARTIFACT_COUNT, PINNED_DIGEST, PINNED_REPOSITORY,
    PINNED_REVISION, PINNED_TOTAL_BYTES, QualifiedDigest, aggregate_digest, hex_digest,
};
use std::collections::BTreeMap;

pub(crate) const LOCK_BYTES: &[u8] =
    include_bytes!("../../contracts/abbey/abbey-contracts.lock.json");
pub(crate) const MANIFEST_BYTES: &[u8] =
    include_bytes!("../../contracts/abbey/corpus/manifest.json");

macro_rules! artifact {
    ($path:literal) => {
        EmbeddedArtifact {
            path: $path,
            bytes: include_bytes!(concat!("../../contracts/abbey/corpus/", $path)),
        }
    };
}

pub(crate) const ARTIFACTS: &[EmbeddedArtifact] = &[
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

impl ContractCorpus {
    pub(crate) fn embedded_lock() -> &'static [u8] {
        LOCK_BYTES
    }

    pub(crate) fn embedded_manifest() -> &'static [u8] {
        MANIFEST_BYTES
    }

    pub(crate) fn embedded_artifacts() -> &'static [EmbeddedArtifact] {
        ARTIFACTS
    }

    pub(crate) fn artifact_bytes(path: &str) -> Option<&'static [u8]> {
        ARTIFACTS
            .iter()
            .find(|artifact| artifact.path == path)
            .map(|artifact| artifact.bytes)
    }

    pub(crate) fn verify_bytes(
        lock_bytes: &[u8],
        manifest_bytes: &[u8],
        artifacts: &[EmbeddedArtifact],
    ) -> Result<QualifiedDigest, ContractError> {
        let lock: super::ContractLock = serde_json::from_slice(lock_bytes)
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
}
