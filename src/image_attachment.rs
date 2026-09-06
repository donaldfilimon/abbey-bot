//! Serenity-free selection of the first decoded Discord attachment image.

use std::future::Future;

use crate::vision;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAttachment {
    pub filename: String,
    pub url: String,
    pub declared_size: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Image { filename: String, bytes: Vec<u8> },
    NoSupportedImage,
    FetchFailed(String),
}

pub trait AttachmentFetcher {
    fn fetch<'a>(
        &'a self,
        attachment: &'a ResolvedAttachment,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send + 'a;
}

/// Inspect only the supplied resolved attachments, in their original order.
/// Names and claimed media types are deliberately absent from the decision.
pub async fn select_first_supported(
    attachments: &[ResolvedAttachment],
    fetcher: &impl AttachmentFetcher,
) -> Selection {
    for attachment in attachments {
        if attachment.declared_size > vision::MAX_IMAGE_BYTES as u64 {
            return Selection::FetchFailed(format!(
                "that image is {} bytes; the cap is {}",
                attachment.declared_size,
                vision::MAX_IMAGE_BYTES
            ));
        }
        let bytes = match fetcher.fetch(attachment).await {
            Ok(bytes) => bytes,
            Err(error) => return Selection::FetchFailed(error),
        };
        match vision::image::prepare_file_bytes(bytes).await {
            Ok(prepared) => {
                return Selection::Image {
                    filename: attachment.filename.clone(),
                    bytes: prepared.bytes,
                };
            }
            Err(error)
                if error.public_message() == Some(vision::image::UNSUPPORTED_IMAGE_PUBLIC)
                    || error.public_message() == Some(vision::image::INVALID_IMAGE_PUBLIC) => {}
            Err(error) => {
                return Selection::FetchFailed(
                    error
                        .public_message()
                        .unwrap_or("image decoding failed")
                        .to_string(),
                );
            }
        }
    }
    Selection::NoSupportedImage
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;

    use super::*;

    struct FixtureFetcher(HashMap<String, Result<Vec<u8>, String>>);
    impl AttachmentFetcher for FixtureFetcher {
        async fn fetch(&self, attachment: &ResolvedAttachment) -> Result<Vec<u8>, String> {
            self.0.get(&attachment.url).cloned().unwrap()
        }
    }

    fn encoded(format: ::image::ImageFormat) -> Vec<u8> {
        let image = ::image::DynamicImage::new_rgb8(2, 2);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    fn attachment(name: &str, url: &str) -> ResolvedAttachment {
        ResolvedAttachment {
            filename: name.into(),
            url: url.into(),
            declared_size: 100,
        }
    }

    #[tokio::test]
    async fn accepts_all_four_formats_by_decoded_content() {
        for format in [
            ::image::ImageFormat::Jpeg,
            ::image::ImageFormat::Png,
            ::image::ImageFormat::WebP,
            ::image::ImageFormat::Gif,
        ] {
            let fetcher = FixtureFetcher(HashMap::from([("u".into(), Ok(encoded(format)))]));
            assert!(matches!(
                select_first_supported(&[attachment("wrong.txt", "u")], &fetcher).await,
                Selection::Image { .. }
            ));
        }
    }

    #[tokio::test]
    async fn ignores_claims_and_uses_first_real_image_in_attachment_order() {
        let second = encoded(::image::ImageFormat::Png);
        let third = encoded(::image::ImageFormat::Jpeg);
        let fetcher = FixtureFetcher(HashMap::from([
            ("one".into(), Ok(b"not an image".to_vec())),
            ("two".into(), Ok(second)),
            ("three".into(), Ok(third)),
        ]));
        let selected = select_first_supported(
            &[
                attachment("fake.png", "one"),
                attachment("real.bin", "two"),
                attachment("later.jpg", "three"),
            ],
            &fetcher,
        )
        .await;
        assert!(matches!(selected, Selection::Image { filename, .. } if filename == "real.bin"));
    }

    #[tokio::test]
    async fn reports_size_fetch_and_decoder_absence_without_other_sources() {
        let fetcher = FixtureFetcher(HashMap::from([(
            "bad".into(),
            Err("network stopped".into()),
        )]));
        let mut too_large = attachment("x", "bad");
        too_large.declared_size = vision::MAX_IMAGE_BYTES as u64 + 1;
        assert!(matches!(
            select_first_supported(&[too_large], &fetcher).await,
            Selection::FetchFailed(_)
        ));
        assert_eq!(
            select_first_supported(&[attachment("x", "bad")], &fetcher).await,
            Selection::FetchFailed("network stopped".into())
        );
        let corrupt = FixtureFetcher(HashMap::from([("bad".into(), Ok(b"GIFbroken".to_vec()))]));
        assert_eq!(
            select_first_supported(&[attachment("x.gif", "bad")], &corrupt).await,
            Selection::NoSupportedImage
        );
        assert_eq!(
            select_first_supported(&[], &corrupt).await,
            Selection::NoSupportedImage
        );
    }
}
