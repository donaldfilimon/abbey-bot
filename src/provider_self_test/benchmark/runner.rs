//! Explicit opt-in infrastructure runner; never constructs canonical application state.
use super::*;
use crate::{
    llm::Backend,
    provider::{ProviderConfig, ProviderRuntime},
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};
mod execution;
const USAGE: &str = "usage: abbey-bot --text-benchmark primary|fm-system --installed-artifact ABSOLUTE_PATH --model-sha256 HASH --hardware-sha256 HASH --json | --text-benchmark compare BASELINE.json CANDIDATE.json --json";
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Primary,
    FmSystem,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Options {
    Measure {
        target: Target,
        installed: PathBuf,
        model_sha256: String,
        hardware_sha256: String,
    },
    Compare {
        baseline: PathBuf,
        candidate: PathBuf,
    },
}
pub(crate) fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Options, String> {
    let bad = || USAGE.to_string();
    let mode = args.next().ok_or_else(bad)?;
    if mode == OsStr::new("compare") {
        let baseline = args.next().ok_or_else(bad)?.into();
        let candidate = args.next().ok_or_else(bad)?.into();
        if args.next().as_deref() != Some(OsStr::new("--json")) || args.next().is_some() {
            return Err(bad());
        }
        return Ok(Options::Compare {
            baseline,
            candidate,
        });
    }
    let target = match mode.to_str() {
        Some("primary") => Target::Primary,
        Some("fm-system") => Target::FmSystem,
        _ => return Err(bad()),
    };
    if args.next().as_deref() != Some(OsStr::new("--installed-artifact")) {
        return Err(bad());
    }
    let installed = PathBuf::from(args.next().ok_or_else(bad)?);
    if !installed.is_absolute() || args.next().as_deref() != Some(OsStr::new("--model-sha256")) {
        return Err(bad());
    }
    let model_sha256 = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or_else(bad)?;
    if !aggregate::hash_valid(&model_sha256)
        || args.next().as_deref() != Some(OsStr::new("--hardware-sha256"))
    {
        return Err(bad());
    }
    let hardware_sha256 = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or_else(bad)?;
    if !aggregate::hash_valid(&hardware_sha256)
        || args.next().as_deref() != Some(OsStr::new("--json"))
        || args.next().is_some()
    {
        return Err(bad());
    }
    Ok(Options::Measure {
        target,
        installed,
        model_sha256,
        hardware_sha256,
    })
}
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| {
            [
                char::from(HEX[usize::from(b >> 4)]),
                char::from(HEX[usize::from(b & 15)]),
            ]
        })
        .collect()
}
fn artifact_hash(path: &Path) -> Result<String, &'static str> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| "artifact_unreadable")?;
    if !file
        .metadata()
        .map_err(|_| "artifact_unreadable")?
        .is_file()
    {
        return Err("artifact_not_regular");
    }
    let mut digest = Sha256::new();
    let mut bytes = [0_u8; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|_| "artifact_unreadable")?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(hex(&digest.finalize()))
}
fn assemble(
    target: Target,
) -> Result<(ProviderRuntime, crate::provider::ProviderIdentity, String), &'static str> {
    let tools_enabled =
        !std::env::var("ABBEY_BOT_LLM_TOOLS").is_ok_and(|v| v.trim().eq_ignore_ascii_case("off"));
    let (backend, fm, identity, selected, manifest) = match target {
        Target::Primary => {
            let backend = Backend::from_values(
                None,
                std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
                std::env::var("ABBEY_BOT_LLM_MODEL").ok(),
            )
            .ok_or("primary_not_configured")?;
            if !backend.is_loopback_openai_compatible() || backend.validate().is_err() {
                return Err("primary_requires_loopback");
            }
            let Backend::OpenAiCompatible { endpoint, model } = &backend else {
                return Err("primary_requires_loopback");
            };
            let identity = crate::provider::primary_identity(endpoint.clone(), model.clone())
                .map_err(|_| "identity_unavailable")?;
            (Some(backend), Vec::new(), identity, "primary", None)
        }
        Target::FmSystem => {
            let config = crate::provider::FmRoute::from_env()
                .map_err(|_| "invalid_fm_configuration")?
                .and_then(|r| {
                    r.instances
                        .into_iter()
                        .find(|c| c.mode == crate::provider::FmMode::System)
                })
                .ok_or("fm_system_not_configured")?;
            if !config.fallback {
                return Err("fm_system_not_admitted");
            }
            let manifest = std::env::var_os("ABBEY_FM_CAPABILITY_MANIFEST")
                .map(PathBuf::from)
                .ok_or("fm_qualification_missing")?;
            let (qualified, _) = crate::provider::qualify_fm(Some(&manifest), &config);
            let qualified = qualified.ok_or("fm_qualification_refused")?;
            let identity =
                crate::provider::fm_identity(&config).map_err(|_| "identity_unavailable")?;
            let fm = crate::provider::FoundationModels::new_qualified(
                config,
                None,
                tools_enabled,
                qualified,
            );
            (
                None,
                vec![fm],
                identity,
                "foundation-models-cli",
                Some(manifest),
            )
        }
    };
    let mut runtime = ProviderRuntime::legacy(
        backend.clone(),
        None,
        fm,
        None,
        tools_enabled,
        crate::runtime::concurrency_from_env(backend.as_ref()),
        crate::runtime::queue_secs_from_value(std::env::var("ABBEY_BOT_LLM_QUEUE_SECS").ok()),
    );
    let mut policy = ProviderConfig::from_iter(std::env::vars_os())
        .map_err(|_| "invalid_provider_configuration")?;
    policy.local_only = true;
    policy
        .disabled
        .insert(crate::provider::ProviderId::parse("foundation-models-server").expect("static ID"));
    runtime.apply_configuration(policy);
    if let Some(manifest) = manifest {
        runtime
            .apply_fm_qualification(&manifest)
            .map_err(|_| "fm_qualification_refused")?;
    }
    Ok((runtime, identity, selected.into()))
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Gap {
    ManagedServiceIdentityUnwitnessed,
    ModelAndHardwareOperatorDeclared,
    DiscordDeliveryUnobserved,
    VoiceContentionUnmeasured,
    NotCapabilityQualification,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    receipt: Receipt,
    summary: Summary,
    qualification_gaps: Vec<Gap>,
    stage_origins: StageOrigins,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StageOrigins {
    queue_wait: QueueOrigin,
    provider_first_text: FirstTextOrigin,
    provider_completed: CompletedOrigin,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum QueueOrigin {
    InitialReservationToAdmission,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FirstTextOrigin {
    FirstNonemptyDeltaSinceItsRoundAdapterStart,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CompletedOrigin {
    FirstAdapterStartToValidatedConversationResult,
}
impl StageOrigins {
    fn current() -> Self {
        Self {
            queue_wait: QueueOrigin::InitialReservationToAdmission,
            provider_first_text: FirstTextOrigin::FirstNonemptyDeltaSinceItsRoundAdapterStart,
            provider_completed: CompletedOrigin::FirstAdapterStartToValidatedConversationResult,
        }
    }
}
fn read_report(path: &Path) -> Result<Receipt, &'static str> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "receipt_unreadable")?;
    if !file.metadata().map_err(|_| "receipt_unreadable")?.is_file() {
        return Err("receipt_not_regular");
    }
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "receipt_unreadable")?;
    if bytes.len() > 256 * 1024 {
        return Err("receipt_too_large");
    }
    let report: Report = serde_json::from_slice(&bytes).map_err(|_| "invalid_receipt")?;
    if report.summary != aggregate::summarize(&report.receipt.probes, report.receipt.population) {
        return Err("invalid_summary");
    }
    aggregate::validate(&report.receipt)?;
    Ok(report.receipt)
}
pub(crate) async fn run(options: Options) -> Result<i32, crate::Error> {
    let failure = |reason: &'static str| -> crate::Error {
        crate::runtime::StartupError(reason.into()).into()
    };
    match options {
        Options::Compare {
            baseline,
            candidate,
        } => {
            let baseline = read_report(&baseline).map_err(failure)?;
            let candidate = read_report(&candidate).map_err(failure)?;
            let result = aggregate::compare(&baseline, &candidate);
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"population_stage_non_regression": result.is_ok(), "population": candidate.population, "acceptance": "population_only_full_text_reliability_unqualified", "reason": result.err(), "baseline_artifact_sha256": baseline.identity.artifact_sha256, "candidate_artifact_sha256": candidate.identity.artifact_sha256, "artifact_changed": baseline.identity.artifact_sha256 != candidate.identity.artifact_sha256, "qualification": "not_capability_qualification"})
                )?
            );
            Ok(if result.is_ok() { 0 } else { 1 })
        }
        Options::Measure {
            target,
            installed,
            model_sha256,
            hardware_sha256,
        } => {
            let measurement_mode = match target {
                Target::Primary => MeasurementMode::PrimaryStreaming,
                Target::FmSystem => MeasurementMode::FmSystemNonStreaming,
            };
            let (runtime, provider_identity, selected) = assemble(target).map_err(failure)?;
            let declared_installed_sha256 = artifact_hash(&installed).map_err(failure)?;
            if declared_installed_sha256 != provider_identity.abbey_binary_sha256 {
                return Err(failure("installed_artifact_mismatch"));
            }
            let identity = Identity {
                artifact_sha256: provider_identity.abbey_binary_sha256.clone(),
                declared_installed_sha256,
                provider: crate::provider::ProviderId::parse(&selected).expect("static provider"),
                provider_config_sha256: hash(&serde_json::to_vec(
                    &serde_json::json!({"endpoint": provider_identity.endpoint, "model": provider_identity.model, "cli_sha256": provider_identity.cli_sha256, "mode": provider_identity.mode}),
                )?),
                model_sha256,
                hardware_sha256,
                os_sha256: hash(provider_identity.os_build.as_bytes()),
                workload: WORKLOAD.into(),
            };
            let cancel = tokio_util::sync::CancellationToken::new();
            let work = execution::measure(&runtime, cancel.clone(), measurement_mode);
            tokio::pin!(work);
            let probes = tokio::select! {
                result = &mut work => result,
                _ = tokio::signal::ctrl_c() => { cancel.cancel(); work.await },
            };
            let summary = aggregate::summarize(&probes, Population::SyntheticProvider48);
            let passed = probes.len() == 48 && probes.iter().all(|p| p.outcome == Outcome::Success);
            let report = Report {
                stage_origins: StageOrigins::current(),
                receipt: Receipt {
                    version: 1,
                    population: Population::SyntheticProvider48,
                    measurement_mode,
                    identity,
                    probes,
                },
                summary,
                qualification_gaps: vec![
                    Gap::ManagedServiceIdentityUnwitnessed,
                    Gap::ModelAndHardwareOperatorDeclared,
                    Gap::DiscordDeliveryUnobserved,
                    Gap::VoiceContentionUnmeasured,
                    Gap::NotCapabilityQualification,
                ],
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(if passed { 0 } else { 1 })
        }
    }
}
#[cfg(test)]
mod tests;
