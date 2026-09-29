//! Foundation Models registration (one CLI provider per mode) and per-mode
//! manifest score evidence. System keeps the historical IDs; PCC is
//! `foundation-models-cli-pcc`, qualified by the `foundation-models-pcc` record.
use super::*;

pub(super) const FM_SERVER: &str = "foundation-models-server";
pub(super) const FM_CLI_SYSTEM: &str = "foundation-models-cli";
pub(super) const FM_CLI_PCC: &str = "foundation-models-cli-pcc";

pub(super) const fn cli_id(mode: FmMode) -> &'static str {
    match mode {
        FmMode::System => FM_CLI_SYSTEM,
        FmMode::Pcc => FM_CLI_PCC,
    }
}

const fn locality(mode: FmMode) -> ExecutionLocality {
    match mode {
        FmMode::System => ExecutionLocality::SameHost,
        FmMode::Pcc => ExecutionLocality::PublicRemote,
    }
}

impl ProviderRuntime {
    /// Registers every FM mode in route order. An unqualified or unrouted
    /// mode is registered but never admitted.
    pub(super) fn register_foundation_models(&mut self) {
        for fm in self.fm.clone() {
            let admitted = fm.config.fallback && fm.is_qualified();
            let locality = locality(fm.config.mode);
            let identity = config_identity(format!("{:?}", fm.config).as_bytes());
            if let Some(backend) = fm.server_backend() {
                let id = ProviderId::parse(FM_SERVER).expect("static ID");
                let adapter = Arc::new(HttpAdapter {
                    id: id.clone(),
                    backend,
                    transport: crate::llm::HttpTransport::default(),
                    tools_rejected: std::sync::atomic::AtomicBool::new(false),
                });
                self.register(
                    id,
                    fm.label(),
                    ProviderClass::OsManagedLocal,
                    ProviderCapabilities {
                        vision: false,
                        ocr: false,
                        ..fm.server_capabilities.unwrap_or_default()
                    },
                    locality,
                    ProviderProvenance::QualifiedManifest,
                    admitted && fm.server_capabilities.is_some(),
                    identity.clone(),
                    Some(adapter),
                    None,
                    false,
                    true,
                );
            }
            let id = ProviderId::parse(cli_id(fm.config.mode)).expect("static ID");
            let adapter = Arc::new(FmCliAdapter {
                id: id.clone(),
                fm: fm.clone(),
            });
            self.register(
                id,
                fm.label(),
                ProviderClass::OsManagedLocal,
                ProviderCapabilities {
                    vision: false,
                    ocr: false,
                    ..fm.cli_capabilities
                },
                locality,
                if fm.is_qualified() {
                    ProviderProvenance::QualifiedManifest
                } else {
                    ProviderProvenance::Configuration
                },
                admitted,
                identity,
                Some(adapter),
                None,
                false,
                false,
            );
        }
    }

    /// Loads manifest score evidence for every qualified mode. Each mode is
    /// independent; the first error is returned after the others applied.
    pub fn apply_fm_qualification(&mut self, path: &std::path::Path) -> Result<(), String> {
        let mut first_error = None;
        for (index, fm) in self.fm.clone().into_iter().enumerate() {
            if !fm.is_qualified() {
                continue;
            }
            let mut names = Vec::new();
            if fm.config.endpoint.is_some() {
                names.push(FM_SERVER);
            }
            names.push(cli_id(fm.config.mode));
            // FM vision is built from the first mode only.
            if index == 0 {
                names.push("vision");
            }
            if let Err(error) = self.apply_fm_mode_qualification(path, &fm.config, &names) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn apply_fm_mode_qualification(
        &mut self,
        path: &std::path::Path,
        config: &FmConfig,
        names: &[&str],
    ) -> Result<(), String> {
        let identity = super::super::qualification::fm_manifest_identity(config)?;
        let document = super::super::manifest::read_manifest(path)
            .map_err(|_| "cannot read verified FM score evidence")?;
        let legacy_identity = super::super::qualification::fm_identity(config)?;
        let record_id = ProviderId::parse(super::super::qualification::fm_record_id(config.mode))
            .expect("static ID");
        let locality = locality(config.mode);
        for &name in names {
            let id = ProviderId::parse(name).expect("static ID");
            let Some(entry) = self.entries.get_mut(&id) else {
                continue;
            };
            let descriptor = self.catalog.descriptor(&id).expect("registered descriptor");
            if descriptor.provenance != ProviderProvenance::QualifiedManifest {
                continue;
            }
            let profiles = RequestClass::ALL
                .into_iter()
                .filter(|class| class.supported_by(descriptor.declared_capabilities))
                .filter_map(|class| match &document {
                    super::super::manifest::ManifestDocument::LegacyV1(_) => document
                        .legacy_score_profile(
                            if name == FM_SERVER {
                                super::super::manifest::LegacyScoreRoute::FmServer
                            } else {
                                super::super::manifest::LegacyScoreRoute::FmCli
                            },
                            &legacy_identity,
                            class,
                            locality,
                            super::super::qualification::unix_now(),
                        )
                        .ok(),
                    super::super::manifest::ManifestDocument::V2(manifest) => manifest
                        .exact_qualified_record(
                            &record_id,
                            ProviderClass::OsManagedLocal,
                            &identity,
                            ProviderCapabilities::default(),
                        )
                        .ok()
                        .and_then(|record| record.score_profile(class, locality).ok()),
                })
                .collect();
            entry.qualification_witness = match &document {
                super::super::manifest::ManifestDocument::LegacyV1(report) => {
                    let evidence = if name == FM_SERVER {
                        &report.fm_server
                    } else {
                        &report.fm_cli
                    };
                    Some(super::super::manifest::sha256_bytes(
                        &serde_json::to_vec(&(report.generated_unix_secs, evidence))
                            .map_err(|_| "cannot encode verified qualification witness")?,
                    ))
                }
                super::super::manifest::ManifestDocument::V2(manifest) => manifest
                    .record(&record_id)
                    .and_then(|record| record.qualification_run_nonce.clone()),
            };
            let (generation, completed) = match &document {
                super::super::manifest::ManifestDocument::LegacyV1(report) => {
                    (None, Some(report.generated_unix_secs))
                }
                super::super::manifest::ManifestDocument::V2(manifest) => manifest
                    .record(&record_id)
                    .map(|record| {
                        (
                            record.qualification_generation,
                            record.qualification_completed_unix_secs,
                        )
                    })
                    .unwrap_or((None, None)),
            };
            entry.qualification_generation = generation;
            entry.qualification_completed_unix_secs = completed;
            entry.identity = identity.clone();
            lock(&self.state).router.requalify(
                id,
                identity.clone(),
                profiles,
                RouteAdmission::QUALIFIED,
            );
        }
        Ok(())
    }
}
