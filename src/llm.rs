//! The generation backend behind `/persona ask`.
//!
//! Mirrors abi-connectors' `Transport`-trait design in miniature: request
//! construction and response extraction are pure and pinned by tests through a
//! recording fake; the single live implementation POSTs JSON. The seam is
//! deliberate — **no test in this crate constructs the live transport**, so the
//! suite passes with no network, no key, and no env vars, which is the property
//! the whole gate already holds.
//!
//! Which backend answers is selected from the environment (proposal option C):
//! `ANTHROPIC_API_KEY` → the external Anthropic Messages API;
//! else `ABBEY_BOT_LLM_ENDPOINT` → an OpenAI-compatible server, usually
//! loopback (llama-server / ollama / mlx); else no backend, and the command
//! replies honestly that none is configured (`crate::ask::degraded_reply`).

use std::fmt;
use std::future::Future;

use serde_json::{Value, json};

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
/// The model name sent when `ABBEY_BOT_LLM_MODEL` is unset. llama-server and
/// mlx serve whatever they were started with and ignore the field; ollama
/// resolves it and rejects an unknown name, which is why it is configurable.
pub const DEFAULT_LOCAL_MODEL: &str = "default";

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

    /// What to call this backend in user-facing copy. Names what actually
    /// answers — the honesty rule is that the bot never claims the answer as
    /// its own generation.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Anthropic { .. } => "external Anthropic API",
            Self::OpenAiCompatible { .. } => "configured OpenAI-compatible endpoint",
        }
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

/// Build the request a backend call would send for a single question. Pure,
/// so the exact wire shape is pinned by tests without any network. This is
/// [`build_chat_request`] with a one-turn transcript.
pub fn build_request(backend: &Backend, system_prompt: &str, question: &str) -> LlmRequest {
    build_chat_request(backend, system_prompt, &[ChatTurn::user(question)])
}

/// Build a multi-turn request. Anthropic takes the system prompt top-level
/// and `messages` alternating user/assistant; OpenAI-compatible servers take
/// the system prompt as the first message. Turns are serialized in the order
/// given — the engine guarantees alternation, this function does not reorder.
pub fn build_chat_request(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
) -> LlmRequest {
    build_chat_request_with_tools(backend, system_prompt, turns, &[])
}

/// [`build_chat_request`] plus a tool vocabulary. With `tools` empty the body
/// is byte-identical to the untooled request (pinned by test), so callers
/// that never offer tools pay nothing. Assistant turns that carried calls and
/// `Role::Tool` results are serialized in each backend's own shape.
pub fn build_chat_request_with_tools(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[crate::tools::ToolSpec],
) -> LlmRequest {
    match backend {
        Backend::Anthropic { api_key } => {
            let mut messages: Vec<Value> = Vec::with_capacity(turns.len());
            for turn in turns {
                match turn.role {
                    Role::User => messages.push(json!({"role": "user", "content": turn.text})),
                    Role::Assistant if turn.tool_calls.is_empty() => {
                        messages.push(json!({"role": "assistant", "content": turn.text}));
                    }
                    Role::Assistant => {
                        let mut blocks = Vec::new();
                        if !turn.text.is_empty() {
                            blocks.push(json!({"type": "text", "text": turn.text}));
                        }
                        for c in &turn.tool_calls {
                            blocks.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.arguments}));
                        }
                        messages.push(json!({"role": "assistant", "content": blocks}));
                    }
                    Role::Tool => {
                        let block = json!({
                            "type": "tool_result",
                            "tool_use_id": turn.tool_call_id.clone().unwrap_or_default(),
                            "content": turn.text,
                        });
                        // Consecutive tool results share one user message.
                        if let Some(last) = messages.last_mut()
                            && last["role"] == "user"
                            && last["content"].is_array()
                        {
                            last["content"].as_array_mut().expect("array").push(block);
                        } else {
                            messages.push(json!({"role": "user", "content": [block]}));
                        }
                    }
                }
            }
            let mut body = json!({
                "model": ANTHROPIC_MODEL,
                "max_tokens": MAX_TOKENS,
                "system": system_prompt,
                "messages": messages,
            });
            if !tools.is_empty() {
                body["tools"] = crate::tools::anthropic_tools_json(tools);
            }
            LlmRequest {
                url: ANTHROPIC_URL.to_string(),
                // The ecosystem's pinned header pair for this API.
                headers: vec![
                    ("x-api-key", api_key.clone()),
                    ("anthropic-version", ANTHROPIC_VERSION.to_string()),
                ],
                body,
            }
        }
        Backend::OpenAiCompatible { endpoint, model } => {
            let mut all = Vec::with_capacity(turns.len() + 1);
            all.push(json!({"role": "system", "content": system_prompt}));
            for turn in turns {
                let mut m = json!({"role": turn.role.as_str(), "content": turn.text});
                if !turn.tool_calls.is_empty() {
                    m["tool_calls"] = Value::Array(
                        turn.tool_calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": { "name": c.name, "arguments": c.arguments.to_string() }
                                })
                            })
                            .collect(),
                    );
                }
                if let Some(id) = &turn.tool_call_id {
                    m["tool_call_id"] = json!(id);
                }
                all.push(m);
            }
            let mut body = json!({
                "model": model,
                "max_tokens": LOCAL_MAX_TOKENS,
                "messages": all,
            });
            if !tools.is_empty() {
                body["tools"] = crate::tools::openai_tools_json(tools);
            }
            LlmRequest {
                url: format!("{}/v1/chat/completions", endpoint.trim_end_matches('/')),
                headers: Vec::new(),
                body,
            }
        }
    }
}

/// Parse a non-streamed response into text and/or tool calls. Unlike
/// [`extract_text`], an empty text with calls present is a normal outcome.
pub fn extract_turn(backend: &Backend, raw: &str) -> Result<ModelTurn, LlmError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| LlmError(format!("the response was not JSON: {e}")))?;
    let turn = match backend {
        Backend::Anthropic { .. } => {
            let content = value.get("content").cloned().unwrap_or(Value::Null);
            let text = content
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            ModelTurn {
                text,
                calls: crate::tools::parse_anthropic_tool_use(&content),
            }
        }
        Backend::OpenAiCompatible { .. } => {
            let message = value
                .pointer("/choices/0/message")
                .cloned()
                .unwrap_or(Value::Null);
            ModelTurn {
                text: message
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                calls: crate::tools::parse_openai_tool_calls(&message),
            }
        }
    };
    if turn.text.trim().is_empty() && turn.calls.is_empty() {
        // Same honesty as extract_text, same reasoning-budget diagnosis.
        return extract_text(backend, raw).map(|text| ModelTurn {
            text,
            calls: Vec::new(),
        });
    }
    Ok(turn)
}

/// Why a backend call produced no answer. Carries no secrets by construction:
/// keys travel in headers, and header values never appear in error text.
#[derive(Debug)]
pub struct LlmError(pub String);

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The seam between request construction and the network. Tests implement this
/// with [`RecordingTransport`]; the binary uses [`HttpTransport`]. Returns the
/// raw response body — text extraction stays pure in [`extract_text`].
pub trait Transport {
    fn post(&self, request: &LlmRequest) -> impl Future<Output = Result<String, LlmError>> + Send;
}

/// Pull the assistant's text out of a raw response body, per backend shape:
/// Anthropic returns `content[]` blocks; OpenAI-compatible servers return
/// `choices[0].message.content`.
pub fn extract_text(backend: &Backend, raw: &str) -> Result<String, LlmError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| LlmError(format!("the response was not JSON: {e}")))?;
    let text = match backend {
        // Find the first text block, then read its text. The bool-to-Option
        // dance this replaced (`.then(...).flatten()` inside a `find_map`
        // inside an `and_then`) expressed the same two steps in a shape that
        // had to be decoded rather than read.
        Backend::Anthropic { .. } => value
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(|block| block.get("text").and_then(Value::as_str)),
        Backend::OpenAiCompatible { .. } => value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str),
    };
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t.to_string()),
        // An empty content array is a real outcome (e.g. a refusal); saying so
        // beats presenting an empty string as an answer. One cause is worth
        // naming: a reasoning model that spent the whole budget in
        // `reasoning` (ollama's field) and never reached the answer.
        _ => {
            let reasoned = value
                .pointer("/choices/0/message/reasoning")
                .and_then(Value::as_str)
                .map_or(0, |r| r.chars().count());
            if reasoned > 0 {
                return Err(LlmError(format!(
                    "the model spent its whole budget reasoning ({reasoned} chars) and produced no answer — try a smaller model or a larger budget"
                )));
            }
            Err(LlmError("the response carried no answer text".to_string()))
        }
    }
}

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

/// The live transport — the only I/O in this module. No test constructs it.
pub struct HttpTransport {
    client: reqwest::Client,
}

/// Default request timeout. Discord's followup window is 15 minutes; a local
/// reasoning model under concurrent load was observed live (2026-08-19) to
/// need more than the old 120 s on a "research …" DM, so the default is 300 s
/// and `ABBEY_BOT_LLM_TIMEOUT_SECS` overrides it.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Parse a timeout override; blank/garbage/zero fall back to the default.
pub fn timeout_from_value(value: Option<String>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

impl Default for HttpTransport {
    fn default() -> Self {
        let secs = timeout_from_value(std::env::var("ABBEY_BOT_LLM_TIMEOUT_SECS").ok());
        Self {
            // Bounded so a hung backend cannot hold the deferred interaction
            // forever — but generously, see `DEFAULT_TIMEOUT_SECS`.
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(secs))
                .build()
                .expect("static reqwest client configuration is valid"),
        }
    }
}

impl Transport for HttpTransport {
    fn post(&self, request: &LlmRequest) -> impl Future<Output = Result<String, LlmError>> + Send {
        let mut builder = self.client.post(&request.url).json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(*name, value);
        }
        async move {
            let response = builder
                .send()
                .await
                .map_err(|e| LlmError(format!("the request failed: {e}")))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|e| LlmError(format!("reading the response failed: {e}")))?;
            if !status.is_success() {
                // Enough body to diagnose a loopback misconfiguration, not
                // enough to flood the channel; the reply is clamped regardless.
                let mut brief: String = body.chars().take(300).collect();
                if body.chars().count() > 300 {
                    brief.push('…');
                }
                return Err(LlmError(format!("HTTP {status}: {brief}")));
            }
            Ok(body)
        }
    }
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

/// The streaming variant of [`build_chat_request`] for the OpenAI-compatible
/// path: the same body with `"stream": true`, so the server answers with
/// server-sent events (`data: {...}` lines ending in `data: [DONE]`). The
/// Anthropic path is not streamed here — it answers in seconds — so a
/// request built for it is returned unchanged and `stream` stays absent.
pub fn build_stream_request(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[crate::tools::ToolSpec],
) -> LlmRequest {
    let mut request = build_chat_request_with_tools(backend, system_prompt, turns, tools);
    if matches!(backend, Backend::OpenAiCompatible { .. }) {
        request.body["stream"] = json!(true);
    }
    request
}

/// Incremental parser for OpenAI-style SSE bodies. Feed it raw chunks as they
/// arrive (they split anywhere, including mid-line); it returns the text
/// deltas found in complete `data:` lines and remembers the partial tail.
/// Pure and allocation-light; pinned by tests with chunks cut mid-JSON.
#[derive(Debug, Default)]
pub struct SseAccumulator {
    buffer: String,
    done: bool,
    /// Streamed `delta.tool_calls`, merged by index: id/name when present,
    /// `arguments` fragments concatenated (OpenAI fragments; ollama sends
    /// each call whole in one delta — both land here).
    calls: Vec<(String, String, String)>,
}

impl SseAccumulator {
    /// Consume one network chunk; return the content deltas it completed.
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        let mut deltas = Vec::new();
        self.buffer.push_str(chunk);
        while let Some(newline) = self.buffer.find('\n') {
            let line = self.buffer[..newline].trim_end_matches('\r').to_string();
            self.buffer.drain(..=newline);
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                self.done = true;
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            if let Some(text) = value
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                && !text.is_empty()
            {
                deltas.push(text.to_string());
            }
            if let Some(calls) = value
                .pointer("/choices/0/delta/tool_calls")
                .and_then(Value::as_array)
            {
                for c in calls {
                    let index = c
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(0);
                    while self.calls.len() <= index {
                        self.calls
                            .push((String::new(), String::new(), String::new()));
                    }
                    let slot = &mut self.calls[index];
                    if let Some(id) = c.get("id").and_then(Value::as_str) {
                        slot.0 = id.to_string();
                    }
                    if let Some(name) = c.pointer("/function/name").and_then(Value::as_str) {
                        slot.1 = name.to_string();
                    }
                    if let Some(args) = c.pointer("/function/arguments").and_then(Value::as_str) {
                        slot.2.push_str(args);
                    }
                }
            }
        }
        deltas
    }

    /// The tool calls streamed so far, parsed. Fragments that never became
    /// valid JSON yield `{}` arguments, like the non-streamed parser.
    pub fn tool_calls(&self) -> Vec<crate::tools::ToolCall> {
        self.calls
            .iter()
            .filter(|(_, name, _)| !name.is_empty())
            .enumerate()
            .map(|(i, (id, name, args))| crate::tools::ToolCall {
                id: if id.is_empty() {
                    format!("call_{i}")
                } else {
                    id.clone()
                },
                name: name.clone(),
                arguments: serde_json::from_str(args).unwrap_or(json!({})),
            })
            .collect()
    }

    /// Whether `data: [DONE]` has been seen.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// A transport that can stream: deliver deltas through `on_delta` as they
/// arrive and resolve with the full text. `HttpTransport` implements it over
/// reqwest's byte stream; tests use a fake that replays canned SSE chunks.
pub trait StreamTransport {
    fn post_stream(
        &self,
        request: &LlmRequest,
        on_delta: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> impl Future<Output = Result<ModelTurn, LlmError>> + Send;
}

impl StreamTransport for HttpTransport {
    fn post_stream(
        &self,
        request: &LlmRequest,
        on_delta: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> impl Future<Output = Result<ModelTurn, LlmError>> + Send {
        let mut builder = self.client.post(&request.url).json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(*name, value);
        }
        async move {
            use futures_util::StreamExt as _;
            let response = builder
                .send()
                .await
                .map_err(|e| LlmError(format!("the request failed: {e}")))?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let brief: String = body.chars().take(300).collect();
                return Err(LlmError(format!("HTTP {status}: {brief}")));
            }
            let mut stream = response.bytes_stream();
            let mut acc = SseAccumulator::default();
            let mut full = String::new();
            while let Some(chunk) = stream.next().await {
                let bytes =
                    chunk.map_err(|e| LlmError(format!("reading the stream failed: {e}")))?;
                for delta in acc.feed(&String::from_utf8_lossy(&bytes)) {
                    full.push_str(&delta);
                    let _ = on_delta.send(delta);
                }
                if acc.is_done() {
                    break;
                }
            }
            let calls = acc.tool_calls();
            if full.trim().is_empty() && calls.is_empty() {
                return Err(LlmError("the stream carried no answer text".to_string()));
            }
            Ok(ModelTurn { text: full, calls })
        }
    }
}

/// Test double: records the exact request it was handed and returns a canned
/// body. `cfg(test)` deliberately — this is a binary crate, where `pub` exempts
/// nothing from the dead-code lint.
#[cfg(test)]
pub struct RecordingTransport {
    canned_response: String,
    recorded: std::sync::Mutex<Option<LlmRequest>>,
}

#[cfg(test)]
impl RecordingTransport {
    pub fn returning(canned_response: &str) -> Self {
        Self {
            canned_response: canned_response.to_string(),
            recorded: std::sync::Mutex::new(None),
        }
    }

    /// The one request this transport was handed.
    pub fn recorded(&self) -> LlmRequest {
        self.recorded
            .lock()
            .expect("recording mutex is never poisoned")
            .clone()
            .expect("a request was posted before asking for it")
    }
}

#[cfg(test)]
impl Transport for RecordingTransport {
    fn post(&self, request: &LlmRequest) -> impl Future<Output = Result<String, LlmError>> + Send {
        *self
            .recorded
            .lock()
            .expect("recording mutex is never poisoned") = Some(request.clone());
        std::future::ready(Ok(self.canned_response.clone()))
    }
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
        let raw = r#"{"content":[{"type":"text","text":"Sure."},{"type":"tool_use","id":"t1","name":"recall","input":{"query":"q"}}]}"#;
        let turn = extract_turn(&anthropic, raw).unwrap();
        assert_eq!(turn.text, "Sure.");
        assert_eq!(turn.calls[0].name, "recall");
        // Empty both ways falls back to extract_text's honest error.
        assert!(extract_turn(&local, r#"{"choices":[{"message":{"content":""}}]}"#).is_err());
    }

    #[test]
    fn sse_accumulator_merges_streamed_tool_calls_whole_and_fragmented() {
        let mut acc = SseAccumulator::default();
        acc.feed("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_a\",\"index\":0,\"function\":{\"name\":\"recall\",\"arguments\":\"{\\\"query\\\":\\\"ru\"}}]}}]}\n");
        acc.feed("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"st\\\"}\"}}]}}]}\n");
        let calls = acc.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].arguments["query"], "rust");
    }

    #[test]
    fn sse_accumulator_handles_chunks_split_mid_line_and_done() {
        let mut acc = SseAccumulator::default();
        let a = acc.feed(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"del",
        );
        assert_eq!(a, vec!["Hel".to_string()]);
        let b = acc.feed(
            "ta\":{\"content\":\"lo\"}}]}\nnot-an-sse-line\ndata: {\"choices\":[{\"delta\":{}}]}\n",
        );
        assert_eq!(b, vec!["lo".to_string()]);
        assert!(!acc.is_done());
        let c = acc.feed("data: [DONE]\n");
        assert!(c.is_empty());
        assert!(acc.is_done());
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
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning":"thinking hard"}}]}"#;
        let err = extract_text(&backend, raw).expect_err("no answer");
        assert!(err.0.contains("reasoning"), "{}", err.0);
        let plain = r#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#;
        assert_eq!(
            extract_text(&backend, plain).expect_err("no answer").0,
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
        let transport =
            RecordingTransport::returning(r#"{"content":[{"type":"text","text":"the answer"}]}"#);

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
            r#"{"choices":[{"message":{"content":"local answer"}}]}"#,
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
        let raw =
            r#"{"content":[{"type":"thinking","thinking":"…"},{"type":"text","text":"visible"}]}"#;
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
