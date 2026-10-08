//! Minimized lexical context. First 32 distinct normalized content tokens,
//! domain-separated through the pinned Wyhash seam; no raw ask survives.
use super::outcome::{self, ReplyOutcome};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_TOKENS: usize = 32;
const DOMAIN: &[u8] = b"abbey:learning:ask-token:v1\0";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskMarkers {
    pub correction: bool,
    pub thanks: bool,
    pub question: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AskSignature {
    pub token_hashes: Vec<u64>,
    pub markers: AskMarkers,
}

impl AskSignature {
    pub fn from_text(text: &str) -> Self {
        let normalized = outcome::normalize(text);
        let mut token_hashes = Vec::new();
        for token in normalized.split(|c: char| !c.is_alphanumeric()) {
            if token.chars().count() <= 1 || outcome::STOPWORDS.contains(&token) {
                continue;
            }
            let mut bytes = Vec::with_capacity(DOMAIN.len() + token.len());
            bytes.extend_from_slice(DOMAIN);
            bytes.extend_from_slice(token.as_bytes());
            let hash = crate::wyhash::hash(0, &bytes);
            if !token_hashes.contains(&hash) {
                token_hashes.push(hash);
                if token_hashes.len() == MAX_TOKENS {
                    break;
                }
            }
        }
        let marker = outcome::marker_outcome(text);
        Self {
            token_hashes,
            markers: AskMarkers {
                correction: marker == Some(ReplyOutcome::Correction),
                thanks: marker == Some(ReplyOutcome::ExplicitThanks),
                question: text.trim_end().ends_with('?'),
            },
        }
    }

    pub fn valid(&self) -> bool {
        self.token_hashes.len() <= MAX_TOKENS
            && self
                .token_hashes
                .iter()
                .enumerate()
                .all(|(i, h)| !self.token_hashes[..i].contains(h))
            && !(self.markers.correction && self.markers.thanks)
    }
}

// Accept old Pending.ask strings once; Serialize always emits the minimized
// object. A bounded sequence visitor rejects oversized/duplicate hashes before
// allocating an arbitrary input vector.
impl<'de> Deserialize<'de> for AskSignature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(deserialize_with = "hashes")]
            token_hashes: Vec<u64>,
            markers: AskMarkers,
        }
        fn hashes<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Vec<u64>;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("at most 32 distinct lexical hashes")
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut out = Vec::new();
                    while let Some(hash) = seq.next_element::<u64>()? {
                        if out.len() == MAX_TOKENS || out.contains(&hash) {
                            return Err(serde::de::Error::custom("invalid lexical hashes"));
                        }
                        out.push(hash);
                    }
                    Ok(out)
                }
            }
            d.deserialize_seq(Visitor)
        }
        struct SignatureVisitor;
        impl<'de> serde::de::Visitor<'de> for SignatureVisitor {
            type Value = AskSignature;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a minimized signature or legacy ask string")
            }
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
                Ok(AskSignature::from_text(text))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                let wire = Wire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(AskSignature {
                    token_hashes: wire.token_hashes,
                    markers: wire.markers,
                })
            }
        }
        let signature = deserializer.deserialize_any(SignatureVisitor)?;
        if !signature.valid() {
            return Err(serde::de::Error::custom("invalid ask markers"));
        }
        Ok(signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_distinct_domain_separated_and_bounded() {
        let a = AskSignature::from_text("HOW the Gateway gateway timeout?");
        assert_eq!(a, AskSignature::from_text("how gateway TIMEOUT?"));
        assert_eq!(a.token_hashes.len(), 2);
        assert_eq!(
            a.token_hashes[0],
            crate::wyhash::hash(0, b"abbey:learning:ask-token:v1\0gateway")
        );
        assert_ne!(a.token_hashes[0], crate::wyhash::hash(0, b"gateway"));
        let long = (0..100)
            .map(|i| format!("token{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(AskSignature::from_text(&long).token_hashes.len(), 32);
        assert_eq!(
            AskSignature::from_text("that’s wrong"),
            AskSignature::from_text("that's wrong")
        );
        assert_eq!(
            AskSignature::from_text("the a an !"),
            AskSignature::default()
        );
    }
    #[test]
    fn first_32_distinct_normalized_tokens_bound_later_overlap() {
        let retained: Vec<_> = (0..32).map(|i| format!("zeta{i}")).collect();
        let prefix = format!("{}?", retained.join(" "));
        // Stopwords and case-folded duplicates must not consume a slot. Later
        // alphabetically earlier tokens must not displace the reviewed prefix.
        let decorated = retained
            .iter()
            .map(|word| format!("HOW the {} {word} a an i", word.to_uppercase()))
            .collect::<Vec<_>>()
            .join(" ");
        let beyond = (32..64)
            .map(|i| format!("alpha{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let bounded = AskSignature::from_text(&format!("{decorated} {beyond}?"));
        assert_eq!(bounded, AskSignature::from_text(&prefix));
        assert_eq!(
            outcome::classify_signature(&prefix, Some(&bounded)),
            Some(ReplyOutcome::RephrasedSameAsk)
        );
        assert_eq!(
            outcome::classify_signature(&format!("{beyond}?"), Some(&bounded)),
            None,
            "sharing only discarded tokens cannot gain represented overlap"
        );
    }

    #[test]
    fn malformed_and_legacy_signatures_fail_closed_or_minimize() {
        let legacy: AskSignature = serde_json::from_str("\"private raw question?\"").unwrap();
        assert!(!serde_json::to_string(&legacy).unwrap().contains("private"));
        for hashes in [vec![1; 2], (0..33).collect()] {
            let wire = serde_json::json!({"token_hashes":hashes,"markers":{"correction":false,"thanks":false,"question":true}});
            assert!(serde_json::from_value::<AskSignature>(wire).is_err());
        }
        for wire in [
            serde_json::json!(3),
            serde_json::json!({"token_hashes":[],"markers":{"correction":true,"thanks":true,"question":false}}),
            serde_json::json!({"token_hashes":[],"markers":{"invented":true}}),
        ] {
            assert!(serde_json::from_value::<AskSignature>(wire).is_err());
        }
    }
}
