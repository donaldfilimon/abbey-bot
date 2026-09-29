//! Environment-to-provider assembly and the FM qualification boundary.

use std::path::{Path, PathBuf};

use crate::llm::Backend;
use crate::provider::{
    FmConfig, FmMode, FmQualificationState, FmRoute, FoundationModels, VerifiedFmCapabilities,
    qualify_fm, verify_fm_manifest,
};
use crate::vision::{ConfiguredVision, FmVision, RemoteVision, VisionConfig, VisionProviderChoice};

use super::{HttpVisionTransport, StartupError};

pub(super) struct ProviderSetup {
    /// One instance per configured FM mode, in route order.
    pub foundation_models: Vec<FoundationModels>,
    pub vision: Option<ConfiguredVision<HttpVisionTransport>>,
}

pub(super) fn from_env(
    backend: Option<&Backend>,
    tools_enabled: bool,
) -> Result<ProviderSetup, StartupError> {
    let vision_choice = VisionProviderChoice::from_env().map_err(StartupError)?;
    let instances = FmRoute::from_env()
        .map_err(StartupError)?
        .map(|route| route.instances)
        .unwrap_or_default();
    let fm_vision = matches!(vision_choice, VisionProviderChoice::FoundationModels);
    let manifest = manifest_path();
    // Text routing degrades per mode; it never refuses to start.
    let foundation_models = instances
        .iter()
        .cloned()
        .map(|config| {
            if !(config.fallback || fm_vision) {
                return FoundationModels::new(config, backend, tools_enabled);
            }
            match load_fm_qualification(manifest.as_deref(), &config) {
                (Some(qualified), _) => {
                    FoundationModels::new_qualified(config, backend, tools_enabled, qualified)
                }
                (None, state) => FoundationModels::new(config, backend, tools_enabled)
                    .with_qualification_state(state),
            }
        })
        .collect();
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
            let config = fm_vision_instance(&instances)?;
            let qualified = fm_vision_qualification(manifest.as_deref(), &config)?;
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

fn manifest_path() -> Option<PathBuf> {
    std::env::var("ABBEY_FM_CAPABILITY_MANIFEST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Per-mode text-routing qualification: an unqualified mode registers
/// unadmitted with its typed state instead of failing startup.
fn load_fm_qualification(
    path: Option<&Path>,
    config: &FmConfig,
) -> (Option<VerifiedFmCapabilities>, FmQualificationState) {
    qualify_fm(path, config)
}

/// FM vision binds to the on-device system mode, never PCC.
pub(super) fn fm_vision_instance(instances: &[FmConfig]) -> Result<FmConfig, StartupError> {
    instances
        .iter()
        .find(|config| config.mode == FmMode::System)
        .cloned()
        .ok_or_else(|| {
            StartupError("ABBEY_VISION_PROVIDER=fm requires system in ABBEY_FM_MODE".into())
        })
}

/// FM vision keeps the strict startup requirement: the system mode must be
/// qualified by a verified manifest.
pub(super) fn fm_vision_qualification(
    path: Option<&Path>,
    config: &FmConfig,
) -> Result<VerifiedFmCapabilities, StartupError> {
    let path = path.ok_or_else(|| {
        StartupError("ABBEY_FM_CAPABILITY_MANIFEST is required when FM vision is enabled".into())
    })?;
    verify_fm_manifest(path, config).map_err(|error| {
        StartupError(format!(
            "ABBEY_VISION_PROVIDER=fm requires a verified FM capability manifest: {error}"
        ))
    })
}
