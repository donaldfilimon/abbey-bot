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

mod image;
mod provider;
mod render;

pub use provider::{RemoteVision, VisionConfig, VisionRequest, VisionTransport};
pub use render::{fold_descriptions, render_ocr, render_see};

#[cfg(test)]
use std::io::Cursor;

#[cfg(test)]
use image::{
    INVALID_IMAGE_PUBLIC, MAX_DECODED_IMAGE_DIMENSION, OVERSIZED_IMAGE_PUBLIC,
    UNSUPPORTED_IMAGE_PUBLIC, data_url_unchecked, sniff_mime,
};

#[cfg(test)]
use provider::{DEFAULT_REMOTE_MODEL, VisionTask, build_vision_request, extract_vision_text};

/// Cap on fetched image size. Attachments are attacker-controlled; a fetcher
/// must stop reading at this many bytes.
pub const MAX_IMAGE_BYTES: usize = 10 << 20;

/// Upper bound on images described per message — keeps one photo dump from
/// turning into a dozen round trips.
pub const MAX_DESCRIBED_IMAGES: usize = 3;

/// Why a vision call produced no answer. Carries no secrets by construction:
/// keys travel in headers, and header values never appear in error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionError {
    message: String,
    public_message: Option<&'static str>,
}

impl VisionError {
    /// Provider/protocol error: useful in redacted logs, but not suitable for
    /// echoing to a Discord interaction.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            public_message: None,
        }
    }

    /// Locally determined input error with fixed safe caller-facing copy.
    fn invalid_image(message: impl Into<String>, public_message: &'static str) -> Self {
        Self {
            message: message.into(),
            public_message: Some(public_message),
        }
    }

    /// Safe fixed copy for invalid caller input, if this error has one.
    pub fn public_message(&self) -> Option<&'static str> {
        self.public_message
    }
}

impl fmt::Display for VisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VisionError {}

/// The seam: natural-language description and OCR over raw image bytes.
/// Fetching (with the [`MAX_IMAGE_BYTES`] cap, `tgfile://` resolution, Slack
/// auth) happens before this, in the orchestrator — the analyzer never sees a
/// URL, so it can never be handed one it lacks credentials for.
pub trait ImageUnderstanding {
    /// ≤2 sentences, suitable for inline folding into a chat message.
    fn describe(&self, image: Vec<u8>) -> impl Future<Output = Result<String, VisionError>> + Send;
    /// OCR only — verbatim text found in the image.
    fn extract_text(
        &self,
        image: Vec<u8>,
    ) -> impl Future<Output = Result<String, VisionError>> + Send;
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
    fn describe(
        &self,
        _image: Vec<u8>,
    ) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.calls
            .lock()
            .expect("never poisoned")
            .push(VisionTask::Describe);
        std::future::ready(Ok(self.canned.clone()))
    }

    fn extract_text(
        &self,
        _image: Vec<u8>,
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

    // Valid one-pixel GIF89a. Unlike the compact PNG magic fixture above,
    // this must decode because production normalizes its first frame.
    const GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ];

    fn encoded_pixel(format: ::image::ImageFormat) -> Vec<u8> {
        let mut encoded = Cursor::new(Vec::new());
        ::image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut encoded, format)
            .expect("one-pixel fixture encodes");
        encoded.into_inner()
    }

    fn png_crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let polynomial = 0xedb8_8320 & 0_u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ polynomial;
            }
        }
        !crc
    }

    fn oversized_png() -> Vec<u8> {
        let mut bytes = encoded_pixel(::image::ImageFormat::Png);
        let ihdr = bytes
            .windows(4)
            .position(|window| window == b"IHDR")
            .expect("PNG has IHDR");
        bytes[ihdr + 4..ihdr + 8].copy_from_slice(&(MAX_DECODED_IMAGE_DIMENSION + 1).to_be_bytes());
        let crc = png_crc32(&bytes[ihdr..ihdr + 17]);
        bytes[ihdr + 17..ihdr + 21].copy_from_slice(&crc.to_be_bytes());
        bytes
    }

    fn oversized_jpeg() -> Vec<u8> {
        let mut bytes = encoded_pixel(::image::ImageFormat::Jpeg);
        let sof = bytes
            .windows(2)
            .position(|window| {
                window[0] == 0xff
                    && matches!(
                        window[1],
                        0xc0 | 0xc1
                            | 0xc2
                            | 0xc3
                            | 0xc5
                            | 0xc6
                            | 0xc7
                            | 0xc9
                            | 0xca
                            | 0xcb
                            | 0xcd
                            | 0xce
                            | 0xcf
                    )
            })
            .expect("JPEG has a start-of-frame marker");
        let oversized = u16::try_from(MAX_DECODED_IMAGE_DIMENSION + 1).expect("fits JPEG width");
        bytes[sof + 7..sof + 9].copy_from_slice(&oversized.to_be_bytes());
        bytes
    }

    fn oversized_webp() -> Vec<u8> {
        let mut bytes = encoded_pixel(::image::ImageFormat::WebP);
        let vp8l = bytes
            .windows(4)
            .position(|window| window == b"VP8L")
            .expect("image encoder emits lossless WebP");
        assert_eq!(bytes[vp8l + 8], 0x2f, "VP8L signature");
        let old = u32::from_le_bytes(
            bytes[vp8l + 9..vp8l + 13]
                .try_into()
                .expect("VP8L dimensions"),
        );
        // VP8L packs width-1 into bits 0..13 and height-1 into bits 14..27.
        // Preserve the alpha/version nibble while advertising an oversized
        // width and the original one-pixel height.
        let dimensions = (old & 0xf000_0000) | MAX_DECODED_IMAGE_DIMENSION;
        bytes[vp8l + 9..vp8l + 13].copy_from_slice(&dimensions.to_le_bytes());
        bytes
    }

    fn oversized_gif() -> Vec<u8> {
        let mut bytes = GIF.to_vec();
        let oversized = u16::try_from(MAX_DECODED_IMAGE_DIMENSION + 1).expect("fits GIF width");
        bytes[6..8].copy_from_slice(&oversized.to_le_bytes());
        bytes
    }

    async fn rejected_before_transport(bytes: &[u8]) -> VisionError {
        let vision = RemoteVision {
            config: VisionConfig {
                base_url: "http://127.0.0.1:8080/v1".into(),
                model: "gemma4".into(),
                api_key: String::new(),
            },
            transport: RecordingTransport {
                canned: r#"{"choices":[{"message":{"content":"must not run"}}]}"#.into(),
                recorded: std::sync::Mutex::new(None),
            },
        };
        let error = vision
            .describe(bytes.to_vec())
            .await
            .expect_err("unsafe image must fail locally");
        assert!(
            vision
                .transport
                .recorded
                .lock()
                .expect("never poisoned")
                .is_none(),
            "unsafe image reached the provider"
        );
        error
    }

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
    fn data_urls_use_standard_padded_base64() {
        assert_eq!(
            data_url_unchecked(&[0xFF, 0xD8, 0xFF]),
            "data:image/jpeg;base64,/9j/"
        );
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
            data_url_unchecked(PNG),
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
        assert_eq!(url, data_url_unchecked(PNG));
    }

    #[test]
    fn blank_key_sends_no_authorization_header() {
        let req = build_vision_request(
            "http://127.0.0.1:8080/v1",
            "m",
            "  ",
            VisionTask::ExtractText,
            data_url_unchecked(PNG),
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
        let payload = data_url_unchecked(PNG);
        let req = build_vision_request(
            "https://x/v1",
            "m",
            SECRET,
            VisionTask::Describe,
            payload.clone(),
        );
        let shown = format!("{req:?}");
        assert!(!shown.contains(SECRET), "request leaked the key: {shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("https://x/v1/chat/completions"), "{shown}");
        assert!(!shown.contains(&payload), "image dumped: {shown}");

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
        assert_eq!(
            VisionConfig::from_values(None, None, None, None, None),
            None
        );
        assert_eq!(
            VisionConfig::from_values(
                Some("off".into()),
                None,
                None,
                Some("http://127.0.0.1:11434".into()),
                Some("gemma4:12b".into())
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
                Some("".into()),
                Some("".into())
            ),
            None
        );
        // Only the LLM endpoint: reused with /v1 appended and the default model.
        let fallback = VisionConfig::from_values(
            None,
            None,
            None,
            Some("http://127.0.0.1:8080/".into()),
            Some(" mlx-community/gemma-4-12B-it-4bit ".into()),
        )
        .expect("falls back");
        assert_eq!(fallback.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(fallback.model, "mlx-community/gemma-4-12B-it-4bit");
        assert_eq!(fallback.api_key, "");
        let unnamed_fallback = VisionConfig::from_values(
            None,
            None,
            None,
            Some("http://127.0.0.1:11434".into()),
            None,
        )
        .expect("falls back with shared default");
        assert_eq!(unnamed_fallback.model, crate::llm::DEFAULT_LOCAL_MODEL);
        // Explicit vision endpoint wins over the LLM one, verbatim.
        let explicit = VisionConfig::from_values(
            Some("https://api.openai.com/v1".into()),
            Some("gpt-4o".into()),
            Some("sk".into()),
            Some("http://127.0.0.1:8080".into()),
            Some("gemma4:12b".into()),
        )
        .expect("selects");
        assert_eq!(explicit.base_url, "https://api.openai.com/v1");
        assert_eq!(explicit.model, "gpt-4o");
        assert_eq!(explicit.api_key, "sk");

        let default_remote = VisionConfig::from_values(
            Some("https://api.openai.com/v1".into()),
            None,
            None,
            Some("http://127.0.0.1:11434".into()),
            Some("gemma4:12b".into()),
        )
        .expect("explicit remote endpoint");
        assert_eq!(default_remote.model, DEFAULT_REMOTE_MODEL);
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
        let png = encoded_pixel(::image::ImageFormat::Png);
        assert_eq!(vision.describe(png.clone()).await.expect("ok"), "two ducks");
        let sent = vision
            .transport
            .recorded
            .lock()
            .expect("never poisoned")
            .clone()
            .expect("posted");
        assert_eq!(sent.url, "http://127.0.0.1:8080/v1/chat/completions");
        assert_eq!(sent.body["model"], "llava");

        assert_eq!(vision.extract_text(png).await.expect("ok"), "two ducks");
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
        let error = vision.describe(huge).await.expect_err("over the cap");
        assert_eq!(error.public_message(), Some(OVERSIZED_IMAGE_PUBLIC));
    }

    #[tokio::test]
    async fn gif_is_normalized_to_png_before_transport() {
        let vision = RemoteVision {
            config: VisionConfig {
                base_url: "http://127.0.0.1:8080/v1".into(),
                model: "gemma4".into(),
                api_key: String::new(),
            },
            transport: RecordingTransport {
                canned: r#"{"choices":[{"message":{"content":"one pixel"}}]}"#.into(),
                recorded: std::sync::Mutex::new(None),
            },
        };

        assert_eq!(
            vision.describe(GIF.to_vec()).await.expect("GIF decodes"),
            "one pixel"
        );
        let sent = vision
            .transport
            .recorded
            .lock()
            .expect("never poisoned")
            .clone()
            .expect("posted");
        let url = sent.body["messages"][0]["content"][1]["image_url"]["url"]
            .as_str()
            .expect("data URL");
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert!(!url.starts_with("data:image/gif;base64,"), "{url}");
    }

    #[tokio::test]
    async fn valid_passthrough_formats_are_decoded_then_keep_their_mime() {
        for (format, mime) in [
            (::image::ImageFormat::Jpeg, "image/jpeg"),
            (::image::ImageFormat::Png, "image/png"),
            (::image::ImageFormat::WebP, "image/webp"),
        ] {
            let bytes = encoded_pixel(format);
            let vision = RemoteVision {
                config: VisionConfig {
                    base_url: "http://127.0.0.1:8080/v1".into(),
                    model: "gemma4".into(),
                    api_key: String::new(),
                },
                transport: RecordingTransport {
                    canned: r#"{"choices":[{"message":{"content":"one pixel"}}]}"#.into(),
                    recorded: std::sync::Mutex::new(None),
                },
            };
            assert_eq!(
                vision.describe(bytes).await.expect("image decodes"),
                "one pixel"
            );
            let sent = vision
                .transport
                .recorded
                .lock()
                .expect("never poisoned")
                .clone()
                .expect("posted");
            let url = sent.body["messages"][0]["content"][1]["image_url"]["url"]
                .as_str()
                .expect("data URL");
            assert!(url.starts_with(&format!("data:{mime};base64,")), "{url}");
        }
    }

    #[tokio::test]
    async fn malformed_supported_formats_are_rejected_before_transport() {
        for bytes in [
            &[0xff, 0xd8, 0xff][..],
            &[0x89, 0x50, 0x4e, 0x47][..],
            b"RIFF\x04\0\0\0WEBPbad".as_slice(),
            b"GIF89a".as_slice(),
        ] {
            let error = rejected_before_transport(bytes).await;
            assert_eq!(error.public_message(), Some(INVALID_IMAGE_PUBLIC));
            assert!(error.to_string().contains("decode failed"), "{error}");
        }
    }

    #[tokio::test]
    async fn oversized_canvases_in_every_supported_format_are_rejected_locally() {
        for bytes in [
            oversized_jpeg(),
            oversized_png(),
            oversized_webp(),
            oversized_gif(),
        ] {
            let error = rejected_before_transport(&bytes).await;
            assert_eq!(error.public_message(), Some(INVALID_IMAGE_PUBLIC));
            assert!(
                error.to_string().contains("local resource limits"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn unknown_format_is_rejected_before_transport() {
        let vision = RemoteVision {
            config: VisionConfig {
                base_url: "http://127.0.0.1:8080/v1".into(),
                model: "gemma4".into(),
                api_key: String::new(),
            },
            transport: RecordingTransport {
                canned: r#"{"choices":[{"message":{"content":"must not run"}}]}"#.into(),
                recorded: std::sync::Mutex::new(None),
            },
        };

        let error = vision
            .describe(b"not an image".to_vec())
            .await
            .expect_err("unknown bytes must fail locally");
        assert_eq!(error.public_message(), Some(UNSUPPORTED_IMAGE_PUBLIC));
        assert!(
            vision
                .transport
                .recorded
                .lock()
                .expect("never poisoned")
                .is_none(),
            "invalid input reached the provider"
        );
    }

    #[tokio::test]
    async fn recording_vision_returns_canned_text_and_records_tasks() {
        let fake = RecordingVision::returning("a sunset");
        assert_eq!(fake.describe(PNG.to_vec()).await.expect("ok"), "a sunset");
        assert_eq!(
            fake.extract_text(PNG.to_vec()).await.expect("ok"),
            "a sunset"
        );
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
