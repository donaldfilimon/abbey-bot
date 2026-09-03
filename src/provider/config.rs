//! Fail-closed provider runtime configuration.
//!
//! Discovery is always directed by explicit provider IDs. This parser never
//! searches the host for binaries, services, credentials, or sessions.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::{Component, PathBuf};

use super::domain::ProviderId;

const PROVIDER_PREFIX: &str = "ABBEY_PROVIDER_";
const MAX_ENDPOINT_BYTES: usize = 2 * 1024;
const MAX_MODEL_BYTES: usize = 4 * 1024;
const MAX_IDENTITY_BYTES: usize = 4 * 1024;
const MAX_PROFILE_BYTES: usize = 1024;
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;

const DISCOVERY: &str = "ABBEY_PROVIDER_DISCOVERY";
const ORDER: &str = "ABBEY_PROVIDER_ORDER";
const DISABLED: &str = "ABBEY_PROVIDER_DISABLED";
const MANIFEST: &str = "ABBEY_PROVIDER_MANIFEST";
const STATE_DIR: &str = "ABBEY_PROVIDER_STATE_DIR";
const CLOUD_ALLOW: &str = "ABBEY_PROVIDER_CLOUD_ALLOW";
const AGENT_CLI_ALLOW: &str = "ABBEY_PROVIDER_AGENT_CLI_ALLOW";
const SANDBOX_RUNNER: &str = "ABBEY_PROVIDER_SANDBOX_RUNNER";
const SANDBOX_PROFILE: &str = "ABBEY_PROVIDER_SANDBOX_PROFILE";

const RESERVED_VARIABLES: &[&str] = &[
    DISCOVERY,
    ORDER,
    DISABLED,
    MANIFEST,
    STATE_DIR,
    CLOUD_ALLOW,
    AGENT_CLI_ALLOW,
    SANDBOX_RUNNER,
    SANDBOX_PROFILE,
];

/// Secret explicitly supplied for one provider.
///
/// It has no `Display`, serialization, or value-bearing `Debug`
/// representation. Adapters must opt in visibly via [`Self::expose_secret`]
/// only when constructing the selected provider's cleared environment.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderCredential(String);

impl ProviderCredential {
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderCredential([REDACTED])")
    }
}

/// Explicit settings for one normalized provider identity.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderSettings {
    pub id: ProviderId,
    pub binary: Option<PathBuf>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub model_identity: Option<String>,
    pub credential: Option<ProviderCredential>,
}

impl ProviderSettings {
    fn new(id: ProviderId) -> Self {
        Self {
            id,
            binary: None,
            endpoint: None,
            model: None,
            model_identity: None,
            credential: None,
        }
    }
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSettings")
            .field("id", &self.id)
            .field("binary_configured", &self.binary.is_some())
            .field("endpoint_configured", &self.endpoint.is_some())
            .field("model_configured", &self.model.is_some())
            .field("model_identity_configured", &self.model_identity.is_some())
            .field("credential_configured", &self.credential.is_some())
            .finish()
    }
}

/// Parsed provider policy and exact provider-specific settings.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    /// Exact provider identities for which bounded discovery is permitted.
    pub discovery: BTreeSet<ProviderId>,
    /// Deterministic operator tie order. Stable provider ID breaks later ties.
    pub order: Vec<ProviderId>,
    pub disabled: BTreeSet<ProviderId>,
    pub manifest: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    /// Empty means no cloud provider is allowed.
    pub cloud_allow: BTreeSet<ProviderId>,
    /// Empty means no agent CLI is allowed.
    pub agent_cli_allow: BTreeSet<ProviderId>,
    pub sandbox_runner: Option<PathBuf>,
    pub sandbox_profile: Option<String>,
    pub providers: BTreeMap<ProviderId, ProviderSettings>,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderConfig")
            .field("discovery", &self.discovery)
            .field("order", &self.order)
            .field("disabled", &self.disabled)
            .field("manifest_configured", &self.manifest.is_some())
            .field("state_dir_configured", &self.state_dir.is_some())
            .field("cloud_allow", &self.cloud_allow)
            .field("agent_cli_allow", &self.agent_cli_allow)
            .field("sandbox_runner_configured", &self.sandbox_runner.is_some())
            .field(
                "sandbox_profile_configured",
                &self.sandbox_profile.is_some(),
            )
            .field("providers", &self.providers)
            .finish()
    }
}

impl ProviderConfig {
    pub fn from_env() -> Result<Self, ProviderConfigError> {
        Self::from_iter(std::env::vars_os())
    }

    /// Parse an injected environment without consulting process-global state.
    /// Unrelated variables, including unrelated secrets, are ignored.
    pub fn from_iter<I, K, V>(variables: I) -> Result<Self, ProviderConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let variables = collect_provider_variables(variables)?;
        let discovery = parse_id_set(DISCOVERY, variables.get(DISCOVERY))?;
        let order = parse_id_order(ORDER, variables.get(ORDER))?;
        let disabled = parse_id_set(DISABLED, variables.get(DISABLED))?;
        let cloud_allow = parse_id_set(CLOUD_ALLOW, variables.get(CLOUD_ALLOW))?;
        let agent_cli_allow = parse_id_set(AGENT_CLI_ALLOW, variables.get(AGENT_CLI_ALLOW))?;
        let manifest = parse_optional_absolute_path(MANIFEST, variables.get(MANIFEST))?;
        let state_dir = parse_optional_absolute_path(STATE_DIR, variables.get(STATE_DIR))?;
        let sandbox_runner =
            parse_optional_absolute_path(SANDBOX_RUNNER, variables.get(SANDBOX_RUNNER))?;
        let sandbox_profile = parse_optional_text(
            SANDBOX_PROFILE,
            variables.get(SANDBOX_PROFILE),
            MAX_PROFILE_BYTES,
        )?;

        let mut providers = BTreeMap::<ProviderId, ProviderSettings>::new();
        for (name, value) in &variables {
            if RESERVED_VARIABLES.contains(&name.as_str()) {
                continue;
            }
            let (id, field) = parse_provider_variable(name)?;
            let canonical_name = provider_variable_name(&id, field);
            let settings = providers
                .entry(id.clone())
                .or_insert_with(|| ProviderSettings::new(id));
            match field {
                ProviderField::Binary => set_once(
                    &mut settings.binary,
                    parse_required_absolute_path(&canonical_name, value)?,
                    &canonical_name,
                )?,
                ProviderField::Endpoint => set_once(
                    &mut settings.endpoint,
                    parse_endpoint(&canonical_name, value)?,
                    &canonical_name,
                )?,
                ProviderField::Model => set_once(
                    &mut settings.model,
                    parse_required_text(&canonical_name, value, MAX_MODEL_BYTES)?,
                    &canonical_name,
                )?,
                ProviderField::ModelIdentity => set_once(
                    &mut settings.model_identity,
                    parse_required_text(&canonical_name, value, MAX_IDENTITY_BYTES)?,
                    &canonical_name,
                )?,
                ProviderField::Credential => set_once(
                    &mut settings.credential,
                    parse_credential(&canonical_name, value)?,
                    &canonical_name,
                )?,
            }
        }

        for id in discovery
            .iter()
            .chain(order.iter())
            .chain(disabled.iter())
            .chain(cloud_allow.iter())
            .chain(agent_cli_allow.iter())
        {
            providers
                .entry(id.clone())
                .or_insert_with(|| ProviderSettings::new(id.clone()));
        }

        Ok(Self {
            discovery,
            order,
            disabled,
            manifest,
            state_dir,
            cloud_allow,
            agent_cli_allow,
            sandbox_runner,
            sandbox_profile,
            providers,
        })
    }

    #[must_use]
    pub fn provider(&self, id: &ProviderId) -> Option<&ProviderSettings> {
        self.providers.get(id)
    }
}

/// A configuration error that names at most a fixed or normalized variable;
/// it never retains or renders the rejected value.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfigError {
    variable: Option<String>,
    reason: &'static str,
}

impl ProviderConfigError {
    fn for_variable(variable: impl Into<String>, reason: &'static str) -> Self {
        Self {
            variable: Some(variable.into()),
            reason,
        }
    }

    fn generic(reason: &'static str) -> Self {
        Self {
            variable: None,
            reason,
        }
    }
}

impl fmt::Debug for ProviderConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for ProviderConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(variable) = &self.variable {
            write!(formatter, "{variable}: {}", self.reason)
        } else {
            formatter.write_str(self.reason)
        }
    }
}

impl Error for ProviderConfigError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderField {
    Binary,
    Endpoint,
    Model,
    ModelIdentity,
    Credential,
}

impl ProviderField {
    const fn suffix(self) -> &'static str {
        match self {
            Self::Binary => "BINARY",
            Self::Endpoint => "ENDPOINT",
            Self::Model => "MODEL",
            Self::ModelIdentity => "MODEL_IDENTITY",
            Self::Credential => "CREDENTIAL",
        }
    }
}

fn collect_provider_variables<I, K, V>(
    variables: I,
) -> Result<BTreeMap<String, String>, ProviderConfigError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<OsString>,
    V: Into<OsString>,
{
    let mut selected = BTreeMap::new();
    for (name, value) in variables {
        let name = name.into();
        let Some(name) = name.to_str() else {
            if name.to_string_lossy().starts_with(PROVIDER_PREFIX) {
                return Err(ProviderConfigError::generic(
                    "an ABBEY_PROVIDER_* variable name was not Unicode",
                ));
            }
            continue;
        };
        if !name.starts_with(PROVIDER_PREFIX) {
            continue;
        }
        let value = value.into();
        let value = value.to_str().ok_or_else(|| {
            let safe_name = safe_variable_name(name);
            ProviderConfigError::for_variable(safe_name, "value was not Unicode")
        })?;
        if selected
            .insert(name.to_string(), value.to_string())
            .is_some()
        {
            return Err(ProviderConfigError::for_variable(
                safe_variable_name(name),
                "variable was provided more than once",
            ));
        }
    }
    Ok(selected)
}

fn parse_id_set(
    name: &'static str,
    value: Option<&String>,
) -> Result<BTreeSet<ProviderId>, ProviderConfigError> {
    Ok(parse_id_list(name, value)?.into_iter().collect())
}

fn parse_id_order(
    name: &'static str,
    value: Option<&String>,
) -> Result<Vec<ProviderId>, ProviderConfigError> {
    parse_id_list(name, value)
}

fn parse_id_list(
    name: &'static str,
    value: Option<&String>,
) -> Result<Vec<ProviderId>, ProviderConfigError> {
    let Some(value) = value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(Vec::new());
    };
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split(',') {
        if raw.trim().is_empty() {
            return Err(ProviderConfigError::for_variable(
                name,
                "provider list contains an empty entry",
            ));
        }
        let id = ProviderId::parse(raw).map_err(|_| {
            ProviderConfigError::for_variable(name, "provider list contains an invalid identity")
        })?;
        if !seen.insert(id.clone()) {
            return Err(ProviderConfigError::for_variable(
                name,
                "provider list contains a duplicate normalized identity",
            ));
        }
        ids.push(id);
    }
    Ok(ids)
}

fn parse_provider_variable(name: &str) -> Result<(ProviderId, ProviderField), ProviderConfigError> {
    let Some(tail) = name.strip_prefix(PROVIDER_PREFIX) else {
        return Err(ProviderConfigError::generic(
            "provider setting did not use the required prefix",
        ));
    };
    // Match the longest suffix first so MODEL_IDENTITY cannot be mistaken for MODEL.
    for field in [
        ProviderField::ModelIdentity,
        ProviderField::Credential,
        ProviderField::Endpoint,
        ProviderField::Binary,
        ProviderField::Model,
    ] {
        let marker = format!("_{}", field.suffix());
        let Some(id_segment) = tail.strip_suffix(&marker) else {
            continue;
        };
        let id = ProviderId::parse(id_segment).map_err(|_| {
            ProviderConfigError::generic(
                "a provider-specific variable contains an invalid provider identity",
            )
        })?;
        if id.env_segment() != id_segment {
            return Err(ProviderConfigError::for_variable(
                provider_variable_name(&id, field),
                "provider-specific variable names must use uppercase underscore identity segments",
            ));
        }
        return Ok((id, field));
    }
    Err(ProviderConfigError::generic(
        "an ABBEY_PROVIDER_* variable uses an unsupported setting suffix",
    ))
}

fn provider_variable_name(id: &ProviderId, field: ProviderField) -> String {
    format!("{PROVIDER_PREFIX}{}_{}", id.env_segment(), field.suffix())
}

fn safe_variable_name(name: &str) -> String {
    if RESERVED_VARIABLES.contains(&name) {
        return name.to_string();
    }
    parse_provider_variable(name)
        .map(|(id, field)| provider_variable_name(&id, field))
        .unwrap_or_else(|_| "ABBEY_PROVIDER_*".to_string())
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), ProviderConfigError> {
    if slot.replace(value).is_some() {
        return Err(ProviderConfigError::for_variable(
            name,
            "setting was provided more than once for the normalized provider identity",
        ));
    }
    Ok(())
}

fn parse_optional_absolute_path(
    name: &'static str,
    value: Option<&String>,
) -> Result<Option<PathBuf>, ProviderConfigError> {
    let Some(value) = value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    validate_absolute_path(name, value).map(Some)
}

fn parse_required_absolute_path(name: &str, value: &str) -> Result<PathBuf, ProviderConfigError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ProviderConfigError::for_variable(
            name,
            "path must not be blank",
        ));
    }
    validate_absolute_path(name, value)
}

fn validate_absolute_path(name: &str, value: &str) -> Result<PathBuf, ProviderConfigError> {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(ProviderConfigError::for_variable(
            name,
            "must be an absolute path without `.` or `..` components",
        ));
    }
    Ok(path)
}

fn parse_endpoint(name: &str, value: &str) -> Result<String, ProviderConfigError> {
    let value = parse_required_text(name, value, MAX_ENDPOINT_BYTES)?;
    let url = reqwest::Url::parse(&value).map_err(|_| {
        ProviderConfigError::for_variable(name, "must be an absolute HTTP or HTTPS URL")
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ProviderConfigError::for_variable(
            name,
            "must be an absolute HTTP or HTTPS URL",
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderConfigError::for_variable(
            name,
            "URL credentials, query strings, and fragments are forbidden",
        ));
    }
    Ok(value)
}

fn parse_optional_text(
    name: &'static str,
    value: Option<&String>,
    max_bytes: usize,
) -> Result<Option<String>, ProviderConfigError> {
    let Some(value) = value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    parse_required_text(name, value, max_bytes).map(Some)
}

fn parse_credential(name: &str, value: &str) -> Result<ProviderCredential, ProviderConfigError> {
    if value.trim().is_empty() {
        return Err(ProviderConfigError::for_variable(
            name,
            "credential must not be blank",
        ));
    }
    if value.trim() != value {
        return Err(ProviderConfigError::for_variable(
            name,
            "credential must not have leading or trailing whitespace",
        ));
    }
    if value.len() > MAX_CREDENTIAL_BYTES {
        return Err(ProviderConfigError::for_variable(
            name,
            "credential exceeds its configured size limit",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(ProviderConfigError::for_variable(
            name,
            "credential must not contain control characters",
        ));
    }
    Ok(ProviderCredential(value.to_string()))
}

fn parse_required_text(
    name: &str,
    value: &str,
    max_bytes: usize,
) -> Result<String, ProviderConfigError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ProviderConfigError::for_variable(
            name,
            "value must not be blank",
        ));
    }
    if value.len() > max_bytes {
        return Err(ProviderConfigError::for_variable(
            name,
            "value exceeds its configured size limit",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(ProviderConfigError::for_variable(
            name,
            "value must not contain control characters",
        ));
    }
    Ok(value.to_string())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
