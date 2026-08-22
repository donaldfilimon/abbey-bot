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

mod foundation_models;
pub(crate) mod image;
mod provider;
mod render;

pub use foundation_models::FmVision;
pub use provider::{RemoteVision, VisionConfig, VisionRequest, VisionTransport};
pub(crate) use provider::{VisionTask, extract_vision_text};
pub use render::{fold_descriptions, render_ocr, render_see};

#[cfg(test)]
use std::io::Cursor;

#[cfg(test)]
use image::{
    INVALID_IMAGE_PUBLIC, MAX_DECODED_IMAGE_DIMENSION, OVERSIZED_IMAGE_PUBLIC,
    UNSUPPORTED_IMAGE_PUBLIC, data_url_unchecked, sniff_mime,
};

#[cfg(test)]
use provider::{DEFAULT_REMOTE_MODEL, build_vision_request};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisionProviderChoice {
    Remote,
    FoundationModels,
    Off,
}

impl VisionProviderChoice {
    pub fn from_value(value: Option<String>) -> Result<Self, String> {
        match value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            None | Some("remote") => Ok(Self::Remote),
            Some("fm") => Ok(Self::FoundationModels),
            Some("off") => Ok(Self::Off),
            Some(_) => Err("ABBEY_VISION_PROVIDER must be remote, fm, or off".into()),
        }
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_value(std::env::var("ABBEY_VISION_PROVIDER").ok())
    }
}

pub enum ConfiguredVision<T> {
    Remote(RemoteVision<T>),
    FoundationModels(FmVision),
}

impl<T: VisionTransport + Sync> ImageUnderstanding for ConfiguredVision<T> {
    async fn describe(&self, image: Vec<u8>) -> Result<String, VisionError> {
        match self {
            Self::Remote(provider) => provider.describe(image).await,
            Self::FoundationModels(provider) => provider.describe(image).await,
        }
    }

    async fn extract_text(&self, image: Vec<u8>) -> Result<String, VisionError> {
        match self {
            Self::Remote(provider) => provider.extract_text(image).await,
            Self::FoundationModels(provider) => provider.extract_text(image).await,
        }
    }
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
mod tests;
