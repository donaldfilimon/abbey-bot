//! Read-only operator configuration and fresh, redirect-free public version check.
use crate::engagement::readiness::{ActivityReadiness, validate_activity_readiness};
use std::time::Duration;
pub(crate) async fn current_activity(now: u64) -> Result<ActivityReadiness, &'static str> {
    let path = std::env::var_os("ABBEY_ACTIVITY_READINESS_FILE")
        .ok_or("Activity invitations are disabled: no operator acceptance record is configured.")?;
    if !std::path::Path::new(&path).is_absolute() {
        return Err(
            "Activity invitations are disabled: operator configuration requires an absolute file path.",
        );
    }
    let length = tokio::fs::metadata(&path)
        .await
        .map_err(
            |_| "Activity invitations are disabled: the operator acceptance record is unavailable.",
        )?
        .len();
    if length > 16_384 {
        return Err("Activity invitations are disabled: the acceptance record is too large.");
    }
    let bytes = tokio::fs::read(&path).await.map_err(
        |_| "Activity invitations are disabled: the operator acceptance record is unavailable.",
    )?;
    if bytes.len() > 16_384 {
        return Err("Activity invitations are disabled: the acceptance record is too large.");
    }
    let record: ActivityReadiness = serde_json::from_slice(&bytes)
        .map_err(|_| "Activity invitations are disabled: the acceptance record is malformed.")?;
    validate_activity_readiness(&record).map_err(|_| "Activity invitations are disabled: a public HTTPS origin, deployed digest and matching iframe/two-participant acceptance records are required.")?;
    acceptance_files(&record, std::path::Path::new(&path)).await?;
    if !acceptance_current(&record, now) {
        return Err(
            "Activity invitations are disabled: operator acceptance is expired or dated in the future.",
        );
    }
    reachable(&record).await?;
    Ok(record)
}
async fn acceptance_files(
    record: &ActivityReadiness,
    config_path: &std::path::Path,
) -> Result<(), &'static str> {
    let error = "Activity invitations are disabled: actual version-matching operator iframe and shared acceptance records are missing or incomplete.";
    let base = config_path.parent().ok_or(error)?;
    for (text, shared) in [
        (&record.iframe_receipt, false),
        (&record.shared_receipt, true),
    ] {
        let receipt: crate::engagement::readiness::Receipt =
            serde_json::from_str(text).map_err(|_| error)?;
        let file = base.join(receipt.record);
        let metadata = tokio::fs::metadata(&file).await.map_err(|_| error)?;
        if !metadata.is_file() || metadata.len() > 16_384 {
            return Err(error);
        }
        let bytes = tokio::fs::read(&file).await.map_err(|_| error)?;
        if bytes.len() > 16_384 {
            return Err(error);
        }
        crate::engagement::readiness::validate_acceptance_evidence(record, &bytes, shared)
            .map_err(|_| error)?;
    }
    Ok(())
}
fn acceptance_current(record: &ActivityReadiness, now: u64) -> bool {
    record.verified_at != 0
        && record.verified_at <= now
        && now.saturating_sub(record.verified_at) <= 30 * 86400
}
pub(crate) async fn candidate_activity(
    state: &super::AppState,
    candidate: &crate::engagement::Candidate,
    now: u64,
) -> Result<ActivityReadiness, crate::work::WorkError> {
    let version = super::AppState::lock(&state.stores)
        .work
        .engagement
        .invitation_requests
        .get(&candidate.id)
        .and_then(|r| r.activity.clone())
        .ok_or(crate::work::WorkError::Denied)?;
    let record = current_activity(now)
        .await
        .map_err(|_| crate::work::WorkError::Denied)?;
    if version.origin != record.https_origin || version.digest != record.deployed_digest {
        return Err(crate::work::WorkError::Denied);
    }
    Ok(record)
}
async fn reachable(record: &ActivityReadiness) -> Result<(), &'static str> {
    let error = "Activity invitations are disabled: the public host is unavailable or its deployed digest differs from accepted evidence.";
    let url = reqwest::Url::parse(&record.https_origin).map_err(|_| error)?;
    let host = url.host_str().ok_or(error)?;
    let addresses: Vec<_> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, url.port_or_known_default().ok_or(error)?)),
    )
    .await
    .map_err(|_| error)?
    .map_err(|_| error)?
    .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_address(a.ip())) {
        return Err(error);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| error)?;
    let response = client.get(url).send().await.map_err(|_| error)?;
    if !response.status().is_success()
        || response
            .headers()
            .get("x-abbey-deployed-digest")
            .and_then(|v| v.to_str().ok())
            != Some(record.deployed_digest.as_str())
    {
        return Err(error);
    }
    Ok(())
}
fn public_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && ip.octets()[0] != 0
                && ip.octets()[0] < 240
                && !(ip.octets()[0] == 192 && ip.octets()[1] == 0 && ip.octets()[2] == 0)
                && !(ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                && !(ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1]))
        }
        std::net::IpAddr::V6(_) => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_readiness_network_rejects_private_loopback_and_special_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.1.1",
            "198.18.0.1",
            "::1",
            "0.0.0.0",
            "192.0.2.1",
        ] {
            assert!(!public_address(address.parse().unwrap()));
        }
        assert!(public_address("1.1.1.1".parse().unwrap()));
    }
    #[tokio::test]
    async fn activity_readiness_missing_actual_acceptance_file_is_disabled() {
        let record=ActivityReadiness {https_origin:"https://court.example.org".into(),deployed_digest:"a".repeat(64),iframe_receipt:serde_json::json!({"digest":"a".repeat(64),"origin":"https://court.example.org","record":"docs/this-acceptance-does-not-exist.json","verified_at":1,"participants":1}).to_string(),shared_receipt:String::new(),verified_at:1};
        assert!(
            acceptance_files(
                &record,
                &std::env::temp_dir().join("task-6-operator-config.json")
            )
            .await
            .is_err()
        );
    }
    #[test]
    fn activity_readiness_expired_future_and_zero_acceptance_are_disabled() {
        let mut r = ActivityReadiness {
            https_origin: String::new(),
            deployed_digest: String::new(),
            iframe_receipt: String::new(),
            shared_receipt: String::new(),
            verified_at: 1,
        };
        assert!(acceptance_current(&r, 1));
        assert!(!acceptance_current(&r, 0));
        assert!(!acceptance_current(&r, 30 * 86400 + 2));
        r.verified_at = 0;
        assert!(!acceptance_current(&r, 1));
    }
    #[tokio::test]
    async fn activity_readiness_unavailable_host_cannot_be_ready() {
        let r = ActivityReadiness {
            https_origin: "https://invalid.invalid".into(),
            deployed_digest: "a".repeat(64),
            iframe_receipt: String::new(),
            shared_receipt: String::new(),
            verified_at: 1,
        };
        assert!(reachable(&r).await.is_err());
    }
}
