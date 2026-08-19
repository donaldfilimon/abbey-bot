//! Vision: image understanding, pure parts.
//!
//! Port of `docs/spec/vision.md`. Abbey reads images the way she reads text:
//! an attachment gets described, the description is folded into the message
//! before anything downstream sees it. The shape mirrors `llm.rs` — request
//! construction and response extraction are pure and pinned by tests; the
//! network sits behind a trait ([`VisionTransport`]) that no test implements
//! with a real client. This module imports no HTTP crate at all: the
//! orchestrator supplies the live transport and the bytes it fetched.
//!
//! The remote analyzer speaks OpenAI-compatible chat completions with an
//! image content part. The image travels as a base64 data URL rather than as
//! its original link, because Discord CDN links are signed and expire and
//! Telegram/Slack URLs need auth the remote end does not have.

use std::fmt;
use std::future::Future;

use serde_json::{Value, json};

/// Cap on fetched image size. Attachments are attacker-controlled; a fetcher
/// must stop reading at this many bytes.
pub const MAX_IMAGE_BYTES: usize = 10 << 20;

/// Output budget for a description or transcription.
/// Output budget. The spec says 200, sized for the answer alone; local
/// reasoning models (gemma4 via ollama) spend ~1.7k characters thinking about
/// the picture first and returned a truncated "This image is" at 200
/// (measured 2026-08-19), so the budget covers the thinking too.
const MAX_TOKENS: u32 = 1024;

/// Model used when `ABBEY_VISION_MODEL` is unset.
const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Upper bound on images described per message — keeps one photo dump from
/// turning into a dozen round trips.
pub const MAX_DESCRIBED_IMAGES: usize = 3;

/// MIME type from the leading magic bytes, exactly the set the spec sniffs.
/// Anything else is `application/octet-stream` and the remote end decides.
pub fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(b"GIF") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

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

/// A fully assembled vision request: exactly what the live transport sends,
/// and exactly what a recording fake captures.
///
/// `Debug` is hand-written: `headers` carries `Authorization`, and a derived
/// `Debug` would print the key through any `{:?}` path (tracing fields, panic
/// messages, failing assertions).
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
        // The body holds the whole image as base64; printing megabytes of it
        // helps nobody, so only its size is shown.
        let body_len = self.body.to_string().len();
        f.debug_struct("VisionRequest")
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_bytes", &body_len)
            .finish()
    }
}

/// Build the chat-completions request for one image. `base_url` is the
/// OpenAI-compatible base *including* `/v1` (e.g. `https://api.openai.com/v1`).
/// A blank `api_key` sends no `Authorization` header — loopback servers want
/// none, and `Bearer ` with nothing after it is a malformed header.
pub fn build_vision_request(
    base_url: &str,
    model: &str,
    api_key: &str,
    task: VisionTask,
    image_bytes: &[u8],
) -> VisionRequest {
    let mime = sniff_mime(image_bytes);
    let data_url = format!("data:{mime};base64,{}", base64_encode(image_bytes));
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

/// Standard base64 (RFC 4648 §4) with padding. Hand-rolled because the crate
/// has no encoding dependency and this is the one place that needs it.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

/// Why a vision call produced no answer. Carries no secrets by construction:
/// keys travel in headers, and header values never appear in error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionError(pub String);

impl fmt::Display for VisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VisionError {}

/// Pull the answer out of a chat-completions body: `choices[0].message.content`.
/// An empty string is a legitimate OCR result (no text in the image), so it
/// is returned as `Ok("")`, not an error — the renderer says "No text found."
pub fn extract_vision_text(raw: &str) -> Result<String, VisionError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| VisionError(format!("the vision response was not JSON: {e}")))?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .ok_or_else(|| VisionError("the vision response carried no message content".to_string()))?;
    if content.is_empty()
        && value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("length")
    {
        return Err(VisionError(
            "the vision model spent its whole budget reasoning and produced no description".into(),
        ));
    }
    Ok(content)
}

/// The seam: natural-language description and OCR over raw image bytes.
/// Fetching (with the [`MAX_IMAGE_BYTES`] cap, `tgfile://` resolution, Slack
/// auth) happens before this, in the orchestrator — the analyzer never sees a
/// URL, so it can never be handed one it lacks credentials for.
pub trait ImageUnderstanding {
    /// ≤2 sentences, suitable for inline folding into a chat message.
    fn describe(&self, image: &[u8]) -> impl Future<Output = Result<String, VisionError>> + Send;
    /// OCR only — verbatim text found in the image.
    fn extract_text(
        &self,
        image: &[u8],
    ) -> impl Future<Output = Result<String, VisionError>> + Send;
}

/// The network seam under [`RemoteVision`]: post one request, return the raw
/// body. The orchestrator implements it with a real client; tests with a fake.
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
    /// Selection from environment values. Pure — takes values, not the
    /// process environment — so tests never touch env state.
    ///
    /// Precedence:
    /// 1. `ABBEY_VISION_ENDPOINT` set → that is the base, verbatim (it must
    ///    already include `/v1`, matching the spec's
    ///    `https://api.openai.com/v1` default shape).
    /// 2. Else `ABBEY_BOT_LLM_ENDPOINT` set → the `/persona ask` server is
    ///    reused with `/v1` appended, since `llm.rs` treats that value as a
    ///    host root. A multimodal local model behind llama-server/ollama/mlx
    ///    then serves both text and vision with one variable.
    /// 3. Else no vision — the caller degrades (descriptions are skipped,
    ///    `/see` and `/ocr` say vision is not configured).
    ///
    /// Model is `ABBEY_VISION_MODEL` or `gpt-4o-mini`; key is
    /// `ABBEY_VISION_KEY` or empty. Blank values count as unset throughout,
    /// because `.env.example` ships blank assignments. `ABBEY_VISION_ENDPOINT=off`
    /// disables vision even when an LLM endpoint is set.
    pub fn from_values(
        vision_endpoint: Option<String>,
        vision_model: Option<String>,
        vision_key: Option<String>,
        fallback_llm_endpoint: Option<String>,
    ) -> Option<Self> {
        let non_blank = |value: Option<String>| {
            value
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let base_url = match non_blank(vision_endpoint) {
            // `off` is the sentinel that stops the LLM-endpoint fallback: a
            // text-only local model (ollama gemma) would otherwise be asked to
            // read images and fail a round-trip per attachment.
            Some(off) if off.eq_ignore_ascii_case("off") => return None,
            Some(explicit) => explicit,
            None => format!(
                "{}/v1",
                non_blank(fallback_llm_endpoint)?.trim_end_matches('/')
            ),
        };
        Some(Self {
            base_url,
            model: non_blank(vision_model).unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            api_key: non_blank(vision_key).unwrap_or_default(),
        })
    }

    /// Selection from the real environment — the runtime path. Tests go
    /// through [`VisionConfig::from_values`].
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            std::env::var("ABBEY_VISION_ENDPOINT").ok(),
            std::env::var("ABBEY_VISION_MODEL").ok(),
            std::env::var("ABBEY_VISION_KEY").ok(),
            std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
        )
    }

    /// Build the request this config would send for `task` over `image`.
    pub fn request(&self, task: VisionTask, image: &[u8]) -> VisionRequest {
        build_vision_request(&self.base_url, &self.model, &self.api_key, task, image)
    }
}

/// The OpenAI-compatible analyzer over any transport — `RemoteVisionAnalyzer`
/// in the spec. The Apple Vision path has no Rust counterpart here; this is
/// the implementation every host gets.
pub struct RemoteVision<T> {
    pub config: VisionConfig,
    pub transport: T,
}

impl<T: VisionTransport + Sync> RemoteVision<T> {
    async fn ask(&self, task: VisionTask, image: &[u8]) -> Result<String, VisionError> {
        if image.len() > MAX_IMAGE_BYTES {
            return Err(VisionError(format!(
                "the image is {} bytes; the cap is {MAX_IMAGE_BYTES}",
                image.len()
            )));
        }
        let request = self.config.request(task, image);
        let raw = self.transport.post(&request).await?;
        extract_vision_text(&raw)
    }
}

impl<T: VisionTransport + Sync> ImageUnderstanding for RemoteVision<T> {
    fn describe(&self, image: &[u8]) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.ask(VisionTask::Describe, image)
    }

    fn extract_text(
        &self,
        image: &[u8],
    ) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.ask(VisionTask::ExtractText, image)
    }
}

/// Fold image descriptions into the message text, one bracketed line per
/// image, at most [`MAX_DESCRIBED_IMAGES`]. This is the string the intent
/// classifier and the persona see.
pub fn fold_descriptions(text: &str, described: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (filename, description) in described.iter().take(MAX_DESCRIBED_IMAGES) {
        out.push_str("\n[image ");
        out.push_str(filename);
        out.push_str(": ");
        out.push_str(description);
        out.push(']');
    }
    out
}

/// The `/see` reply: the persona speaking, then the description.
pub fn render_see(persona_label: &str, description: &str) -> String {
    let description = description.trim();
    if description.is_empty() {
        return format!("**{persona_label}** — I couldn't make anything out in that image.");
    }
    format!("**{persona_label}** — {description}")
}

/// The `/ocr` reply: the transcribed text in a code block, or a plain note
/// when the image carried none. Backticks inside the text would close the
/// block early, so they are softened.
pub fn render_ocr(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "No text found.".to_string();
    }
    format!("```\n{}\n```", text.replace("```", "ʼʼʼ"))
}

/// Test double: answers every call with canned text and records which task
/// it was asked. `cfg(test)` because `pub` exempts nothing in a binary crate.
#[cfg(test)]
pub struct RecordingVision {
    canned: String,
    calls: std::sync::Mutex<Vec<VisionTask>>,
}

#[cfg(test)]
impl RecordingVision {
    pub fn returning(canned: &str) -> Self {
        Self {
            canned: canned.to_string(),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<VisionTask> {
        self.calls.lock().expect("never poisoned").clone()
    }
}

#[cfg(test)]
impl ImageUnderstanding for RecordingVision {
    fn describe(&self, _image: &[u8]) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.calls
            .lock()
            .expect("never poisoned")
            .push(VisionTask::Describe);
        std::future::ready(Ok(self.canned.clone()))
    }

    fn extract_text(
        &self,
        _image: &[u8],
    ) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.calls
            .lock()
            .expect("never poisoned")
            .push(VisionTask::ExtractText);
        std::future::ready(Ok(self.canned.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Transport fake: records the request, returns a canned body.
    struct RecordingTransport {
        canned: String,
        recorded: std::sync::Mutex<Option<VisionRequest>>,
    }

    impl VisionTransport for RecordingTransport {
        fn post(
            &self,
            request: &VisionRequest,
        ) -> impl Future<Output = Result<String, VisionError>> + Send {
            *self.recorded.lock().expect("never poisoned") = Some(request.clone());
            std::future::ready(Ok(self.canned.clone()))
        }
    }

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn sniff_mime_recognises_each_magic_and_nothing_else() {
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_mime(PNG), "image/png");
        assert_eq!(sniff_mime(b"GIF89a"), "image/gif");
        let mut webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec();
        webp.push(0);
        assert_eq!(sniff_mime(&webp), "image/webp");
        // Exactly 12 bytes is too short for the WEBP rule (spec: count > 12).
        assert_eq!(
            sniff_mime(b"RIFF\x00\x00\x00\x00WEBP"),
            "application/octet-stream"
        );
        assert_eq!(sniff_mime(b"hello"), "application/octet-stream");
        assert_eq!(sniff_mime(&[]), "application/octet-stream");
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(&[0xFF, 0xFF, 0xFF]), "////");
    }

    #[test]
    fn task_prompts_are_the_spec_strings() {
        assert_eq!(
            VisionTask::Describe.prompt(),
            "Describe this image in at most two short sentences. Factual, no preamble."
        );
        assert_eq!(
            VisionTask::ExtractText.prompt(),
            "Transcribe all text visible in this image verbatim. Output only the text."
        );
    }

    #[test]
    fn request_shape_is_chat_completions_with_a_data_url() {
        let req = build_vision_request(
            "https://api.openai.com/v1/",
            "gpt-4o-mini",
            "sk-test",
            VisionTask::Describe,
            PNG,
        );
        assert_eq!(req.url, "https://api.openai.com/v1/chat/completions");
        assert!(
            req.headers
                .contains(&("Authorization".to_string(), "Bearer sk-test".to_string()))
        );
        assert_eq!(req.body["model"], "gpt-4o-mini");
        assert_eq!(req.body["max_tokens"], 1024);
        let content = &req.body["messages"][0]["content"];
        assert_eq!(req.body["messages"][0]["role"], "user");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], VisionTask::Describe.prompt());
        assert_eq!(content[1]["type"], "image_url");
        let url = content[1]["image_url"]["url"].as_str().expect("a string");
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert_eq!(&url["data:image/png;base64,".len()..], base64_encode(PNG));
    }

    #[test]
    fn blank_key_sends_no_authorization_header() {
        let req = build_vision_request(
            "http://127.0.0.1:8080/v1",
            "m",
            "  ",
            VisionTask::ExtractText,
            PNG,
        );
        assert!(
            req.headers
                .iter()
                .all(|(n, _)| !n.eq_ignore_ascii_case("authorization"))
        );
        assert_eq!(
            req.body["messages"][0]["content"][0]["text"],
            VisionTask::ExtractText.prompt()
        );
    }

    #[test]
    fn debug_redacts_the_key_and_the_image_payload() {
        const SECRET: &str = "sk-vision-super-secret";
        let req = build_vision_request("https://x/v1", "m", SECRET, VisionTask::Describe, PNG);
        let shown = format!("{req:?}");
        assert!(!shown.contains(SECRET), "request leaked the key: {shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("https://x/v1/chat/completions"), "{shown}");
        assert!(
            !shown.contains(&base64_encode(PNG)),
            "image dumped: {shown}"
        );

        let config = VisionConfig {
            base_url: "https://x/v1".into(),
            model: "m".into(),
            api_key: SECRET.into(),
        };
        let shown = format!("{config:?}");
        assert!(!shown.contains(SECRET), "config leaked the key: {shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        let keyless = VisionConfig {
            api_key: String::new(),
            ..config
        };
        assert!(format!("{keyless:?}").contains("<none>"));
    }

    #[test]
    fn extraction_reads_choices_zero_and_rejects_junk() {
        assert_eq!(
            extract_vision_text(r#"{"choices":[{"message":{"content":"  a cat on a sofa "}}]}"#)
                .expect("parses"),
            "a cat on a sofa"
        );
        // Empty content is a legitimate OCR outcome, not an error.
        assert_eq!(
            extract_vision_text(r#"{"choices":[{"message":{"content":""}}]}"#).expect("parses"),
            ""
        );
        assert!(extract_vision_text("not json").is_err());
        assert!(extract_vision_text(r#"{"choices":[]}"#).is_err());
        assert!(extract_vision_text(r#"{"error":{"message":"nope"}}"#).is_err());
    }

    #[test]
    fn config_precedence_explicit_then_llm_fallback_then_none() {
        assert_eq!(VisionConfig::from_values(None, None, None, None), None);
        assert_eq!(
            VisionConfig::from_values(
                Some("off".into()),
                None,
                None,
                Some("http://127.0.0.1:11434".into())
            ),
            None,
            "the off sentinel beats the LLM-endpoint fallback"
        );
        // Blank counts as unset everywhere.
        assert_eq!(
            VisionConfig::from_values(
                Some("  ".into()),
                Some(String::new()),
                Some(" ".into()),
                Some("".into())
            ),
            None
        );
        // Only the LLM endpoint: reused with /v1 appended and the default model.
        let fallback =
            VisionConfig::from_values(None, None, None, Some("http://127.0.0.1:8080/".into()))
                .expect("falls back");
        assert_eq!(fallback.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(fallback.model, "gpt-4o-mini");
        assert_eq!(fallback.api_key, "");
        // Explicit vision endpoint wins over the LLM one, verbatim.
        let explicit = VisionConfig::from_values(
            Some("https://api.openai.com/v1".into()),
            Some("gpt-4o".into()),
            Some("sk".into()),
            Some("http://127.0.0.1:8080".into()),
        )
        .expect("selects");
        assert_eq!(explicit.base_url, "https://api.openai.com/v1");
        assert_eq!(explicit.model, "gpt-4o");
        assert_eq!(explicit.api_key, "sk");
    }

    #[tokio::test]
    async fn remote_vision_runs_the_whole_path_over_a_fake_transport() {
        let transport = RecordingTransport {
            canned: r#"{"choices":[{"message":{"content":"two ducks"}}]}"#.into(),
            recorded: std::sync::Mutex::new(None),
        };
        let vision = RemoteVision {
            config: VisionConfig {
                base_url: "http://127.0.0.1:8080/v1".into(),
                model: "llava".into(),
                api_key: String::new(),
            },
            transport,
        };
        assert_eq!(vision.describe(PNG).await.expect("ok"), "two ducks");
        let sent = vision
            .transport
            .recorded
            .lock()
            .expect("never poisoned")
            .clone()
            .expect("posted");
        assert_eq!(sent.url, "http://127.0.0.1:8080/v1/chat/completions");
        assert_eq!(sent.body["model"], "llava");

        assert_eq!(vision.extract_text(PNG).await.expect("ok"), "two ducks");
        let sent = vision
            .transport
            .recorded
            .lock()
            .expect("never poisoned")
            .clone()
            .expect("posted");
        assert_eq!(
            sent.body["messages"][0]["content"][0]["text"],
            VisionTask::ExtractText.prompt()
        );

        // Oversized input is refused before any request is built.
        let huge = vec![0u8; MAX_IMAGE_BYTES + 1];
        assert!(vision.describe(&huge).await.is_err());
    }

    #[tokio::test]
    async fn recording_vision_returns_canned_text_and_records_tasks() {
        let fake = RecordingVision::returning("a sunset");
        assert_eq!(fake.describe(PNG).await.expect("ok"), "a sunset");
        assert_eq!(fake.extract_text(PNG).await.expect("ok"), "a sunset");
        assert_eq!(
            fake.calls(),
            vec![VisionTask::Describe, VisionTask::ExtractText]
        );
    }

    #[test]
    fn fold_appends_bracketed_lines_and_stops_at_three() {
        let described: Vec<(String, String)> = (1..=5)
            .map(|i| (format!("p{i}.png"), format!("desc {i}")))
            .collect();
        let folded = fold_descriptions("look at these", &described);
        assert_eq!(
            folded,
            "look at these\n[image p1.png: desc 1]\n[image p2.png: desc 2]\n[image p3.png: desc 3]"
        );
        assert_eq!(fold_descriptions("plain", &[]), "plain");
    }

    #[test]
    fn renderers_read_well_and_handle_empty() {
        assert_eq!(
            render_see("Abbey", " a red bicycle "),
            "**Abbey** — a red bicycle"
        );
        assert!(render_see("Abbey", "").contains("couldn't make anything out"));
        assert_eq!(render_ocr("  "), "No text found.");
        assert_eq!(render_ocr("hello\nworld"), "```\nhello\nworld\n```");
        let fenced = render_ocr("a ``` b");
        assert!(!fenced[3..fenced.len() - 3].contains("```"), "{fenced}");
    }
}
