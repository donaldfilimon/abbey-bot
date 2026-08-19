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
}

impl Role {
    /// The wire name — identical on both backends.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One turn of conversation history. The engine keeps these per scope and the
/// request builder serializes them in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurn {
    pub role: Role,
    pub text: String,
}

impl ChatTurn {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
        }
    }
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
    let messages: Vec<Value> = turns
        .iter()
        .map(|turn| json!({"role": turn.role.as_str(), "content": turn.text}))
        .collect();
    match backend {
        Backend::Anthropic { api_key } => LlmRequest {
            url: ANTHROPIC_URL.to_string(),
            // The ecosystem's pinned header pair for this API.
            headers: vec![
                ("x-api-key", api_key.clone()),
                ("anthropic-version", ANTHROPIC_VERSION.to_string()),
            ],
            body: json!({
                "model": ANTHROPIC_MODEL,
                "max_tokens": MAX_TOKENS,
                "system": system_prompt,
                "messages": messages,
            }),
        },
        Backend::OpenAiCompatible { endpoint, model } => {
            let mut all = Vec::with_capacity(messages.len() + 1);
            all.push(json!({"role": "system", "content": system_prompt}));
            all.extend(messages);
            LlmRequest {
                url: format!("{}/v1/chat/completions", endpoint.trim_end_matches('/')),
                headers: Vec::new(),
                body: json!({
                    "model": model,
                    "max_tokens": LOCAL_MAX_TOKENS,
                    "messages": all,
                }),
            }
        }
    }
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

impl Default for HttpTransport {
    fn default() -> Self {
        Self {
            // Bounded so a hung backend cannot hold the deferred interaction
            // forever: Discord's followup window is 15 minutes, and a large
            // local model can legitimately take a couple of them — 120s is the
            // compromise between that and never answering at all.
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
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
