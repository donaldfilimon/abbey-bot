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
/// Output budget on both paths. Replies are clamped to 2,000 Discord
/// codepoints anyway, so a small ceiling wastes neither tokens nor money.
const MAX_TOKENS: u32 = 1024;

/// Which generation backend the environment selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// `ANTHROPIC_API_KEY`: the external Anthropic Messages API.
    Anthropic { api_key: String },
    /// `ABBEY_BOT_LLM_ENDPOINT`: an OpenAI-compatible server, usually loopback.
    OpenAiCompatible { endpoint: String },
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
    ) -> Option<Self> {
        let non_blank = |value: Option<String>| {
            value
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        if let Some(api_key) = non_blank(anthropic_api_key) {
            return Some(Self::Anthropic { api_key });
        }
        non_blank(endpoint).map(|endpoint| Self::OpenAiCompatible { endpoint })
    }

    /// Selection from the real environment — the runtime path. Tests go
    /// through [`Backend::from_values`] instead, which is this minus the read.
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            std::env::var("ANTHROPIC_API_KEY").ok(),
            std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
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
#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    pub url: String,
    /// Header name/value pairs in send order. `content-type: application/json`
    /// is implied by the JSON body and set by the live transport.
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

/// Build the request a backend call would send. Pure, so the exact wire shape
/// is pinned by tests without any network.
pub fn build_request(backend: &Backend, system_prompt: &str, question: &str) -> LlmRequest {
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
                "messages": [{"role": "user", "content": question}],
            }),
        },
        Backend::OpenAiCompatible { endpoint } => LlmRequest {
            url: format!("{}/v1/chat/completions", endpoint.trim_end_matches('/')),
            headers: Vec::new(),
            body: json!({
                // llama-server and mlx serve whatever model they were started
                // with and ignore this field; it exists because the schema
                // requires one.
                "model": "default",
                "max_tokens": MAX_TOKENS,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": question},
                ],
            }),
        },
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
        Backend::Anthropic { .. } => {
            value
                .get("content")
                .and_then(Value::as_array)
                .and_then(|blocks| {
                    blocks.iter().find_map(|block| {
                        (block.get("type").and_then(Value::as_str) == Some("text"))
                            .then(|| block.get("text").and_then(Value::as_str))
                            .flatten()
                    })
                })
        }
        Backend::OpenAiCompatible { .. } => value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str),
    };
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t.to_string()),
        // An empty content array is a real outcome (e.g. a refusal); saying so
        // beats presenting an empty string as an answer.
        _ => Err(LlmError("the response carried no answer text".to_string())),
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
    fn no_env_values_selects_no_backend_so_the_suite_needs_no_network() {
        // The gate rule, in code: the suite runs with no env vars and no
        // network. With neither variable set, selection yields no backend, so
        // `/persona ask` resolves to the degradation reply without any
        // transport existing at all — there is no code path from "no env" to
        // the network.
        assert_eq!(Backend::from_values(None, None), None);
    }

    #[test]
    fn blank_env_values_count_as_unset() {
        // `.env.example` ships blank assignments; copying it unfilled must not
        // select a backend that cannot work.
        assert_eq!(
            Backend::from_values(Some("  ".into()), Some(String::new())),
            None
        );
    }

    #[test]
    fn anthropic_wins_when_both_backends_are_configured() {
        let backend =
            Backend::from_values(Some("key".into()), Some("http://127.0.0.1:8080".into()))
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
                "max_tokens": 1024,
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
}
