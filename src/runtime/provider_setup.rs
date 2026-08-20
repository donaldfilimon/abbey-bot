//! Environment-to-provider assembly and the FM qualification boundary.

use std::path::PathBuf;

use crate::llm::Backend;
use crate::provider::{FmConfig, FoundationModels, VerifiedFmCapabilities, verify_fm_manifest};
use crate::vision::{ConfiguredVision, FmVision, RemoteVision, VisionConfig, VisionProviderChoice};

use super::{HttpVisionTransport, StartupError};

pub(super) struct ProviderSetup {
    pub foundation_models: Option<FoundationModels>,
    pub vision: Option<ConfiguredVision<HttpVisionTransport>>,
}

pub(super) fn from_env(
    backend: Option<&Backend>,
    tools_enabled: bool,
) -> Result<ProviderSetup, StartupError> {
    let vision_choice = VisionProviderChoice::from_env().map_err(StartupError)?;
    let fm_config = FmConfig::from_env().map_err(StartupError)?;
    let qualified_fm = load_fm_qualification(
        fm_config.as_ref(),
        fm_config.as_ref().is_some_and(|config| config.fallback)
            || matches!(vision_choice, VisionProviderChoice::FoundationModels),
    )?;
    let foundation_models = fm_config.clone().map(|config| match qualified_fm {
        Some(qualified) => {
            FoundationModels::new_qualified(config, backend, tools_enabled, qualified)
        }
        None => FoundationModels::new(config, backend, tools_enabled),
    });
    let vision = match vision_choice {
        VisionProviderChoice::Off => None,
        VisionProviderChoice::Remote => {
            let vision_config = VisionConfig::from_env();
            if let Some(config) = &vision_config {
                crate::llm::validate_remote_endpoint(&config.base_url, "ABBEY_VISION_ENDPOINT")
                    .map_err(StartupError)?;
            }
            vision_config.map(|config| {
                ConfiguredVision::Remote(RemoteVision {
                    config,
                    transport: HttpVisionTransport::default(),
                })
            })
        }
        VisionProviderChoice::FoundationModels => {
            let config = fm_config.clone().ok_or_else(|| {
                StartupError("ABBEY_VISION_PROVIDER=fm requires ABBEY_FM_MODE=system or pcc".into())
            })?;
            let qualified = qualified_fm.ok_or_else(|| {
                StartupError(
                    "ABBEY_VISION_PROVIDER=fm requires a verified FM capability manifest".into(),
                )
            })?;
            let fm = FoundationModels::new_qualified(config, backend, tools_enabled, qualified);
            Some(ConfiguredVision::FoundationModels(
                FmVision::new(fm).map_err(StartupError)?,
            ))
        }
    };
    Ok(ProviderSetup {
        foundation_models,
        vision,
    })
}

fn load_fm_qualification(
    config: Option<&FmConfig>,
    required: bool,
) -> Result<Option<VerifiedFmCapabilities>, StartupError> {
    if !required {
        return Ok(None);
    }
    let config = config.ok_or_else(|| {
        StartupError("FM qualification was requested without ABBEY_FM_MODE=system or pcc".into())
    })?;
    let path = std::env::var("ABBEY_FM_CAPABILITY_MANIFEST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            StartupError(
                "ABBEY_FM_CAPABILITY_MANIFEST is required when FM fallback or vision is enabled"
                    .into(),
            )
        })?;
    verify_fm_manifest(&path, config)
        .map(Some)
        .map_err(StartupError)
}
