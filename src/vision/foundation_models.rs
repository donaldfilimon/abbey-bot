//! Qualified file-based Apple Foundation Models vision adapter.

use std::future::Future;

use crate::provider::{FmImageTask, FoundationModels, ProviderRoute};

use super::{ImageUnderstanding, VisionError, image};

pub struct FmVision {
    provider: FoundationModels,
}

impl FmVision {
    pub fn new(provider: FoundationModels) -> Result<Self, String> {
        let capabilities = provider
            .router
            .effective_capabilities(ProviderRoute::FoundationModelsCli)
            .ok_or_else(|| "the FM CLI has no verified capability evidence".to_string())?;
        if !(capabilities.vision && capabilities.ocr) {
            return Err(
                "ABBEY_VISION_PROVIDER=fm requires verified FM vision and OCR capabilities".into(),
            );
        }
        Ok(Self { provider })
    }

    async fn ask(&self, task: FmImageTask, bytes: Vec<u8>) -> Result<String, VisionError> {
        let capabilities = self
            .provider
            .router
            .effective_capabilities(ProviderRoute::FoundationModelsCli)
            .ok_or_else(|| VisionError::internal("FM vision capability is unavailable"))?;
        let allowed = match task {
            FmImageTask::Describe | FmImageTask::QualificationShapes => capabilities.vision,
            FmImageTask::ExtractText | FmImageTask::QualificationOcr => capabilities.ocr,
        };
        if !allowed {
            return Err(VisionError::internal(
                "FM image capability is not present in the verified manifest",
            ));
        }
        let prepared = image::prepare_file_bytes(bytes).await?;
        self.provider
            .image_turn(task, &prepared.bytes, prepared.extension)
            .await
            .map_err(|_| VisionError::internal("the qualified FM image request failed"))
    }
}

impl ImageUnderstanding for FmVision {
    fn describe(&self, image: Vec<u8>) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.ask(FmImageTask::Describe, image)
    }

    fn extract_text(
        &self,
        image: Vec<u8>,
    ) -> impl Future<Output = Result<String, VisionError>> + Send {
        self.ask(FmImageTask::ExtractText, image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{FmConfig, FmMode, ProviderCapabilities, VerifiedFmCapabilities};

    fn provider(capabilities: ProviderCapabilities) -> FoundationModels {
        FoundationModels::new_qualified(
            FmConfig {
                mode: FmMode::System,
                endpoint: None,
                cli: "/usr/bin/fm".into(),
                fallback: true,
                timeout_secs: 30,
            },
            None,
            true,
            VerifiedFmCapabilities {
                server: None,
                cli: capabilities,
            },
        )
    }

    #[test]
    fn fm_vision_requires_both_manifest_derived_image_capabilities() {
        let text_only = ProviderCapabilities::text_with_tools();
        assert!(FmVision::new(provider(text_only)).is_err());

        let fully_qualified = ProviderCapabilities {
            vision: true,
            ocr: true,
            ..text_only
        };
        assert!(FmVision::new(provider(fully_qualified)).is_ok());
    }
}
