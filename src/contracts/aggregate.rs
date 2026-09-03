use super::{AGGREGATE_DOMAIN, ContractError, ContractErrorCode, Manifest, ManifestArtifact};
use sha2::{Digest as _, Sha256};

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    lower_hex(&digest)
}

pub(crate) fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub(crate) fn aggregate_digest(manifest: &Manifest) -> Result<String, ContractError> {
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
