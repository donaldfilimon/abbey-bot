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
        let mut out = String::with_capacity(64 + self.vectors.len() * 320 + self.kv.len() * 96);
        out.push_str(HEADER);
        out.push('\n');
        for (id, values) in &self.vectors {
            let line = VectorLine {
                r#type: "vector",
                id: *id,
                values,
            };
            out.push_str(&serde_json::to_string(&line).unwrap_or_default());
            out.push('\n');
        }
        for (key, value) in &self.kv {
            let line = KvLine {
                r#type: "kv",
                key,
                value,
            };
            out.push_str(&serde_json::to_string(&line).unwrap_or_default());
            out.push('\n');
        }
        for line in &self.unknown {
            out.push_str(line);
            out.push('\n');
        }
        out
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
mod tests {
    use super::*;

    const SAMPLE: &str = "# ABI-WDBX v1\n\
{\"type\":\"vector\",\"id\":1,\"values\":[0.5,-0.25,0.125]}\n\
{\"type\":\"kv\",\"key\":\"completion:1\",\"value\":\"{\\\"kind\\\":\\\"completion\\\"}\"}\n\
{\"type\":\"block\",\"hash\":\"abc\",\"prev_hash\":\"0\",\"sequence\":0}\n";

    #[test]
    fn parse_then_render_round_trips_including_unknown_records() {
        let store = WdbxStore::parse(SAMPLE).expect("parses");
        assert_eq!(store.vector(1), Some(&[0.5, -0.25, 0.125][..]));
        assert_eq!(
            store.get_kv("completion:1"),
            Some("{\"kind\":\"completion\"}")
        );
        let rendered = store.render();
        assert_eq!(
            rendered, SAMPLE,
            "render must reproduce the sample byte for byte"
        );
        // And unknown lines survive a second pass too.
        let again = WdbxStore::parse(&rendered).expect("re-parses");
        assert_eq!(again, store);
    }

    #[test]
    fn vectors_round_trip_exactly_through_json() {
        let mut store = WdbxStore::new();
        let v = text_embedding("round trip").to_vec();
        let id = store.insert_vector(v.clone());
        let back = WdbxStore::parse(&store.render()).expect("parses");
        let got = back.vector(id).expect("vector present");
        assert!(got.iter().zip(&v).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn header_is_required() {
        assert!(matches!(
            WdbxStore::parse(""),
            Err(WdbxError::MissingHeader { .. })
        ));
        let err = WdbxStore::parse("{\"type\":\"kv\",\"key\":\"a\",\"value\":\"b\"}\n")
            .expect_err("no header");
        assert!(err.to_string().contains("ABI-WDBX v1"), "{err}");
        // A header-only file is a valid empty store; CRLF is tolerated.
        assert_eq!(
            WdbxStore::parse("# ABI-WDBX v1\r\n").expect("parses"),
            WdbxStore::new()
        );
    }

    #[test]
    fn checksum_trailer_is_dropped_and_comments_preserved() {
        let text = "# ABI-WDBX v1\n# note\n# checksum:deadbeef\n";
        let store = WdbxStore::parse(text).expect("parses");
        assert_eq!(store.render(), "# ABI-WDBX v1\n# note\n");
    }

    #[test]
    fn malformed_lines_are_errors_not_panics() {
        let cases = [
            "# ABI-WDBX v1\nnot json\n",
            "# ABI-WDBX v1\n[1,2,3]\n",
            "# ABI-WDBX v1\n{\"key\":\"no type\"}\n",
            "# ABI-WDBX v1\n{\"type\":\"kv\",\"key\":\"a\"}\n",
            "# ABI-WDBX v1\n{\"type\":\"kv\",\"key\":\"a\",\"value\":7}\n",
            "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":-1,\"values\":[]}\n",
            "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":1,\"values\":[\"x\"]}\n",
            "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":1,\"values\":[]}\n{\"type\":\"vector\",\"id\":1,\"values\":[]}\n",
        ];
        for text in cases {
            let err = WdbxStore::parse(text).expect_err(text);
            assert!(
                matches!(err, WdbxError::MalformedLine { line: 2 | 3, .. }),
                "{text}: {err}"
            );
        }
    }

    #[test]
    fn vector_ids_are_monotonic_even_after_removal_and_reload() {
        let mut store = WdbxStore::new();
        let a = store.insert_vector(vec![1.0]);
        let b = store.insert_vector(vec![1.0]);
        assert!(b > a);
        assert!(store.remove_vector(b));
        let c = store.insert_vector(vec![1.0]);
        assert!(c > b, "a removed id is never reissued");
        let reloaded = WdbxStore::parse(&store.render()).expect("parses");
        let mut reloaded = reloaded;
        assert!(reloaded.insert_vector(vec![1.0]) > c);
    }

    #[test]
    fn search_ranks_identical_text_first_and_honours_filter() {
        let mut store = WdbxStore::new();
        let target = store.insert_vector(text_embedding("abbey likes tea").to_vec());
        let other = store.insert_vector(text_embedding("the server restarts nightly").to_vec());
        let query = text_embedding("abbey likes tea");
        let hits = store.search(&query, 5, |_| true);
        assert_eq!(hits[0].0, target);
        assert!((hits[0].1 - 1.0).abs() < 1e-5);
        assert_eq!(hits.len(), 2);
        let filtered = store.search(&query, 5, |id| id == other);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, other);
        assert!(store.search(&query, 0, |_| true).is_empty());
    }

    #[test]
    fn recall_never_crosses_guilds() {
        let mut memory = Recall::new();
        let a_id = memory.remember("guild-a", "user-1", "abbey prefers earl grey", 10);
        memory.remember("guild-b", "user-1", "abbey prefers coffee", 11);

        let in_a = memory.recall_for_guild_admin("guild-a", "abbey prefers earl grey", 10);
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].id, a_id);
        assert_eq!(in_a[0].text, "abbey prefers earl grey");
        assert_eq!(in_a[0].user, "user-1");
        assert_eq!(in_a[0].at, 10);

        let in_b = memory.recall_for_guild_admin("guild-b", "abbey prefers earl grey", 10);
        assert_eq!(in_b.len(), 1, "guild B sees only its own fact");
        assert_eq!(in_b[0].text, "abbey prefers coffee");

        assert!(
            memory
                .recall_for_guild_admin("guild-c", "abbey", 10)
                .is_empty()
        );
        assert_eq!(memory.count("guild-a"), 1);
        assert_eq!(memory.count("guild-b"), 1);
        assert_eq!(memory.count("guild-c"), 0);
        // A guild id that is a prefix of another must not match it.
        assert_eq!(memory.count("guild"), 0);
    }

    #[test]
    fn recall_ranks_the_closest_fact_first() {
        let mut memory = Recall::new();
        memory.remember("g", "u", "the weekly raid is on thursday", 1);
        let best = memory.remember("g", "u", "donald's favourite editor is helix", 2);
        memory.remember("g", "u", "welcome channel is #lobby", 3);
        let hits = memory.recall_for_guild_admin("g", "favourite editor", 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, best);
        assert!(hits[0].score >= hits[1].score);
    }

    #[test]
    fn person_scoped_recall_never_crosses_users_in_one_guild() {
        let mut memory = Recall::new();
        let alice = memory.remember("g", "alice", "alice's launch code is violet", 1);
        memory.remember("g", "bob", "bob's launch code is orange", 2);

        let hits = memory.recall_for_user("g", "alice", "launch code", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, alice);
        assert_eq!(hits[0].user, "alice");
        assert_eq!(hits[0].text, "alice's launch code is violet");
        assert!(
            memory
                .recall_for_user("g", "carol", "launch code", 10)
                .is_empty()
        );
    }

    #[test]
    fn numeric_fact_ids_remain_searchable_past_lexicographic_order() {
        let mut memory = Recall::new();
        for id in 1..=12 {
            memory.remember("g", "u", &format!("fact number {id}"), id);
        }
        let hits = memory.recall_for_user("g", "u", "fact number 2", 20);
        assert_eq!(hits.len(), 12, "all numeric ids pass the guild/user filter");
    }

    #[test]
    fn forget_removes_only_within_its_guild() {
        let mut memory = Recall::new();
        let id = memory.remember("g1", "u", "fact one", 1);
        assert!(!memory.forget("g2", id), "another guild cannot forget it");
        assert_eq!(memory.count("g1"), 1);
        assert!(memory.forget("g1", id));
        assert!(!memory.forget("g1", id), "second forget reports absence");
        assert_eq!(memory.count("g1"), 0);
        assert!(
            memory
                .recall_for_guild_admin("g1", "fact one", 5)
                .is_empty()
        );
        assert_eq!(
            memory.store().vector_count(),
            0,
            "vector goes with the fact"
        );
    }

    #[test]
    fn facts_for_user_lists_that_user_only_oldest_first() {
        let mut memory = Recall::new();
        let first = memory.remember("g", "alice", "alice likes cats", 1);
        memory.remember("g", "bob", "bob likes dogs", 2);
        let second = memory.remember("g", "alice", "alice plays bass", 3);
        memory.remember("other", "alice", "alice elsewhere", 4);
        let facts = memory.facts_for_user("g", "alice");
        assert_eq!(
            facts.iter().map(|f| f.id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(facts.iter().all(|f| f.score == 1.0));
        assert!(memory.facts_for_user("g", "carol").is_empty());
    }

    #[test]
    fn fact_kv_is_double_encoded_json_under_the_scoped_key() {
        let mut memory = Recall::new();
        let id = memory.remember("123:456", "u", "hello", 99);
        let raw = memory
            .store()
            .get_kv(&format!("mem:123:456:{id}"))
            .expect("kv under scoped key");
        let parsed: serde_json::Value = serde_json::from_str(raw).expect("value is JSON");
        assert_eq!(parsed["user"], "u");
        assert_eq!(parsed["text"], "hello");
        assert_eq!(parsed["at"], 99);
    }

    #[test]
    fn projection_reconcile_replaces_only_memory_records() {
        let mut memory = Recall::new();
        let stale_id = memory.remember("g", "u", "stale fact", 1);
        let unrelated_id = memory.store.insert_vector(vec![0.25, 0.5]);
        memory.store.put_kv("completion:1", "kept");
        memory
            .store
            .unknown
            .push(r#"{"type":"block","hash":"abc","prev_hash":"0","sequence":0}"#.into());

        memory.reconcile_memory_facts([
            ("g".into(), "u".into(), "canonical fact".into(), 2),
            ("other".into(), "v".into(), "second fact".into(), 3),
        ]);
        assert_eq!(
            memory
                .all_memory_facts()
                .into_iter()
                .map(|(guild, fact)| (guild, fact.user, fact.text, fact.at))
                .collect::<Vec<_>>(),
            [
                ("g".into(), "u".into(), "canonical fact".into(), 2),
                ("other".into(), "v".into(), "second fact".into(), 3),
            ]
        );
        assert!(memory.store.vector(stale_id).is_none());
        assert_eq!(memory.store.vector(unrelated_id), Some(&[0.25, 0.5][..]));
        assert_eq!(memory.store.get_kv("completion:1"), Some("kept"));
        assert!(memory.store.render().contains(r#"{"type":"block""#));

        let stable = memory.clone();
        memory.reconcile_memory_facts([
            ("g".into(), "u".into(), "canonical fact".into(), 2),
            ("other".into(), "v".into(), "second fact".into(), 3),
        ]);
        assert_eq!(memory, stable, "an identical projection does not churn ids");
    }

    #[test]
    fn load_and_save_round_trip_in_a_temp_dir() {
        let dir = std::env::temp_dir().join(format!(
            "abbey-bot-wdbx-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("memory.wdbx");

        let loaded_missing = Recall::load(&path).expect("missing file is empty memory");
        assert_eq!(loaded_missing.count("g"), 0);

        let mut memory = Recall::new();
        let id = memory.remember("g", "u", "persisted fact", 42);
        memory.save(&path).expect("saves");
        assert!(
            !dir.join("memory.wdbx.tmp").exists(),
            "temp file is renamed away"
        );

        let text = fs::read_to_string(&path).expect("readable");
        assert!(text.starts_with("# ABI-WDBX v1\n"));

        let back = Recall::load(&path).expect("loads");
        assert_eq!(back, memory);
        let hits = back.recall_for_guild_admin("g", "persisted fact", 1);
        assert_eq!(hits[0].id, id);
        assert_eq!(hits[0].at, 42);

        let err = WdbxStore::load(&dir.join("nope.wdbx")).expect_err("missing file");
        assert!(matches!(err, WdbxError::Io { op: "read", .. }), "{err}");

        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn error_display_is_one_sentence_each() {
        let missing = WdbxError::MissingHeader {
            found: "garbage".into(),
        };
        assert_eq!(
            missing.to_string(),
            "WDBX store header is `garbage`, expected `# ABI-WDBX v1`"
        );
        let bad = WdbxError::MalformedLine {
            line: 4,
            reason: "not JSON".into(),
        };
        assert_eq!(bad.to_string(), "WDBX store line 4 is malformed: not JSON");
    }
}
