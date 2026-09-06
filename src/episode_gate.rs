//! Constitutional episode gate client, default-off.
//!
//! The federation boundary (ABI constitution, 2026-08-22) makes WDBX the one
//! authority over durable episodes and the bots adapters that *propose* to
//! it. This module is abbey-bot's first caller of that gate. It is
//! deliberately narrow:
//!
//! - It records two things. A guild administrator's request to toggle
//!   adaptive learning goes in as a `proposal` event of an operation chain;
//!   approval, execution, and terminal events belong to the constitutional
//!   host, so nothing here claims the toggle was *authorized* by the ledger,
//!   and the local toggle applies immediately. Memory writes go in as
//!   `memory_candidate` events (amendment 2026-09-06): the adapter proposes
//!   *before* it writes locally and writes only on `appended`, so with the
//!   gate configured a refused or unreachable gate means nothing is stored
//!   and the person is told so. The ledger holds the SHA-256 of the payload
//!   and its size, never the payload.
//! - It never links the `abi` workspace. The gateway is reached by running
//!   the `abi` binary (`abi wdbx episode propose <write.json> --json`), the
//!   same "transcribe, never depend" stance as `wyhash.rs` and `wdbx.rs`.
//!   The write vocabulary below is a transcription of
//!   `abi-wdbx::v3::episode` pinned by `tests/fixtures/episode_write_proposal.json`
//!   and `tests/fixtures/episode_write_memory_candidate.json`, both generated
//!   from the canonical types.
//! - It is off unless `ABBEY_EPISODE_GATE_CONFIG` names a JSON file. Unset,
//!   the bot's behaviour is byte-identical to before this module existed.
//!   The file's optional `guilds` list scopes the gate to named guilds
//!   (deployment scoping, so one guild can go first); an uncovered scope is
//!   byte-identical to no gate.
//!
//! Privacy: every value that reaches the ledger is content-free by
//! construction. Principal ids are keyed hashes of scoped ids, never Discord
//! snowflakes; the write carries no message text, no channel, no timestamp.
//! The bearer token stays in the file the config names and only the `abi`
//! process reads it.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _};

use crate::wyhash;

/// Environment variable naming the JSON configuration file. Unset means off.
pub const CONFIG_ENV: &str = "ABBEY_EPISODE_GATE_CONFIG";

const DEFAULT_TIMEOUT_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 120;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_STDOUT_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;
const MAX_DETAIL_CHARS: usize = 240;
const DEFAULT_SERVICE_PRINCIPAL: &str = "abbey-service";
/// Mirrors the store's identifier bound for request, operation, and principal ids.
const MAX_IDENTIFIER_LEN: usize = 64;
/// Mirrors the store's bound for guild references.
const MAX_GUILD_REF_LEN: usize = 128;
const TOKEN_COST: u64 = 1;

const PRINCIPAL_SEED: u64 = 0x6162_6265_795f_6764; // "abbey_gd"
const OPERATION_SEED: u64 = 0x6c65_6172_6e5f_6f70; // "learn_op"
const REQUEST_SEED: u64 = 0x6c65_6172_6e5f_7271; // "learn_rq"
const MEMORY_OPERATION_SEED: u64 = 0x6d65_6d6f_7279_6f70; // "memoryop"
const MEMORY_REQUEST_SEED: u64 = 0x6d65_6d6f_7279_7271; // "memoryrq"

/// The environment the `abi` child inherits: locale and temp dir only, never
/// the bot's own credentials.
const ALLOWED_ENVIRONMENT: &[&str] = &[
    "HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "__CF_USER_TEXT_ENCODING",
];

static NEXT_WRITE_FILE: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Transcribed write vocabulary (subset abbey-bot emits).
// ---------------------------------------------------------------------------

/// Identity class of an actor. Transcribed from `abi-wdbx::v3::episode::ActorKind`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    HumanSubject,
    OrganizationOwner,
    GuildOwner,
    GuildAdministrator,
    GuildManager,
    Service,
}

/// Bounded, opaque authority identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActorRef {
    pub principal_id: String,
    pub kind: ActorKind,
}

/// Source class bound into the commitment.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeSource {
    Proposal,
    DiscordGuild,
    DiscordVoice,
    LocalRuntime,
}

/// Evidence maturity. The canonical type serializes its variant names
/// verbatim (`"C0"`), which is why this enum carries no `rename_all`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum EvidenceLevel {
    C0,
    C1,
    C2,
    C3,
}

/// Memory record class. Transcribed from `abi-wdbx::v3::episode::MemoryClass`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryClass {
    Fact,
    Experience,
    Embedding,
    Summary,
}

impl MemoryClass {
    /// Stable label, used in operation ids and log lines.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Experience => "experience",
            Self::Embedding => "embedding",
            Self::Summary => "summary",
        }
    }
}

/// Retention class. Transcribed from `abi-wdbx::v3::episode::RetentionClass`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    Session,
    Operational,
    Durable,
}

/// Content-free description of one memory write, field order identical to
/// the canonical struct. The store's rules (a `forgets` candidate carries zero
/// bytes and the all-zero commitment; every other one is nonzero; `dimension`
/// exactly for `embedding`) are enforced by WDBX, and [`memory_candidate_write`]
/// builds only shapes it admits.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub class: MemoryClass,
    pub retention: RetentionClass,
    pub payload_commitment: [u8; 32],
    pub payload_bytes: u64,
    pub dimension: Option<u16>,
    pub embedding_version: Option<String>,
    pub member_scoped: bool,
    pub supersedes: Option<[u8; 32]>,
    pub forgets: Option<[u8; 32]>,
}

/// Operation lifecycle event. This bot emits `proposal` and `memory_candidate`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EpisodeEvent {
    Proposal {
        requested_by: ActorRef,
        proposed_by: ActorRef,
    },
    MemoryCandidate {
        recorded_by: ActorRef,
        candidate: MemoryCandidate,
    },
}

/// One proposed append, field order identical to the canonical struct.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EpisodeWrite {
    pub request_id: String,
    pub operation_id: String,
    pub contract_revision: u64,
    pub contract_digest: [u8; 32],
    pub guild_ref: String,
    pub consent_epoch: Option<u64>,
    pub source_type: EpisodeSource,
    pub policy_version: String,
    pub evidence_level: EvidenceLevel,
    pub event: EpisodeEvent,
    pub token_cost: u64,
    pub expected_commitment: Option<[u8; 32]>,
    pub quiet: bool,
}

// ---------------------------------------------------------------------------
// Configuration.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    abi_cli: String,
    endpoint: String,
    token_file: String,
    #[serde(default)]
    ca_cert: Option<String>,
    policy_version: String,
    contract_revision: u64,
    contract_digest: String,
    #[serde(default)]
    service_principal: Option<String>,
    #[serde(default)]
    evidence_level: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Deployment scoping: the scoped guild ids (`discord:<id>`) the gate
    /// applies to. Absent means every scope, which is how the gate behaved
    /// before this key existed.
    #[serde(default)]
    guilds: Option<Vec<String>>,
}

/// Validated operator configuration. Holds paths and policy bindings only;
/// the bearer token is never read into this process, which is why a derived
/// `Debug` is safe here (see the `Backend` trap in `CLAUDE.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeGateConfig {
    abi_cli: PathBuf,
    endpoint: String,
    token_file: PathBuf,
    ca_cert: Option<PathBuf>,
    policy_version: String,
    contract_revision: u64,
    contract_digest: [u8; 32],
    service_principal: String,
    evidence_level: EvidenceLevel,
    timeout_secs: u64,
    /// `None` covers every scope. Otherwise only these scoped guild ids are
    /// gated: an uncovered scope behaves exactly as if no gate were
    /// configured. This is deployment scoping (which guilds this bot
    /// instance proposes for), not a hole in the constitution: the ledger's
    /// own per-guild policy still decides what it admits.
    guilds: Option<BTreeSet<String>>,
}

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

pub fn parse_digest(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    (digest != [0; 32]).then_some(digest)
}

/// Same rule the store applies, so a bad id fails here with a named reason
/// instead of as a bare `invalid_argument` from the gateway.
pub fn bounded_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

// ---------------------------------------------------------------------------
// Pure builders.
// ---------------------------------------------------------------------------

/// The ledger's guild reference for a scoped guild id. The store admits only
/// `[a-z0-9_.-]`, so `discord:123` becomes `discord-123`; the operator's
/// gateway policy must key the guild under that exact string.
pub fn guild_ref_for(scoped_guild: &str) -> Option<String> {
    let guild_ref = scoped_guild.to_ascii_lowercase().replace(':', "-");
    bounded_identifier(&guild_ref, MAX_GUILD_REF_LEN).then_some(guild_ref)
}

/// Content-free principal for the requesting administrator: a keyed hash of
/// the scoped guild and user, so the same person is stable within a guild
/// and unlinkable across guilds, and no Discord id reaches the ledger.
pub fn requester_principal(scoped_guild: &str, scoped_user: &str) -> String {
    let mut input = Vec::with_capacity(scoped_guild.len() + scoped_user.len() + 1);
    input.extend_from_slice(scoped_guild.as_bytes());
    input.push(0x1f);
    input.extend_from_slice(scoped_user.as_bytes());
    format!("admin-{:016x}", wyhash::hash(PRINCIPAL_SEED, &input))
}

fn keyed_id(prefix: &str, seed: u64, guild_ref: &str, now: u64, nonce: u64) -> String {
    let mut input = Vec::with_capacity(guild_ref.len() + 17);
    input.extend_from_slice(guild_ref.as_bytes());
    input.push(0x1f);
    input.extend_from_slice(&now.to_le_bytes());
    input.extend_from_slice(&nonce.to_le_bytes());
    format!("{prefix}-{:016x}", wyhash::hash(seed, &input))
}

/// What the command hands over: plain values, no Discord types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningToggleRequest {
    pub scoped_guild: String,
    pub scoped_user: String,
    /// Wall clock from `runtime::now()`; the pure path never reads it.
    pub now: u64,
    /// Per-process counter so two toggles in one second stay distinct.
    pub nonce: u64,
}

/// Build the proposal write for a learning toggle. `Err` names the reason the
/// guild cannot be addressed; nothing was sent.
pub fn learning_toggle_proposal(
    config: &EpisodeGateConfig,
    request: &LearningToggleRequest,
) -> Result<EpisodeWrite, String> {
    let guild_ref = guild_ref_for(&request.scoped_guild)
        .ok_or_else(|| "scoped guild id does not map to a ledger guild reference".to_string())?;
    let requester = requester_principal(&request.scoped_guild, &request.scoped_user);
    if requester == config.service_principal {
        return Err("requester principal collides with the service principal".into());
    }
    Ok(EpisodeWrite {
        request_id: keyed_id("req", REQUEST_SEED, &guild_ref, request.now, request.nonce),
        operation_id: keyed_id(
            "learning-toggle",
            OPERATION_SEED,
            &guild_ref,
            request.now,
            request.nonce,
        ),
        contract_revision: config.contract_revision,
        contract_digest: config.contract_digest,
        guild_ref,
        consent_epoch: None,
        source_type: EpisodeSource::DiscordGuild,
        policy_version: config.policy_version.clone(),
        evidence_level: config.evidence_level,
        event: EpisodeEvent::Proposal {
            requested_by: ActorRef {
                principal_id: requester,
                kind: ActorKind::GuildAdministrator,
            },
            proposed_by: ActorRef {
                principal_id: config.service_principal.clone(),
                kind: ActorKind::Service,
            },
        },
        token_cost: TOKEN_COST,
        expected_commitment: None,
        quiet: false,
    })
}

/// What a memory write site hands over: the scope, the class, and the
/// canonical payload bytes. The bytes never leave this process; only their
/// SHA-256 and length reach the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryCandidateRequest {
    pub scoped_guild: String,
    pub class: MemoryClass,
    pub retention: RetentionClass,
    /// Canonical payload bytes; empty exactly when `forgets` is set.
    pub payload: Vec<u8>,
    /// True when the record is guild-plus-user isolated.
    pub member_scoped: bool,
    /// Episode digest of the candidate this replaces.
    pub supersedes: Option<[u8; 32]>,
    /// Episode digest of the candidate this erases.
    pub forgets: Option<[u8; 32]>,
    pub now: u64,
    pub nonce: u64,
}

/// Build the memory-candidate write. `Err` names the reason nothing can be
/// proposed; nothing was sent. The bot never proposes the `embedding` class:
/// the projection's vector is derived deterministically from the fact, so the
/// fact candidate already commits to it.
pub fn memory_candidate_write(
    config: &EpisodeGateConfig,
    request: &MemoryCandidateRequest,
) -> Result<EpisodeWrite, String> {
    use sha2::{Digest as _, Sha256};

    let guild_ref = guild_ref_for(&request.scoped_guild)
        .ok_or_else(|| "scoped guild id does not map to a ledger guild reference".to_string())?;
    if request.class == MemoryClass::Embedding {
        return Err("the bot never proposes embedding candidates".into());
    }
    let (payload_commitment, payload_bytes) = if request.forgets.is_some() {
        if request.supersedes.is_some() || !request.payload.is_empty() {
            return Err("a forget candidate carries no payload and no supersedes edge".into());
        }
        ([0_u8; 32], 0)
    } else {
        if request.payload.is_empty() {
            return Err("a memory candidate needs payload bytes to commit to".into());
        }
        let digest: [u8; 32] = Sha256::digest(&request.payload).into();
        (
            digest,
            u64::try_from(request.payload.len()).map_err(|_| "payload too large".to_string())?,
        )
    };
    Ok(EpisodeWrite {
        request_id: keyed_id(
            "req",
            MEMORY_REQUEST_SEED,
            &guild_ref,
            request.now,
            request.nonce,
        ),
        operation_id: keyed_id(
            &format!("memory-{}", request.class.label()),
            MEMORY_OPERATION_SEED,
            &guild_ref,
            request.now,
            request.nonce,
        ),
        contract_revision: config.contract_revision,
        contract_digest: config.contract_digest,
        guild_ref,
        consent_epoch: None,
        source_type: EpisodeSource::DiscordGuild,
        policy_version: config.policy_version.clone(),
        evidence_level: config.evidence_level,
        event: EpisodeEvent::MemoryCandidate {
            recorded_by: ActorRef {
                principal_id: config.service_principal.clone(),
                kind: ActorKind::Service,
            },
            candidate: MemoryCandidate {
                class: request.class,
                retention: request.retention,
                payload_commitment,
                payload_bytes,
                dimension: None,
                embedding_version: None,
                member_scoped: request.member_scoped,
                supersedes: request.supersedes,
                forgets: request.forgets,
            },
        },
        token_cost: TOKEN_COST,
        expected_commitment: None,
        quiet: false,
    })
}

/// Content-free counters `inspect_status` shows so a refusing or unreachable
/// gate is visible (decision 78) rather than silent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GateCounters {
    pub appended: u64,
    pub rejected: u64,
    pub unavailable: u64,
    /// Local deletions of facts the ledger never admitted (stored before the
    /// gate existed), so no tombstone edge could be proposed.
    pub ungated_forgets: u64,
    /// Scopes the config names (`guilds`), `None` when every scope is gated.
    pub covered_guilds: Option<usize>,
}

// ---------------------------------------------------------------------------
// The gate: run `abi wdbx episode propose` and report what happened.
// ---------------------------------------------------------------------------

/// What the ledger said. Every string is content-free: digests, sequence
/// numbers, and the gateway's closed reason labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateOutcome {
    /// The gateway appended the proposal.
    Appended {
        digest_hex: String,
        sequence: String,
    },
    /// The gateway answered and refused (exit 1: a gRPC code plus the store's
    /// reason label, e.g. `FailedPrecondition: learning_disabled`).
    Rejected { detail: String },
    /// No answer: the `abi` binary could not start, timed out, or returned
    /// something this module does not understand. Nothing is known about
    /// the ledger's state.
    Unavailable { detail: String },
}

impl GateOutcome {
    /// One log line, content-free.
    pub fn summary(&self) -> String {
        match self {
            Self::Appended {
                digest_hex,
                sequence,
            } => format!("appended sequence={sequence} digest={digest_hex}"),
            Self::Rejected { detail } => format!("rejected: {detail}"),
            Self::Unavailable { detail } => format!("unavailable: {detail}"),
        }
    }
}

pub struct EpisodeGate {
    service: std::sync::OnceLock<crate::service::OperationRegistry>,
    config: EpisodeGateConfig,
    nonce: AtomicU64,
    appended: AtomicU64,
    rejected: AtomicU64,
    unavailable: AtomicU64,
    ungated_forgets: AtomicU64,
}

impl EpisodeGate {
    pub fn new(config: EpisodeGateConfig) -> Self {
        Self {
            service: std::sync::OnceLock::new(),
            config,
            nonce: AtomicU64::new(0),
            appended: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            unavailable: AtomicU64::new(0),
            ungated_forgets: AtomicU64::new(0),
        }
    }

    pub fn attach_service(&self, registry: crate::service::OperationRegistry) {
        assert!(
            self.service.set(registry).is_ok(),
            "episode service attached once"
        );
    }
    pub fn counters(&self) -> GateCounters {
        GateCounters {
            appended: self.appended.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            unavailable: self.unavailable.load(Ordering::Relaxed),
            ungated_forgets: self.ungated_forgets.load(Ordering::Relaxed),
            covered_guilds: self.config.coverage(),
        }
    }

    /// Whether this scope is gated; see [`EpisodeGateConfig::covers`].
    pub fn covers(&self, scoped_guild: &str) -> bool {
        self.config.covers(scoped_guild)
    }

    /// A fact the ledger never admitted was deleted locally; visible, not silent.
    pub fn note_ungated_forget(&self) {
        self.ungated_forgets.fetch_add(1, Ordering::Relaxed);
    }

    fn count(&self, outcome: &GateOutcome) {
        let counter = match outcome {
            GateOutcome::Appended { .. } => &self.appended,
            GateOutcome::Rejected { .. } => &self.rejected,
            GateOutcome::Unavailable { .. } => &self.unavailable,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Propose one memory write. The caller writes locally only on
    /// [`GateOutcome::Appended`]; the returned digest is the receipt it keys
    /// the local record by. Logs are content-free (class and outcome labels).
    pub async fn record_memory_candidate(&self, request: MemoryCandidateRequest) -> GateOutcome {
        let class = request.class.label();
        let edge = if request.forgets.is_some() {
            "forgets"
        } else if request.supersedes.is_some() {
            "supersedes"
        } else {
            "new"
        };
        let outcome = match memory_candidate_write(&self.config, &request) {
            Ok(write) => self.propose(&write).await,
            Err(detail) => GateOutcome::Unavailable { detail },
        };
        self.count(&outcome);
        match &outcome {
            GateOutcome::Appended { .. } => {
                tracing::info!(class, edge, outcome = %outcome.summary(), "episode gate: memory candidate admitted");
            }
            GateOutcome::Rejected { .. } | GateOutcome::Unavailable { .. } => {
                tracing::warn!(class, edge, outcome = %outcome.summary(), "episode gate: memory candidate not admitted; nothing written");
            }
        }
        outcome
    }

    pub fn config(&self) -> &EpisodeGateConfig {
        &self.config
    }

    pub fn next_nonce(&self) -> u64 {
        self.nonce.fetch_add(1, Ordering::Relaxed)
    }

    /// Mirror a learning toggle into the ledger and log the outcome. Never
    /// fails the caller: the toggle already applied locally.
    pub async fn record_learning_toggle(&self, request: LearningToggleRequest) -> GateOutcome {
        let outcome = match learning_toggle_proposal(&self.config, &request) {
            Ok(write) => self.propose(&write).await,
            Err(detail) => GateOutcome::Unavailable { detail },
        };
        self.count(&outcome);
        match &outcome {
            GateOutcome::Appended { .. } => {
                tracing::info!(outcome = %outcome.summary(), "episode gate: learning toggle proposed");
            }
            GateOutcome::Rejected { .. } | GateOutcome::Unavailable { .. } => {
                tracing::warn!(outcome = %outcome.summary(), "episode gate: learning toggle not recorded");
            }
        }
        outcome
    }

    /// Send one write through `abi wdbx episode propose --json`.
    pub async fn propose(&self, write: &EpisodeWrite) -> GateOutcome {
        let encoded = match serde_json::to_vec(write) {
            Ok(bytes) => bytes,
            Err(_) => {
                return GateOutcome::Unavailable {
                    detail: "the write could not be serialized".into(),
                };
            }
        };
        let created = if let Some(registry) = self.service.get() {
            match registry.blocking_result(crate::service::OperationKind::Episode, move || {
                WriteFile::create(&encoded)
            }) {
                Ok(result) => result
                    .await
                    .unwrap_or_else(|_| Err("episode write preparation interrupted".into())),
                Err(_) => Err("service is shutting down; episode was not started".into()),
            }
        } else {
            WriteFile::create(&encoded)
        };
        let file = match created {
            Ok(file) => file,
            Err(detail) => return GateOutcome::Unavailable { detail },
        };
        let mut args: Vec<OsString> = vec![
            "wdbx".into(),
            "episode".into(),
            "propose".into(),
            file.path.as_os_str().to_owned(),
            "--json".into(),
            "--endpoint".into(),
            self.config.endpoint.clone().into(),
            "--token-file".into(),
            self.config.token_file.as_os_str().to_owned(),
        ];
        if let Some(ca_cert) = &self.config.ca_cert {
            args.push("--ca-cert".into());
            args.push(ca_cert.as_os_str().to_owned());
        }
        if let Some(registry) = self.service.get() {
            let program = self.config.abi_cli.clone();
            let timeout = self.config.timeout_secs;
            let cancel = registry.cancellation();
            match registry.spawn_result(crate::service::OperationKind::Episode, async move {
                let _file_owner = file;
                run_abi_owned(&program, &args, timeout, Some(cancel)).await
            }) {
                Ok(result) => result.await.unwrap_or(GateOutcome::Unavailable {
                    detail: "episode operation interrupted".into(),
                }),
                Err(_) => GateOutcome::Unavailable {
                    detail: "service is shutting down; episode was not started".into(),
                },
            }
        } else {
            run_abi(&self.config.abi_cli, &args, self.config.timeout_secs).await
        }
    }
}

/// The write file handed to `abi`: owner-only, removed on drop.
struct WriteFile {
    path: PathBuf,
}

impl WriteFile {
    fn create(bytes: &[u8]) -> Result<Self, String> {
        let name = format!(
            "abbey-episode-{}-{}.json",
            std::process::id(),
            NEXT_WRITE_FILE.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|error| format!("could not create the write file: {error}"))?;
        std::io::Write::write_all(&mut file, bytes)
            .map_err(|error| format!("could not write the write file: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for WriteFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn run_abi(program: &Path, args: &[OsString], timeout_secs: u64) -> GateOutcome {
    run_abi_owned(program, args, timeout_secs, None).await
}
async fn run_abi_owned(
    program: &Path,
    args: &[OsString],
    timeout_secs: u64,
    cancel: Option<tokio_util::sync::CancellationToken>,
) -> GateOutcome {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| {
            ALLOWED_ENVIRONMENT
                .iter()
                .any(|allowed| name == OsStr::new(allowed))
        })
        .collect();
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(environment.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return GateOutcome::Unavailable {
                detail: format!("could not start the abi binary: {error}"),
            };
        }
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return GateOutcome::Unavailable {
            detail: "the abi child had no output pipes".into(),
        };
    };
    let operation = async {
        let (stdout, stderr, status) = tokio::try_join!(
            read_capped(stdout, MAX_STDOUT_BYTES),
            read_capped(stderr, MAX_STDERR_BYTES),
            async { child.wait().await.map_err(|error| error.to_string()) },
        )?;
        Ok::<_, String>((stdout, stderr, status))
    };
    let completed = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(timeout_secs), operation) => Some(result),
        () = async { match cancel { Some(cancel) => cancel.cancelled().await, None => std::future::pending().await } } => None,
    };
    let (stdout, stderr, status) = match completed {
        Some(Ok(Ok(result))) => result,
        failure => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return GateOutcome::Unavailable {
                detail: match failure {
                    Some(Ok(Err(detail))) => detail,
                    Some(Err(_)) => format!("the abi binary did not answer within {timeout_secs}s"),
                    None => "the abi operation was cancelled during shutdown".into(),
                    Some(Ok(Ok(_))) => unreachable!("success handled above"),
                },
            };
        }
    };
    classify(status.code(), &stdout, &stderr)
}

/// Exit 0 with a JSON object is an append; exit 1 is the gateway's refusal;
/// anything else is unknown.
pub fn classify(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> GateOutcome {
    match code {
        Some(0) => parse_appended(stdout),
        Some(1) => GateOutcome::Rejected {
            detail: first_line(stderr),
        },
        Some(other) => GateOutcome::Unavailable {
            detail: format!(
                "the abi binary exited with status {other}: {}",
                first_line(stderr)
            ),
        },
        None => GateOutcome::Unavailable {
            detail: "the abi binary was killed by a signal".into(),
        },
    }
}

fn parse_appended(stdout: &[u8]) -> GateOutcome {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        return GateOutcome::Unavailable {
            detail: "the abi binary printed something other than one JSON object".into(),
        };
    };
    let field = |name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_owned);
    match (
        field("decision"),
        field("episode_digest"),
        field("sequence"),
    ) {
        (Some(decision), Some(digest_hex), Some(sequence)) if decision == "appended" => {
            GateOutcome::Appended {
                digest_hex,
                sequence,
            }
        }
        (Some(decision), _, _) => GateOutcome::Unavailable {
            detail: format!("unexpected decision {decision}"),
        },
        _ => GateOutcome::Unavailable {
            detail: "the abi binary's JSON lacked decision, episode_digest, or sequence".into(),
        },
    }
}

fn first_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.is_empty() {
        return "no diagnostic".into();
    }
    line.chars().take(MAX_DETAIL_CHARS).collect()
}

async fn read_capped(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > limit {
            return Err(format!("the abi binary's output exceeded {limit} bytes"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(test)]
mod acceptance;
#[cfg(test)]
mod tests;
