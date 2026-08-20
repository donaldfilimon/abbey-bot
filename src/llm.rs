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
        f.debug_struct("LlmRequest")
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body", &self.body)
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
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_api_key() {
        // A derived Debug on either type prints the credential in full. These
        // assertions fail the moment someone re-derives it — which is the only
        // way this protection stays real, since the leak paths (tracing fields,
        // panic messages, CI assertion output) are all invisible until they fire.
        const SECRET: &str = "sk-ant-super-secret-value";

        let backend = Backend::Anthropic {
            api_key: SECRET.to_string(),
        };
        let shown = format!("{backend:?}");
        assert!(!shown.contains(SECRET), "backend leaked the key: {shown}");
        assert!(shown.contains("<redacted>"), "{shown}");

        let request = build_request(&backend, "system", "question");
        let shown = format!("{request:?}");
        assert!(!shown.contains(SECRET), "request leaked the key: {shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        // Non-secret fields must still be legible, or the redaction has made
        // the type useless for the debugging it exists to serve.
        assert!(shown.contains("api.anthropic.com"), "{shown}");
    }

    #[test]
    fn the_local_model_name_comes_from_the_env_and_defaults_honestly() {
        let named = Backend::from_values(
            None,
            Some("http://127.0.0.1:11434".into()),
            Some(" gemma4:26b ".into()),
        )
        .expect("selected");
        assert_eq!(
            named,
            Backend::OpenAiCompatible {
                endpoint: "http://127.0.0.1:11434".into(),
                model: "gemma4:26b".into()
            }
        );
        let unnamed = Backend::from_values(
            None,
            Some("http://127.0.0.1:8080".into()),
            Some("  ".into()),
        )
        .expect("selected");
        assert!(
            matches!(unnamed, Backend::OpenAiCompatible { model, .. } if model == DEFAULT_LOCAL_MODEL)
        );
        let request = build_request(&named, "S", "Q");
        assert_eq!(request.body["model"], "gemma4:26b");
        assert_eq!(request.body["max_tokens"], LOCAL_MAX_TOKENS);
    }

    #[test]
    fn stream_request_sets_stream_only_on_the_local_path() {
        let local = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma4:e4b".into(),
        };
        assert_eq!(
            build_stream_request(&local, "S", &[ChatTurn::user("Q")], &[]).body["stream"],
            true
        );
        let anthropic = Backend::Anthropic {
            api_key: "k".into(),
        };
        assert!(
            build_stream_request(&anthropic, "S", &[ChatTurn::user("Q")], &[])
                .body
                .get("stream")
                .is_none()
        );
    }

    #[test]
    fn tooled_requests_serialize_both_shapes_and_untooled_is_byte_identical() {
        let tools = crate::tools::abbey_tools();
        let call = crate::tools::ToolCall {
            id: "c1".into(),
            name: "recall".into(),
            arguments: json!({"query": "rust"}),
        };
        let result = crate::tools::ToolResult {
            call_id: "c1".into(),
            name: "recall".into(),
            content: "• nightly".into(),
        };
        let turns = vec![
            ChatTurn::user("what do you remember?"),
            ChatTurn::assistant_calls("", vec![call]),
            ChatTurn::tool_result(&result),
        ];
        let local = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gpt-oss:20b".into(),
        };
        let body = build_chat_request_with_tools(&local, "S", &turns, &tools).body;
        assert_eq!(body["tools"].as_array().unwrap().len(), 5);
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["name"],
            "recall"
        );
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{\"query\":\"rust\"}"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
        let anthropic = Backend::Anthropic {
            api_key: "k".into(),
        };
        let body = build_chat_request_with_tools(&anthropic, "S", &turns, &tools).body;
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["role"], "user");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "c1");
        // No tools → the classic body, byte for byte.
        let plain = [ChatTurn::user("Q")];
        assert_eq!(
            build_chat_request(&local, "S", &plain),
            build_chat_request_with_tools(&local, "S", &plain, &[])
        );
        assert!(
            build_chat_request(&local, "S", &plain)
                .body
                .get("tools")
                .is_none()
        );
    }

    #[test]
    fn extract_turn_reads_calls_and_text_on_both_backends() {
        let local = Backend::OpenAiCompatible {
            endpoint: "e".into(),
            model: "m".into(),
        };
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"remember_fact","arguments":"{\"fact\":\"x\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let turn = extract_turn(&local, raw).unwrap();
        assert_eq!(turn.calls.len(), 1);
        assert_eq!(turn.calls[0].arguments["fact"], "x");
        let anthropic = Backend::Anthropic {
            api_key: "k".into(),
        };
        let raw = r#"{"content":[{"type":"text","text":"Sure."},{"type":"tool_use","id":"t1","name":"recall","input":{"query":"q"}}],"stop_reason":"tool_use"}"#;
        let turn = extract_turn(&anthropic, raw).unwrap();
        assert_eq!(turn.text, "Sure.");
        assert_eq!(turn.calls[0].name, "recall");
        // Empty both ways falls back to extract_text's honest error.
        assert!(
            extract_turn(
                &local,
                r#"{"choices":[{"message":{"content":""},"finish_reason":"stop"}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn sse_accumulator_merges_streamed_tool_calls_whole_and_fragmented() {
        let mut acc = SseAccumulator::default();
        acc.feed(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_a\",\"type\":\"function\",\"index\":0,\"function\":{\"name\":\"recall\",\"arguments\":\"{\\\"query\\\":\\\"ru\"}}]},\"finish_reason\":null}]}\n").unwrap();
        acc.feed(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"st\\\"}\"}}]},\"finish_reason\":null}]}\n").unwrap();
        acc.feed(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\ndata: [DONE]\n").unwrap();
        acc.finish().unwrap();
        let calls = acc.tool_calls().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].arguments["query"], "rust");
    }

    #[test]
    fn sse_accumulator_handles_chunks_split_mid_line_and_done() {
        let mut acc = SseAccumulator::default();
        let a = acc.feed(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"del",
        ).unwrap();
        assert_eq!(a, vec!["Hel".to_string()]);
        let b = acc.feed(
            b"ta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n",
        ).unwrap();
        assert_eq!(b, vec!["lo".to_string()]);
        assert!(!acc.is_done());
        let c = acc.feed(b"data: [DONE]\n").unwrap();
        assert!(c.is_empty());
        assert!(acc.is_done());
        acc.finish().unwrap();
    }

    #[test]
    fn completed_responses_bound_calls_and_reject_truncation() {
        let local = Backend::OpenAiCompatible {
            endpoint: "e".into(),
            model: "m".into(),
        };
        let calls: Vec<Value> = (0..=MAX_TOOL_CALLS_PER_TURN)
            .map(|index| {
                json!({
                    "id": format!("c{index}"),
                    "type": "function",
                    "function": {"name": "recall", "arguments": "{}"},
                })
            })
            .collect();
        let raw = json!({
            "choices": [{
                "message": {"content": "", "tool_calls": calls},
                "finish_reason": "tool_calls",
            }]
        })
        .to_string();
        assert!(extract_turn(&local, &raw).is_err());
        assert!(
            extract_turn(
                &local,
                r#"{"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}"#
            )
            .is_err()
        );

        let anthropic = Backend::Anthropic {
            api_key: "k".into(),
        };
        assert!(
            extract_turn(
                &anthropic,
                r#"{"content":[{"type":"text","text":"partial"}],"stop_reason":"max_tokens"}"#
            )
            .is_err()
        );
        assert!(
            extract_turn(&anthropic, r#"{"content":[{"type":"text","text":"done"}]}"#).is_err()
        );
        let content: Vec<Value> = (0..=MAX_TOOL_CALLS_PER_TURN)
            .map(|index| {
                json!({
                    "type": "tool_use",
                    "id": format!("t{index}"),
                    "name": "recall",
                    "input": {"query": "q"},
                })
            })
            .collect();
        let raw = json!({"content": content, "stop_reason": "tool_use"}).to_string();
        assert!(extract_turn(&anthropic, &raw).is_err());
    }

    #[test]
    fn timeout_override_parses_and_falls_back() {
        assert_eq!(timeout_from_value(None), DEFAULT_TIMEOUT_SECS);
        assert_eq!(timeout_from_value(Some(" 45 ".into())), 45);
        assert_eq!(timeout_from_value(Some("0".into())), DEFAULT_TIMEOUT_SECS);
        assert_eq!(
            timeout_from_value(Some("soon".into())),
            DEFAULT_TIMEOUT_SECS
        );
    }

    #[test]
    fn a_reasoning_only_response_is_named_as_such() {
        let backend = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma4:26b".into(),
        };
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning":"thinking hard"},"finish_reason":"stop"}]}"#;
        let err = extract_text(&backend, raw).expect_err("no answer");
        assert!(err.detail().contains("reasoning"), "{err}");
        assert_eq!(err.kind(), LlmErrorKind::ResponseBudget);
        let plain =
            r#"{"choices":[{"message":{"role":"assistant","content":""},"finish_reason":"stop"}]}"#;
        assert_eq!(
            extract_text(&backend, plain)
                .expect_err("no answer")
                .detail(),
            "the response carried no answer text"
        );
    }

    #[test]
    fn a_key_bearing_backend_is_not_confused_with_a_loopback_one() {
        let local = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8080".to_string(),
            model: "gemma4:26b".to_string(),
        };
        let shown = format!("{local:?}");
        // The loopback endpoint is not a secret and stays visible.
        assert!(shown.contains("127.0.0.1:8080"), "{shown}");
        assert!(!shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn no_env_values_selects_no_backend_so_the_suite_needs_no_network() {
        // The gate rule, in code: the suite runs with no env vars and no
        // network. With neither variable set, selection yields no backend, so
        // `/persona ask` resolves to the degradation reply without any
        // transport existing at all — there is no code path from "no env" to
        // the network.
        assert_eq!(Backend::from_values(None, None, None), None);
    }

    #[test]
    fn blank_env_values_count_as_unset() {
        // `.env.example` ships blank assignments; copying it unfilled must not
        // select a backend that cannot work.
        assert_eq!(
            Backend::from_values(Some("  ".into()), Some(String::new()), None),
            None
        );
    }

    #[test]
    fn endpoint_policy_allows_https_and_loopback_http_only() {
        let backend = |endpoint: &str| Backend::OpenAiCompatible {
            endpoint: endpoint.into(),
            model: "m".into(),
        };
        assert!(backend("https://models.example.com").validate().is_ok());
        assert!(backend("http://127.0.0.1:11434").validate().is_ok());
        assert!(backend("http://[::1]:11434").validate().is_ok());
        assert!(backend("http://models.example.com").validate().is_err());
        assert!(
            backend("https://user:secret@models.example.com")
                .validate()
                .is_err()
        );
        assert!(
            backend("https://models.example.com?token=secret")
                .validate()
                .is_err()
        );
        assert!(backend("file:///tmp/model").validate().is_err());
    }

    #[test]
    fn live_transport_routes_every_loopback_shape_to_the_no_proxy_client() {
        let transport = HttpTransport::default();
        for endpoint in [
            "http://127.0.0.1:11434/v1/chat/completions",
            "http://localhost:8080/v1/chat/completions",
            "http://[::1]:8181/v1/chat/completions",
        ] {
            assert!(std::ptr::eq(
                transport.client_for(endpoint),
                &transport.loopback_client
            ));
        }
        assert!(std::ptr::eq(
            transport.client_for("https://api.anthropic.com/v1/messages"),
            &transport.remote_client
        ));
    }

    #[test]
    fn anthropic_wins_when_both_backends_are_configured() {
        let backend = Backend::from_values(
            Some("key".into()),
            Some("http://127.0.0.1:8080".into()),
            None,
        )
        .expect("a backend is selected");
        assert!(matches!(backend, Backend::Anthropic { .. }));
    }

    #[tokio::test]
    async fn recording_transport_pins_the_exact_anthropic_request_shape() {
        // Never a live request in tests: the recording fake captures precisely
        // what a call would send, and the wire contract is pinned as literals —
        // URL, the x-api-key + anthropic-version header pair, model, body.
        let backend = Backend::Anthropic {
            api_key: "test-key-not-real".into(),
        };
        let transport = RecordingTransport::returning(
            r#"{"content":[{"type":"text","text":"the answer"}],"stop_reason":"end_turn"}"#,
        );

        let answer = ask_backend(&transport, &backend, "SYSTEM PROMPT", "the question")
            .await
            .expect("the canned response parses");
        assert_eq!(answer, "the answer");

        let request = transport.recorded();
        assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            request.headers,
            vec![
                ("x-api-key", "test-key-not-real".to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ]
        );
        assert_eq!(
            request.body,
            json!({
                "model": "claude-sonnet-5",
                "max_tokens": 1024,
                "thinking": {"type": "disabled"},
                "system": "SYSTEM PROMPT",
                "messages": [{"role": "user", "content": "the question"}],
            })
        );
    }

    #[tokio::test]
    async fn recording_transport_pins_the_openai_compatible_request_shape() {
        let backend = Backend::OpenAiCompatible {
            // Trailing slash on purpose: the join must not produce `//v1`.
            endpoint: "http://127.0.0.1:8080/".into(),
            model: "default".into(),
        };
        let transport = RecordingTransport::returning(
            r#"{"choices":[{"message":{"content":"local answer"},"finish_reason":"stop"}]}"#,
        );

        let answer = ask_backend(&transport, &backend, "SYSTEM PROMPT", "the question")
            .await
            .expect("the canned response parses");
        assert_eq!(answer, "local answer");

        let request = transport.recorded();
        assert_eq!(request.url, "http://127.0.0.1:8080/v1/chat/completions");
        assert!(request.headers.is_empty(), "loopback sends no auth headers");
        assert_eq!(
            request.body,
            json!({
                "model": "default",
                "max_tokens": 4096,
                "messages": [
                    {"role": "system", "content": "SYSTEM PROMPT"},
                    {"role": "user", "content": "the question"},
                ],
            })
        );
    }

    #[test]
    fn extraction_refuses_a_response_with_no_text() {
        // An empty content array is what a refusal looks like; it must surface
        // as an error, never as an empty string presented as an answer.
        let backend = Backend::Anthropic {
            api_key: "k".into(),
        };
        assert!(extract_text(&backend, r#"{"content":[]}"#).is_err());
        assert!(extract_text(&backend, "not json at all").is_err());
    }

    #[test]
    fn extraction_reads_the_first_text_block_only() {
        let backend = Backend::Anthropic {
            api_key: "k".into(),
        };
        let raw = r#"{"content":[{"type":"thinking","thinking":"…"},{"type":"text","text":"visible"}],"stop_reason":"end_turn"}"#;
        assert_eq!(extract_text(&backend, raw).expect("parses"), "visible");
    }

    #[test]
    fn chat_request_keeps_system_top_level_and_alternates_on_anthropic() {
        let backend = Backend::Anthropic {
            api_key: "k".into(),
        };
        let turns = [
            ChatTurn::user("q1"),
            ChatTurn::assistant("a1"),
            ChatTurn::user("q2"),
        ];
        let request = build_chat_request(&backend, "SYS", &turns);
        assert_eq!(request.body["system"], "SYS");
        assert_eq!(
            request.body["messages"],
            json!([
                {"role": "user", "content": "q1"},
                {"role": "assistant", "content": "a1"},
                {"role": "user", "content": "q2"},
            ])
        );
    }

    #[test]
    fn chat_request_puts_system_first_on_openai_compatible() {
        let backend = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8080".into(),
            model: "default".into(),
        };
        let turns = [
            ChatTurn::user("q1"),
            ChatTurn::assistant("a1"),
            ChatTurn::user("q2"),
        ];
        let request = build_chat_request(&backend, "SYS", &turns);
        assert!(request.body.get("system").is_none(), "no top-level system");
        assert_eq!(
            request.body["messages"],
            json!([
                {"role": "system", "content": "SYS"},
                {"role": "user", "content": "q1"},
                {"role": "assistant", "content": "a1"},
                {"role": "user", "content": "q2"},
            ])
        );
    }

    #[test]
    fn single_question_request_is_the_one_turn_chat_request() {
        for backend in [
            Backend::Anthropic {
                api_key: "k".into(),
            },
            Backend::OpenAiCompatible {
                endpoint: "http://127.0.0.1:1".into(),
                model: "default".into(),
            },
        ] {
            assert_eq!(
                build_request(&backend, "S", "Q"),
                build_chat_request(&backend, "S", &[ChatTurn::user("Q")])
            );
        }
    }
}
