//! A WDBX-v1-compatible local store for Abbey's semantic memory.
//!
//! abi owns the canonical WDBX vector store (`abi-wdbx`); this module speaks the
//! same on-disk format so a file written here loads there and vice versa, but
//! it is a deliberately small transcription — no segments, no manifest, no
//! audit chain. abbey-bot takes no dependency on abi (see CLAUDE.md).
//!
//! ## Format (`# ABI-WDBX v1`)
//!
//! First line is the header, then one JSON object per line:
//!
//! ```text
//! # ABI-WDBX v1
//! {"type":"kv","key":"...","value":"..."}
//! {"type":"vector","id":1,"values":[0.1,-0.2,...]}
//! ```
//!
//! `value` is an opaque string — often double-encoded JSON, which is how the
//! memory facts below are stored. Record types this module does not model
//! (`block`, `spatial`, `temporal_*`, anything abi adds later) are kept as raw
//! lines and written back verbatim, so a file shared with abi round-trips
//! without loss. The one thing dropped is a `# checksum:` trailer: it covers
//! the byte content above it, this module cannot maintain it across edits, and
//! abi treats its absence as "no checksum" rather than an error.
//!
//! ## Namespacing
//!
//! [`Recall`] is the memory layer over the store. Every fact is keyed by the
//! *scoped* guild id — `mem:{scoped_guild_id}:{vector_id}` — and inference/tool
//! reads additionally filter by the scoped Discord user. Guild isolation keeps
//! servers apart; guild-plus-user isolation is the privacy boundary between
//! members of the same server. The only guild-wide query is a private helper
//! used by storage-level tests, so production callers cannot accidentally opt
//! back into cross-member recall.
//!
//! Nothing here imports serenity or poise, and nothing reads the clock: `at`
//! timestamps come from the caller.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::embedding::{cosine, text_embedding};

/// Magic first line of a store file. Identical to abi's `SEGMENT_HEADER`.
pub const HEADER: &str = "# ABI-WDBX v1";

/// Prefix of abi's optional checksum trailer, which this module drops on parse.
const CHECKSUM_PREFIX: &str = "# checksum:";

/// The in-memory projection contained a value the JSON wire format cannot
/// represent. Deliberately content-free so callers cannot leak state through
/// diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WdbxEncodeError;

/// Why a store could not be parsed, loaded, or saved.
#[derive(Debug)]
pub enum WdbxError {
    /// The first line was not [`HEADER`].
    MissingHeader {
        /// What the first line actually was (empty for an empty input).
        found: String,
    },
    /// A body line was not a well-formed record.
    MalformedLine {
        /// 1-based line number in the file (the header is line 1).
        line: usize,
        /// What was wrong, in one sentence.
        reason: String,
    },
    /// The filesystem refused.
    Io {
        /// What was being attempted, e.g. `"read"`, `"write"`, `"rename"`.
        op: &'static str,
        /// The path involved.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl fmt::Display for WdbxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHeader { found } if found.is_empty() => {
                write!(f, "WDBX store is empty; expected header `{HEADER}`")
            }
            Self::MissingHeader { found } => {
                write!(f, "WDBX store header is `{found}`, expected `{HEADER}`")
            }
            Self::MalformedLine { line, reason } => {
                write!(f, "WDBX store line {line} is malformed: {reason}")
            }
            Self::Io { op, path, source } => {
                write!(f, "WDBX store {op} failed for {path}: {source}")
            }
        }
    }
}

impl std::error::Error for WdbxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Serde shape of a `kv` line, used for rendering.
#[derive(Serialize)]
struct KvLine<'a> {
    r#type: &'static str,
    key: &'a str,
    value: &'a str,
}

/// Serde shape of a `vector` line, used for rendering.
#[derive(Serialize)]
struct VectorLine<'a> {
    r#type: &'static str,
    id: u64,
    values: &'a [f32],
}

/// An in-memory WDBX store: key/value entries, vectors, and the raw lines of
/// any record type this module does not model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WdbxStore {
    kv: BTreeMap<String, String>,
    /// Vectors in file order. Ids are unique; lookups are linear, which is fine
    /// for the sizes a Discord guild's memory reaches.
    vectors: Vec<(u64, Vec<f32>)>,
    /// The next id [`insert_vector`](Self::insert_vector) hands out — always
    /// above every id ever seen, so ids stay monotonic even after removals.
    next_id: u64,
    /// Unmodelled record lines (and non-checksum comments), verbatim, in order.
    unknown: Vec<String>,
}

impl WdbxStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    /// Parse a whole file's text.
    ///
    /// Strict about the header and about every body line: a line that is not
    /// a JSON object with a string `type` is an error, and so is a `kv` or
    /// `vector` record with the wrong field shapes. Unknown `type`s are kept
    /// verbatim. Never panics on malformed input.
    pub fn parse(text: &str) -> Result<Self, WdbxError> {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or_default().trim_end_matches('\r');
        if header != HEADER {
            return Err(WdbxError::MissingHeader {
                found: header.to_string(),
            });
        }

        let mut store = Self::new();
        for (index, raw) in lines.enumerate() {
            let line_no = index + 2;
            let line = raw.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            if line.starts_with(CHECKSUM_PREFIX) {
                continue;
            }
            if line.starts_with('#') {
                store.unknown.push(line.to_string());
                continue;
            }
            store.parse_record(line, line_no)?;
        }
        Ok(store)
    }

    fn parse_record(&mut self, line: &str, line_no: usize) -> Result<(), WdbxError> {
        let malformed = |reason: String| WdbxError::MalformedLine {
            line: line_no,
            reason,
        };
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| malformed(format!("not JSON ({e})")))?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("not a JSON object".to_string()))?;
        let type_name = object
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| malformed("missing string field `type`".to_string()))?;

        match type_name {
            "kv" => {
                let key = string_field(object, "key").map_err(&malformed)?;
                let value = string_field(object, "value").map_err(&malformed)?;
                self.kv.insert(key, value);
            }
            "vector" => {
                let id = object
                    .get("id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| malformed("vector `id` is not a non-negative integer".into()))?;
                let items = object
                    .get("values")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| malformed("vector `values` is not an array".into()))?;
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    let n = item.as_f64().ok_or_else(|| {
                        malformed(format!("vector element {item} is not a number"))
                    })?;
                    // The format stores f32; f64 is only the JSON transport type.
                    values.push(n as f32);
                }
                if self.vectors.iter().any(|(existing, _)| *existing == id) {
                    return Err(malformed(format!("duplicate vector id {id}")));
                }
                self.vectors.push((id, values));
                self.next_id = self.next_id.max(id.saturating_add(1));
            }
            _ => self.unknown.push(line.to_string()),
        }
        Ok(())
    }

    /// Render the store as file text: header, vectors, key/value entries, then
    /// every preserved unknown line in its original order. Ends with a newline.
    #[must_use]
    pub fn render(&self) -> String {
        self.try_render().unwrap_or_default()
    }

    /// Render the exact WDBX-v1 wire form, reporting records that JSON cannot
    /// represent instead of silently publishing a partial projection.
    pub fn try_render(&self) -> Result<String, WdbxEncodeError> {
        let mut out = String::with_capacity(64 + self.vectors.len() * 320 + self.kv.len() * 96);
        out.push_str(HEADER);
        out.push('\n');
        for (id, values) in &self.vectors {
            if values.iter().any(|value| !value.is_finite()) {
                return Err(WdbxEncodeError);
            }
            let line = VectorLine {
                r#type: "vector",
                id: *id,
                values,
            };
            out.push_str(&serde_json::to_string(&line).map_err(|_| WdbxEncodeError)?);
            out.push('\n');
        }
        for (key, value) in &self.kv {
            let line = KvLine {
                r#type: "kv",
                key,
                value,
            };
            out.push_str(&serde_json::to_string(&line).map_err(|_| WdbxEncodeError)?);
            out.push('\n');
        }
        for line in &self.unknown {
            out.push_str(line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Read and parse a store file.
    pub fn load(path: &Path) -> Result<Self, WdbxError> {
        let text = fs::read_to_string(path).map_err(|source| WdbxError::Io {
            op: "read",
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Write the store atomically: render to `<path>.tmp`, then rename over
    /// `path`, so a crash mid-write leaves the previous file intact.
    pub fn save(&self, path: &Path) -> Result<(), WdbxError> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = Path::new(&tmp);
        fs::write(tmp, self.render()).map_err(|source| WdbxError::Io {
            op: "write",
            path: tmp.display().to_string(),
            source,
        })?;
        fs::rename(tmp, path).map_err(|source| WdbxError::Io {
            op: "rename",
            path: path.display().to_string(),
            source,
        })
    }

    /// Set a key/value entry, replacing any previous value.
    pub fn put_kv(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.kv.insert(key.into(), value.into());
    }

    /// Read a key/value entry.
    #[must_use]
    pub fn get_kv(&self, key: &str) -> Option<&str> {
        self.kv.get(key).map(String::as_str)
    }

    /// Remove a key/value entry, returning the old value.
    pub fn remove_kv(&mut self, key: &str) -> Option<String> {
        self.kv.remove(key)
    }

    /// Iterate key/value entries whose key starts with `prefix`, in key order.
    pub fn kv_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.kv
            .range(prefix.to_string()..)
            .take_while(move |(key, _)| key.starts_with(prefix))
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// Store a vector and return its new id (strictly increasing).
    pub fn insert_vector(&mut self, values: Vec<f32>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.vectors.push((id, values));
        id
    }

    /// Look up a vector by id.
    #[must_use]
    pub fn vector(&self, id: u64) -> Option<&[f32]> {
        self.vectors
            .iter()
            .find(|(existing, _)| *existing == id)
            .map(|(_, values)| values.as_slice())
    }

    /// Remove a vector by id. Returns whether it existed.
    pub fn remove_vector(&mut self, id: u64) -> bool {
        let before = self.vectors.len();
        self.vectors.retain(|(existing, _)| *existing != id);
        self.vectors.len() != before
    }

    /// Number of stored vectors.
    #[must_use]
    pub fn vector_count(&self) -> usize {
        self.vectors.len()
    }

    /// Brute-force nearest neighbours by cosine, best first, at most `k`,
    /// considering only ids for which `filter` returns true.
    pub fn search(&self, query: &[f32], k: usize, filter: impl Fn(u64) -> bool) -> Vec<(u64, f32)> {
        let mut scored: Vec<(u64, f32)> = self
            .vectors
            .iter()
            .filter(|(id, _)| filter(*id))
            .map(|(id, values)| (*id, cosine(query, values)))
            .collect();
        // Descending by score; ties by id so the order is deterministic.
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
    }
}

fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing string field `{field}`"))
}

/// What the kv side of a memory fact holds (double-encoded as the kv value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FactRecord {
    user: String,
    text: String,
    at: u64,
}

/// A fact returned by [`Recall`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecalledFact {
    /// The vector id, which is also the handle `forget` takes.
    pub id: u64,
    /// The already-scoped user id the fact belongs to.
    pub user: String,
    /// The fact itself.
    pub text: String,
    /// Unix seconds, as supplied at store time.
    pub at: u64,
    /// Cosine similarity to the query; `1.0` for listing calls that have none.
    pub score: f32,
}

/// Guild-namespaced semantic memory over a [`WdbxStore`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recall {
    store: WdbxStore,
}

impl Recall {
    /// Key prefix for one guild's facts. Every read goes through this.
    fn fact_key_prefix(scoped_guild_id: &str) -> String {
        format!("mem:{scoped_guild_id}:")
    }

    fn fact_key(scoped_guild_id: &str, id: u64) -> String {
        format!("mem:{scoped_guild_id}:{id}")
    }

    /// An empty memory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: WdbxStore::new(),
        }
    }

    /// Wrap an existing store (e.g. one just loaded from disk).
    #[must_use]
    pub fn from_store(store: WdbxStore) -> Self {
        Self { store }
    }

    /// Load from a store file; a missing file is an empty memory.
    pub fn load(path: &Path) -> Result<Self, WdbxError> {
        if path.exists() {
            WdbxStore::load(path).map(Self::from_store)
        } else {
            Ok(Self::new())
        }
    }

    /// Save atomically. See [`WdbxStore::save`].
    pub fn save(&self, path: &Path) -> Result<(), WdbxError> {
        self.store.save(path)
    }

    /// The underlying store.
    #[must_use]
    pub fn store(&self) -> &WdbxStore {
        &self.store
    }

    /// Store a fact for `user` in `scoped_guild_id` and return its id.
    pub fn remember(&mut self, scoped_guild_id: &str, user: &str, text: &str, at: u64) -> u64 {
        let id = self.store.insert_vector(text_embedding(text).to_vec());
        let record = FactRecord {
            user: user.to_string(),
            text: text.to_string(),
            at,
        };
        let value = serde_json::to_string(&record).unwrap_or_default();
        self.store
            .put_kv(Self::fact_key(scoped_guild_id, id), value);
        id
    }

    /// Decode one fact by id within a guild, or `None` if the key is absent
    /// or unreadable.
    fn fact(&self, scoped_guild_id: &str, id: u64, score: f32) -> Option<RecalledFact> {
        let value = self.store.get_kv(&Self::fact_key(scoped_guild_id, id))?;
        let record: FactRecord = serde_json::from_str(value).ok()?;
        Some(RecalledFact {
            id,
            user: record.user,
            text: record.text,
            at: record.at,
            score,
        })
    }

    /// Ids of every fact in one guild, from the kv side (the authority on
    /// ownership — a vector alone says nothing about which guild it serves).
    fn guild_ids(&self, scoped_guild_id: &str) -> Vec<u64> {
        let prefix = Self::fact_key_prefix(scoped_guild_id);
        let mut ids: Vec<u64> = self
            .store
            .kv_with_prefix(&prefix)
            .filter_map(|(key, _)| key[prefix.len()..].parse().ok())
            .collect();
        // KV iteration is lexicographic (1, 10, 2), while search filters use
        // numeric binary search. Keep the authority list numerically ordered.
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Guild-wide storage query for administrative tests. Inference and tool
    /// call sites must use [`Self::recall_for_user`]. Keeping this private makes
    /// the privacy-safe boundary the only production API.
    #[cfg(test)]
    #[must_use]
    fn recall_for_guild_admin(
        &self,
        scoped_guild_id: &str,
        query_text: &str,
        k: usize,
    ) -> Vec<RecalledFact> {
        let ids = self.guild_ids(scoped_guild_id);
        let query = text_embedding(query_text);
        self.store
            .search(&query, k, |id| ids.binary_search(&id).is_ok())
            .into_iter()
            .filter_map(|(id, score)| self.fact(scoped_guild_id, id, score))
            .collect()
    }

    /// The `k` facts belonging to one person in one guild most similar to the
    /// query. This is the safe context/tool boundary: a guild is not a privacy
    /// boundary between its members.
    #[must_use]
    pub fn recall_for_user(
        &self,
        scoped_guild_id: &str,
        user: &str,
        query_text: &str,
        k: usize,
    ) -> Vec<RecalledFact> {
        let ids: Vec<u64> = self
            .guild_ids(scoped_guild_id)
            .into_iter()
            .filter(|id| {
                self.fact(scoped_guild_id, *id, 0.0)
                    .is_some_and(|fact| fact.user == user)
            })
            .collect();
        let query = text_embedding(query_text);
        self.store
            .search(&query, k, |id| ids.binary_search(&id).is_ok())
            .into_iter()
            .filter_map(|(id, score)| self.fact(scoped_guild_id, id, score))
            .collect()
    }

    /// Remove one fact. Returns whether it existed *in that guild* — an id
    /// belonging to another guild is untouched and reports `false`.
    pub fn forget(&mut self, scoped_guild_id: &str, id: u64) -> bool {
        if self
            .store
            .remove_kv(&Self::fact_key(scoped_guild_id, id))
            .is_none()
        {
            return false;
        }
        self.store.remove_vector(id);
        true
    }

    /// Every fact `user` holds in `scoped_guild_id`, oldest id first.
    #[must_use]
    pub fn facts_for_user(&self, scoped_guild_id: &str, user: &str) -> Vec<RecalledFact> {
        self.guild_ids(scoped_guild_id)
            .into_iter()
            .filter_map(|id| self.fact(scoped_guild_id, id, 1.0))
            .filter(|fact| fact.user == user)
            .collect()
    }

    /// Every decodable memory fact across guilds, ordered by vector id. This
    /// is the migration-only boundary used to recover WDBX-only records before
    /// JSON memory becomes the canonical fact source.
    #[must_use]
    pub fn all_memory_facts(&self) -> Vec<(String, RecalledFact)> {
        let mut locations: Vec<(u64, String)> = self
            .store
            .kv_with_prefix("mem:")
            .filter_map(|(key, _)| {
                let (guild, id) = key.strip_prefix("mem:")?.rsplit_once(':')?;
                Some((id.parse().ok()?, guild.to_string()))
            })
            .collect();
        locations.sort_by_key(|(id, _)| *id);
        locations.dedup();
        locations
            .into_iter()
            .filter_map(|(id, guild)| self.fact(&guild, id, 1.0).map(|fact| (guild, fact)))
            .collect()
    }

    /// Rebuild only the `mem:*` projection from canonical facts. Unrelated KV
    /// rows, vectors, and unknown ABI-WDBX record types remain untouched.
    /// Identical projections are left in place so vector ids do not churn on
    /// every restart.
    pub fn reconcile_memory_facts(
        &mut self,
        facts: impl IntoIterator<Item = (String, String, String, u64)>,
    ) {
        let wanted: Vec<(String, String, String, u64)> = facts.into_iter().collect();
        let current: Vec<(String, String, String, u64)> = self
            .all_memory_facts()
            .into_iter()
            .map(|(guild, fact)| (guild, fact.user, fact.text, fact.at))
            .collect();
        if current == wanted {
            return;
        }

        let memory_keys: Vec<(String, Option<u64>)> = self
            .store
            .kv_with_prefix("mem:")
            .map(|(key, _)| {
                let id = key
                    .strip_prefix("mem:")
                    .and_then(|suffix| suffix.rsplit_once(':'))
                    .and_then(|(_, id)| id.parse().ok());
                (key.to_string(), id)
            })
            .collect();
        for (key, id) in memory_keys {
            self.store.remove_kv(&key);
            if let Some(id) = id {
                self.store.remove_vector(id);
            }
        }
        for (guild, user, text, at) in wanted {
            self.remember(&guild, &user, &text, at);
        }
    }

    /// Number of facts stored for one guild.
    #[must_use]
    pub fn count(&self, scoped_guild_id: &str) -> usize {
        self.guild_ids(scoped_guild_id).len()
    }
}

#[cfg(test)]
mod tests;
