//! Validated episode-gate environment and transport configuration.
use super::*;

impl EpisodeGateConfig {
    /// `Ok(None)` when the variable is unset or blank: the gate is off.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(raw) = std::env::var(CONFIG_ENV).ok() else {
            return Ok(None);
        };
        let path = raw.trim();
        if path.is_empty() {
            return Ok(None);
        }
        Self::from_path(Path::new(path)).map(Some)
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let path = absolute_path(path, CONFIG_ENV)?;
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("{CONFIG_ENV}: cannot read {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "{CONFIG_ENV}: {} is not a regular file",
                path.display()
            ));
        }
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(format!(
                "{CONFIG_ENV}: {} exceeds {MAX_CONFIG_BYTES} bytes",
                path.display()
            ));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{CONFIG_ENV}: cannot read {}: {error}", path.display()))?;
        let config = Self::from_json(&text)?;
        regular_file(&config.abi_cli, "abi_cli")?;
        regular_file(&config.token_file, "token_file")?;
        if let Some(ca_cert) = &config.ca_cert {
            regular_file(ca_cert, "ca_cert")?;
        }
        Ok(config)
    }

    /// Parse and validate. Every failure names the field; none echoes a value
    /// that could be a secret.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let raw: RawConfig = serde_json::from_str(text).map_err(|error| {
            format!(
                "{CONFIG_ENV}: invalid JSON at line {} column {}",
                error.line(),
                error.column()
            )
        })?;
        let abi_cli = absolute_path(Path::new(raw.abi_cli.trim()), "abi_cli")?;
        let token_file = absolute_path(Path::new(raw.token_file.trim()), "token_file")?;
        let ca_cert = raw
            .ca_cert
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| absolute_path(Path::new(value), "ca_cert"))
            .transpose()?;
        let endpoint = raw.endpoint.trim().to_string();
        check_endpoint_transport(&endpoint, ca_cert.is_some())?;
        let policy_version = raw.policy_version.trim().to_string();
        if !bounded_identifier(&policy_version, MAX_IDENTIFIER_LEN) {
            return Err("policy_version must be 1-64 chars of [a-z0-9_.-]".into());
        }
        if raw.contract_revision == 0 {
            return Err("contract_revision must be greater than zero".into());
        }
        let contract_digest = parse_digest(raw.contract_digest.trim())
            .ok_or("contract_digest must be 64 hexadecimal characters and not all zero")?;
        let service_principal = raw
            .service_principal
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_SERVICE_PRINCIPAL)
            .to_string();
        if !bounded_identifier(&service_principal, MAX_IDENTIFIER_LEN) {
            return Err("service_principal must be 1-64 chars of [a-z0-9_.-]".into());
        }
        let evidence_level = match raw.evidence_level.as_deref().map(str::trim) {
            None | Some("") | Some("c0" | "C0") => EvidenceLevel::C0,
            Some("c1" | "C1") => EvidenceLevel::C1,
            Some("c2" | "C2") => EvidenceLevel::C2,
            Some("c3" | "C3") => EvidenceLevel::C3,
            Some(_) => return Err("evidence_level must be one of c0, c1, c2, c3".into()),
        };
        let timeout_secs = match raw.timeout_secs {
            None => DEFAULT_TIMEOUT_SECS,
            Some(value) if (1..=MAX_TIMEOUT_SECS).contains(&value) => value,
            Some(_) => return Err(format!("timeout_secs must be 1-{MAX_TIMEOUT_SECS}")),
        };
        let guilds = match raw.guilds {
            None => None,
            Some(entries) => {
                let mut covered = BTreeSet::new();
                for entry in &entries {
                    let scoped_guild = entry.trim();
                    if scoped_guild.is_empty() || guild_ref_for(scoped_guild).is_none() {
                        return Err(
                            "guilds entries must be scoped guild ids that map to a ledger guild reference (for example discord:123)"
                                .into(),
                        );
                    }
                    covered.insert(scoped_guild.to_owned());
                }
                if covered.is_empty() {
                    return Err(
                        "guilds must name at least one scoped guild id or be omitted".into(),
                    );
                }
                Some(covered)
            }
        };
        Ok(Self {
            abi_cli,
            endpoint,
            token_file,
            ca_cert,
            policy_version,
            contract_revision: raw.contract_revision,
            contract_digest,
            service_principal,
            evidence_level,
            timeout_secs,
            guilds,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// Whether this scope is gated. Every scope is when `guilds` is absent.
    pub fn covers(&self, scoped_guild: &str) -> bool {
        self.guilds
            .as_ref()
            .is_none_or(|covered| covered.contains(scoped_guild))
    }

    /// How many scopes the config names, `None` for "every scope".
    pub fn coverage(&self) -> Option<usize> {
        self.guilds.as_ref().map(BTreeSet::len)
    }
}

/// Names-only existence check for a configured path; never reads it.
fn regular_file(path: &Path, field: &str) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!("{field}: {} is not a regular file", path.display())),
        Err(error) => Err(format!("{field}: cannot read {}: {error}", path.display())),
    }
}

/// The same rule the `abi` CLI enforces before it sends a bearer token, so a
/// misconfigured endpoint is a startup error naming the field rather than a
/// gateway "rejection" hours later: plain `http` only to loopback, any other
/// host needs `https` and a `ca_cert`.
fn check_endpoint_transport(endpoint: &str, has_ca_cert: bool) -> Result<(), String> {
    let (scheme, rest) = endpoint
        .split_once("://")
        .ok_or("endpoint must start with http:// or https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.strip_prefix('[').map_or_else(
        || {
            authority
                .rsplit_once(':')
                .map_or(authority, |(host, _)| host)
        },
        |bracketed| bracketed.split(']').next().unwrap_or(""),
    );
    let loopback = matches!(host, "127.0.0.1" | "::1" | "localhost");
    match scheme {
        "http" if loopback => Ok(()),
        "http" => Err("endpoint: non-loopback endpoints require https and ca_cert".into()),
        "https" if loopback || has_ca_cert => Ok(()),
        "https" => Err("endpoint: ca_cert is required for a non-loopback https endpoint".into()),
        _ => Err("endpoint must start with http:// or https://".into()),
    }
}

fn absolute_path(path: &Path, field: &str) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || !path.is_absolute()
        || path.components().any(|part| part == Component::ParentDir)
    {
        return Err(format!("{field} must be an absolute path without `..`"));
    }
    Ok(path.to_path_buf())
}
