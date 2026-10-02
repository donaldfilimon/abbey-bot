//! Operator-maintained version-bound acceptance; public health is only reachability.
use crate::work::WorkError;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityReadiness {
    pub https_origin: String,
    pub deployed_digest: String,
    pub iframe_receipt: String,
    pub shared_receipt: String,
    pub verified_at: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    digest: String,
    origin: String,
    pub(crate) record: String,
    verified_at: u64,
    participants: u8,
}
pub fn validate_activity_readiness(r: &ActivityReadiness) -> Result<(), WorkError> {
    let url = reqwest::Url::parse(&r.https_origin).map_err(|_| WorkError::Invalid)?;
    let host = url.host_str().ok_or(WorkError::Invalid)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.starts_with('[')
        || !host.contains('.')
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with('.')
        || r.deployed_digest.len() != 64
        || !r.deployed_digest.bytes().all(|b| b.is_ascii_hexdigit())
        || r.verified_at == 0
    {
        return Err(WorkError::Invalid);
    }
    for (text, minimum) in [(&r.iframe_receipt, 1), (&r.shared_receipt, 2)] {
        let proof: Receipt = serde_json::from_str(text).map_err(|_| WorkError::Invalid)?;
        if proof.digest != r.deployed_digest
            || proof.origin != r.https_origin
            || proof.verified_at != r.verified_at
            || proof.participants < minimum
            || !proof.record.starts_with("docs/")
            || proof
                .record
                .split('/')
                .any(|part| matches!(part, ".." | "." | ""))
            || proof.record.contains('\\')
            || proof.record.len() > 1024
        {
            return Err(WorkError::Denied);
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;

/// Metadata-only human acceptance record; operators supply the actual evidence.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptanceEvidence {
    digest: String,
    origin: String,
    verified_at: u64,
    participants: u8,
    discord_iframe: bool,
    shared_room_case_votes: bool,
    disconnection_recovery: bool,
    no_solo_vote_upload: bool,
}
pub(crate) fn validate_acceptance_evidence(
    record: &ActivityReadiness,
    bytes: &[u8],
    shared: bool,
) -> Result<(), WorkError> {
    let evidence: AcceptanceEvidence =
        serde_json::from_slice(bytes).map_err(|_| WorkError::Invalid)?;
    if evidence.digest != record.deployed_digest
        || evidence.origin != record.https_origin
        || evidence.verified_at != record.verified_at
        || !evidence.discord_iframe
        || evidence.participants < if shared { 2 } else { 1 }
        || shared
            && !(evidence.shared_room_case_votes
                && evidence.disconnection_recovery
                && evidence.no_solo_vote_upload)
    {
        return Err(WorkError::Denied);
    }
    Ok(())
}
