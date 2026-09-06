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
            let response = builder.send().await.map_err(LlmError::transport)?;
            let status = response.status();
            let rejection =
                LlmError::http(status, response.headers().get(reqwest::header::RETRY_AFTER));
            if status.is_success()
                && response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER)
            {
                return Err(LlmError::classified(
                    "incompatible provider delay metadata",
                    crate::provider::ProviderFailureKind::ProtocolDrift,
                ));
            }
            if !status.is_success() {
                // Error bodies are untrusted diagnostics. Their read outcome
                // must never replace the already observed HTTP classification.
                let _ = crate::http_body::read_capped(response, MAX_ERROR_RESPONSE_BYTES).await;
                return Err(rejection);
            }
            let body = crate::http_body::read_capped(response, MAX_RESPONSE_BYTES)
                .await
                .map_err(LlmError::body_read)?;
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

#[cfg(test)]
mod runtime_failure_tests {
    use super::*;
    use crate::provider::ProviderFailureKind as F;

    #[test]
    fn body_failures_retain_transport_categories() {
        for (body, expected) in [
            (
                crate::http_body::BodyReadError::Read { timeout: false },
                F::TransportUnavailable,
            ),
            (
                crate::http_body::BodyReadError::Read { timeout: true },
                F::Timeout,
            ),
            (
                crate::http_body::BodyReadError::TooLarge { max: 3 },
                F::ResponseSchema,
            ),
        ] {
            assert_eq!(LlmError::body_read(body).provider_failure(), expected);
        }
    }

    #[tokio::test]
    async fn rejected_http_status_survives_broken_error_body_for_both_transports() {
        use crate::llm::StreamTransport as _;
        use std::io::{Read as _, Write as _};
        for (status, expected) in [
            (401, F::Authentication),
            (429, F::RateLimited),
            (503, F::Http5xx),
        ] {
            for streaming in [false, true] {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let address = listener.local_addr().unwrap();
                let server = std::thread::spawn(move || {
                    let (mut connection, _) = listener.accept().unwrap();
                    connection
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        connection.read_exact(&mut byte).unwrap();
                        header.push(byte[0]);
                        assert!(header.len() <= 16384);
                    }
                    let header = String::from_utf8(header).unwrap();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    let mut body = vec![0; length];
                    connection.read_exact(&mut body).unwrap();
                    write!(connection, "HTTP/1.1 {status} Synthetic\r\nContent-Length: 100\r\nRetry-After: 3\r\nConnection: close\r\n\r\nx").unwrap();
                });
                let backend =
                    crate::llm::Backend::from_values(None, Some(format!("http://{address}")), None)
                        .unwrap();
                let request = crate::llm::build_stream_request(&backend, "", &[], &[]);
                let transport = HttpTransport::default();
                let error = if streaming {
                    let (sender, _) = tokio::sync::mpsc::unbounded_channel();
                    transport.post_stream(&request, sender).await.unwrap_err()
                } else {
                    transport.post(&request).await.unwrap_err()
                };
                server.join().unwrap();
                assert_eq!(error.provider_failure(), expected);
                assert_eq!(
                    error.retry_after(),
                    crate::provider::RetryAfter::from_seconds(Some("3"))
                );
            }
        }
    }
}
