//! Receipt liveness and explicit deletion reconciliation. Verification grants
//! no native Work access and proves no remote payload/card commitment.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryCandidateState {
    Live,
    Forgotten,
    Unknown,
}
#[derive(Deserialize)]
struct VerifiedCandidate {
    found: String,
    guild_ref: String,
    event_kind: String,
    signature_status: String,
    memory_forgotten: String,
}
fn decode(code: Option<i32>, stdout: &[u8], guild: &str) -> MemoryCandidateState {
    if code != Some(0) {
        return MemoryCandidateState::Unknown;
    }
    let Ok(value) = serde_json::from_slice::<VerifiedCandidate>(stdout) else {
        return MemoryCandidateState::Unknown;
    };
    if value.found != "true"
        || value.guild_ref != guild
        || value.event_kind != "memory_candidate"
        || !matches!(value.signature_status.as_str(), "unsigned" | "valid")
    {
        return MemoryCandidateState::Unknown;
    }
    match value.memory_forgotten.as_str() {
        "false" => MemoryCandidateState::Live,
        "true" => MemoryCandidateState::Forgotten,
        _ => MemoryCandidateState::Unknown,
    }
}
impl EpisodeGate {
    pub(crate) async fn verify_memory(&self, scope: &str, receipt: &str) -> MemoryCandidateState {
        if !self.covers(scope)
            || receipt.len() != 64
            || !receipt
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return MemoryCandidateState::Unknown;
        }
        let Some(guild) = guild_ref_for(scope) else {
            return MemoryCandidateState::Unknown;
        };
        let mut args: Vec<OsString> = vec![
            "wdbx".into(),
            "episode".into(),
            "verify".into(),
            guild.clone().into(),
            receipt.into(),
            "--json".into(),
            "--endpoint".into(),
            self.config.endpoint.clone().into(),
            "--token-file".into(),
            self.config.token_file.as_os_str().to_owned(),
        ];
        if let Some(ca) = &self.config.ca_cert {
            args.push("--ca-cert".into());
            args.push(ca.as_os_str().to_owned());
        }
        let program = self.config.abi_cli.clone();
        let timeout = self.config.timeout_secs;
        let outcome = if let Some(service) = self.service.get() {
            let cancel = service.cancellation();
            let Ok(result) = service
                .spawn_result(crate::service::OperationKind::Episode, async move {
                    process::run_raw_owned(&program, &args, timeout, Some(cancel)).await
                })
            else {
                return MemoryCandidateState::Unknown;
            };
            let Ok(result) = result.await else {
                return MemoryCandidateState::Unknown;
            };
            result
        } else {
            process::run_raw_owned(&program, &args, timeout, None).await
        };
        match outcome {
            Ok(output) => decode(output.code, &output.stdout, &guild),
            Err(_) => MemoryCandidateState::Unknown,
        }
    }
}
#[cfg(test)]
mod tests;
