//! The generation backend behind `/persona ask`.
//!
//! Mirrors abi-connectors' `Transport`-trait design in miniature: request
//! construction and response extraction are pure and pinned by tests through a
//! recording fake; the single live implementation POSTs JSON. The seam is
//! deliberate — **no test in this crate performs network I/O**, so the suite
//! passes with no key or environment configuration, which is the property the
//! whole gate already holds.
//!
//! Which backend answers is selected from the environment (proposal option C):
//! `ANTHROPIC_API_KEY` → the external Anthropic Messages API;
//! else `ABBEY_BOT_LLM_ENDPOINT` → an OpenAI-compatible server, usually
//! loopback (llama-server / ollama / mlx); else no backend, and the command
//! replies honestly that none is configured (`crate::ask::degraded_reply`).

use std::fmt;

use serde_json::Value;
#[cfg(test)]
use serde_json::json;

mod protocol;
mod stream;
mod transport;

pub use protocol::{
    build_chat_request, build_chat_request_with_tools, build_request, extract_text, extract_turn,
};
#[cfg(test)]
pub use stream::SseAccumulator;
pub use stream::{StreamTransport, build_stream_request};
#[cfg(test)]
pub use transport::{DEFAULT_TIMEOUT_SECS, RecordingTransport, timeout_from_value};
pub use transport::{HttpTransport, Transport};

/// The Anthropic Messages API endpoint. The key travels in a header, never in
/// the URL, so no error path can leak it.
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
/// The pinned Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// The model `/persona ask` requests on the Anthropic path.
const ANTHROPIC_MODEL: &str = "claude-sonnet-5";
/// Output budget on the Anthropic path. Replies are clamped to 2,000 Discord
/// codepoints anyway, so a small ceiling wastes neither tokens nor money.
const MAX_TOKENS: u32 = 1024;
/// Output budget on the OpenAI-compatible path. Larger, because local
/// reasoning models (gemma4, qwen3 on ollama) spend tokens in a `reasoning`
/// field *before* the answer, and a budget sized for the answer alone returns
/// an empty `content` — observed live 2026-08-19: "Say hi in three words"
/// cost 739 tokens of reasoning for a four-word reply.
const LOCAL_MAX_TOKENS: u32 = 4096;
/// The model name sent when `ABBEY_BOT_LLM_MODEL` is unset. OpenAI-compatible
/// servers differ on whether they validate, resolve, or ignore this field;
/// Ollama and MLX-VLM require the configured name to resolve to a served model.
pub const DEFAULT_LOCAL_MODEL: &str = "gemma4:12b";
/// Maximum JSON/SSE response retained from a generation backend.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum diagnostic body retained from a failed backend response.
const MAX_ERROR_RESPONSE_BYTES: usize = 4 * 1024;
/// Maximum calls a backend may ask the runtime to dispatch in one turn.
///
/// This same bound is enforced for OpenAI-compatible JSON, Anthropic content
/// blocks, and streamed tool-call indices before a [`ModelTurn`] is returned.
pub const MAX_TOOL_CALLS_PER_TURN: usize = 8;
/// Trusted internal detail used when the generation semaphore times out.
pub const BUSY_ERROR_DETAIL: &str =
    "the model is busy answering someone else; try again in a minute";

pub(crate) fn validate_remote_endpoint(value: &str, name: &str) -> Result<(), String> {
    let url =
        reqwest::Url::parse(value).map_err(|_| format!("{name} must be a valid absolute URL"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} must not contain credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{name} must not contain a query or fragment"));
    }
    if url.host_str().is_none() {
        return Err(format!("{name} must include a host"));
    }
    let loopback = url_is_loopback(&url);
    match url.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        "http" => Err(format!("{name} requires HTTPS unless it targets loopback")),
        _ => Err(format!("{name} must use HTTP or HTTPS")),
    }
}

pub(crate) fn url_is_loopback(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

/// Which generation backend the environment selected.
///
/// `Debug` is hand-written, not derived: this type holds an API key, and a
/// derived `Debug` prints it in full. That matters because `{:?}` reaches
/// output through paths nobody plans — a `tracing` field, a panic message, a
/// failing `assert_eq!` in CI logs. Redacting at the type keeps the secret out
/// of every one of them at once.
#[derive(Clone, PartialEq, Eq)]
pub enum Backend {
    /// `ANTHROPIC_API_KEY`: the external Anthropic Messages API.
    Anthropic { api_key: String },
    /// `ABBEY_BOT_LLM_ENDPOINT`: an OpenAI-compatible server, usually loopback,
    /// with `ABBEY_BOT_LLM_MODEL` naming the model (default
    /// [`DEFAULT_LOCAL_MODEL`]).
    OpenAiCompatible { endpoint: String, model: String },
}

impl Backend {
    /// Backend selection given the two environment values. Pure — takes values
    /// rather than reading the process environment — so tests never depend on
    /// or mutate env state. Precedence per the proposal: Anthropic wins when
    /// both are set.
    ///
    /// A blank value counts as unset: `.env.example` ships blank assignments,
    /// and a copied-but-unfilled file must not select a backend that cannot
    /// possibly work.
    pub fn from_values(
        anthropic_api_key: Option<String>,
        endpoint: Option<String>,
        model: Option<String>,
    ) -> Option<Self> {
        let non_blank = |value: Option<String>| {
            value
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        if let Some(api_key) = non_blank(anthropic_api_key) {
            return Some(Self::Anthropic { api_key });
        }
        non_blank(endpoint).map(|endpoint| Self::OpenAiCompatible {
            endpoint,
            model: non_blank(model).unwrap_or_else(|| DEFAULT_LOCAL_MODEL.to_string()),
        })
    }

    /// Selection from the real environment — the runtime path. Tests go
    /// through [`Backend::from_values`] instead, which is this minus the read.
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            std::env::var("ANTHROPIC_API_KEY").ok(),
            std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
            std::env::var("ABBEY_BOT_LLM_MODEL").ok(),
        )
    }

    /// Reject endpoint shapes that can leak traffic or credentials. Remote
    /// backends require HTTPS; plain HTTP is accepted only on loopback for
    /// local model servers.
    pub fn validate(&self) -> Result<(), LlmError> {
        let Self::OpenAiCompatible { endpoint, .. } = self else {
            return Ok(());
        };
        validate_remote_endpoint(endpoint, "ABBEY_BOT_LLM_ENDPOINT").map_err(LlmError::backend)
    }

    /// What to call this backend in user-facing copy. Names what actually
    /// answers — the honesty rule is that the bot never claims the answer as
    /// its own generation.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Anthropic { .. } => "external Anthropic API",
            Self::OpenAiCompatible { .. } => "configured OpenAI-compatible endpoint",
        }
    }

    /// Whether this backend is an OpenAI-compatible service bound to this
    /// machine. Local voice uses this guard so its own HTTP hop cannot send a
    /// transcript off-host. The operator still controls whether that local
    /// service serves a resident model or proxies an upstream model.
    #[must_use]
    pub fn is_loopback_openai_compatible(&self) -> bool {
        let Self::OpenAiCompatible { endpoint, .. } = self else {
            return false;
        };
        reqwest::Url::parse(endpoint).is_ok_and(|url| url_is_loopback(&url))
    }
}

/// A fully assembled backend request: exactly what the live transport sends,
/// and exactly what the recording fake captures for assertion.
///
/// `Debug` is hand-written for the same reason as [`Backend`]: `headers`
/// carries `x-api-key`, so a derived `Debug` would print the credential.
#[derive(Clone, PartialEq)]
pub struct LlmRequest {
    pub url: String,
    /// Header name/value pairs in send order. `content-type: application/json`
    /// is implied by the JSON body and set by the live transport.
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

/// Header names whose values are credentials and must never be printed.
const SECRET_HEADERS: &[&str] = &["x-api-key", "authorization"];

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The key is replaced, not shortened: a prefix of a secret is still
            // a piece of a secret, and it is enough to identify the account.
            Self::Anthropic { .. } => f.write_str("Anthropic { api_key: <redacted> }"),
            Self::OpenAiCompatible { endpoint, model } => {
                write!(
                    f,
                    "OpenAiCompatible {{ endpoint: {endpoint:?}, model: {model:?} }}"
                )
            }
        }
    }
}

impl std::fmt::Debug for LlmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let shown = if SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                    "<redacted>"
                } else {
                    value.as_str()
                };
                (*name, shown)
            })
            .collect();
        let body_bytes = self.body.to_string().len();
        f.debug_struct("LlmRequest")
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_bytes", &body_bytes)
            .finish()
    }
}

/// Who spoke a turn in a multi-turn transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// A tool result (OpenAI `role: "tool"`; Anthropic folds it into a user
    /// message of `tool_result` blocks — the builder handles the difference).
    Tool,
}

impl Role {
    /// The wire name on the OpenAI side (Anthropic has no `tool` role).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// One turn of conversation history. The engine keeps these per scope and the
/// request builder serializes them in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurn {
    pub role: Role,
    pub text: String,
    /// Calls the assistant made in this turn (empty for plain text).
    pub tool_calls: Vec<crate::tools::ToolCall>,
    /// For `Role::Tool`: which call this result answers.
    pub tool_call_id: Option<String>,
}

impl ChatTurn {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// An assistant turn that asked for tools (possibly with some text).
    pub fn assistant_calls(text: impl Into<String>, calls: Vec<crate::tools::ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            tool_calls: calls,
            tool_call_id: None,
        }
    }

    /// The result of one call, keyed back to it.
    pub fn tool_result(result: &crate::tools::ToolResult) -> Self {
        Self {
            role: Role::Tool,
            text: result.content.clone(),
            tool_calls: Vec::new(),
            tool_call_id: Some(result.call_id.clone()),
        }
    }
}

/// A parsed model turn: text, tool calls, or both.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelTurn {
    pub text: String,
    pub calls: Vec<crate::tools::ToolCall>,
}

/// Why a backend call produced no answer. Carries no secrets by construction:
/// keys travel in headers, and header values never appear in error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmError {
    detail: String,
    kind: LlmErrorKind,
}

/// Stable public category for an [`LlmError`]. Provider-controlled detail is
/// never rendered; only exact internal sentinel shapes receive special copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorKind {
    Busy,
    ResponseBudget,
    Backend,
}

impl LlmError {
    /// A backend, transport, or protocol failure. Provider-controlled text can
    /// reach only this generic public category.
    pub fn backend(detail: String) -> Self {
        Self {
            detail,
            kind: LlmErrorKind::Backend,
        }
    }

    /// The trusted local generation semaphore timed out.
    pub fn busy() -> Self {
        Self {
            detail: BUSY_ERROR_DETAIL.to_string(),
            kind: LlmErrorKind::Busy,
        }
    }

    /// A structurally validated response spent its budget on hidden reasoning.
    pub(crate) fn response_budget(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            kind: LlmErrorKind::ResponseBudget,
        }
    }

    pub const fn kind(&self) -> LlmErrorKind {
        self.kind
    }

    /// Private operational detail for logs and protocol-aware fallback logic.
    /// Never render this string directly to a user.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for LlmError {}

/// The whole ask path over any transport: build the request, post it, extract
/// the text. The command runs this with [`HttpTransport`]; tests run the same
/// path with [`RecordingTransport`].
pub async fn ask_backend<T: Transport>(
    transport: &T,
    backend: &Backend,
    system_prompt: &str,
    question: &str,
) -> Result<String, LlmError> {
    let request = build_request(backend, system_prompt, question);
    let raw = transport.post(&request).await?;
    extract_text(backend, &raw)
}

/// Non-streamed generation with a tool vocabulary: the parsed [`ModelTurn`]
/// (text and/or calls) out. `tools` empty = plain chat.
pub async fn chat_turn<T: Transport>(
    transport: &T,
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[crate::tools::ToolSpec],
) -> Result<ModelTurn, LlmError> {
    let request = build_chat_request_with_tools(backend, system_prompt, turns, tools);
    let raw = transport.post(&request).await?;
    extract_turn(backend, &raw)
}

/// Multi-turn variant of [`ask_backend`]: the engine's prepared transcript in,
/// the assistant's text out. Same seam, same extraction.
pub async fn chat_backend<T: Transport>(
    transport: &T,
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
) -> Result<String, LlmError> {
    let request = build_chat_request(backend, system_prompt, turns);
    let raw = transport.post(&request).await?;
    extract_text(backend, &raw)
}

#[cfg(test)]
mod tests;
