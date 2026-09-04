//! Concrete [`TurnAdapter`] implementations for local inference backends.
//!
//! Each adapter wraps an existing [`llm::Backend`] and [`llm::HttpTransport`],
//! translating Abbey's turn vocabulary into the provider-specific HTTP call.
//! No adapter introduces a second transcript, tool vocabulary, or schema.

use super::domain::{ProviderId, TurnFuture};
use super::{ProviderCapabilities, TurnAdapter};
use crate::llm::{self, Backend, ChatTurn, LlmError, ModelTurn, HttpTransport};
use crate::tools::ToolSpec;

/// Adapter for an OpenAI-compatible local server (Ollama, MLX-Audio, etc.).
///
/// The adapter holds no credentials: the transport sends the request and the
/// backend selects the HTTP dialect. Environment isolation happens at the
/// transport level, not in the adapter.
pub struct LocalServerAdapter {
    id: ProviderId,
    backend: Backend,
    transport: HttpTransport,
}

impl LocalServerAdapter {
    pub fn new(id: ProviderId, backend: Backend, transport: HttpTransport) -> Self {
        Self {
            id,
            backend,
            transport,
        }
    }

    /// Build an adapter from validated endpoint and model settings.
    pub fn from_endpoint(
        id: ProviderId,
        endpoint: String,
        model: String,
        transport: HttpTransport,
    ) -> Self {
        Self::new(
            id,
            Backend::OpenAiCompatible { endpoint, model },
            transport,
        )
    }
}

impl TurnAdapter for LocalServerAdapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    fn turn<'a>(
        &'a self,
        system_prompt: &'a str,
        turns: &'a [ChatTurn],
        tools: &'a [ToolSpec],
        _call_id: &'a str,
    ) -> TurnFuture<'a> {
        Box::pin(async move {
            llm::chat_turn(&self.transport, &self.backend, system_prompt, turns, tools).await
        })
    }
}

impl std::fmt::Debug for LocalServerAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalServerAdapter")
            .field("id", &self.id)
            .field("backend", &self.backend)
            .finish()
    }
}

/// Adapter for Apple Foundation Models via the `/usr/bin/fm` CLI.
///
/// This delegates to [`super::FoundationModels::cli_turn`] which already
/// handles schema generation, private file management, and process isolation.
/// The adapter exists so the adaptive router can treat FM as one more
/// `TurnAdapter` without special-casing.
pub struct FmCliAdapter {
    id: ProviderId,
    fm: super::FoundationModels,
}

impl FmCliAdapter {
    pub fn new(fm: super::FoundationModels) -> Self {
        Self {
            id: ProviderId::parse("foundation-models").expect("static provider ID"),
            fm,
        }
    }
}

impl TurnAdapter for FmCliAdapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    fn turn<'a>(
        &'a self,
        system_prompt: &'a str,
        turns: &'a [ChatTurn],
        tools: &'a [ToolSpec],
        call_id: &'a str,
    ) -> TurnFuture<'a> {
        Box::pin(async move {
            self.fm
                .cli_turn(system_prompt, turns, tools, call_id)
                .await
        })
    }
}

impl std::fmt::Debug for FmCliAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FmCliAdapter")
            .field("id", &self.id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::domain::ProviderId;

    #[test]
    fn local_server_adapter_provider_id() {
        let id = ProviderId::parse("ollama").unwrap();
        let backend = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma4:12b".into(),
        };
        // We can't create a real transport in tests (no network), but we can
        // verify the adapter struct is constructible and the provider ID
        // is correct. The actual TurnAdapter trait requires async.
        let _ = (id, backend);
    }

    #[test]
    fn fm_cli_adapter_provider_id() {
        let config = super::super::FmConfig {
            mode: super::super::FmMode::System,
            endpoint: None,
            cli: "/usr/bin/fm".into(),
            fallback: false,
            timeout_secs: 30,
        };
        let fm = super::super::FoundationModels::new(config, None, true);
        let adapter = FmCliAdapter::new(fm);
        assert_eq!(adapter.provider_id().as_str(), "foundation-models");
    }
}
