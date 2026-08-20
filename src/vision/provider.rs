//! OpenAI-compatible vision configuration, request/response wire contract,
//! and transport-generic analyzer.

use std::fmt;
use std::future::Future;

use serde_json::{Value, json};

use super::{ImageUnderstanding, VisionError, image};

/// Output budget. The spec says 200, sized for the answer alone; local
/// reasoning models spend part of this budget reasoning about the picture.
const MAX_TOKENS: u32 = 1024;

/// Conventional default for a separately configured remote vision provider.
pub(super) const DEFAULT_REMOTE_MODEL: &str = "gpt-4o-mini";

/// What is being asked of the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisionTask {
    /// A short natural-language description, for folding into chat.
    Describe,
    /// OCR only — verbatim text.
    ExtractText,
}

impl VisionTask {
    pub const fn prompt(self) -> &'static str {
        match self {
            Self::Describe => {
                "Describe this image in at most two short sentences. Factual, no preamble."
            }
            Self::ExtractText => {
                "Transcribe all text visible in this image verbatim. Output only the text."
            }
        }
    }
}

/// A fully assembled vision request: exactly what the live transport sends.
#[derive(Clone, PartialEq)]
pub struct VisionRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl fmt::Debug for VisionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let shown = if name.eq_ignore_ascii_case("authorization") {
                    "<redacted>"
                } else {
                    value.as_str()
                };
                (name.as_str(), shown)
            })
            .collect();
        let body_len = self.body.to_string().len();
        f.debug_struct("VisionRequest")
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_bytes", &body_len)
            .finish()
    }
}

pub(super) fn build_vision_request(
    base_url: &str,
    model: &str,
    api_key: &str,
    task: VisionTask,
    data_url: String,
) -> VisionRequest {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if !api_key.trim().is_empty() {
        headers.push(("Authorization".to_string(), format!("Bearer {api_key}")));
    }
    VisionRequest {
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        headers,
        body: json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": task.prompt()},
                    {"type": "image_url", "image_url": {"url": data_url}},
                ],
            }],
        }),
    }
}

/// Pull `choices[0].message.content` from a chat-completions response.
pub fn extract_vision_text(raw: &str) -> Result<String, VisionError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| VisionError::internal(format!("the vision response was not JSON: {e}")))?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .ok_or_else(|| VisionError::internal("the vision response carried no message content"))?;
    if content.is_empty()
        && value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("length")
    {
        return Err(VisionError::internal(
            "the vision model spent its whole budget reasoning and produced no description",
        ));
    }
    Ok(content)
}

/// The network seam under [`RemoteVision`].
pub trait VisionTransport {
    fn post(
        &self,
        request: &VisionRequest,
    ) -> impl Future<Output = Result<String, VisionError>> + Send;
}

/// Where the remote analyzer points. Holds the key, so `Debug` is hand-written.
#[derive(Clone, PartialEq, Eq)]
pub struct VisionConfig {
    /// OpenAI-compatible base including `/v1`.
    pub base_url: String,
    pub model: String,
    /// May be empty: loopback servers take no key.
    pub api_key: String,
}

impl fmt::Debug for VisionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VisionConfig")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<none>"
                } else {
                    "<redacted>"
                },
            )
            .finish()
    }
}

impl VisionConfig {
    /// Pure environment-value selection. Explicit vision configuration wins;
    /// `off` disables fallback; otherwise the text endpoint is reused at `/v1`.
    pub fn from_values(
        vision_endpoint: Option<String>,
        vision_model: Option<String>,
        vision_key: Option<String>,
        fallback_llm_endpoint: Option<String>,
        fallback_llm_model: Option<String>,
    ) -> Option<Self> {
        let non_blank = |value: Option<String>| {
            value
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let (base_url, reused_llm_endpoint) = match non_blank(vision_endpoint) {
            Some(off) if off.eq_ignore_ascii_case("off") => return None,
            Some(explicit) => (explicit, false),
            None => (
                format!(
                    "{}/v1",
                    non_blank(fallback_llm_endpoint)?.trim_end_matches('/')
                ),
                true,
            ),
        };
        let model = non_blank(vision_model)
            .or_else(|| {
                reused_llm_endpoint
                    .then(|| non_blank(fallback_llm_model))
                    .flatten()
            })
            .unwrap_or_else(|| {
                if reused_llm_endpoint {
                    crate::llm::DEFAULT_LOCAL_MODEL.to_string()
                } else {
                    DEFAULT_REMOTE_MODEL.to_string()
                }
            });
        Some(Self {
            base_url,
            model,
            api_key: non_blank(vision_key).unwrap_or_default(),
        })
    }

    pub fn from_env() -> Option<Self> {
        Self::from_values(
            std::env::var("ABBEY_VISION_ENDPOINT").ok(),
            std::env::var("ABBEY_VISION_MODEL").ok(),
            std::env::var("ABBEY_VISION_KEY").ok(),
            std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
            std::env::var("ABBEY_BOT_LLM_MODEL").ok(),
        )
    }

    fn request(&self, task: VisionTask, data_url: String) -> VisionRequest {
        build_vision_request(&self.base_url, &self.model, &self.api_key, task, data_url)
    }
}

/// OpenAI-compatible analyzer over any transport.
pub struct RemoteVision<T> {
    pub config: VisionConfig,
    pub transport: T,
}

impl<T: VisionTransport + Sync> RemoteVision<T> {
    async fn ask(&self, task: VisionTask, bytes: Vec<u8>) -> Result<String, VisionError> {
        let data_url = image::prepare_data_url(bytes).await?;
        let request = self.config.request(task, data_url);
        let raw = self.transport.post(&request).await?;
        extract_vision_text(&raw)
    }
}

impl<T: VisionTransport + Sync> ImageUnderstanding for RemoteVision<T> {
    fn describe(&self, image: Vec<u8>) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.ask(VisionTask::Describe, image)
    }

    fn extract_text(
        &self,
        image: Vec<u8>,
    ) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.ask(VisionTask::ExtractText, image)
    }
}
