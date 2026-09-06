//! Scoring schema boundary: malformed envelopes fail the record; malformed known
//! class entries and duplicates remove that class only. Unknown classes grant nothing.
//! A duplicate request_class key poisons every class named in that entry. Parsing
//! preserves duplicate keys until validation, before any serde_json::Value conversion.
use super::manifest::*;
use super::scoring::{QualificationScoreEvidence, RequestClass, ScoreProducerPolicy};
use super::{ProviderClass, ProviderId};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordWire {
    version: u32,
    fixture_version: String,
    #[serde(deserialize_with = "deserialize_canonical_provider_id")]
    provider_id: ProviderId,
    provider_class: ProviderClass,
    identity: ProviderIdentityHashes,
    declared_capabilities: DeclaredCapabilities,
    isolation_capabilities: QualifiedIsolation,
    qualification_status: QualificationStatus,
    #[serde(default, deserialize_with = "present")]
    score_policy: Option<u8>,
    #[serde(default, deserialize_with = "present")]
    score_profiles: Option<Profiles>,
}
impl<'de> Deserialize<'de> for ProviderRecord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = RecordWire::deserialize(d)?;
        if !matches!(
            (&w.score_policy, &w.score_profiles),
            (None, None) | (Some(1), Some(_))
        ) {
            return Err(serde::de::Error::custom("invalid scoring envelope"));
        }
        let profiles = w.score_profiles.map(|p| {
            p.0.into_iter()
                .filter(|p| {
                    p.request_class
                        .supported_by(w.declared_capabilities.as_provider_capabilities())
                })
                .collect()
        });
        Ok(Self {
            version: w.version,
            fixture_version: w.fixture_version,
            provider_id: w.provider_id,
            provider_class: w.provider_class,
            identity: w.identity,
            declared_capabilities: w.declared_capabilities,
            isolation_capabilities: w.isolation_capabilities,
            qualification_status: w.qualification_status,
            score_policy: w.score_policy,
            score_profiles: profiles,
        })
    }
}
/// Retain duplicate keys and arbitrary invalid JSON shapes until the class boundary.
#[derive(Debug)]
enum Raw {
    Object(Vec<(String, Raw)>),
    Array(Vec<Raw>),
    Scalar(Value),
}
impl Raw {
    fn value(self) -> Option<Value> {
        match self {
            Self::Scalar(v) => Some(v),
            Self::Array(v) => v
                .into_iter()
                .map(Self::value)
                .collect::<Option<Vec<_>>>()
                .map(Value::Array),
            Self::Object(v) => {
                let mut map = serde_json::Map::new();
                for (k, v) in v {
                    if map.insert(k, v.value()?).is_some() {
                        return None;
                    }
                }
                Some(Value::Object(map))
            }
        }
    }
    fn named_classes(&self) -> Vec<RequestClass> {
        match self {
            Self::Object(fields) => fields
                .iter()
                .filter(|(k, _)| k == "request_class")
                .filter_map(|(_, v)| match v {
                    Self::Scalar(v) => serde_json::from_value(v.clone()).ok(),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }
}
impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Raw;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON profile")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> Result<Raw, M::Error> {
                let mut v = vec![];
                while let Some(e) = m.next_entry()? {
                    v.push(e);
                }
                Ok(Raw::Object(v))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut s: S) -> Result<Raw, S::Error> {
                let mut v = vec![];
                while let Some(e) = s.next_element()? {
                    v.push(e);
                }
                Ok(Raw::Array(v))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Raw, E> {
                Ok(Raw::Scalar(Value::from(v)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Raw, E> {
                Ok(Raw::Scalar(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Raw, E> {
                Ok(Raw::Scalar(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
struct Profiles(Vec<QualificationScoreEvidence>);
impl<'de> Deserialize<'de> for Profiles {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Vec::<Raw>::deserialize(d)?;
        let mut seen = BTreeSet::new();
        let mut rejected = BTreeSet::new();
        let mut accepted = BTreeMap::new();
        for raw in raw {
            let classes = raw.named_classes();
            for class in &classes {
                if !seen.insert(*class) {
                    rejected.insert(*class);
                }
            }
            let valid = raw
                .value()
                .and_then(|v| serde_json::from_value::<QualificationScoreEvidence>(v).ok())
                .filter(|p| ScoreProducerPolicy::V1.qualification(p).is_ok());
            match valid {
                Some(profile) => {
                    accepted.insert(profile.request_class, profile);
                }
                None => rejected.extend(classes),
            }
        }
        Ok(Self(
            accepted
                .into_iter()
                .filter(|(c, _)| !rejected.contains(c))
                .map(|(_, p)| p)
                .collect(),
        ))
    }
}
#[cfg(test)]
#[path = "manifest_score_tests.rs"]
mod tests;
