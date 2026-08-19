//! Deterministic text embeddings via signed feature hashing.
//!
//! A behaviour-exact transcription of abi's `abi-ai/src/embedding.rs` (itself a
//! port of the Zig `helpers.zig` original). abbey-bot takes no dependency on
//! abi, so the algorithm lives here and is pinned by golden vectors computed
//! from abi's own implementation (see the tests).
//!
//! Each character n-gram (unigram, bigram, trigram) hashes to a bucket **and a
//! sign**, so strings sharing n-grams land on overlapping buckets with consistent
//! signs and score high cosine similarity. Trigrams are weighted above bigrams
//! above unigrams, because a shared rare trigram is stronger evidence of
//! similarity than an incidental shared character.
//!
//! ## Honest scope
//!
//! This is a classical, non-learned embedding. It carries real lexical signal —
//! but it has no trained semantics, no subword vocabulary, and no notion of
//! meaning beyond surface form. It is not a sentence-transformer. Recall over
//! it finds *similarly worded* facts, not *related* ones.
//!
//! ## Why the hash is not interchangeable
//!
//! Both the bucket and the sign come from [`crate::wyhash`], the deliberate port
//! of Zig's `std.hash.Wyhash`. The vectors produced here are persisted into a
//! WDBX-format store that abi can read alongside its own; a different hash would
//! make cosine search across the two silently wrong.

use crate::wyhash;

/// Embedding dimensionality. Fixed by the persisted format — do not change.
pub const EMBED_DIM: usize = 32;

/// n-gram widths and their weights, in the order Zig applied them.
///
/// The order does not affect the result — every gram adds into the same
/// accumulator — but it is preserved so the implementations diff cleanly.
const GRAMS: [(usize, f32); 3] = [(1, 0.5), (2, 1.0), (3, 1.5)];

/// Embed `input` into a unit vector.
///
/// An empty input, or one whose signed features cancel exactly, maps to the fixed
/// unit vector `e₀` — never to a zero vector, which would make cosine similarity
/// undefined.
#[must_use]
pub fn text_embedding(input: &str) -> [f32; EMBED_DIM] {
    text_embedding_bytes(input.as_bytes())
}

/// Embed raw bytes, for callers holding a non-UTF-8 buffer.
///
/// The n-gram window is byte-oriented: a multi-byte character contributes its
/// individual bytes rather than one code point. That is a property of the
/// original, preserved deliberately — changing it would alter every persisted
/// vector.
#[must_use]
pub fn text_embedding_bytes(input: &[u8]) -> [f32; EMBED_DIM] {
    let mut out = [0.0_f32; EMBED_DIM];
    if input.is_empty() {
        out[0] = 1.0;
        return out;
    }

    let mut window = [0_u8; 3];
    for (n, weight) in GRAMS {
        let mut i = 0;
        while i + n <= input.len() {
            for k in 0..n {
                window[k] = input[i + k].to_ascii_lowercase();
            }
            // Seeding by `n` keeps the unigram, bigram, and trigram feature
            // spaces distinct, so "ab" cannot collide with the unigram "a".
            let h = wyhash::hash(n as u64, &window[..n]);
            // EMBED_DIM is 32, so the remainder always fits in usize on every target.
            let bucket = (h % EMBED_DIM as u64) as usize;
            // Sign from the top bit: this is what makes shared n-grams
            // constructive and unrelated ones cancel.
            out[bucket] += if (h >> 63) & 1 == 0 { weight } else { -weight };
            i += 1;
        }
    }

    let norm: f32 = out.iter().map(|v| v * v).sum();
    if norm == 0.0 {
        out[0] = 1.0;
        return out;
    }
    let scale = norm.sqrt();
    for value in &mut out {
        *value /= scale;
    }
    out
}

/// Cosine similarity of two vectors.
///
/// Handles arbitrary (including mismatched) lengths by zipping to the shorter,
/// and returns `0.0` when either side has zero norm, so a degenerate stored
/// vector ranks last instead of producing `NaN`.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|v| v * v).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|v| v * v).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(vector: &[f32; EMBED_DIM]) -> f32 {
        vector.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    fn assert_close(actual: &[f32; EMBED_DIM], expected: &[f32; EMBED_DIM], label: &str) {
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (a - e).abs() < 1e-6,
                "{label}: dimension {i} is {a}, abi gives {e}"
            );
        }
    }

    #[test]
    fn empty_input_maps_to_the_first_basis_vector() {
        let vector = text_embedding("");
        assert_eq!(vector[0].to_bits(), 1.0_f32.to_bits());
        assert!(vector[1..].iter().all(|v| v.to_bits() == 0.0_f32.to_bits()));
    }

    #[test]
    fn embeddings_are_unit_length() {
        for input in [
            "a",
            "ab",
            "abc",
            "hello world",
            "the quick brown fox",
            "日本語",
        ] {
            let vector = text_embedding(input);
            assert!(
                (norm(&vector) - 1.0).abs() < 0.001,
                "{input} has norm {}",
                norm(&vector)
            );
        }
    }

    #[test]
    fn embedding_is_deterministic_and_case_insensitive() {
        let a = text_embedding("Hello World");
        let b = text_embedding("hello world");
        let c = text_embedding("hello world");
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
        assert!(b.iter().zip(&c).all(|(x, y)| x.to_bits() == y.to_bits()));
    }

    #[test]
    fn similar_strings_score_above_unrelated_ones() {
        let base = text_embedding("hello world");
        let near = text_embedding("hello worlds");
        let far = text_embedding("zzzz qqqq vvvv");
        let (near_score, far_score) = (cosine(&base, &near), cosine(&base, &far));
        assert!(near_score > far_score, "near={near_score} far={far_score}");
        assert!(near_score > 0.9, "near={near_score}");
    }

    #[test]
    fn cosine_is_one_for_identical_and_zero_for_degenerate() {
        let v = text_embedding("abbey");
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&v, &[0.0; EMBED_DIM]), 0.0);
        assert_eq!(cosine(&[], &v), 0.0);
    }

    #[test]
    fn non_utf8_bytes_are_accepted() {
        let vector = text_embedding_bytes(&[0xff, 0xfe, 0x00, 0x41]);
        assert!((norm(&vector) - 1.0).abs() < 0.001);
    }

    /// Golden vectors computed by running abi's `abi_ai::embedding::text_embedding`
    /// (a scratch crate depending on `~/dev/active/abi/crates/abi-ai` by path,
    /// 2026-08-19). If this test fails, persisted vectors no longer match abi's —
    /// fix the transcription, never the constants.
    #[test]
    fn matches_abi_golden_vector_abbey_bot() {
        let expected = [
            0.306_186_2,
            -0.102_062_07,
            -0.102_062_07,
            0.306_186_2,
            0.102_062_07,
            0.0,
            0.0,
            0.0,
            0.0,
            -0.102_062_07,
            0.0,
            0.0,
            0.204_124_14,
            0.510_310_35,
            0.0,
            0.0,
            0.0,
            0.0,
            -0.204_124_14,
            0.0,
            0.0,
            -0.408_248_28,
            -0.204_124_14,
            -0.204_124_14,
            0.0,
            0.0,
            -0.306_186_2,
            -0.204_124_14,
            0.0,
            -0.204_124_14,
            0.0,
            0.0,
        ];
        assert_close(&text_embedding("abbey bot"), &expected, "abbey bot");
    }

    #[test]
    fn matches_abi_golden_vector_quick_brown_fox() {
        let expected = [
            -0.189_736_66,
            0.063_245_56,
            -0.252_982_23,
            -0.126_491_11,
            0.063_245_56,
            0.0,
            0.0,
            -0.252_982_23,
            0.0,
            0.252_982_23,
            0.0,
            0.126_491_11,
            0.0,
            -0.063_245_56,
            0.126_491_11,
            -0.126_491_11,
            -0.316_227_76,
            -0.063_245_56,
            -0.063_245_56,
            0.126_491_11,
            0.0,
            -0.442_718_9,
            0.0,
            0.0,
            0.0,
            0.316_227_76,
            0.126_491_11,
            0.316_227_76,
            0.0,
            -0.126_491_11,
            -0.379_473_33,
            0.0,
        ];
        assert_close(
            &text_embedding("The quick brown fox"),
            &expected,
            "The quick brown fox",
        );
    }

    #[test]
    fn matches_abi_golden_vector_hello_world() {
        let expected = [
            0.0,
            0.0,
            -0.069_337_524,
            0.138_675_05,
            0.0,
            -0.346_687_61,
            0.0,
            -0.069_337_524,
            0.208_012_58,
            -0.346_687_61,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            -0.069_337_524,
            0.0,
            0.346_687_61,
            -0.485_362_68,
            0.0,
            0.0,
            -0.208_012_58,
            0.0,
            -0.208_012_58,
            0.138_675_05,
            -0.346_687_61,
            -0.138_675_05,
            -0.138_675_05,
            0.208_012_58,
            0.0,
            0.138_675_05,
        ];
        assert_close(&text_embedding("hello world"), &expected, "hello world");
    }
}
