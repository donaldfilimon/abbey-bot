//! Relevance selection over a user's durable facts.
//!
//! Every fact a user has ever had remembered used to enter every prompt:
//! `PersonaContext::render` joined the whole `Vec<String>` with `"; "`. At the
//! hundred-fact cap ([`crate::memory::MAX_FACTS`]) that is up to 30,000
//! characters of mostly-irrelevant biography in front of a message like "what
//! time is it", which both crowds the model's attention and wastes the local
//! 12B's context window.
//!
//! This module picks the facts worth showing for *this* message. It is pure,
//! deterministic, and does no I/O — no embedding call, no network, nothing on
//! the message hot path that can fail or stall. Ranking is lexical overlap
//! weighted by how rare each term is across the user's own facts, so a term
//! every fact shares carries almost no signal while a distinctive one
//! dominates.
//!
//! Scope is deliberate: selection *ranks*, it never deletes. `/forget` remains
//! the only thing that removes a fact, and it still erases from both stores.

/// Facts shown for one message before the rest are summarized as a count.
pub const MAX_CONTEXT_FACTS: usize = 8;

/// Character ceiling for the rendered fact list, independent of the count cap.
/// Four maximum-length facts ([`crate::memory::MAX_FACT_CHARS`]) fit.
pub const FACT_CONTEXT_CHARS: usize = 1_200;

/// English function words, dropped before ranking.
///
/// Rarity weighting turns any term appearing in few facts into a strong
/// signal, which backfires badly on function words: the "a" in "a rust
/// question" appears in exactly one stored fact, so it scored as decisively as
/// "rust" and dragged an unrelated fact to the top. Short-token filtering
/// would fix that too, but it would also discard `go`, `ai`, `js`, `c`, and
/// `os` — real subjects on a developer's server. An explicit list keeps those.
const STOPWORDS: &[&str] = &[
    "a", "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can",
    "could", "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "here", "his",
    "how", "i", "if", "in", "into", "is", "it", "its", "just", "me", "my", "no", "not", "of", "on",
    "or", "our", "out", "over", "please", "she", "should", "so", "some", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "those", "to", "too", "us", "very",
    "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would",
    "you", "your",
];

/// Lowercase alphanumeric words with function words removed. Everything that
/// is not alphanumeric separates, so `"Rust,"`, `"rust"` and `"RUST!"` are one
/// token.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| !STOPWORDS.contains(&word.as_str()))
        .collect()
}

/// What [`select`] chose, plus how much it held back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection<'a> {
    /// Chosen facts, most relevant first.
    pub facts: Vec<&'a str>,
    /// Facts that existed but did not fit the count or character budget.
    pub omitted: usize,
}

/// Rank `facts` against `query` and take the best that fit both budgets.
///
/// Ordering is by score descending, then by recency (later entries in `facts`
/// are newer, since `MemoryBank::remember` pushes). Facts sharing no term with
/// the query score zero and fill any remaining slots newest-first, so a user
/// with only a handful of facts still gets all of them regardless of wording.
///
/// A fact longer than the whole character budget is skipped rather than
/// allowed to blow it; selection continues with the next candidate.
#[must_use]
pub fn select<'a>(
    facts: &'a [String],
    query: &str,
    max_facts: usize,
    char_budget: usize,
) -> Selection<'a> {
    if facts.is_empty() || max_facts == 0 {
        return Selection {
            facts: Vec::new(),
            omitted: facts.len(),
        };
    }

    let fact_tokens: Vec<Vec<String>> = facts.iter().map(|fact| tokens(fact)).collect();

    // Document frequency across this user's own facts. A term in every fact
    // (their name, "rust" for a Rust developer) carries nearly no signal; a
    // term in one fact is decisive.
    let mut query_terms = tokens(query);
    query_terms.sort();
    query_terms.dedup();

    let mut scored: Vec<(f64, usize)> = Vec::with_capacity(facts.len());
    for (index, fact) in fact_tokens.iter().enumerate() {
        let mut score = 0.0_f64;
        for term in &query_terms {
            if fact.contains(term) {
                let df = fact_tokens
                    .iter()
                    .filter(|candidate| candidate.contains(term))
                    .count();
                score += 1.0 / (1.0 + df as f64);
            }
        }
        scored.push((score, index));
    }

    // Score descending, then newest first. Both keys are total, so the order
    // is deterministic for a given input — no dependence on sort stability.
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(right.1.cmp(&left.1))
    });

    let mut chosen = Vec::new();
    let mut used = 0usize;
    for (_, index) in scored {
        if chosen.len() >= max_facts {
            break;
        }
        let fact = facts[index].as_str();
        let cost = fact.chars().count();
        if used + cost > char_budget {
            continue;
        }
        used += cost;
        chosen.push(fact);
    }

    let omitted = facts.len() - chosen.len();
    Selection {
        facts: chosen,
        omitted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_matching_fact_outranks_unrelated_ones() {
        let stored = facts(&["likes hiking", "runs a homelab", "writes rust daily"]);
        let picked = select(
            &stored,
            "any rust tips?",
            MAX_CONTEXT_FACTS,
            FACT_CONTEXT_CHARS,
        );
        assert_eq!(picked.facts[0], "writes rust daily");
        assert_eq!(picked.omitted, 0, "all three still fit the budget");
    }

    #[test]
    fn a_term_shared_by_every_fact_carries_almost_no_signal() {
        // "rust" is in all three, so it cannot discriminate; "postgres" is in
        // one, so it decides. Without the rarity weighting the three would tie
        // and recency alone would answer, which is the bug this guards.
        let stored = facts(&[
            "rust and postgres at work",
            "rust hobby projects",
            "rust at a previous job",
        ]);
        let picked = select(&stored, "rust postgres question", 3, FACT_CONTEXT_CHARS);
        assert_eq!(picked.facts[0], "rust and postgres at work");
    }

    #[test]
    fn unrelated_facts_still_fill_remaining_slots_newest_first() {
        // Someone with few facts should keep seeing all of them, whatever the
        // wording of the message — focusing must not amount to forgetting.
        let stored = facts(&["oldest", "middle", "newest"]);
        let picked = select(&stored, "completely unrelated query", 3, FACT_CONTEXT_CHARS);
        assert_eq!(picked.facts, vec!["newest", "middle", "oldest"]);
        assert_eq!(picked.omitted, 0);
    }

    #[test]
    fn the_count_cap_reports_what_it_held_back() {
        let stored = facts(&["a", "b", "c", "d", "e"]);
        let picked = select(&stored, "", 2, FACT_CONTEXT_CHARS);
        assert_eq!(picked.facts.len(), 2);
        assert_eq!(picked.omitted, 3);
    }

    #[test]
    fn the_character_budget_is_never_exceeded() {
        let long = "x".repeat(300);
        let stored = facts(&[&long, &long, &long, &long, &long]);
        let picked = select(&stored, "", MAX_CONTEXT_FACTS, 1_000);
        assert_eq!(picked.facts.len(), 3, "1000 chars holds three 300s");
        assert_eq!(picked.omitted, 2);
    }

    #[test]
    fn an_oversized_fact_is_skipped_rather_than_bursting_the_budget() {
        let huge = "y".repeat(400);
        let stored = facts(&[&huge, "short and relevant"]);
        let picked = select(&stored, "short", MAX_CONTEXT_FACTS, 100);
        assert_eq!(picked.facts, vec!["short and relevant"]);
        assert_eq!(picked.omitted, 1);
    }

    #[test]
    fn selection_is_deterministic_across_repeated_calls() {
        let stored = facts(&["alpha one", "alpha two", "beta three", "alpha four"]);
        let first = select(&stored, "alpha", 3, FACT_CONTEXT_CHARS);
        for _ in 0..25 {
            assert_eq!(select(&stored, "alpha", 3, FACT_CONTEXT_CHARS), first);
        }
    }

    #[test]
    fn function_words_do_not_drive_ranking() {
        // Regression: rarity weighting made the "a" in "a rust question" look
        // decisive because it happened to appear in exactly one stored fact,
        // which floated an unrelated fact above the relevant one.
        let stored = facts(&["likes rust", "runs a homelab"]);
        let picked = select(&stored, "a rust question", 2, FACT_CONTEXT_CHARS);
        assert_eq!(picked.facts[0], "likes rust");
    }

    #[test]
    fn short_technical_terms_survive_stopword_filtering() {
        // The cheap fix for the bug above — dropping tokens under three
        // characters — would silently discard these real subjects.
        for term in ["go", "ai", "js", "c", "os"] {
            let stored = facts(&[&format!("writes {term} daily"), "unrelated entry"]);
            let picked = select(&stored, term, 1, FACT_CONTEXT_CHARS);
            assert_eq!(
                picked.facts,
                vec![format!("writes {term} daily")],
                "{term} must remain a usable retrieval key"
            );
        }
    }

    #[test]
    fn punctuation_and_case_do_not_change_matching() {
        let stored = facts(&["prefers PostgreSQL", "likes tea"]);
        let picked = select(&stored, "postgresql?", 1, FACT_CONTEXT_CHARS);
        assert_eq!(picked.facts, vec!["prefers PostgreSQL"]);
    }

    #[test]
    fn empty_inputs_are_handled_without_panicking() {
        assert_eq!(
            select(&[], "anything", MAX_CONTEXT_FACTS, FACT_CONTEXT_CHARS),
            Selection {
                facts: Vec::new(),
                omitted: 0
            }
        );
        let stored = facts(&["one"]);
        assert_eq!(
            select(&stored, "one", 0, FACT_CONTEXT_CHARS).omitted,
            1,
            "a zero cap holds everything back rather than panicking"
        );
    }
}
