//! Bounded image validation and provider data-URL preparation.
//!
//! Attachments are attacker-controlled. Decoding, GIF normalization, and
//! base64 allocation therefore run on Tokio's blocking pool behind a small
//! semaphore and bounded queue wait instead of occupying async runtime workers
//! or creating an unbounded blocking queue.

use std::io::Cursor;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use tokio::sync::Semaphore;

use super::{MAX_IMAGE_BYTES, VisionError};

/// At most two image decodes/encodes may occupy blocking workers at once.
/// A single chat message describes at most three images, so this keeps useful
/// parallelism without letting attachment floods monopolize the process.
const MAX_IMAGE_PROCESSING_JOBS: usize = 2;
/// Short bounded queue window. Ordinary concurrent decodes can hand off a
/// slot, while sustained attacker load receives stable busy copy instead of
/// accumulating blocking-pool work.
const IMAGE_PROCESSING_QUEUE_WAIT: Duration = Duration::from_millis(100);

/// Decode ceiling for every accepted image. The fetched-file cap alone does
/// not stop a tiny compressed image from advertising an enormous canvas.
pub(super) const MAX_DECODED_IMAGE_DIMENSION: u32 = 8_192;

/// Allocation ceiling used by each decoder in addition to the strict dimension
/// ceiling above. `ImageReader::decode` reserves decoded output against this
/// limit before allocating it.
const MAX_DECODED_IMAGE_ALLOC: u64 = 96 << 20;

pub(super) const UNSUPPORTED_IMAGE_PUBLIC: &str = "That attachment is not a supported image. Upload a JPEG, PNG, WebP, or GIF file; convert HEIC, AVIF, JXL, SVG, and PDF files first.";
pub(super) const INVALID_IMAGE_PUBLIC: &str = "I couldn't decode that image safely. Re-export it as a normal JPEG, PNG, WebP, or GIF no larger than 8192 by 8192 pixels and try again.";
pub(super) const OVERSIZED_IMAGE_PUBLIC: &str =
    "That image is too large. Keep the file at or under 10 MB and try again.";
const BUSY_IMAGE_PUBLIC: &str =
    "Image processing is busy right now. Wait a moment and try that image again.";

static IMAGE_PROCESSING: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn global_limiter() -> Arc<Semaphore> {
    Arc::clone(IMAGE_PROCESSING.get_or_init(|| Arc::new(Semaphore::new(MAX_IMAGE_PROCESSING_JOBS))))
}

/// MIME type from the leading magic bytes, exactly the set the spec sniffs.
/// Anything else is rejected before transport.
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

/// Validate/normalize one owned attachment and produce the exact data URL a
/// provider receives. Ownership lets `spawn_blocking` move the original
/// allocation instead of copying up to 10 MiB on an async worker.
pub(super) async fn prepare_data_url(bytes: Vec<u8>) -> Result<String, VisionError> {
    prepare_data_url_with_limiter(bytes, global_limiter()).await
}

pub(super) struct PreparedImage {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

/// Validate and normalize bytes for a file-based provider. The exact same
/// decoder and allocation ceilings protect both the remote and FM transports.
pub(super) async fn prepare_file_bytes(bytes: Vec<u8>) -> Result<PreparedImage, VisionError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(VisionError::invalid_image(
            format!(
                "the image is {} bytes; the cap is {MAX_IMAGE_BYTES}",
                bytes.len()
            ),
            OVERSIZED_IMAGE_PUBLIC,
        ));
    }
    run_bounded_blocking(global_limiter(), move || {
        let prepared = prepare_image(bytes)?;
        let extension = match sniff_mime(&prepared) {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            _ => {
                return Err(VisionError::internal(
                    "the validated image had no provider-safe file extension",
                ));
            }
        };
        Ok(PreparedImage {
            bytes: prepared,
            extension,
        })
    })
    .await
}

async fn prepare_data_url_with_limiter(
    bytes: Vec<u8>,
    limiter: Arc<Semaphore>,
) -> Result<String, VisionError> {
    // Preserve the established error priority: an oversized payload is always
    // reported as oversized, even while all decoder slots are occupied.
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(VisionError::invalid_image(
            format!(
                "the image is {} bytes; the cap is {MAX_IMAGE_BYTES}",
                bytes.len()
            ),
            OVERSIZED_IMAGE_PUBLIC,
        ));
    }
    run_bounded_blocking(limiter, move || {
        let prepared = prepare_image(bytes)?;
        Ok(data_url_unchecked(&prepared))
    })
    .await
}

async fn run_bounded_blocking<R: Send + 'static>(
    limiter: Arc<Semaphore>,
    work: impl FnOnce() -> Result<R, VisionError> + Send + 'static,
) -> Result<R, VisionError> {
    let permit = tokio::time::timeout(IMAGE_PROCESSING_QUEUE_WAIT, limiter.acquire_owned())
        .await
        .map_err(|_| {
            VisionError::invalid_image("the image processing workers are busy", BUSY_IMAGE_PUBLIC)
        })?
        .map_err(|_| VisionError::internal("the image processing gate is closed"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|error| VisionError::internal(format!("the image worker failed: {error}")))?
}

/// Return provider-ready bytes without lying about their media type.
/// Pass-through formats are fully decoded under local limits, while GIF is
/// normalized to its first rendered frame as PNG for Ollama compatibility.
fn prepare_image(bytes: Vec<u8>) -> Result<Vec<u8>, VisionError> {
    match sniff_mime(&bytes) {
        "image/jpeg" => {
            validate_preserved_image(&bytes, ::image::ImageFormat::Jpeg, "JPEG")?;
            Ok(bytes)
        }
        "image/png" => {
            validate_preserved_image(&bytes, ::image::ImageFormat::Png, "PNG")?;
            Ok(bytes)
        }
        "image/webp" => {
            validate_preserved_image(&bytes, ::image::ImageFormat::WebP, "WebP")?;
            Ok(bytes)
        }
        "image/gif" => normalize_gif_first_frame(&bytes),
        _ => Err(VisionError::invalid_image(
            "unsupported attachment image format",
            UNSUPPORTED_IMAGE_PUBLIC,
        )),
    }
}

fn decode_with_limits(
    bytes: &[u8],
    format: ::image::ImageFormat,
    format_label: &'static str,
) -> Result<::image::DynamicImage, VisionError> {
    let mut reader = ::image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_DECODED_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_IMAGE_ALLOC);
    reader.limits(limits);
    reader.decode().map_err(|_| {
        VisionError::invalid_image(
            format!("{format_label} decode failed or exceeded local resource limits"),
            INVALID_IMAGE_PUBLIC,
        )
    })
}

fn validate_preserved_image(
    bytes: &[u8],
    format: ::image::ImageFormat,
    format_label: &'static str,
) -> Result<(), VisionError> {
    drop(decode_with_limits(bytes, format, format_label)?);
    Ok(())
}

fn normalize_gif_first_frame(bytes: &[u8]) -> Result<Vec<u8>, VisionError> {
    let first_frame = decode_with_limits(bytes, ::image::ImageFormat::Gif, "GIF")?;
    let mut encoded = Cursor::new(Vec::new());
    first_frame
        .write_to(&mut encoded, ::image::ImageFormat::Png)
        .map_err(|_| {
            VisionError::invalid_image("GIF-to-PNG encoding failed", INVALID_IMAGE_PUBLIC)
        })?;
    let encoded = encoded.into_inner();
    if encoded.len() > MAX_IMAGE_BYTES {
        return Err(VisionError::invalid_image(
            format!(
                "normalized GIF is {} bytes; the cap is {MAX_IMAGE_BYTES}",
                encoded.len()
            ),
            INVALID_IMAGE_PUBLIC,
        ));
    }
    Ok(encoded)
}

/// Standard RFC 4648 base64 with padding, supplied by the audited dependency
/// already shared with the voice providers.
pub(crate) fn data_url_unchecked(bytes: &[u8]) -> String {
    let mime = sniff_mime(bytes);
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn busy_gate_times_out_and_does_not_run_the_work() {
        let limiter = Arc::new(Semaphore::new(1));
        let _occupied = Arc::clone(&limiter).acquire_owned().await.unwrap();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran_in_work = Arc::clone(&ran);
        let error = run_bounded_blocking(limiter, move || {
            ran_in_work.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .expect_err("occupied gate must refuse work");
        assert_eq!(error.public_message(), Some(BUSY_IMAGE_PUBLIC));
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn image_work_runs_off_the_async_runtime_thread() {
        let caller = std::thread::current().id();
        let worker = run_bounded_blocking(Arc::new(Semaphore::new(1)), || {
            Ok(std::thread::current().id())
        })
        .await
        .expect("blocking work runs");
        assert_ne!(caller, worker);
    }

    #[tokio::test]
    async fn oversized_error_wins_even_when_gate_is_busy() {
        let limiter = Arc::new(Semaphore::new(1));
        let _occupied = Arc::clone(&limiter).acquire_owned().await.unwrap();
        let error = prepare_data_url_with_limiter(vec![0; MAX_IMAGE_BYTES + 1], limiter)
            .await
            .expect_err("oversized input must fail");
        assert_eq!(error.public_message(), Some(OVERSIZED_IMAGE_PUBLIC));
    }
}
