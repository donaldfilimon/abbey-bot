//! Derived work projection, isolated from generic memory. Canonical admitted
//! rows (including retired rows) own storage; a current canonical allowlist owns
//! retrieval. No admission, filesystem commit or native authorization occurs here.
use super::WdbxStore;
use crate::{
    embedding::text_embedding,
    work::recall::{AdmittedWorkEvidence, WorkRecallState},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const PREFIX: &str = "workrecall:v1:";
const MANIFEST: &str = "workrecall:v1:manifest";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionError {
    InvalidAuthority,
    AllocationUnavailable,
    Encoding,
    #[cfg(test)]
    QueryTooLarge,
}

/// This describes an in-memory projection. Only an observed successful disk
/// write (or loading and checking the actual disk file) establishes durability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionManifest {
    pub version: u8,
    pub revision: u64,
    pub records_digest: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    version: u8,
    vector_id: u64,
    row: AdmittedWorkEvidence,
}

/// Only row identity and similarity cross this seam; join content from current
/// canonical authority, never expose retained cache payloads to callers.
#[cfg(test)]
#[derive(Debug, PartialEq)]
pub struct WorkHit {
    pub evidence_id: u64,
    pub score: f32,
}

fn encode<T: Serialize>(value: &T) -> Result<String, ProjectionError> {
    serde_json::to_string(value).map_err(|_| ProjectionError::Encoding)
}
fn manifest(state: &WorkRecallState) -> Result<ProjectionManifest, ProjectionError> {
    state
        .validate()
        .map_err(|_| ProjectionError::InvalidAuthority)?;
    let bytes = encode(&state.records)?;
    Ok(ProjectionManifest {
        version: 1,
        revision: state.projection_revision,
        records_digest: Sha256::digest(bytes.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    })
}
fn key(id: u64) -> String {
    format!("{PREFIX}{id}")
}
fn entry(key: &str, value: &str) -> Option<Entry> {
    let row: Entry = serde_json::from_str(value).ok()?;
    (row.version == 1 && row.vector_id != 0 && key == self::key(row.row.id)).then_some(row)
}

/// Opaque records are preserved. A numeric token may be an unknown schema's
/// vector reference, so ambiguity denies reuse/removal rather than erasing data.
fn mentions(value: &str, id: u64) -> bool {
    fn tokens(value: &str, id: u64) -> bool {
        value
            .split(|c: char| !c.is_ascii_digit())
            .any(|s| s.parse::<u64>().ok() == Some(id))
    }
    fn decoded(value: &serde_json::Value, id: u64) -> bool {
        match value {
            serde_json::Value::Number(n) => n.as_u64() == Some(id) || n.as_f64() == Some(id as f64),
            serde_json::Value::String(s) => tokens(s, id),
            serde_json::Value::Array(a) => a.iter().any(|v| decoded(v, id)),
            serde_json::Value::Object(o) => o.iter().any(|(k, v)| tokens(k, id) || decoded(v, id)),
            _ => false,
        }
    }
    tokens(value, id) || serde_json::from_str(value).is_ok_and(|v| decoded(&v, id))
}

fn exclusive(store: &WdbxStore, owner_key: &str, id: u64) -> bool {
    if store
        .unknown
        .iter()
        .any(|line| !line.starts_with('#') && mentions(line, id))
    {
        return false;
    }
    for (k, v) in &store.kv {
        if k == owner_key || k == MANIFEST {
            continue;
        }
        if k.starts_with("mem:") {
            if k.rsplit_once(':')
                .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
                == Some(id)
            {
                return false;
            }
        } else if k.starts_with(PREFIX) {
            match entry(k, v) {
                Some(other) if other.vector_id != id => (),
                Some(_) => return false,
                None if mentions(k, id) || mentions(v, id) => return false,
                None => (),
            }
        } else if mentions(k, id) || mentions(v, id) {
            return false;
        }
    }
    true
}
fn intact(store: &WdbxStore, key: &str, row: &Entry) -> bool {
    exclusive(store, key, row.vector_id)
        && store
            .vectors
            .iter()
            .filter(|(id, _)| *id == row.vector_id)
            .count()
            == 1
        && store.vector(row.vector_id) == Some(text_embedding(&row.row.payload.text).as_slice())
}

impl super::Recall {
    /// Transactionally reconcile an owned candidate before publishing. Allocation
    /// is checked for the complete batch; no KV/vector/allocator partial mutation.
    pub fn reconcile_work_evidence(
        &mut self,
        state: &WorkRecallState,
    ) -> Result<ProjectionManifest, ProjectionError> {
        let wanted_manifest = manifest(state)?;
        let current: BTreeMap<String, Entry> = self
            .store
            .kv_with_prefix(PREFIX)
            .filter_map(|(k, v)| entry(k, v).map(|row| (k.to_string(), row)))
            .collect();
        let stable: BTreeSet<u64> = current
            .iter()
            .filter_map(|(k, row)| {
                (state.records.get(&row.row.id) == Some(&row.row) && intact(&self.store, k, row))
                    .then_some(row.row.id)
            })
            .collect();
        let needed = state.records.len() - stable.len();
        let count = u64::try_from(needed).map_err(|_| ProjectionError::AllocationUnavailable)?;
        let start = self.store.next_id;
        let end = start
            .checked_add(count)
            .ok_or(ProjectionError::AllocationUnavailable)?;
        if needed != 0
            && (start == 0
                || start == u64::MAX
                || self.store.vectors.iter().any(|(id, _)| *id >= start))
        {
            return Err(ProjectionError::AllocationUnavailable);
        }
        let mut next = self.store.clone();
        let keys: Vec<_> = next
            .kv_with_prefix(PREFIX)
            .map(|(k, _)| k.to_string())
            .collect();
        for k in keys {
            if let Some(row) = current.get(&k) {
                if stable.contains(&row.row.id) {
                    continue;
                }
                if intact(&self.store, &k, row) {
                    next.remove_vector(row.vector_id);
                }
            }
            next.remove_kv(&k);
        }
        // Removed cache keys have no surviving references. Check the candidate,
        // preserving references in every retained foreign/unknown row.
        if (start..end).any(|id| !exclusive(&next, "", id)) {
            return Err(ProjectionError::AllocationUnavailable);
        }
        let mut vector_id = start;
        for (id, row) in &state.records {
            if stable.contains(id) {
                continue;
            }
            next.vectors
                .push((vector_id, text_embedding(&row.payload.text).to_vec()));
            next.put_kv(
                key(*id),
                encode(&Entry {
                    version: 1,
                    vector_id,
                    row: row.clone(),
                })?,
            );
            vector_id += 1; // complete range checked before any candidate mutation
        }
        next.next_id = end;
        next.put_kv(MANIFEST, encode(&wanted_manifest)?);
        self.store = next;
        Ok(wanted_manifest)
    }

    /// Compare this instance with canonical authority. Call on the *loaded disk
    /// store* to detect durable debt; calling on live memory proves no disk write.
    pub fn work_projection_current(
        &self,
        state: &WorkRecallState,
    ) -> Result<bool, ProjectionError> {
        let expected = manifest(state)?;
        if self
            .store
            .get_kv(MANIFEST)
            .and_then(|s| serde_json::from_str::<ProjectionManifest>(s).ok())
            .as_ref()
            != Some(&expected)
        {
            return Ok(false);
        }
        let expected_keys: BTreeSet<_> = state
            .records
            .keys()
            .map(|id| key(*id))
            .chain([MANIFEST.to_string()])
            .collect();
        let actual_keys: BTreeSet<_> = self
            .store
            .kv_with_prefix(PREFIX)
            .map(|(k, _)| k.to_string())
            .collect();
        Ok(expected_keys == actual_keys
            && state.records.iter().all(|(id, row)| {
                let k = key(*id);
                self.store
                    .get_kv(&k)
                    .and_then(|v| entry(&k, v))
                    .is_some_and(|e| e.row == *row && intact(&self.store, &k, &e))
            }))
    }

    /// Caller must derive `allowed` from eligible_recall_ids against the same
    /// current canonical work snapshot and freshly authorized private audience.
    /// No global scoring occurs: canonical IDs are joined/validated first.
    #[cfg(test)]
    pub fn search_work_evidence(
        &self,
        state: &WorkRecallState,
        allowed: &BTreeSet<u64>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WorkHit>, ProjectionError> {
        state
            .validate()
            .map_err(|_| ProjectionError::InvalidAuthority)?;
        if query.chars().count() > 512 {
            return Err(ProjectionError::QueryTooLarge);
        }
        // Decode ownership once per query. Re-parsing every work envelope for
        // every candidate makes an authorized thousand-row query quadratic.
        let entries: BTreeMap<_, _> = self
            .store
            .kv_with_prefix(PREFIX)
            .filter_map(|(k, v)| entry(k, v).map(|e| (k.to_string(), e)))
            .collect();
        let mut owners = BTreeMap::<u64, usize>::new();
        for e in entries.values() {
            *owners.entry(e.vector_id).or_default() += 1;
        }
        let mut vector_counts = BTreeMap::<u64, usize>::new();
        for (id, _) in &self.store.vectors {
            *vector_counts.entry(*id).or_default() += 1;
        }
        // Preserve the conservative opaque-reference checks unchanged. Known
        // work envelopes are covered by owners, with malformed entries left in.
        let mut opaque = self.store.clone();
        for k in entries.keys() {
            opaque.remove_kv(k);
        }
        let vectors: BTreeMap<_, _> = allowed
            .iter()
            .filter_map(|id| {
                let row = state.records.get(id)?;
                let k = key(*id);
                let e = entries.get(&k)?;
                (e.row == *row
                    && owners.get(&e.vector_id) == Some(&1)
                    && vector_counts.get(&e.vector_id) == Some(&1)
                    && exclusive(&opaque, &k, e.vector_id)
                    && self.store.vector(e.vector_id)
                        == Some(text_embedding(&row.payload.text).as_slice()))
                .then_some((e.vector_id, *id))
            })
            .collect();
        Ok(self
            .store
            .search(&text_embedding(query), limit.min(8), |id| {
                vectors.contains_key(&id)
            })
            .into_iter()
            .map(|(id, score)| WorkHit {
                evidence_id: vectors[&id],
                score,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
