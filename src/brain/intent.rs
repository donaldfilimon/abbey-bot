//! Rule-based intent classification (`docs/spec/brain.md`, "IntentClassifier.swift").
//!
//! Pure: no serenity, no poise, no I/O. The rules are ordered exactly as the spec
//! orders them, because the first matching rule wins and the state encoder's
//! one-hot depends on [`Intent::ALL`] keeping the spec's declaration order.

/// What a message is trying to do. Declaration order is load-bearing: the state
/// encoder's one-hot slot for an intent is its position in [`Intent::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    Question,
    Greeting,
    ModRequest,
    MemoryStore,
    PersonaSwitch,
    RepQuery,
    SmallTalk,
    Command,
    Unknown,
}

impl Intent {
    /// Every intent, in the spec's `allCases` order.
    pub const ALL: [Intent; 9] = [
        Intent::Question,
        Intent::Greeting,
        Intent::ModRequest,
        Intent::MemoryStore,
        Intent::PersonaSwitch,
        Intent::RepQuery,
        Intent::SmallTalk,
        Intent::Command,
        Intent::Unknown,
    ];

    /// Position of this intent in [`Intent::ALL`] — the one-hot slot.
    pub fn index(self) -> usize {
        Intent::ALL
            .iter()
            .position(|&i| i == self)
            .expect("every Intent variant is listed in Intent::ALL")
    }

    /// Interaction quality fed to the reputation engine.
    pub fn quality(self) -> f64 {
        match self {
            Intent::Question | Intent::ModRequest | Intent::MemoryStore => 0.8,
            Intent::Greeting | Intent::SmallTalk => 0.5,
            Intent::Unknown => 0.2,
            Intent::PersonaSwitch | Intent::RepQuery | Intent::Command => 0.6,
        }
    }
}

const GREETING_PREFIXES: [&str; 5] = ["hi", "hey", "yo", "sup", "hello"];

const COMPLETION_CORPUS: [&str; 5] = [
    "what do you think about",
    "how does",
    "can you help me with",
    "remind me",
    "what is the reputation of",
];

/// Classify a message. First matching rule wins, in the spec's order.
///
/// `Intent::Unknown` is unreachable by design: the spec's fallthrough is
/// `SmallTalk`, and the spec records that as an open decision ("Either the
/// fallthrough should be `.unknown` … or `.unknown` should be dropped — Donald's
/// call, left as-is"). `Intent::ModRequest` is likewise never produced by these
/// rules. Both are preserved here exactly as the spec has them; do not "fix" the
/// fallthrough without that decision being made.
pub fn classify(text: &str) -> Intent {
    let lower = text.to_lowercase();
    if lower.starts_with('!') || lower.starts_with('/') {
        return Intent::Command;
    }
    if lower.contains("remember") || lower.contains("note that") {
        return Intent::MemoryStore;
    }
    if lower.contains("rep") || lower.contains("reputation") {
        return Intent::RepQuery;
    }
    if lower.contains("switch") || lower.contains("be aviva") {
        return Intent::PersonaSwitch;
    }
    if lower.ends_with('?') || lower.starts_with("what") || lower.starts_with("how") {
        return Intent::Question;
    }
    if GREETING_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return Intent::Greeting;
    }
    Intent::SmallTalk
}

/// Corpus phrases that begin with `partial` (case-insensitive on the input).
pub fn suggest_completions(partial: &str) -> Vec<&'static str> {
    let lower = partial.to_lowercase();
    COMPLETION_CORPUS
        .iter()
        .copied()
        .filter(|c| c.starts_with(lower.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_has_nine_distinct_intents_in_declaration_order() {
        assert_eq!(Intent::ALL.len(), 9);
        for (i, intent) in Intent::ALL.iter().enumerate() {
            assert_eq!(intent.index(), i, "{intent:?} one-hot slot");
        }
        assert_eq!(Intent::ALL[0], Intent::Question);
        assert_eq!(Intent::ALL[8], Intent::Unknown);
    }

    #[test]
    fn quality_values_match_spec() {
        assert_eq!(Intent::Question.quality(), 0.8);
        assert_eq!(Intent::ModRequest.quality(), 0.8);
        assert_eq!(Intent::MemoryStore.quality(), 0.8);
        assert_eq!(Intent::Greeting.quality(), 0.5);
        assert_eq!(Intent::SmallTalk.quality(), 0.5);
        assert_eq!(Intent::Unknown.quality(), 0.2);
        assert_eq!(Intent::PersonaSwitch.quality(), 0.6);
        assert_eq!(Intent::RepQuery.quality(), 0.6);
        assert_eq!(Intent::Command.quality(), 0.6);
    }

    #[test]
    fn command_prefix_beats_everything() {
        assert_eq!(classify("!remember rep switch what?"), Intent::Command);
        assert_eq!(classify("/hey remember?"), Intent::Command);
    }

    #[test]
    fn remember_is_memory_store_and_beats_later_rules() {
        assert_eq!(classify("remember this"), Intent::MemoryStore);
        assert_eq!(classify("Note that I like tea"), Intent::MemoryStore);
        assert_eq!(classify("remember my rep?"), Intent::MemoryStore);
    }

    #[test]
    fn rep_beats_switch_and_question() {
        assert_eq!(classify("what's my reputation?"), Intent::RepQuery);
        assert_eq!(classify("switch rep"), Intent::RepQuery);
    }

    #[test]
    fn switch_is_persona_switch() {
        assert_eq!(classify("switch persona"), Intent::PersonaSwitch);
        assert_eq!(classify("please be aviva?"), Intent::PersonaSwitch);
    }

    #[test]
    fn question_by_suffix_or_prefix() {
        assert_eq!(classify("is it late?"), Intent::Question);
        assert_eq!(classify("What time is it"), Intent::Question);
        assert_eq!(classify("how about no"), Intent::Question);
    }

    #[test]
    fn greeting_prefixes() {
        assert_eq!(classify("hey there"), Intent::Greeting);
        assert_eq!(classify("Hello!"), Intent::Greeting);
        assert_eq!(classify("yo"), Intent::Greeting);
        assert_eq!(classify("sup"), Intent::Greeting);
        assert_eq!(classify("hi"), Intent::Greeting);
        // "hey?" ends with '?' so the question rule wins first.
        assert_eq!(classify("hey?"), Intent::Question);
    }

    #[test]
    fn fallthrough_is_small_talk_and_unknown_is_never_produced() {
        assert_eq!(classify("nice weather today"), Intent::SmallTalk);
        assert_eq!(classify(""), Intent::SmallTalk);
        let corpus = [
            "",
            "!ban",
            "/persona",
            "remember me",
            "rep",
            "switch",
            "why?",
            "what",
            "howdy",
            "hi",
            "hey",
            "yo",
            "sup",
            "hello",
            "just chilling",
            "🔥🔥🔥",
            "   ",
            "ok",
        ];
        for text in corpus {
            assert_ne!(classify(text), Intent::Unknown, "{text:?}");
        }
    }

    #[test]
    fn completions_filter_by_prefix_case_insensitively() {
        assert_eq!(
            suggest_completions("what"),
            vec!["what do you think about", "what is the reputation of"]
        );
        assert_eq!(suggest_completions("HOW"), vec!["how does"]);
        assert_eq!(suggest_completions(""), COMPLETION_CORPUS.to_vec());
        assert!(suggest_completions("zzz").is_empty());
    }
}
