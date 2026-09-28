//! Pure canonical projection authority. Native controls invalidate in place;
//! only the future admission coordinator may add/delete admitted payloads.
use super::*;
use sha2::{Digest, Sha256};

pub const MAX_ROWS: usize = 20_000;
pub const MAX_SOURCES: usize = 20_000;
pub const MAX_SCOPES: usize = 10_000;
pub const MAX_ATTEMPTS: usize = 4_096;
pub const MAX_TERMINALS: usize = 256;
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024;
pub const MAX_TOTAL_PAYLOAD: usize = 32 * 1024 * 1024;
pub const MAX_METADATA: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WorkSourceKey {
    Task {
        project: u64,
        id: u64,
    },
    Decision {
        project: u64,
        id: u64,
    },
    Preference {
        scope: WorkScope,
        delivery: u64,
        actor: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceVersion {
    pub revision: u64,
    pub generation: u64,
    pub recall_enabled: bool,
    pub recall_disabled_exhausted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeRecallControl {
    // Compact wire names keep the entire typed entry <=128 bytes even with
    // maximum-width Discord IDs and an exhausted generation counter.
    #[serde(rename = "gen")]
    pub generation: u64,
    #[serde(rename = "off")]
    pub recall_disabled_exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEvidencePayload {
    pub version: u8,
    pub source: WorkSourceKey,
    pub revision: u64,
    pub generation: u64,
    pub scope: WorkScope,
    pub contributing_projects: BTreeSet<u64>,
    pub delivered_source_digest: Option<String>,
    pub actor: Option<u64>,
    pub at: Option<u64>,
    pub native_digest: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkAdmission {
    Appended {
        digest_hex: String,
        scoped_guild: String,
    },
    Uncovered {
        scoped_guild: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmittedWorkEvidence {
    pub id: u64,
    pub payload: WorkEvidencePayload,
    pub payload_digest: String,
    pub admission: WorkAdmission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptState {
    Prepared,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionOperation {
    Add { row: u64 },
    Forget { row: u64 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAttempt {
    pub id: u64,
    pub source: WorkSourceKey,
    pub operation: ProjectionOperation,
    pub revision: u64,
    pub generation: u64,
    pub payload_digest: String,
    pub payload_bytes: usize,
    pub at: u64,
    pub nonce: u64,
    pub config_digest: String,
    pub state: AttemptState,
    /// Content-free proof observed remotely when local settlement could not commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_admission: Option<WorkAdmission>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalOutcome {
    Appended,
    Uncovered,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSummary {
    pub attempt: u64,
    pub at: u64,
    pub outcome: TerminalOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkRecallState {
    pub schema_version: u32,
    pub sequence: u64,
    pub projection_revision: u64,
    pub revision_exhausted: bool,
    #[serde(with = "entries")]
    pub records: BTreeMap<u64, AdmittedWorkEvidence>,
    #[serde(with = "entries")]
    pub source_versions: BTreeMap<WorkSourceKey, SourceVersion>,
    #[serde(with = "entries")]
    pub scope_controls: BTreeMap<WorkScope, ScopeRecallControl>,
    #[serde(with = "entries")]
    pub attempts: BTreeMap<u64, ProjectionAttempt>,
    pub terminal: std::collections::VecDeque<TerminalSummary>,
}
impl Default for WorkRecallState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            projection_revision: 0,
            revision_exhausted: false,
            records: BTreeMap::new(),
            source_versions: BTreeMap::new(),
            scope_controls: BTreeMap::new(),
            attempts: BTreeMap::new(),
            terminal: Default::default(),
        }
    }
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, WorkError> {
    serde_json::to_vec(value).map_err(|_| WorkError::Invalid)
}
fn scope_valid(scope: &WorkScope) -> bool {
    match scope {
        WorkScope::Personal { owner } => *owner != 0,
        WorkScope::Team { guild, channel } => *guild != 0 && *channel != 0,
    }
}
impl WorkScope {
    /// Exact existing gate identity; channel and project are never episode guilds.
    pub fn recall_gate_scope(&self) -> (String, bool) {
        match self {
            Self::Personal { owner } => (format!("discord:dm:{owner}"), true),
            Self::Team { guild, .. } => (format!("discord:{guild}"), false),
        }
    }
}
impl WorkSourceKey {
    fn valid(&self) -> bool {
        match self {
            Self::Task { project, id } | Self::Decision { project, id } => {
                *project != 0 && *id != 0
            }
            Self::Preference {
                scope,
                delivery,
                actor,
            } => scope_valid(scope) && *delivery != 0 && *actor != 0,
        }
    }
}
impl WorkEvidencePayload {
    pub fn encoded(&self) -> Result<Vec<u8>, WorkError> {
        let encoded = bytes(self)?;
        if encoded.len() > MAX_PAYLOAD_BYTES || self.text.chars().count() > 1024 {
            return Err(WorkError::Full);
        }
        if self.version != 1
            || !self.source.valid()
            || !scope_valid(&self.scope)
            || !valid_digest(&self.native_digest)
            || self.text.is_empty()
            || self.contributing_projects.is_empty()
            || self.contributing_projects.len() > 128
            || self.contributing_projects.contains(&0)
            || self
                .delivered_source_digest
                .as_ref()
                .is_some_and(|v| !valid_digest(v))
        {
            return Err(WorkError::Invalid);
        }
        let valid = match &self.source {
            WorkSourceKey::Task { project, .. } | WorkSourceKey::Decision { project, .. } => {
                self.contributing_projects == BTreeSet::from([*project])
                    && self.delivered_source_digest.is_none()
            }
            WorkSourceKey::Preference { scope, actor, .. } => {
                scope == &self.scope
                    && self.actor == Some(*actor)
                    && self.delivered_source_digest.is_some()
            }
        };
        if !valid {
            return Err(WorkError::Invalid);
        }
        Ok(encoded)
    }
}
impl AdmittedWorkEvidence {
    fn validate(&self) -> Result<(), WorkError> {
        if self.id == 0 || digest(&self.payload.encoded()?) != self.payload_digest {
            return Err(WorkError::Invalid);
        }
        let expected = self.payload.scope.recall_gate_scope().0;
        let scope = match &self.admission {
            WorkAdmission::Appended {
                digest_hex,
                scoped_guild,
            } => {
                if !valid_digest(digest_hex) {
                    return Err(WorkError::Invalid);
                }
                scoped_guild
            }
            WorkAdmission::Uncovered { scoped_guild } => scoped_guild,
        };
        if *scope != expected {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }
}

// Typed maps encode as ordered entries and reject duplicates rather than letting
// serde's map collection silently overwrite authority on load.
pub(super) mod entries {
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    #[derive(Serialize, Deserialize)]
    struct Entry<K, V> {
        key: K,
        value: V,
    }
    pub fn serialize<S: serde::Serializer, K: Serialize, V: Serialize>(
        map: &BTreeMap<K, V>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter()
            .map(|(key, value)| Entry { key, value })
            .collect::<Vec<_>>()
            .serialize(s)
    }
    pub fn deserialize<
        'de,
        D: serde::Deserializer<'de>,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    >(
        d: D,
    ) -> Result<BTreeMap<K, V>, D::Error> {
        let mut map = BTreeMap::new();
        for Entry { key, value } in Vec::<Entry<K, V>>::deserialize(d)? {
            if map.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate recall key"));
            }
        }
        Ok(map)
    }
}
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub enum RecallAudience {
    Private { principal: u64 },
    Channel,
}

mod deletion;
mod lifecycle;
mod sources;
#[cfg(test)]
mod tests;
mod validation;
