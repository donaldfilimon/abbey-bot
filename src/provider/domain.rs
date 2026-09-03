//! Provider identities and runtime-only eligibility state.
//!
//! These types deliberately reuse the application's existing turn and tool
//! contracts. Provider adapters do not introduce a second transcript or tool
//! vocabulary.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ProviderCapabilities;
use crate::llm::{ChatTurn, LlmError, ModelTurn};
use crate::tools::ToolSpec;

const MAX_PROVIDER_ID_BYTES: usize = 64;

/// Stable, normalized provider identity.
///
/// The canonical form is lowercase ASCII kebab-case. Configuration accepts
/// uppercase letters and underscores so an environment segment such as
/// `FOUNDATION_MODELS` maps injectively to `foundation-models`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn parse(raw: &str) -> Result<Self, ProviderIdError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(ProviderIdError::Empty);
        }
        if raw.len() > MAX_PROVIDER_ID_BYTES {
            return Err(ProviderIdError::TooLong);
        }

        let mut normalized = String::with_capacity(raw.len());
        let mut previous_separator = false;
        for (index, byte) in raw.bytes().enumerate() {
            let is_separator = matches!(byte, b'-' | b'_');
            if is_separator {
                if index == 0 || previous_separator {
                    return Err(ProviderIdError::InvalidSeparator);
                }
                normalized.push('-');
                previous_separator = true;
            } else if byte.is_ascii_alphanumeric() {
                normalized.push(char::from(byte.to_ascii_lowercase()));
                previous_separator = false;
            } else {
                return Err(ProviderIdError::InvalidCharacter);
            }
        }
        if previous_separator {
            return Err(ProviderIdError::InvalidSeparator);
        }
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Canonical environment-variable segment for provider-specific settings.
    #[must_use]
    pub fn env_segment(&self) -> String {
        self.0.replace('-', "_").to_ascii_uppercase()
    }
}

impl fmt::Debug for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ProviderId").field(&self.0).finish()
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ProviderId {
    type Err = ProviderIdError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

impl Serialize for ProviderId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Fixed, value-free reason a provider identity was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderIdError {
    Empty,
    TooLong,
    InvalidCharacter,
    InvalidSeparator,
}

impl fmt::Display for ProviderIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "provider identity must not be empty",
            Self::TooLong => "provider identity is too long",
            Self::InvalidCharacter => {
                "provider identity may contain only ASCII letters, digits, hyphens, and underscores"
            }
            Self::InvalidSeparator => "provider identity separators must be single and internal",
        })
    }
}

impl Error for ProviderIdError {}

/// Operational class used by policy hard filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderClass {
    LocalServer,
    OsManagedLocal,
    Cloud,
    AgentCli,
}

impl ProviderClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalServer => "local_server",
            Self::OsManagedLocal => "os_managed_local",
            Self::Cloud => "cloud",
            Self::AgentCli => "agent_cli",
        }
    }
}

/// Result of bounded, configuration-directed discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionState {
    NotDetected,
    Detected,
    Ambiguous,
    InvalidConfiguration,
}

/// Safe shape of the operator-directed discovery boundary. Exact values stay
/// private in [`crate::provider::config::ProviderSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryBoundary {
    Unconfigured,
    ExactBinary,
    ExactEndpoint,
    ExactBinaryAndEndpoint,
    OsManaged,
}

/// Fixed reasons for temporary routing exclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporaryUnavailableReason {
    Busy,
    BudgetExhausted,
    CircuitOpen,
    RetryAfter,
}

/// Fixed reasons a provider is blocked until policy or identity changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedReason {
    NotDetected,
    Ambiguous,
    InvalidConfiguration,
    OperatorDisabled,
    CloudNotAllowed,
    AgentCliNotAllowed,
    SandboxRequired,
    Unqualified,
    IdentityMismatch,
    CapabilityUnavailable,
    RequalificationRequired,
}

/// Current runtime eligibility. This is intentionally not manifest evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Routable,
    TemporarilyUnavailable(TemporaryUnavailableReason),
    Blocked(BlockedReason),
}

impl Eligibility {
    #[must_use]
    pub const fn is_routable(self) -> bool {
        matches!(self, Self::Routable)
    }
}

/// Safe provenance exposed by provider inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProvenance {
    Configuration,
    QualifiedManifest,
}

/// Qualified process and state isolation properties.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolationCapabilities {
    pub environment_cleared: bool,
    pub absolute_no_shell_execution: bool,
    pub process_tree_contained: bool,
    pub private_runtime_state: bool,
    pub loopback_only: bool,
    pub sandbox_attested: bool,
}

/// Safe catalog view. Exact paths, endpoints, models, and hashes live in
/// configuration or qualification records and never in this inspectable view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub class: ProviderClass,
    pub discovery: DiscoveryBoundary,
    pub detection: DetectionState,
    pub eligibility: Eligibility,
    pub declared_capabilities: ProviderCapabilities,
    pub isolation: IsolationCapabilities,
    pub provenance: ProviderProvenance,
}

impl ProviderDescriptor {
    #[must_use]
    pub fn unqualified(id: ProviderId, class: ProviderClass) -> Self {
        Self {
            id,
            class,
            discovery: DiscoveryBoundary::Unconfigured,
            detection: DetectionState::NotDetected,
            eligibility: Eligibility::Blocked(BlockedReason::NotDetected),
            declared_capabilities: ProviderCapabilities::default(),
            isolation: IsolationCapabilities::default(),
            provenance: ProviderProvenance::Configuration,
        }
    }
}

/// Object-safe future returned by [`TurnAdapter`].
pub type TurnFuture<'a> = Pin<Box<dyn Future<Output = Result<ModelTurn, LlmError>> + Send + 'a>>;

/// One asynchronous provider turn expressed in Abbey's existing vocabulary.
///
/// Returning a boxed future keeps the interface object-safe on every supported
/// compiler without introducing a second async-trait dependency.
pub trait TurnAdapter: Send + Sync {
    fn provider_id(&self) -> &ProviderId;

    fn turn<'a>(
        &'a self,
        system_prompt: &'a str,
        turns: &'a [ChatTurn],
        tools: &'a [ToolSpec],
        call_id: &'a str,
    ) -> TurnFuture<'a>;
}

#[cfg(test)]
#[path = "domain_tests.rs"]
mod tests;
