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
use blocks::BlockStore;

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
struct OperationalState {
    router: AdaptiveRouter,
    blocks: BlockStore,
}

pub struct ProviderRuntime {
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
        self.catalog.descriptor(id).is_some_and(|d| {
            d.eligibility.is_routable() && class.supported_by(d.declared_capabilities)
        }) && state.router.profile(id, class).is_some()
            && state
                .router
                .snapshot()
                .iter()
                .find(|s| &s.provider_id == id)
                .is_some_and(|s| {
                    matches!(
                        s.circuit.phase,
                        CircuitPhase::Closed | CircuitPhase::HalfOpen
                    ) || (s.circuit.phase == CircuitPhase::Open
                        && s.circuit
                            .open_until_ms
                            .is_some_and(|until| self.clock.now_ms() >= until))
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

    pub fn inspect_snapshot(&self) -> Vec<crate::inspect::ProviderRouteInspect> {
        use super::domain::TemporaryUnavailableReason as T;
        use crate::inspect::{ProviderProvenance as P, ProviderRouteInspect, ProviderRouteLabel};
        let state = lock(&self.state);
        let circuits = state.router.snapshot();
        self.catalog
            .descriptors()
            .map(|descriptor| {
                let id = &descriptor.id;
                let route = match id.as_str() {
                    "primary" => ProviderRouteLabel::Primary,
                    "local-fallback" => ProviderRouteLabel::LocalFallback,
                    "foundation-models-server" => ProviderRouteLabel::FoundationModelsServer,
                    "foundation-models-cli" => ProviderRouteLabel::FoundationModelsCli,
                    _ => ProviderRouteLabel::Vision,
                };
                let snapshot = circuits.iter().find(|snapshot| &snapshot.provider_id == id);
                let eligibility = if !descriptor.eligibility.is_routable() {
                    descriptor.eligibility
                } else if state.blocks.failed() {
                    Eligibility::TemporarilyUnavailable(T::BudgetExhausted)
                } else if let Some(snapshot) = snapshot {
                    match snapshot.circuit.phase {
                        CircuitPhase::Blocked => {
                            Eligibility::Blocked(match snapshot.circuit.reason {
                                Some(
                                    ProviderFailureKind::ExecutableIdentity
                                    | ProviderFailureKind::ModelIdentity
                                    | ProviderFailureKind::SandboxIdentity,
                                ) => BlockedReason::IdentityMismatch,
                                _ => BlockedReason::RequalificationRequired,
                            })
                        }
                        CircuitPhase::Open => Eligibility::TemporarilyUnavailable(
                            if matches!(
                                snapshot.circuit.reason,
                                Some(
                                    ProviderFailureKind::RateLimited | ProviderFailureKind::Http5xx
                                )
                            ) {
                                T::RetryAfter
                            } else {
                                T::CircuitOpen
                            },
                        ),
                        CircuitPhase::HalfOpen if snapshot.circuit.probe_reserved => {
                            Eligibility::TemporarilyUnavailable(T::Busy)
                        }
                        _ if !RequestClass::ALL
                            .into_iter()
                            .any(|class| state.router.profile(id, class).is_some()) =>
                        {
                            Eligibility::Blocked(BlockedReason::CapabilityUnavailable)
                        }
                        _ if self
                            .entries
                            .get(id)
                            .is_some_and(|entry| entry.slots.available_permits() == 0) =>
                        {
                            Eligibility::TemporarilyUnavailable(T::Busy)
                        }
                        _ => Eligibility::Routable,
                    }
                } else {
                    Eligibility::Blocked(BlockedReason::Unqualified)
                };
                let caps = descriptor.declared_capabilities;
                ProviderRouteInspect::new(
                    route,
                    eligibility.is_routable(),
                    caps.text,
                    caps.tools
                        && self.tools_enabled
                        && self
                            .entries
                            .get(id)
                            .and_then(|entry| entry.adapter.as_ref())
                            .is_some_and(|adapter| adapter.tools_enabled()),
                    caps.vision,
                    caps.ocr,
                    if descriptor.provenance == ProviderProvenance::QualifiedManifest {
                        P::QualifiedManifest
                    } else {
                        P::Configuration
                    },
                )
                .with_runtime(
                    descriptor.clone(),
                    eligibility,
                    snapshot.and_then(|snapshot| snapshot.circuit.reason),
                )
            })
            .collect()
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
impl ProviderConversation<'_> {
    pub fn effects(&self) -> ConversationEffects {
        self.effects.clone()
    }
    pub fn label(&self) -> &'static str {
        lock(&self.effects.0)
            .selected()
            .and_then(|id| self.runtime.entries.get(id))
            .map_or("generation provider", |entry| entry.label)
    }
    pub fn tools_available(&self) -> bool {
        self.class == RequestClass::TextWithTools
            && lock(&self.effects.0).selected().is_none_or(|id| {
                self.runtime.entries[id]
                    .adapter
                    .as_ref()
                    .is_some_and(|adapter| adapter.tools_enabled())
            })
    }
    pub fn streams(&self) -> bool {
        self.streaming
            && lock(&self.effects.0)
                .selected()
                .and_then(|id| self.runtime.catalog.descriptor(id))
                .is_some_and(|d| d.declared_capabilities.streaming)
    }
    pub fn fallback(&mut self, error: &LlmError) -> bool {
        error.unavailable().is_none()
            && lock(&self.effects.0)
                .begin_fallback(error.retry_after().classify(error.provider_failure()))
    }
    pub async fn reserve(&mut self) -> Result<(), LlmError> {
        if self.lease.is_some() {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(if self.local {
                crate::runtime::voice_queue_secs(self.runtime.queue_secs)
            } else {
                self.runtime.queue_secs
            });
        loop {
            let selected = lock(&self.effects.0)
                .selected()
                .cloned()
                .or_else(|| self.initial.clone());
            let excluded = lock(&self.effects.0).excluded().clone();
            let mut reason = RouteUnavailableReason::NoConfiguredProvider;
            let mut waiting = None;
            {
                let mut state = lock(&self.runtime.state);
                for id in &self.runtime.order {
                    let entry = &self.runtime.entries[id];
                    let descriptor = self
                        .runtime
                        .catalog
                        .descriptor(id)
                        .expect("registered descriptor");
                    let supported_adapter = match self.class {
                        RequestClass::VisionDescribe | RequestClass::VisionOcr => {
                            entry.image.is_some()
                        }
                        _ => entry.adapter.is_some(),
                    };
                    let allowed = supported_adapter
                        && self.class.supported_by(descriptor.declared_capabilities)
                        && (!entry.stream_only
                            || (self.streaming && self.class == RequestClass::TextReadOnly));
                    let capacity = entry.slots.available_permits() > 0;
                    let budget_available = !state.blocks.failed();
                    state.router.set_admission(
                        id,
                        RouteAdmission {
                            configured: entry.adapter.is_some() || entry.image.is_some(),
                            identity_current: descriptor.eligibility.is_routable(),
                            capability_allowed: allowed,
                            policy_allowed: !self.local || entry.local_voice,
                            budget_available,
                            capacity_available: capacity,
                        },
                    );
                    if !capacity
                        && allowed
                        && descriptor.eligibility.is_routable()
                        && (!self.local || entry.local_voice)
                        && selected.as_ref().is_none_or(|pin| pin == id)
                        && !excluded.contains(id)
                        && waiting.is_none()
                    {
                        waiting = Some(entry.slots.clone());
                    }
                }
                let mut chosen = None;
                if self.runtime.legacy_order && selected.is_none() {
                    for id in &self.runtime.order {
                        match state.router.select(
                            self.class,
                            self.runtime.clock.now_ms(),
                            Some(id),
                            &excluded,
                        ) {
                            Ok(selection) => {
                                chosen = Some(selection);
                                break;
                            }
                            Err(error) => reason = reason.max(error),
                        }
                    }
                } else {
                    match state.router.select(
                        self.class,
                        self.runtime.clock.now_ms(),
                        selected.as_ref(),
                        &excluded,
                    ) {
                        Ok(selection) => chosen = Some(selection),
                        Err(error) => reason = error,
                    }
                }
                if let Some((decision, attempt)) = chosen {
                    let id = decision.provider_id.clone();
                    let entry = &self.runtime.entries[&id];
                    lock(&self.effects.0).accept_selection(&decision);
                    self.initial = None;
                    let mut lease = AttemptLease {
                        runtime: self.runtime,
                        id,
                        attempt: Some(attempt),
                        started: self.runtime.clock.now_ms(),
                        permit: None,
                    };
                    match entry.slots.clone().try_acquire_owned() {
                        Ok(permit) => {
                            lease.permit = Some(permit);
                            self.lease = Some(lease);
                            return Ok(());
                        }
                        Err(_) => {
                            state.router.complete(
                                lease.attempt.take().expect("reserved"),
                                ProviderFailureKind::Busy,
                                RetryAfter::Absent,
                                None,
                                self.runtime.clock.now_ms(),
                            );
                            return Err(LlmError::busy());
                        }
                    }
                }
            }
            if reason != RouteUnavailableReason::Busy {
                return Err(LlmError::route_unavailable(reason));
            }
            let Some(slots) = waiting else {
                return Err(LlmError::route_unavailable(reason));
            };
            match tokio::time::timeout_at(deadline, slots.acquire_owned()).await {
                Ok(Ok(permit)) => drop(permit),
                _ => return Err(LlmError::route_unavailable(RouteUnavailableReason::Busy)),
            }
        }
    }
    pub async fn execute(
        &mut self,
        system: &str,
        turns: &[ChatTurn],
        tools: &[crate::tools::ToolSpec],
        style: ResponseStyle,
        deltas: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<ModelTurn, LlmError> {
        self.reserve().await?;
        let mut lease = self.lease.take().expect("reserved attempt");
        let entry = &self.runtime.entries[&lease.id];
        let result = if !tools.is_empty() && !self.tools_available() {
            Err(LlmError::classified(
                "tools are forbidden for this conversation",
                ProviderFailureKind::InvalidRequest,
            ))
        } else if let Some(adapter) = &entry.adapter {
            adapter
                .execute(AdapterRequest {
                    system,
                    turns,
                    tools,
                    call_id: "runtime-turn",
                    style,
                    deltas,
                })
                .await
                .and_then(|turn| {
                    if turn.text.trim().is_empty() && turn.calls.is_empty() {
                        Err(LlmError::backend(
                            "the response carried no answer text".into(),
                        ))
                    } else if (!adapter.tools_enabled() && !turn.calls.is_empty())
                        || turn
                            .calls
                            .iter()
                            .any(|call| !tools.iter().any(|tool| tool.name == call.name))
                    {
                        Err(LlmError::classified(
                            "unrequested tool calls",
                            ProviderFailureKind::ToolSchema,
                        ))
                    } else {
                        Ok(turn)
                    }
                })
        } else {
            Err(LlmError::classified(
                "no text adapter",
                ProviderFailureKind::Configuration,
            ))
        };
        lease.complete(result.as_ref().err());
        result
    }
}
struct AttemptLease<'a> {
    runtime: &'a ProviderRuntime,
    id: ProviderId,
    attempt: Option<RouteAttempt>,
    started: u64,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl AttemptLease<'_> {
    fn complete(&mut self, error: Option<&LlmError>) {
        let Some(attempt) = self.attempt.take() else {
            return;
        };
        let now = self.runtime.clock.now_ms();
        let mut state = lock(&self.runtime.state);
        let kind = state.router.complete(
            attempt,
            error.map_or(ProviderFailureKind::Success, LlmError::provider_failure),
            error.map_or(RetryAfter::Absent, LlmError::retry_after),
            Some(now.saturating_sub(self.started)),
            now,
        );
        if let Some(kind) = kind.filter(|kind| kind.is_blocked()) {
            let entry = &self.runtime.entries[&self.id];
            state.blocks.block(blocks::BlockRecord {
                id: self.id.clone(),
                identity: entry.identity.clone(),
                qualification_witness: entry.qualification_witness.clone(),
                qualification_generation: entry.qualification_generation,
                blocked_unix_secs: Some(self.runtime.clock.unix_secs()),
                qualification_completed_unix_secs: entry.qualification_completed_unix_secs,
                reason: kind,
            });
        }
    }
}
impl Drop for AttemptLease<'_> {
    fn drop(&mut self) {
        if self.attempt.is_some() {
            self.complete(Some(&LlmError::classified(
                "provider attempt cancelled",
                ProviderFailureKind::Cancelled,
            )));
        }
    }
}

impl ImageUnderstanding for ProviderRuntime {
    async fn describe(&self, bytes: Vec<u8>) -> Result<String, VisionError> {
        self.image(false, bytes).await
    }
    async fn extract_text(&self, bytes: Vec<u8>) -> Result<String, VisionError> {
        self.image(true, bytes).await
    }
}
impl ProviderRuntime {
    async fn image(&self, ocr: bool, bytes: Vec<u8>) -> Result<String, VisionError> {
        let mut conversation = self.conversation(RequestClass::image(ocr), false, false, None);
        conversation
            .reserve()
            .await
            .map_err(|_| VisionError::internal("vision route unavailable"))?;
        let mut lease = conversation.lease.take().expect("image attempt reserved");
        let Some(adapter) = &self.entries[&lease.id].image else {
            return Err(VisionError::internal("no image adapter"));
        };
        conversation.effects.mark_image_submitted();
        let result = adapter.image(ocr, bytes).await;
        let error = result.as_ref().err().map(|error| {
            LlmError::classified("image provider failed", error.provider_failure())
                .with_retry_after(error.retry_after())
        });
        lease.complete(error.as_ref());
        result
    }
}
fn backend_locality(backend: &Backend) -> ExecutionLocality {
    match backend {
        Backend::Anthropic { .. } => ExecutionLocality::PublicRemote,
        Backend::OpenAiCompatible { endpoint, .. } => endpoint_locality(endpoint),
    }
}
fn endpoint_locality(endpoint: &str) -> ExecutionLocality {
    let evidence = reqwest::Url::parse(endpoint)
        .ok()
        .and_then(|url| {
            url.host_str()
                .and_then(|host| host.trim_matches(['[', ']']).parse().ok())
        })
        .map(ExecutionLocality::address);
    ExecutionLocality::least_local(evidence)
}

fn backend_identity(backend: &Backend) -> ProviderIdentityHashes {
    match backend {
        Backend::Anthropic { api_key } => config_identity(api_key.as_bytes()),
        Backend::OpenAiCompatible { endpoint, model } => {
            config_identity(format!("{endpoint}\n{model}").as_bytes())
        }
    }
}
static BINARY_IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn config_identity(value: &[u8]) -> ProviderIdentityHashes {
    ProviderIdentityHashes {
        abbey_binary_sha256: BINARY_IDENTITY
            .get_or_init(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|path| std::fs::read(path).ok())
                    .map(|bytes| super::manifest::sha256_bytes(&bytes))
                    .expect("running executable must remain readable for provider identity")
            })
            .clone(),
        provider_binary_sha256: None,
        model_sha256: Some(super::manifest::sha256_bytes(value)),
        os_sha256: None,
        tool_schema_sha256: super::manifest::production_tool_schema_sha256()
            .expect("static tool vocabulary"),
        sandbox_sha256: None,
    }
}

#[cfg(test)]
mod tests;
