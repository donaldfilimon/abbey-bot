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
                    // `fm serve` binds to the on-device system model and
                    // shares its measured window.
                    prompt_budget: (fm.config.mode == FmMode::System)
                        .then(crate::prompt_budget::Budget::fm_system),
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

    /// Loads manifest score evidence for every qualified mode from one read
    /// of the manifest. Each mode is applied atomically: if any of its routes
    /// cannot be rekeyed, every route of that mode is demoted (fail closed),
    /// its state records why, and the other modes still apply. The first
    /// error is returned for logging.
    pub fn apply_fm_qualification(&mut self, path: &std::path::Path) -> Result<(), String> {
        let document = super::super::manifest::read_manifest(path)
            .map_err(super::super::qualification::classify_manifest_error);
        let mut first_error = None;
        for fm in self.fm.clone() {
            if !fm.is_qualified() {
                continue;
            }
            let names = self.fm_route_names(&fm.config);
            let prepared = match &document {
                Ok(document) => self.prepare_fm_mode(document, &fm.config, &names),
                Err(rejection) => Err(rejection.clone()),
            };
            match prepared {
                Ok(updates) => self.commit_fm_mode(updates),
                Err((state, message)) => {
                    for name in &names {
                        self.demote_route(&ProviderId::parse(name).expect("static ID"));
                    }
                    fm.demote(state);
                    first_error.get_or_insert(format!(
                        "FM {} demoted: {message}",
                        fm.config.mode.as_str()
                    ));
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Every route ID backed by this mode: its server (only the system mode
    /// carries an endpoint), its CLI, and FM vision (bound to system).
    fn fm_route_names(&self, config: &FmConfig) -> Vec<&'static str> {
        let mut names = Vec::new();
        if config.endpoint.is_some() {
            names.push(FM_SERVER);
        }
        names.push(cli_id(config.mode));
        if config.mode == FmMode::System {
            names.push("vision");
        }
        names
            .into_iter()
            .filter(|name| {
                let id = ProviderId::parse(name).expect("static ID");
                self.entries.contains_key(&id)
                    && self.catalog.descriptor(&id).is_some_and(|descriptor| {
                        descriptor.provenance == ProviderProvenance::QualifiedManifest
                    })
            })
            .collect()
    }

    fn demote_route(&mut self, id: &ProviderId) {
        if let Some(descriptor) = self.catalog.descriptor(id).cloned() {
            self.catalog.register_runtime(ProviderDescriptor {
                eligibility: Eligibility::Blocked(BlockedReason::Unqualified),
                ..descriptor
            });
        }
        lock(&self.state).router.set_admission(
            id,
            RouteAdmission {
                identity_current: false,
                ..RouteAdmission::QUALIFIED
            },
        );
    }

    /// Computes every route's rekeyed evidence without mutating anything.
    fn prepare_fm_mode(
        &self,
        document: &super::super::manifest::ManifestDocument,
        config: &FmConfig,
        names: &[&'static str],
    ) -> Result<Vec<FmRouteUpdate>, super::super::qualification::FmRejection> {
        use super::super::manifest::{LegacyScoreRoute, ManifestDocument};
        let refused = |message: String| (FmQualificationState::Refused(message.clone()), message);
        let identity =
            super::super::qualification::fm_manifest_identity(config).map_err(refused)?;
        let legacy_identity = super::super::qualification::fm_identity(config).map_err(refused)?;
        let record_id = ProviderId::parse(super::super::qualification::fm_record_id(config.mode))
            .expect("static ID");
        let locality = locality(config.mode);
        let mut updates = Vec::with_capacity(names.len());
        for &name in names {
            let id = ProviderId::parse(name).expect("static ID");
            let descriptor = self.catalog.descriptor(&id).expect("registered descriptor");
            let profiles = RequestClass::ALL
                .into_iter()
                .filter(|class| class.supported_by(descriptor.declared_capabilities))
                .filter_map(|class| match document {
                    ManifestDocument::LegacyV1(_) => document
                        .legacy_score_profile(
                            if name == FM_SERVER {
                                LegacyScoreRoute::FmServer
                            } else {
                                LegacyScoreRoute::FmCli
                            },
                            &legacy_identity,
                            class,
                            locality,
                            super::super::qualification::unix_now(),
                        )
                        .ok(),
                    ManifestDocument::V2(manifest) => manifest
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
            let witness = match document {
                ManifestDocument::LegacyV1(report) => {
                    let evidence = if name == FM_SERVER {
                        &report.fm_server
                    } else {
                        &report.fm_cli
                    };
                    Some(super::super::manifest::sha256_bytes(
                        &serde_json::to_vec(&(report.generated_unix_secs, evidence)).map_err(
                            |_| refused("cannot encode verified qualification witness".into()),
                        )?,
                    ))
                }
                ManifestDocument::V2(manifest) => manifest
                    .record(&record_id)
                    .and_then(|record| record.qualification_run_nonce.clone()),
            };
            let (generation, completed) = match document {
                ManifestDocument::LegacyV1(report) => (None, Some(report.generated_unix_secs)),
                ManifestDocument::V2(manifest) => manifest
                    .record(&record_id)
                    .map(|record| {
                        (
                            record.qualification_generation,
                            record.qualification_completed_unix_secs,
                        )
                    })
                    .unwrap_or((None, None)),
            };
            updates.push(FmRouteUpdate {
                id,
                identity: identity.clone(),
                profiles,
                witness,
                generation,
                completed,
            });
        }
        Ok(updates)
    }

    fn commit_fm_mode(&mut self, updates: Vec<FmRouteUpdate>) {
        for update in updates {
            let entry = self
                .entries
                .get_mut(&update.id)
                .expect("prepared route is registered");
            entry.qualification_witness = update.witness;
            entry.qualification_generation = update.generation;
            entry.qualification_completed_unix_secs = update.completed;
            entry.identity = update.identity.clone();
            lock(&self.state).router.requalify(
                update.id,
                update.identity,
                update.profiles,
                RouteAdmission::QUALIFIED,
            );
        }
    }
}

struct FmRouteUpdate {
    id: ProviderId,
    identity: ProviderIdentityHashes,
    profiles: Vec<super::super::scoring::ProviderScoreProfile>,
    witness: Option<String>,
    generation: Option<u64>,
    completed: Option<u64>,
}
