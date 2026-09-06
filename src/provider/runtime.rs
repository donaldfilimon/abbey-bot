//! Single execution authority: admission, capacity, circuit and conversation effects.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::adapters::{FmCliAdapter, HttpAdapter};
use super::domain::AdapterRequest;
use super::*;
use crate::llm::{Backend, ChatTurn, LlmError, ModelTurn, ResponseStyle};
use crate::vision::{ConfiguredVision, ImageUnderstanding, VisionError};

mod blocks;
mod conversation;
mod identity;
mod inspection;
use blocks::BlockStore;
pub(crate) use blocks::BlockWriter;
use identity::{backend_identity, backend_locality, config_identity, endpoint_locality};

pub trait ProviderClock: Send + Sync {
    fn now_ms(&self) -> u64;
    fn unix_secs(&self) -> u64 {
        super::qualification::unix_now()
    }
}
struct MonotonicClock(Instant);
impl ProviderClock for MonotonicClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Shared only inside one conversation; never stored in the runtime/router.
#[derive(Clone, Default)]
pub struct ConversationEffects(Arc<Mutex<ConversationRoute>>);
impl ConversationEffects {
    pub fn mark_visible_output(&self) {
        lock(&self.0).mark_visible_output();
    }
    pub fn mark_tool_dispatched(&self) {
        lock(&self.0).mark_tool_dispatched();
    }
    fn mark_image_submitted(&self) {
        lock(&self.0).mark_image_submitted();
    }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

trait VisionAdapter: Send + Sync {
    fn image(
        &self,
        ocr: bool,
        bytes: Vec<u8>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, VisionError>> + Send + '_>>;
}
impl<T: ImageUnderstanding + Send + Sync> VisionAdapter for T {
    fn image(
        &self,
        ocr: bool,
        bytes: Vec<u8>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, VisionError>> + Send + '_>>
    {
        Box::pin(async move {
            if ocr {
                self.extract_text(bytes).await
            } else {
                self.describe(bytes).await
            }
        })
    }
}
struct Entry {
    adapter: Option<Arc<dyn TurnAdapter>>,
    image: Option<Arc<dyn VisionAdapter>>,
    label: &'static str,
    local_voice: bool,
    stream_only: bool,
    slots: Arc<tokio::sync::Semaphore>,
    identity: ProviderIdentityHashes,
    qualification_witness: Option<String>,
    qualification_generation: Option<u64>,
    qualification_completed_unix_secs: Option<u64>,
}
impl Entry {
    fn admission(
        &self,
        descriptor: &ProviderDescriptor,
        class: RequestClass,
        streaming: bool,
        local: bool,
        budget_available: bool,
    ) -> RouteAdmission {
        let supported_adapter = match class {
            RequestClass::VisionDescribe | RequestClass::VisionOcr => self.image.is_some(),
            _ => self.adapter.is_some(),
        };
        RouteAdmission {
            configured: self.adapter.is_some() || self.image.is_some(),
            identity_current: descriptor.eligibility.is_routable(),
            capability_allowed: supported_adapter
                && class.supported_by(descriptor.declared_capabilities)
                && (!self.stream_only || (streaming && class == RequestClass::TextReadOnly)),
            policy_allowed: !local || self.local_voice,
            budget_available,
            capacity_available: self.slots.available_permits() > 0,
        }
    }
}
struct OperationalState {
    router: AdaptiveRouter,
    blocks: BlockStore,
}

pub struct ProviderRuntime {
    operational_events: std::sync::OnceLock<crate::service::telemetry::TelemetryRequests>,
    catalog: ProviderCatalog,
    entries: BTreeMap<ProviderId, Entry>,
    order: Vec<ProviderId>,
    legacy_order: bool,
    state: Mutex<OperationalState>,
    clock: Arc<dyn ProviderClock>,
    tools_enabled: bool,
    queue_secs: u64,
    capacity: usize,
    fm: Option<Arc<FoundationModels>>,
}

impl ProviderRuntime {
    pub(crate) fn attach_observability(
        &self,
        events: crate::service::telemetry::TelemetryRequests,
    ) {
        assert!(
            self.operational_events.set(events).is_ok(),
            "provider telemetry attached once"
        );
    }
    pub(crate) fn attach_service(&self, registry: crate::service::OperationRegistry) {
        if let Some(fm) = &self.fm {
            fm.attach_service(registry);
        }
    }
    pub(crate) fn attach_block_writer(&self) -> BlockWriter {
        lock(&self.state).blocks.attach_writer()
    }
    pub fn empty() -> Self {
        Self::legacy(
            None,
            None,
            None,
            None,
            true,
            1,
            crate::runtime::DEFAULT_QUEUE_SECS,
        )
    }

    pub fn legacy(
        primary: Option<Backend>,
        fallback: Option<Backend>,
        fm: Option<FoundationModels>,
        vision: Option<ConfiguredVision<crate::runtime::HttpVisionTransport>>,
        tools_enabled: bool,
        capacity: usize,
        queue_secs: u64,
    ) -> Self {
        let config = ProviderConfig::from_iter(std::iter::empty::<(&str, &str)>())
            .expect("empty provider config");
        let mut runtime = Self {
            catalog: ProviderCatalog::new(&config),
            entries: BTreeMap::new(),
            order: Vec::new(),
            legacy_order: true,
            operational_events: std::sync::OnceLock::new(),
            state: Mutex::new(OperationalState {
                router: AdaptiveRouter::new(Vec::new()),
                blocks: BlockStore::memory(),
            }),
            clock: Arc::new(MonotonicClock(Instant::now())),
            tools_enabled,
            queue_secs,
            capacity,
            fm: fm.map(Arc::new),
        };
        for (name, backend) in [("primary", primary), ("local-fallback", fallback)] {
            if let Some(backend) = backend {
                let caps = ProviderCapabilities::primary(&backend, true);
                let locality = backend_locality(&backend);
                let label = backend.label();
                let local = backend.is_loopback_openai_compatible();
                let identity = backend_identity(&backend);
                let id = ProviderId::parse(name).expect("static provider ID");
                let adapter = Arc::new(HttpAdapter {
                    id: id.clone(),
                    backend,
                    transport: crate::llm::HttpTransport::default(),
                    tools_rejected: std::sync::atomic::AtomicBool::new(false),
                });
                runtime.register(
                    id,
                    label,
                    if local {
                        ProviderClass::LocalServer
                    } else {
                        ProviderClass::Cloud
                    },
                    caps,
                    locality,
                    ProviderProvenance::Configuration,
                    true,
                    identity,
                    Some(adapter),
                    None,
                    local,
                    false,
                );
            }
        }
        if let Some(fm) = runtime.fm.clone() {
            let admitted = fm.config.fallback && fm.is_qualified();
            let locality = if fm.config.mode == FmMode::System {
                ExecutionLocality::SameHost
            } else {
                ExecutionLocality::PublicRemote
            };
            let identity = config_identity(format!("{:?}", fm.config).as_bytes());
            if let Some(backend) = fm.server_backend() {
                let id = ProviderId::parse("foundation-models-server").expect("static ID");
                let adapter = Arc::new(HttpAdapter {
                    id: id.clone(),
                    backend,
                    transport: crate::llm::HttpTransport::default(),
                    tools_rejected: std::sync::atomic::AtomicBool::new(false),
                });
                runtime.register(
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
            let id = ProviderId::parse("foundation-models-cli").expect("static ID");
            let adapter = Arc::new(FmCliAdapter {
                id: id.clone(),
                fm: fm.clone(),
            });
            runtime.register(
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
        if let Some(vision) = vision {
            runtime.set_vision(vision);
        }
        runtime
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one private assembly boundary makes capability and qualification evidence explicit"
    )]
    fn register(
        &mut self,
        id: ProviderId,
        label: &'static str,
        class: ProviderClass,
        caps: ProviderCapabilities,
        locality: ExecutionLocality,
        provenance: ProviderProvenance,
        admitted: bool,
        identity: ProviderIdentityHashes,
        adapter: Option<Arc<dyn TurnAdapter>>,
        image: Option<Arc<dyn VisionAdapter>>,
        local_voice: bool,
        stream_only: bool,
    ) {
        let admitted = admitted
            && adapter
                .as_ref()
                .is_none_or(|adapter| adapter.provider_id() == &id);
        let descriptor = ProviderDescriptor {
            id: id.clone(),
            class,
            discovery: DiscoveryBoundary::ExactEndpoint,
            detection: DetectionState::Detected,
            eligibility: if admitted {
                Eligibility::Routable
            } else {
                Eligibility::Blocked(BlockedReason::Unqualified)
            },
            declared_capabilities: caps,
            isolation: IsolationCapabilities {
                loopback_only: local_voice,
                ..IsolationCapabilities::default()
            },
            provenance,
        };
        let profiles = RequestClass::ALL
            .into_iter()
            .filter_map(|class| {
                ScoreProducerPolicy::V1
                    .compatibility(class, caps, locality)
                    .ok()
            })
            .collect();
        lock(&self.state).router.requalify(
            id.clone(),
            identity.clone(),
            profiles,
            RouteAdmission {
                identity_current: admitted,
                ..RouteAdmission::QUALIFIED
            },
        );
        self.catalog.register_runtime(descriptor);
        self.order.retain(|other| other != &id);
        self.order.push(id.clone());
        self.entries.insert(
            id,
            Entry {
                adapter,
                image,
                label,
                local_voice,
                stream_only,
                slots: Arc::new(tokio::sync::Semaphore::new(self.capacity)),
                identity,
                qualification_witness: None,
                qualification_generation: None,
                qualification_completed_unix_secs: None,
            },
        );
    }

    pub fn apply_configuration(&mut self, mut config: ProviderConfig) {
        // Explicit legacy configuration is the existing cloud authorization boundary.
        for id in &self.order {
            if matches!(id.as_str(), "primary" | "local-fallback" | "vision") {
                config.cloud_allow.insert(id.clone());
            }
        }
        self.catalog.reapply_policy(&config);
        if !config.order.is_empty() {
            self.legacy_order = false;
            lock(&self.state).router.set_order(config.order.clone());
            self.order.sort_by_key(|id| {
                (
                    config
                        .order
                        .iter()
                        .position(|entry| entry == id)
                        .unwrap_or(usize::MAX),
                    id.clone(),
                )
            });
        }
        // Described future adapters never become executable from discovery/configuration alone.
        for id in config.discovery.iter().chain(config.providers.keys()) {
            if !self.entries.contains_key(id) {
                self.catalog
                    .register_runtime(ProviderDescriptor::unqualified(
                        id.clone(),
                        ProviderClass::LocalServer,
                    ));
            }
        }
    }

    pub fn apply_fm_qualification(&mut self, path: &std::path::Path) -> Result<(), String> {
        let Some(fm) = self.fm.as_ref().filter(|fm| fm.is_qualified()) else {
            return Ok(());
        };
        let identity = super::qualification::fm_manifest_identity(&fm.config)?;
        let document = super::manifest::read_manifest(path)
            .map_err(|_| "cannot read verified FM score evidence")?;
        let legacy_identity = super::qualification::fm_identity(&fm.config)?;
        for name in [
            "foundation-models-server",
            "foundation-models-cli",
            "vision",
        ] {
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
                    super::manifest::ManifestDocument::LegacyV1(_) => document
                        .legacy_score_profile(
                            if name == "foundation-models-server" {
                                super::manifest::LegacyScoreRoute::FmServer
                            } else {
                                super::manifest::LegacyScoreRoute::FmCli
                            },
                            &legacy_identity,
                            class,
                            ExecutionLocality::SameHost,
                            super::qualification::unix_now(),
                        )
                        .ok(),
                    super::manifest::ManifestDocument::V2(manifest) => manifest
                        .exact_qualified_record(
                            &ProviderId::parse("foundation-models").expect("static ID"),
                            ProviderClass::OsManagedLocal,
                            &identity,
                            ProviderCapabilities::default(),
                        )
                        .ok()
                        .and_then(|record| {
                            record
                                .score_profile(class, ExecutionLocality::SameHost)
                                .ok()
                        }),
                })
                .collect();
            entry.qualification_witness = match &document {
                super::manifest::ManifestDocument::LegacyV1(report) => {
                    let evidence = if name == "foundation-models-server" {
                        &report.fm_server
                    } else {
                        &report.fm_cli
                    };
                    Some(super::manifest::sha256_bytes(
                        &serde_json::to_vec(&(report.generated_unix_secs, evidence))
                            .map_err(|_| "cannot encode verified qualification witness")?,
                    ))
                }
                super::manifest::ManifestDocument::V2(manifest) => manifest
                    .record(&ProviderId::parse("foundation-models").expect("static ID"))
                    .and_then(|record| record.qualification_run_nonce.clone()),
            };
            let (generation, completed) = match &document {
                super::manifest::ManifestDocument::LegacyV1(report) => {
                    (None, Some(report.generated_unix_secs))
                }
                super::manifest::ManifestDocument::V2(manifest) => manifest
                    .record(&ProviderId::parse("foundation-models").expect("static ID"))
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

    pub fn restore_blocks(&self, path: std::path::PathBuf) -> Result<(), String> {
        let blocks = BlockStore::read(path)?;
        let mut state = lock(&self.state);
        for record in blocks.records() {
            let fresh_qualification = self.entries.get(&record.id).is_some_and(|entry| {
                entry.identity == record.identity
                    && entry.qualification_witness.is_some()
                    && entry.qualification_witness != record.qualification_witness
                    && record.blocked_unix_secs.is_some_and(|blocked| {
                        entry
                            .qualification_completed_unix_secs
                            .is_some_and(|completed| {
                                completed
                                    > blocked
                                        .max(record.qualification_completed_unix_secs.unwrap_or(0))
                                    && completed <= self.clock.unix_secs()
                            })
                    })
                    && entry
                        .qualification_generation
                        .is_none_or(|new| new > record.qualification_generation.unwrap_or(0))
            });
            if fresh_qualification {
                continue;
            }
            state
                .router
                .restore_blocked(&record.id, &record.identity, record.reason);
        }
        state.blocks = blocks;
        Ok(())
    }

    pub fn set_vision(&mut self, vision: ConfiguredVision<crate::runtime::HttpVisionTransport>) {
        let (provenance, locality, identity) = match &vision {
            ConfiguredVision::Remote(remote) => (
                ProviderProvenance::Configuration,
                endpoint_locality(&remote.config.base_url),
                config_identity(
                    format!(
                        "{}\n{}\n{}",
                        remote.config.base_url, remote.config.model, remote.config.api_key
                    )
                    .as_bytes(),
                ),
            ),
            ConfiguredVision::FoundationModels(_) => (
                ProviderProvenance::QualifiedManifest,
                ExecutionLocality::SameHost,
                config_identity(b"qualified-fm-vision"),
            ),
        };
        self.register(
            ProviderId::parse("vision").expect("static ID"),
            "vision provider",
            ProviderClass::LocalServer,
            ProviderCapabilities {
                vision: true,
                ocr: true,
                ..ProviderCapabilities::default()
            },
            locality,
            provenance,
            true,
            identity,
            None,
            Some(Arc::new(vision)),
            false,
            false,
        );
    }

    #[cfg(test)]
    pub fn set_primary(&mut self, backend: Option<Backend>) {
        let id = ProviderId::parse("primary").unwrap();
        self.entries.remove(&id);
        self.order.retain(|entry| entry != &id);
        if let Some(backend) = backend {
            let caps = ProviderCapabilities::primary(&backend, true);
            let local = backend.is_loopback_openai_compatible();
            self.register(
                id.clone(),
                backend.label(),
                if local {
                    ProviderClass::LocalServer
                } else {
                    ProviderClass::Cloud
                },
                caps,
                backend_locality(&backend),
                ProviderProvenance::Configuration,
                true,
                backend_identity(&backend),
                Some(Arc::new(HttpAdapter {
                    id,
                    backend,
                    transport: crate::llm::HttpTransport::default(),
                    tools_rejected: std::sync::atomic::AtomicBool::new(false),
                })),
                None,
                local,
                false,
            );
        }
    }
    #[cfg(test)]
    pub fn set_fm(&mut self, fm: Option<FoundationModels>) {
        *self = Self::legacy(
            None,
            None,
            fm,
            None,
            self.tools_enabled,
            self.capacity,
            self.queue_secs,
        );
    }
    #[cfg(test)]
    pub fn clear_vision(&mut self) {
        let id = ProviderId::parse("vision").unwrap();
        self.entries.remove(&id);
        self.order.retain(|entry| entry != &id);
    }

    pub fn tools_enabled(&self) -> bool {
        self.tools_enabled
    }
    pub fn generation_label(&self) -> Option<&'static str> {
        self.order
            .iter()
            .filter(|id| {
                self.catalog
                    .descriptor(id)
                    .is_some_and(|descriptor| descriptor.eligibility.is_routable())
            })
            .filter_map(|id| self.entries.get(id))
            .find(|entry| entry.adapter.is_some())
            .map(|entry| entry.label)
    }
    /// Snapshot-only request-class projection. No reservation, network call or qualification.
    pub fn request_readiness(&self, class: RequestClass) -> Result<(), RouteUnavailableReason> {
        self.request_readiness_for(class, false)
    }

    /// Snapshot-only readiness for the execution mode used by the caller.
    /// Stream-only adapters cannot serve ordinary nonstreaming command requests.
    pub fn request_readiness_for(
        &self,
        class: RequestClass,
        streaming: bool,
    ) -> Result<(), RouteUnavailableReason> {
        let state = lock(&self.state);
        let now = self.clock.now_ms();
        let mut reason = RouteUnavailableReason::NoConfiguredProvider;
        for id in &self.order {
            let descriptor = self.catalog.descriptor(id).expect("registered descriptor");
            let admission = self.entries[id].admission(
                descriptor,
                class,
                streaming,
                false,
                !state.blocks.failed(),
            );
            match state.router.assess(id, class, now, admission) {
                Ok(()) => return Ok(()),
                Err(error) => reason = reason.max(error),
            }
        }
        Err(reason)
    }
    pub fn generation_available(&self) -> bool {
        self.available(RequestClass::TextReadOnly, false)
    }
    pub fn vision_available(&self) -> bool {
        self.available(RequestClass::VisionDescribe, false)
    }
    pub fn local_voice_route(&self) -> Option<ProviderId> {
        self.order
            .iter()
            .find(|id| {
                self.entries[*id].local_voice && self.eligible(id, RequestClass::TextReadOnly)
            })
            .cloned()
    }
    fn available(&self, class: RequestClass, local: bool) -> bool {
        self.order
            .iter()
            .any(|id| (!local || self.entries[id].local_voice) && self.eligible(id, class))
    }
    fn eligible(&self, id: &ProviderId, class: RequestClass) -> bool {
        let state = lock(&self.state);
        self.catalog.descriptor(id).is_some_and(|descriptor| {
            state
                .router
                .assess(
                    id,
                    class,
                    self.clock.now_ms(),
                    RouteAdmission {
                        identity_current: descriptor.eligibility.is_routable(),
                        capability_allowed: class.supported_by(descriptor.declared_capabilities),
                        ..RouteAdmission::QUALIFIED
                    },
                )
                .is_ok()
        })
    }
    pub fn foundation_models(&self) -> Option<&FoundationModels> {
        self.fm.as_deref()
    }

    pub fn begin(&self, with_tools: bool, streaming: bool) -> ProviderConversation<'_> {
        self.conversation(
            RequestClass::text(with_tools && self.tools_enabled),
            streaming,
            false,
            None,
        )
    }
    pub fn voice(&self, id: &ProviderId) -> ProviderConversation<'_> {
        self.conversation(RequestClass::TextReadOnly, false, true, Some(id.clone()))
    }
    fn conversation(
        &self,
        class: RequestClass,
        streaming: bool,
        local: bool,
        initial: Option<ProviderId>,
    ) -> ProviderConversation<'_> {
        ProviderConversation {
            runtime: self,
            effects: ConversationEffects::default(),
            class,
            streaming,
            local,
            initial,
            lease: None,
        }
    }

    pub async fn chat(
        &self,
        system: &str,
        turns: &[ChatTurn],
    ) -> Result<(String, &'static str), LlmError> {
        let mut conversation = self.begin(false, false);
        loop {
            conversation.reserve().await?;
            let result = conversation
                .execute(system, turns, &[], ResponseStyle::Default, None)
                .await;
            match result {
                Ok(turn) => return Ok((turn.text, conversation.label())),
                Err(error) if conversation.fallback(&error) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

pub struct ProviderConversation<'a> {
    runtime: &'a ProviderRuntime,
    effects: ConversationEffects,
    class: RequestClass,
    streaming: bool,
    local: bool,
    initial: Option<ProviderId>,
    lease: Option<AttemptLease<'a>>,
}
struct AttemptLease<'a> {
    runtime: &'a ProviderRuntime,
    id: ProviderId,
    attempt: Option<RouteAttempt>,
    started: u64,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

#[cfg(test)]
mod tests;
