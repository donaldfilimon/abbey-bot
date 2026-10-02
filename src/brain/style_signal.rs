//! Closed-vocabulary style feedback: "too long", "less formal", "no emoji".
//!
//! [`classify`] maps one member's message to at most one [`StyleSignal`]
//! through the fixed [`LEXICON`] below. It is deliberately conservative:
//! lowercase whole phrases on word boundaries, a negated phrase ("not too
//! long") never counts, a message that asks for both directions of one knob
//! is ambiguous and yields nothing, and anything longer than
//! [`MAX_CLASSIFIED_CHARS`] is ignored as conversation rather than feedback.
//! Quoted or code-formatted messages are also ignored: pasted feedback is
//! not evidence of the member's own preference, even in a short message.
//!
//! The signal is the only thing that leaves this module. The member's words
//! never reach [`crate::brain::addenda`], so they can never reach a prompt.
//!
//! Pure: no clock, no I/O, no randomness.

use serde::{Deserialize, Serialize};

/// Messages longer than this are conversation, not a style note.
pub const MAX_CLASSIFIED_CHARS: usize = 200;

/// One direction of feedback on one [`StyleKnob`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StyleSignal {
    TooLong,
    TooShort,
    TooFormal,
    TooCasual,
    NoEmoji,
    MoreEmoji,
    PreferCode,
    FewerFollowUps,
}

/// The closed adjustable dimensions; FollowUp can only reduce contact. Their
/// order is the render order and the priority order: when bytes run out, the last knob is dropped first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StyleKnob {
    Length,
    Formality,
    Emoji,
    Code,
    FollowUp,
}

impl StyleSignal {
    pub fn knob(self) -> StyleKnob {
        match self {
            Self::TooLong | Self::TooShort => StyleKnob::Length,
            Self::TooFormal | Self::TooCasual => StyleKnob::Formality,
            Self::NoEmoji | Self::MoreEmoji => StyleKnob::Emoji,
            Self::PreferCode => StyleKnob::Code,
            Self::FewerFollowUps => StyleKnob::FollowUp,
        }
    }

    /// The signal that pulls the same knob the other way, if any.
    pub fn opposite(self) -> Option<Self> {
        match self {
            Self::TooLong => Some(Self::TooShort),
            Self::TooShort => Some(Self::TooLong),
            Self::TooFormal => Some(Self::TooCasual),
            Self::TooCasual => Some(Self::TooFormal),
            Self::NoEmoji => Some(Self::MoreEmoji),
            Self::MoreEmoji => Some(Self::NoEmoji),
            Self::PreferCode | Self::FewerFollowUps => None,
        }
    }
}

/// The whole vocabulary, in priority order: the first phrase that matches
/// decides the signal. Every phrase is lowercase ASCII.
pub const LEXICON: &[(&str, StyleSignal)] = &[
    ("fewer follow-ups", StyleSignal::FewerFollowUps),
    ("less follow-up", StyleSignal::FewerFollowUps),
    ("too long", StyleSignal::TooLong),
    ("too wordy", StyleSignal::TooLong),
    ("tl;dr", StyleSignal::TooLong),
    ("tldr", StyleSignal::TooLong),
    ("wall of text", StyleSignal::TooLong),
    ("shorter please", StyleSignal::TooLong),
    ("keep it short", StyleSignal::TooLong),
    ("be more concise", StyleSignal::TooLong),
    ("too short", StyleSignal::TooShort),
    ("more detail", StyleSignal::TooShort),
    ("more details", StyleSignal::TooShort),
    ("longer please", StyleSignal::TooShort),
    ("too formal", StyleSignal::TooFormal),
    ("less formal", StyleSignal::TooFormal),
    ("too stiff", StyleSignal::TooFormal),
    ("more casual", StyleSignal::TooFormal),
    ("too casual", StyleSignal::TooCasual),
    ("less casual", StyleSignal::TooCasual),
    ("more formal", StyleSignal::TooCasual),
    ("more professional", StyleSignal::TooCasual),
    ("no emoji", StyleSignal::NoEmoji),
    ("no emojis", StyleSignal::NoEmoji),
    ("fewer emoji", StyleSignal::NoEmoji),
    ("fewer emojis", StyleSignal::NoEmoji),
    ("less emoji", StyleSignal::NoEmoji),
    ("stop using emoji", StyleSignal::NoEmoji),
    ("stop using emojis", StyleSignal::NoEmoji),
    ("more emoji", StyleSignal::MoreEmoji),
    ("more emojis", StyleSignal::MoreEmoji),
    ("use emoji", StyleSignal::MoreEmoji),
    ("use emojis", StyleSignal::MoreEmoji),
    ("show the code", StyleSignal::PreferCode),
    ("show me the code", StyleSignal::PreferCode),
    ("just the code", StyleSignal::PreferCode),
    ("code please", StyleSignal::PreferCode),
    ("use code blocks", StyleSignal::PreferCode),
];

/// Words that, directly before a phrase, turn it into its negation.
const NEGATIONS: &[&str] = &["not", "never", "isn't", "wasn't", "aren't", "don't"];

/// Map one message to at most one style signal. See the module docs.
pub fn classify(text: &str) -> Option<StyleSignal> {
    if text.chars().count() > MAX_CLASSIFIED_CHARS || contains_quotation(text) {
        return None;
    }
    let lowered = text.to_lowercase();
    let signal = LEXICON
        .iter()
        .find(|(phrase, _)| matches_phrase(&lowered, phrase))
        .map(|&(_, signal)| signal)?;
    let contradicted = signal.opposite().is_some_and(|opposite| {
        LEXICON
            .iter()
            .any(|&(phrase, other)| other == opposite && matches_phrase(&lowered, phrase))
    });
    (!contradicted).then_some(signal)
}

/// Conservatively reject quotation/code markup rather than attributing pasted
/// words to the member. Apostrophes inside words remain ordinary feedback.
fn contains_quotation(text: &str) -> bool {
    if text.contains(['`', '"', '“', '”', '‘'])
        || text
            .lines()
            .any(|line| line.trim_start().starts_with('>') || line.trim_start().starts_with("~~~"))
    {
        return true;
    }
    let mut previous = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\''
            && !previous.is_some_and(char::is_alphanumeric)
            && chars.peek().is_some_and(|next| next.is_alphanumeric())
        {
            return true;
        }
        previous = Some(c);
    }
    false
}

/// Whether `phrase` occurs in `hay` as whole words, not negated.
fn matches_phrase(hay: &str, phrase: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = hay[from..].find(phrase) {
        let start = from + offset;
        let end = start + phrase.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) && !negated(&hay[..start]) {
            return true;
        }
        from = start + phrase.chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '\''
}

/// Whether the word immediately before a match is a negation.
fn negated(prefix: &str) -> bool {
    prefix
        .split(|c: char| !is_word(c))
        .rfind(|word| !word.is_empty())
        .is_some_and(|word| NEGATIONS.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicon_phrases_are_lowercase_ascii() {
        for (phrase, _) in LEXICON {
            assert!(phrase.is_ascii(), "{phrase}");
            assert_eq!(*phrase, phrase.to_lowercase(), "{phrase}");
        }
    }

    #[test]
    fn positive_cases_map_to_their_signal() {
        let cases = [
            ("way too long", StyleSignal::TooLong),
            ("TL;DR please", StyleSignal::TooLong),
            ("Too Short!", StyleSignal::TooShort),
            ("can you give more details?", StyleSignal::TooShort),
            ("abbey you're too formal", StyleSignal::TooFormal),
            ("a bit more professional please", StyleSignal::TooCasual),
            ("no emojis pls", StyleSignal::NoEmoji),
            ("more emoji!!", StyleSignal::MoreEmoji),
            ("just the code", StyleSignal::PreferCode),
        ];
        for (text, expected) in cases {
            assert_eq!(classify(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn negative_cases_yield_nothing() {
        let cases = [
            "",
            "thanks, that worked",
            "the movie was toolong",
            "tldrs are my favourite",
            "not too long at all",
            "that wasn't too formal",
            "too long, actually no, too short",
            "the noemoji flag",
            "no emojify here",
            "microcode please",
        ];
        for text in cases {
            assert_eq!(classify(text), None, "{text}");
        }
    }

    #[test]
    fn long_messages_are_conversation_not_feedback() {
        let text = format!("too long {}", "x".repeat(MAX_CLASSIFIED_CHARS));
        assert_eq!(classify(&text), None);
        let at_cap = format!("too long{}", " ".repeat(MAX_CLASSIFIED_CHARS - 8));
        assert_eq!(classify(&at_cap), Some(StyleSignal::TooLong));
    }

    #[test]
    fn pasted_style_phrases_are_not_member_feedback() {
        for text in [
            "\"too long\"",
            "she said 'too long'",
            "“no emoji”",
            "‘more casual’",
            "`too long`",
            "```text\ntoo long\n```",
            "~~~text\ntoo long\n~~~",
            "> too long",
            "  > no emoji",
            "quoted feedback:\n>>> more detail",
            "too long\n> too short",
            "```\ntoo long",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
    }

    #[test]
    fn direct_feedback_with_contractions_still_counts() {
        for text in ["Abbey, you're too formal", "Abbey, you’re too formal"] {
            assert_eq!(classify(text), Some(StyleSignal::TooFormal), "{text}");
        }
        assert_eq!(classify("that's too long"), Some(StyleSignal::TooLong));
    }

    #[test]
    fn first_match_in_priority_order_wins_across_knobs() {
        assert_eq!(
            classify("no emoji and too long"),
            Some(StyleSignal::TooLong)
        );
    }

    #[test]
    fn every_signal_but_code_has_an_opposite_on_its_own_knob() {
        for (_, signal) in LEXICON {
            if let Some(opposite) = signal.opposite() {
                assert_eq!(opposite.knob(), signal.knob());
                assert_eq!(opposite.opposite(), Some(*signal));
            }
        }
    }
}

#[cfg(test)]
mod reduction_tests {
    use super::*;
    #[test]
    fn follow_up_feedback_rejects_quoted_code_and_negated_examples() {
        assert_eq!(
            classify("fewer follow-ups please"),
            Some(StyleSignal::FewerFollowUps)
        );
        for text in [
            "\"fewer follow-ups\"",
            "`fewer follow-ups`",
            "> fewer follow-ups",
            "~~~\nfewer follow-ups\n~~~",
            "not fewer follow-ups",
            "she said 'fewer follow-ups'",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
    }
}
