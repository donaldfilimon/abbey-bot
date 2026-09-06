//! Endpoint locality and configured provider identity.
use super::*;

pub(super) fn backend_locality(backend: &Backend) -> ExecutionLocality {
    match backend {
        Backend::Anthropic { .. } => ExecutionLocality::PublicRemote,
        Backend::OpenAiCompatible { endpoint, .. } => endpoint_locality(endpoint),
    }
}
pub(super) fn endpoint_locality(endpoint: &str) -> ExecutionLocality {
    let evidence = reqwest::Url::parse(endpoint)
        .ok()
        .and_then(|url| {
            url.host_str()
                .and_then(|host| host.trim_matches(['[', ']']).parse().ok())
        })
        .map(ExecutionLocality::address);
    ExecutionLocality::least_local(evidence)
}

pub(super) fn backend_identity(backend: &Backend) -> ProviderIdentityHashes {
    match backend {
        Backend::Anthropic { api_key } => config_identity(api_key.as_bytes()),
        Backend::OpenAiCompatible { endpoint, model } => {
            config_identity(format!("{endpoint}\n{model}").as_bytes())
        }
    }
}
static BINARY_IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub(super) fn config_identity(value: &[u8]) -> ProviderIdentityHashes {
    ProviderIdentityHashes {
        abbey_binary_sha256: BINARY_IDENTITY
            .get_or_init(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|path| std::fs::read(path).ok())
                    .map(|bytes| super::manifest::sha256_bytes(&bytes))
                    .expect("running executable must remain readable for provider identity")
            })
            .clone(),
        provider_binary_sha256: None,
        model_sha256: Some(super::manifest::sha256_bytes(value)),
        os_sha256: None,
        tool_schema_sha256: super::manifest::production_tool_schema_sha256()
            .expect("static tool vocabulary"),
        sandbox_sha256: None,
    }
}
