//! Provider runtime contracts plus the compatible Foundation Models route.
//!
//! Generic provider configuration, discovery, qualification, and routing stay
//! behind Abbey's existing turn and tool vocabulary. Foundation Models is
//! never selected merely because `/usr/bin/fm` or a server happens to exist.
//! The operator must select `system` or `pcc` and separately enable fallback.
//! The HTTP server and `fm respond` CLI remain separate capabilities: the
//! server is text-only here and can never inherit CLI tool capability.

use std::path::{Component, PathBuf};

use serde::{Deserialize, Serialize};

use crate::llm::Backend;

mod adapters;
mod runtime;
pub(crate) use runtime::BlockWriter;
pub use runtime::{ConversationEffects, ProviderConversation, ProviderRuntime};
mod catalog;
mod circuit;
mod config;
#[cfg(test)]
mod discovery;
mod domain;
mod foundation_models;
mod manifest;
mod manifest_scores;
mod qualification;
mod routing;
#[cfg(test)]
mod score_fixtures;
mod scoring;

pub use catalog::ProviderCatalog;
pub use circuit::{CircuitPhase, ProviderFailureKind, RetryAfter};
pub use config::ProviderConfig;
pub use domain::{
    BlockedReason, DetectionState, DiscoveryBoundary, Eligibility, IsolationCapabilities,
    ProviderClass, ProviderDescriptor, ProviderId, ProviderProvenance, TurnAdapter, TurnFuture,
};
pub use foundation_models::FoundationModels;
pub(crate) use foundation_models::parse_cli_output;
#[cfg(test)]
use foundation_models::{
    CliInvocation, PrivateImageFile, PrivateSchemaFile, decision_schema, filtered_environment,
    render_transcript,
};
pub use manifest::ProviderIdentityHashes;
#[cfg(test)]
pub use manifest::{
    DeclaredCapabilities, PROVIDER_MANIFEST_VERSION, ProviderManifest, QualifiedIsolation,
};
#[cfg(all(test, unix))]
pub use manifest::{ProviderRecord, publish_v2};
pub use qualification::{
    CapabilityEvidence, CapabilityEvidenceSet, FIXTURE_VERSION, ProbeStatus, ProviderEvidence,
    ProviderIdentity, QUALIFICATION_VERSION, QualificationReport, QualificationTarget,
    VerifiedFmCapabilities, fm_identity, primary_identity, unix_now, verify_fm_manifest,
};
pub use routing::{
    AdaptiveRouter, ConversationRoute, RouteAdmission, RouteAttempt, RouteUnavailableReason,
};
pub use scoring::{ExecutionLocality, RequestClass, ScoreProducerPolicy};

const DEFAULT_FM_CLI: &str = "/usr/bin/fm";
const DEFAULT_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmImageTask {
    Describe,
    ExtractText,
    QualificationShapes,
    QualificationOcr,
}

impl FmImageTask {
    const fn prompt(self) -> &'static str {
        match self {
            Self::Describe => {
                "Describe this image in at most two short sentences. Factual, no preamble."
            }
            Self::ExtractText => {
                "Transcribe all text visible in this image verbatim. Output only the text."
            }
            Self::QualificationShapes => {
                "Identify the two colored shapes from left to right. Output only two lowercase color-and-shape labels separated by a comma and one space."
            }
            Self::QualificationOcr => {
                "Transcribe the image. Output exactly the visible ASCII text and nothing else."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FmMode {
    System,
    Pcc,
}

impl FmMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Pcc => "pcc",
        }
    }
}

/// Validated operator configuration. This type contains no credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FmConfig {
    pub mode: FmMode,
    pub endpoint: Option<String>,
    pub cli: PathBuf,
    pub fallback: bool,
    pub timeout_secs: u64,
}

impl FmConfig {
    pub fn from_values(
        mode: Option<String>,
        endpoint: Option<String>,
        cli: Option<String>,
        fallback: Option<String>,
        timeout_secs: Option<String>,
    ) -> Result<Option<Self>, String> {
        let value = |raw: Option<String>| {
            raw.map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let fallback = match value(fallback).as_deref() {
            None | Some("0" | "false" | "off") => false,
            Some("1" | "true" | "on") => true,
            Some(_) => return Err("ABBEY_FM_FALLBACK must be 1 or 0".into()),
        };
        let mode = match value(mode).as_deref() {
            None | Some("off") => {
                if fallback {
                    return Err("ABBEY_FM_FALLBACK=1 requires ABBEY_FM_MODE=system or pcc".into());
                }
                return Ok(None);
            }
            Some("system") => FmMode::System,
            Some("pcc") => FmMode::Pcc,
            Some(_) => return Err("ABBEY_FM_MODE must be off, system, or pcc".into()),
        };

        let endpoint = value(endpoint)
            .map(|endpoint| validate_fm_endpoint(&endpoint).map(|()| endpoint))
            .transpose()?;
        let cli = PathBuf::from(value(cli).unwrap_or_else(|| DEFAULT_FM_CLI.to_string()));
        if !cli.is_absolute() || cli.components().any(|part| part == Component::ParentDir) {
            return Err("ABBEY_FM_CLI must be an absolute path without `..`".into());
        }
        let timeout_secs = match value(timeout_secs) {
            None => DEFAULT_TIMEOUT_SECS,
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|seconds| *seconds > 0)
                .ok_or_else(|| {
                    "ABBEY_BOT_LLM_TIMEOUT_SECS must be a positive integer for FM".to_string()
                })?,
        };
        Ok(Some(Self {
            mode,
            endpoint,
            cli,
            fallback,
            timeout_secs,
        }))
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        let config = Self::from_values(
            std::env::var("ABBEY_FM_MODE").ok(),
            std::env::var("ABBEY_FM_ENDPOINT").ok(),
            std::env::var("ABBEY_FM_CLI").ok(),
            std::env::var("ABBEY_FM_FALLBACK").ok(),
            std::env::var("ABBEY_BOT_LLM_TIMEOUT_SECS").ok(),
        )?;
        #[cfg(not(target_os = "macos"))]
        if config.is_some() {
            return Err(
                "Apple Foundation Models is supported only on macOS; set ABBEY_FM_MODE=off".into(),
            );
        }
        Ok(config)
    }
}

fn validate_fm_endpoint(raw: &str) -> Result<(), String> {
    crate::llm::validate_remote_endpoint(raw, "ABBEY_FM_ENDPOINT")?;
    let url = reqwest::Url::parse(raw)
        .map_err(|_| "ABBEY_FM_ENDPOINT must be a valid absolute URL".to_string())?;
    if !crate::llm::url_is_loopback(&url) {
        return Err("ABBEY_FM_ENDPOINT must target loopback".into());
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err("ABBEY_FM_ENDPOINT must be a server base URL without a path".into());
    }
    Ok(())
}

/// Independently qualified provider behavior.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub text: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub tools: bool,
    pub vision: bool,
    pub ocr: bool,
}

impl ProviderCapabilities {
    #[must_use]
    pub const fn satisfies(self, required: Self) -> bool {
        (!required.text || self.text)
            && (!required.streaming || self.streaming)
            && (!required.structured_output || self.structured_output)
            && (!required.tools || self.tools)
            && (!required.vision || self.vision)
            && (!required.ocr || self.ocr)
    }

    #[must_use]
    pub const fn primary(backend: &Backend, tools: bool) -> Self {
        Self {
            text: true,
            streaming: matches!(backend, Backend::OpenAiCompatible { .. }),
            structured_output: tools,
            tools,
            vision: false,
            ocr: false,
        }
    }

    #[must_use]
    pub const fn text() -> Self {
        Self {
            text: true,
            streaming: false,
            structured_output: false,
            tools: false,
            vision: false,
            ocr: false,
        }
    }

    #[must_use]
    pub const fn text_with_tools() -> Self {
        Self {
            text: true,
            streaming: false,
            structured_output: true,
            tools: true,
            vision: false,
            ocr: false,
        }
    }
}

#[cfg(test)]
mod tests;
