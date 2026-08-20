//! HTTP transport boundary for generation backends.
//!
//! Request/response protocol handling lives in the sibling modules. This file
//! owns only client policy, capped body reads, and the recording test double.

use std::future::Future;

use super::{LlmError, LlmRequest, MAX_ERROR_RESPONSE_BYTES, MAX_RESPONSE_BYTES, url_is_loopback};

/// The seam between request construction and the network. Tests implement this
/// with [`RecordingTransport`]; the binary uses [`HttpTransport`]. Returns the
/// raw response body so response extraction remains pure.
pub trait Transport {
    fn post(&self, request: &LlmRequest) -> impl Future<Output = Result<String, LlmError>> + Send;
}

/// The live transport. Request builders and parsers remain I/O-free.
pub struct HttpTransport {
    pub(super) remote_client: reqwest::Client,
    pub(super) loopback_client: reqwest::Client,
}

/// Default request timeout. Discord's followup window is 15 minutes; a local
/// reasoning model under concurrent load was observed live (2026-08-19) to
/// need more than the old 120 s, so the default is 300 s and the environment
/// may override it.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Parse a timeout override; blank, invalid, and zero values use the default.
pub fn timeout_from_value(value: Option<String>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

impl Default for HttpTransport {
    fn default() -> Self {
        let secs = timeout_from_value(std::env::var("ABBEY_BOT_LLM_TIMEOUT_SECS").ok());
        let client = |no_proxy| {
            let builder = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(secs))
                // Never forward an API key or future endpoint credential
                // through a server-selected redirect.
                .redirect(reqwest::redirect::Policy::none());
            let builder = if no_proxy {
                builder.no_proxy()
            } else {
                builder
            };
            builder
                .build()
                .expect("static reqwest client configuration is valid")
        };
        Self {
            remote_client: client(false),
            // A process-wide proxy must never receive a local transcript,
            // persona/context, or memory recall destined for Ollama/MLX.
            loopback_client: client(true),
        }
    }
}

impl HttpTransport {
    pub(super) fn client_for(&self, raw_url: &str) -> &reqwest::Client {
        if reqwest::Url::parse(raw_url).is_ok_and(|url| url_is_loopback(&url)) {
            &self.loopback_client
        } else {
            &self.remote_client
        }
    }
}

impl Transport for HttpTransport {
    fn post(&self, request: &LlmRequest) -> impl Future<Output = Result<String, LlmError>> + Send {
        let mut builder = self
            .client_for(&request.url)
            .post(&request.url)
            .json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(*name, value);
        }
        async move {
            let response = builder
                .send()
                .await
                .map_err(|e| LlmError::backend(format!("the request failed: {e}")))?;
            let status = response.status();
            let limit = if status.is_success() {
                MAX_RESPONSE_BYTES
            } else {
                MAX_ERROR_RESPONSE_BYTES
            };
            let body = crate::http_body::read_capped(response, limit)
                .await
                .map_err(|e| LlmError::backend(format!("HTTP {status}: {e}")))?;
            if !status.is_success() {
                // Provider diagnostics are private and bounded. Lossy decoding
                // is acceptable here because this text is never parsed.
                let body = String::from_utf8_lossy(&body);
                let mut brief: String = body.chars().take(300).collect();
                if body.chars().count() > 300 {
                    brief.push('…');
                }
                return Err(LlmError::backend(format!("HTTP {status}: {brief}")));
            }
            String::from_utf8(body).map_err(|_| {
                LlmError::backend("the backend returned response bytes that were not UTF-8".into())
            })
        }
    }
}

/// Test double that records the exact request and returns a canned body.
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
