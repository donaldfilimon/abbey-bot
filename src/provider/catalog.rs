//! Provider catalog registration and configuration policy hard filters.
//!
//! The catalog retains every configured provider, including fixed ineligible
//! states, so inspection never confuses "not routable" with "not present".
//! Dynamic eligibility remains runtime state and is never written to the
//! qualification manifest.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::ProviderCapabilities;
use super::config::ProviderConfig;
#[cfg(test)]
use super::discovery::{
    DiscoveryLimits, DiscoveryRequest, DiscoveryResult, ExecutableIdentity, discover,
};
#[cfg(test)]
use super::domain::ProviderProvenance;
use super::domain::{
    BlockedReason, DetectionState, Eligibility, IsolationCapabilities, ProviderClass,
    ProviderDescriptor, ProviderId,
};
#[cfg(test)]
use super::manifest::{
    ManifestError, ProviderIdentityHashes, ProviderManifest, ProviderRecord, QualificationStatus,
};

/// Value-free provider policy derived from validated runtime configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct CatalogPolicy {
    disabled: BTreeSet<ProviderId>,
    cloud_allow: BTreeSet<ProviderId>,
    agent_cli_allow: BTreeSet<ProviderId>,
    sandbox_configured: bool,
}

impl CatalogPolicy {
    #[must_use]
    pub fn from_config(config: &ProviderConfig) -> Self {
        Self {
            disabled: config.disabled.clone(),
            cloud_allow: config.cloud_allow.clone(),
            agent_cli_allow: config.agent_cli_allow.clone(),
            sandbox_configured: config.sandbox_runner.is_some() && config.sandbox_profile.is_some(),
        }
    }
}

impl fmt::Debug for CatalogPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogPolicy")
            .field("disabled", &self.disabled)
            .field("cloud_allow", &self.cloud_allow)
            .field("agent_cli_allow", &self.agent_cli_allow)
            .field("sandbox_configured", &self.sandbox_configured)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct CatalogEntry {
    descriptor: ProviderDescriptor,
    configured_capabilities: ProviderCapabilities,
    configured_isolation: IsolationCapabilities,
    admission: Eligibility,
    #[cfg(test)]
    executable: Option<ExecutableIdentity>,
}

impl fmt::Debug for CatalogEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogEntry")
            .field("descriptor", &self.descriptor)
            .field("admission", &self.admission)
            .finish()
    }
}

/// Deterministic provider registry keyed by stable provider ID.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderCatalog {
    entries: BTreeMap<ProviderId, CatalogEntry>,
    policy: CatalogPolicy,
}

impl ProviderCatalog {
    #[must_use]
    pub fn new(config: &ProviderConfig) -> Self {
        Self {
            entries: BTreeMap::new(),
            policy: CatalogPolicy::from_config(config),
        }
    }

    /// Trusted assembly of an explicitly configured, implemented legacy adapter.
    /// Qualification is verified by the builder before this boundary.
    pub(super) fn register_runtime(&mut self, descriptor: ProviderDescriptor) {
        self.entries.insert(
            descriptor.id.clone(),
            CatalogEntry {
                configured_capabilities: descriptor.declared_capabilities,
                configured_isolation: descriptor.isolation,
                admission: descriptor.eligibility,
                descriptor,
                #[cfg(test)]
                executable: None,
            },
        );
    }

    /// Discover only provider IDs explicitly selected by
    /// `ABBEY_PROVIDER_DISCOVERY`. Requests for every other ID are registered
    /// as not detected without inspecting or executing their candidates.
    #[cfg(test)]
    pub async fn discover_configured(
        config: &ProviderConfig,
        requests: impl IntoIterator<Item = DiscoveryRequest>,
        limits: DiscoveryLimits,
    ) -> Self {
        let mut catalog = Self::new(config);
        for request in requests {
            let discovered = if config.discovery.contains(&request.id) {
                discover(request, limits).await
            } else {
                DiscoveryResult::not_configured(request)
            };
            catalog.register_discovery(discovered);
        }
        catalog
    }

    /// Register a bounded discovery result. Duplicate stable IDs are retained
    /// as one explicit ambiguous entry rather than choosing by arrival order.
    #[cfg(test)]
    pub fn register_discovery(&mut self, discovered: DiscoveryResult) {
        let (mut descriptor, executable) = discovered.into_parts();
        let id = descriptor.id.clone();
        if let Some(existing) = self.entries.get_mut(&id) {
            existing.descriptor.detection = DetectionState::Ambiguous;
            existing.descriptor.eligibility = Eligibility::Blocked(BlockedReason::Ambiguous);
            existing.descriptor.provenance = ProviderProvenance::Configuration;
            existing.admission = Eligibility::Blocked(BlockedReason::Ambiguous);
            existing.executable = None;
            return;
        }

        let admission = descriptor.eligibility;
        descriptor.eligibility = apply_policy(&self.policy, &descriptor, admission);
        let configured_capabilities = descriptor.declared_capabilities;
        let configured_isolation = descriptor.isolation;
        self.entries.insert(
            id,
            CatalogEntry {
                descriptor,
                configured_capabilities,
                configured_isolation,
                admission,
                executable,
            },
        );
    }

    /// Resolve and apply one exact manifest identity.
    ///
    /// A successful record can make a detected provider routable only after
    /// the catalog independently rechecks provider ID, class, detected binary
    /// hash, operator policy, allowlists, and sandbox evidence. Every failure
    /// maps to a fixed content-free blocked reason.
    #[cfg(test)]
    pub fn apply_manifest(
        &mut self,
        provider_id: &ProviderId,
        manifest: &ProviderManifest,
        identity: &ProviderIdentityHashes,
        required: ProviderCapabilities,
    ) -> bool {
        let Some(provider_class) = self
            .entries
            .get(provider_id)
            .map(|entry| entry.descriptor.class)
        else {
            return false;
        };
        let qualification =
            manifest.exact_qualified_record(provider_id, provider_class, identity, required);
        self.apply_manifest_result(provider_id, qualification)
    }

    /// Adaptive class admission adds score evidence to the existing exact identity gate.
    /// Keeping this separate preserves legacy FM qualification until runtime cutover.
    #[cfg(test)]
    pub fn qualified_score_profile(
        &self,
        provider_id: &ProviderId,
        manifest: &ProviderManifest,
        identity: &ProviderIdentityHashes,
        required: ProviderCapabilities,
        class: super::scoring::RequestClass,
        locality: super::scoring::ExecutionLocality,
    ) -> Result<super::scoring::ProviderScoreProfile, ManifestError> {
        let descriptor = self
            .descriptor(provider_id)
            .ok_or(ManifestError::QualificationMissing)?;
        if !descriptor.eligibility.is_routable() {
            return Err(ManifestError::NotQualified);
        }
        manifest
            .exact_qualified_record(provider_id, descriptor.class, identity, required)?
            .score_profile(class, locality)
    }

    #[cfg(test)]
    fn apply_manifest_result(
        &mut self,
        provider_id: &ProviderId,
        qualification: Result<&ProviderRecord, ManifestError>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(provider_id) else {
            return false;
        };

        match qualification {
            Ok(record) => {
                let record_matches = record.provider_id == entry.descriptor.id
                    && record.provider_class == entry.descriptor.class
                    && matches!(record.qualification_status, QualificationStatus::Qualified)
                    && entry.executable.as_ref().is_none_or(|executable| {
                        record.identity.provider_binary_sha256.as_deref()
                            == Some(executable.binary_sha256())
                    });
                if record_matches {
                    entry.descriptor.declared_capabilities =
                        record.declared_capabilities.as_provider_capabilities();
                    entry.descriptor.isolation = record.isolation_capabilities.into();
                    entry.descriptor.provenance = ProviderProvenance::QualifiedManifest;
                    entry.admission = Eligibility::Routable;
                } else {
                    reset_to_configured(entry);
                    entry.admission = Eligibility::Blocked(BlockedReason::IdentityMismatch);
                }
            }
            Err(error) => {
                reset_to_configured(entry);
                entry.admission = Eligibility::Blocked(match error {
                    ManifestError::QualificationMissing | ManifestError::NotQualified => {
                        BlockedReason::Unqualified
                    }
                    ManifestError::IdentityMismatch => BlockedReason::IdentityMismatch,
                    ManifestError::CapabilityMismatch => BlockedReason::CapabilityUnavailable,
                    _ => BlockedReason::RequalificationRequired,
                });
            }
        }
        entry.descriptor.eligibility =
            apply_policy(&self.policy, &entry.descriptor, entry.admission);
        true
    }

    /// Reapply dynamic operator policy without changing qualification
    /// provenance or admission evidence.
    pub fn reapply_policy(&mut self, config: &ProviderConfig) {
        self.policy = CatalogPolicy::from_config(config);
        for entry in self.entries.values_mut() {
            entry.descriptor.eligibility =
                apply_policy(&self.policy, &entry.descriptor, entry.admission);
        }
    }

    #[must_use]
    pub fn descriptor(&self, provider_id: &ProviderId) -> Option<&ProviderDescriptor> {
        self.entries.get(provider_id).map(|entry| &entry.descriptor)
    }

    #[must_use]
    #[cfg(test)]
    pub fn executable_identity(&self, provider_id: &ProviderId) -> Option<&ExecutableIdentity> {
        self.entries
            .get(provider_id)
            .and_then(|entry| entry.executable.as_ref())
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.entries.values().map(|entry| &entry.descriptor)
    }

    #[cfg(test)]
    pub fn routable(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.descriptors()
            .filter(|descriptor| descriptor.eligibility.is_routable())
    }

    #[must_use]
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Debug for ProviderCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderCatalog")
            .field("entries", &self.entries)
            .field("policy", &self.policy)
            .finish()
    }
}

#[cfg(test)]
fn reset_to_configured(entry: &mut CatalogEntry) {
    entry.descriptor.declared_capabilities = entry.configured_capabilities;
    entry.descriptor.isolation = entry.configured_isolation;
    entry.descriptor.provenance = ProviderProvenance::Configuration;
}

fn apply_policy(
    policy: &CatalogPolicy,
    descriptor: &ProviderDescriptor,
    admission: Eligibility,
) -> Eligibility {
    match descriptor.detection {
        DetectionState::NotDetected => return Eligibility::Blocked(BlockedReason::NotDetected),
        #[cfg(test)]
        DetectionState::Ambiguous => return Eligibility::Blocked(BlockedReason::Ambiguous),
        #[cfg(test)]
        DetectionState::InvalidConfiguration => {
            return Eligibility::Blocked(BlockedReason::InvalidConfiguration);
        }
        DetectionState::Detected => {}
    }
    if policy.disabled.contains(&descriptor.id) {
        return Eligibility::Blocked(BlockedReason::OperatorDisabled);
    }
    match descriptor.class {
        ProviderClass::Cloud if !policy.cloud_allow.contains(&descriptor.id) => {
            return Eligibility::Blocked(BlockedReason::CloudNotAllowed);
        }
        ProviderClass::AgentCli if !policy.agent_cli_allow.contains(&descriptor.id) => {
            return Eligibility::Blocked(BlockedReason::AgentCliNotAllowed);
        }
        ProviderClass::AgentCli
            if !policy.sandbox_configured || !descriptor.isolation.sandbox_attested =>
        {
            return Eligibility::Blocked(BlockedReason::SandboxRequired);
        }
        _ => {}
    }
    admission
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
