//! What the human did *next* — the delayed half of the reward signal.
//!
//! `reward.rs` already holds a reply open for 150 s and collects evidence:
//! reactions (±1), a human reply (+0.5), a deletion (−2). That evidence is
//! real, but it is **untyped**: a reply that says "perfect, thanks" and a reply
//! that says "no, that's wrong" both score exactly +0.5, so the policy cannot
//! tell a reply that helped from one that had to be corrected. This module
//! types that observation, and `reward.rs` blends the typed value alongside the
//! existing heuristic rather than in place of it.
//!
//! Claim-honest scope: this makes the loop *closable*. [`classify`] is a
//! deterministic lexicon over one message's text plus the ask it followed — not
//! a model, and not a claim that Abbey understands whether she was helpful. It
//! is wired to the Discord reply-to path and to same-channel follow-ups
//! (`pipeline.rs`); nothing here reads reactions, edits, thread creation, or
//! voice, and in-scope attribution is a heuristic that can mis-credit a busy
//! channel (see [`crate::brain::reward::RewardCollector::observe_in_scope`]).
//!
//! Pure: no clock, no I/O, no randomness.

/// A typed, observable post-reply outcome.
///
/// Deliberately small. Every variant has to be recoverable from what Discord
/// actually hands the bot — message text and a reply-to pointer — with no
/// model call and no user survey.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReplyOutcome {
    /// The human thanked or affirmed ("thanks", "that worked", "perfect").
    /// The strongest positive evidence available from text alone.
    ExplicitThanks,
    /// A *new* question that shares topic with the ask Abbey answered. The
    /// conversation moved forward from the reply, which is weak positive
    /// evidence: useful enough to build on, not an endorsement.
    FollowUpQuestion,
    /// The same ask again in different words. The reply did not land — the
    /// human is still trying to get the original answer.
    RephrasedSameAsk,
    /// The human contradicted or corrected the reply ("no", "that's wrong",
    /// "doesn't work"). The clearest negative signal text can carry.
    Correction,
    /// Nothing further happened, or what happened was unrelated.
    ///
    /// **Weak evidence, not a negative.** Silence has too many innocent causes
    /// — the human read it and was satisfied, left, or never saw it — so this
    /// contributes exactly nothing to the delayed channel (see
    /// [`ReplyOutcome::delayed_value`]). The existing `REPLY_BASELINE` of −0.2
    /// already makes speaking-into-the-void cost something; charging silence a
    /// second time here would double-penalize it.
    NoEngagement,
}

impl ReplyOutcome {
    /// This outcome's contribution to the delayed channel, −1…1.
    ///
    /// Magnitudes are ordered by how much the observation actually tells us:
    /// an explicit thanks and an explicit correction are unambiguous (±1); a
    /// topical follow-up is engagement without a verdict (+0.4); a rephrase
    /// says the reply missed but not that it was harmful (−0.5).
    pub fn delayed_value(self) -> f32 {
        match self {
            ReplyOutcome::ExplicitThanks => 1.0,
            ReplyOutcome::FollowUpQuestion => 0.4,
            ReplyOutcome::RephrasedSameAsk => -0.5,
            ReplyOutcome::Correction => -1.0,
            ReplyOutcome::NoEngagement => 0.0,
        }
    }

    /// Whether crediting this outcome *without* a reply-to pointer requires the
    /// speaker to be the person the turn answered.
    ///
    /// `ExplicitThanks` and `Correction` come from marker words alone, so
    /// nothing about them says who they are aimed at: in a busy channel "thanks
    /// Carol!" would otherwise land on Abbey's open turn at full weight. Those
    /// need the original asker to corroborate them.
    ///
    /// The topical variants carry their own corroboration — they only fire when
    /// the message shares content words with the ask Abbey answered — so they
    /// stand on their own and may be credited to anyone in the channel.
    pub fn needs_the_original_asker(self) -> bool {
        matches!(
            self,
            ReplyOutcome::ExplicitThanks | ReplyOutcome::Correction
        )
    }
}

/// How loudly the delayed channel speaks relative to the immediate heuristic.
///
/// At 1.0 a clear thanks is worth about one positive reaction, which keeps the
/// two channels commensurate: a settled reply that was thanked lands near the
/// same reward as one that was 👍'd, and the ±3 clamp in `reward.rs` still
/// bounds the total.
pub const DELAYED_BLEND_WEIGHT: f32 = 1.0;

/// Blend the delayed channel into the immediate heuristic.
///
/// `immediate` is the accumulator `reward.rs` has always kept (baseline,
/// reactions, human reply, deletion). `delayed_sum`/`delayed_count` are the
/// typed outcomes credited to the same turn.
///
/// **With no outcome the result is `immediate`, returned unchanged** — not
/// `immediate + 0.0`, which would flip a −0.0 accumulator's sign. That is the
/// graceful-degradation guarantee: a turn nobody engaged with settles at
/// exactly the number it settled at before this module existed.
pub fn blend(immediate: f32, delayed_sum: f32, delayed_count: u16) -> f32 {
    if delayed_count == 0 {
        return immediate;
    }
    // Mean, not sum: three follow-up questions are more evidence than one, but
    // they are not three times the verdict, and an unbounded sum would let a
    // chatty channel saturate the clamp on engagement alone.
    immediate + DELAYED_BLEND_WEIGHT * (delayed_sum / f32::from(delayed_count))
}

/// Multi-word markers, matched against the lowercased message.
const CORRECTION_PHRASES: [&str; 14] = [
    "not correct",
    "not right",
    "that's not",
    "thats not",
    "that is not",
    "doesn't work",
    "doesnt work",
    "didn't work",
    "didnt work",
    "not what i",
    "you're wrong",
    "youre wrong",
    "you are wrong",
    "still broken",
];

/// Single-token markers, matched against whole tokens so `wrong` fires but
/// `wrongdoing` does not.
///
/// `false` is deliberately absent: in a developer channel it is a literal far
/// more often than a verdict.
const CORRECTION_TOKENS: [&str; 3] = ["wrong", "incorrect", "nope"];

/// A message that *opens* with one of these is a correction. Only the first
/// token counts — "there is no way to do that" is a statement, "no, that's the
/// other port" is a correction — which is also why bare `no` is not in
/// [`CORRECTION_TOKENS`].
const CORRECTION_OPENERS: [&str; 3] = ["no", "nope", "nah"];

/// Idioms that open with `no` and mean the opposite of a correction. Checked
/// before [`CORRECTION_OPENERS`] so "no worries" is not read as pushback.
const FRIENDLY_NO_OPENERS: [&str; 4] = ["no problem", "no worries", "no rush", "no need"];

const THANKS_PHRASES: [&str; 8] = [
    "thank you",
    "that helped",
    "that worked",
    "appreciate it",
    "makes sense",
    "nice one",
    "good bot",
    "that did it",
];

const THANKS_TOKENS: [&str; 9] = [
    "thanks",
    "thx",
    "ty",
    "tysm",
    "tyvm",
    "appreciate",
    "appreciated",
    "perfect",
    "helpful",
];

/// Dropped before measuring topic overlap: they are shared by every English
/// sentence and would make unrelated questions look like rephrasings.
const STOPWORDS: [&str; 34] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "does", "for", "from",
    "how", "i", "in", "is", "it", "me", "my", "of", "on", "or", "that", "the", "this", "to", "was",
    "what", "when", "where", "why", "you",
];

/// Share of the prior ask's content words that must reappear for the new
/// question to count as the *same* ask rather than a new one.
const REPHRASE_OVERLAP: f32 = 0.6;
/// Share that makes it the same *topic* — enough to read as building on the
/// reply, not enough to be the same question.
const FOLLOW_UP_OVERLAP: f32 = 0.25;

/// Type one observed message as a post-reply outcome.
///
/// `prior_ask` is the user message Abbey's turn answered; without it the
/// topical variants are unknowable and only the explicit markers can fire.
///
/// Returns `None` when the text carries no usable signal. `None` is *not*
/// [`ReplyOutcome::NoEngagement`] — the caller decides whether an
/// unclassifiable message means the human moved on (`NoEngagement`) or means
/// nothing at all. Silence produces no message, so this function can never
/// return `NoEngagement` itself.
///
/// Precedence is deliberate: a correction beats a thanks, because "thanks, but
/// that's wrong" is a correction wearing a courtesy. Explicit markers beat
/// topical inference, because they say what happened rather than infer it.
pub fn classify(text: &str, prior_ask: Option<&str>) -> Option<ReplyOutcome> {
    let lower = normalize(text);
    let tokens = content_tokens_keeping_stopwords(&lower);
    if tokens.is_empty() {
        return None;
    }

    let friendly_no = FRIENDLY_NO_OPENERS.iter().any(|p| lower.starts_with(p));
    let opens_with_no = !friendly_no
        && tokens
            .first()
            .is_some_and(|t| CORRECTION_OPENERS.contains(t));
    if opens_with_no
        || CORRECTION_PHRASES.iter().any(|p| lower.contains(p))
        || tokens.iter().any(|t| CORRECTION_TOKENS.contains(t))
    {
        return Some(ReplyOutcome::Correction);
    }
    if THANKS_PHRASES.iter().any(|p| lower.contains(p))
        || tokens.iter().any(|t| THANKS_TOKENS.contains(t))
    {
        return Some(ReplyOutcome::ExplicitThanks);
    }

    // Topical inference needs both a question and something to compare it to.
    // `ends_with('?')` matches `state::encode`'s question feature exactly, so
    // the two never disagree about what a question is.
    if !text.trim_end().ends_with('?') {
        return None;
    }
    let prior = normalize(prior_ask?);
    let prior_content = content_tokens(&prior);
    if prior_content.is_empty() {
        return None;
    }
    let new_content = content_tokens(&lower);
    let shared = prior_content
        .iter()
        .filter(|t| new_content.contains(*t))
        .count();
    let overlap = shared as f32 / prior_content.len() as f32;
    if overlap >= REPHRASE_OVERLAP {
        Some(ReplyOutcome::RephrasedSameAsk)
    } else if overlap >= FOLLOW_UP_OVERLAP {
        Some(ReplyOutcome::FollowUpQuestion)
    } else {
        // A question about something else is not evidence about the reply.
        None
    }
}

/// Lowercase, and fold the typographic apostrophe onto the ASCII one so
/// `that’s not` (what phones and Discord autocorrect produce) matches the same
/// phrase list as `that's not`.
fn normalize(text: &str) -> String {
    text.trim().to_lowercase().replace('\u{2019}', "'")
}

/// Lowercased alphanumeric tokens, stopwords included — what the marker lists
/// match against (`no` and `ty` are stopword-shaped but load-bearing).
fn content_tokens_keeping_stopwords(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Deduplicated content words: stopwords and one-character tokens dropped.
fn content_tokens(lower: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for t in content_tokens_keeping_stopwords(lower) {
        if t.chars().count() > 1 && !STOPWORDS.contains(&t) && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn no_engagement_is_worth_exactly_nothing() {
        assert_eq!(ReplyOutcome::NoEngagement.delayed_value(), 0.0);
    }

    #[test]
    fn only_marker_derived_outcomes_need_the_original_asker() {
        assert!(ReplyOutcome::ExplicitThanks.needs_the_original_asker());
        assert!(ReplyOutcome::Correction.needs_the_original_asker());
        // These fire only on shared content words, which is corroboration
        // enough on its own.
        assert!(!ReplyOutcome::FollowUpQuestion.needs_the_original_asker());
        assert!(!ReplyOutcome::RephrasedSameAsk.needs_the_original_asker());
        assert!(!ReplyOutcome::NoEngagement.needs_the_original_asker());
    }

    #[test]
    fn delayed_values_are_ordered_by_how_much_they_tell_us() {
        let v = |o: ReplyOutcome| o.delayed_value();
        assert!(v(ReplyOutcome::ExplicitThanks) > v(ReplyOutcome::FollowUpQuestion));
        assert!(v(ReplyOutcome::FollowUpQuestion) > v(ReplyOutcome::NoEngagement));
        assert!(v(ReplyOutcome::NoEngagement) > v(ReplyOutcome::RephrasedSameAsk));
        assert!(v(ReplyOutcome::RephrasedSameAsk) > v(ReplyOutcome::Correction));
        for o in [
            ReplyOutcome::ExplicitThanks,
            ReplyOutcome::FollowUpQuestion,
            ReplyOutcome::RephrasedSameAsk,
            ReplyOutcome::Correction,
            ReplyOutcome::NoEngagement,
        ] {
            assert!((-1.0..=1.0).contains(&o.delayed_value()), "{o:?}");
        }
    }

    #[test]
    fn blend_without_any_outcome_returns_the_immediate_value_untouched() {
        for immediate in [-3.0f32, -0.2, 0.0, 0.3, 2.8] {
            let blended = blend(immediate, 0.0, 0);
            assert_eq!(
                blended.to_bits(),
                immediate.to_bits(),
                "no delayed evidence must not perturb {immediate} even in its bits"
            );
        }
        // Including the sign of negative zero, which `x + 0.0` would erase.
        assert_eq!(blend(-0.0, 0.0, 0).to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn blend_adds_the_weighted_mean_of_the_delayed_channel() {
        // One thanks on a baseline reply.
        assert!(approx(
            blend(-0.2, ReplyOutcome::ExplicitThanks.delayed_value(), 1),
            -0.2 + DELAYED_BLEND_WEIGHT * 1.0
        ));
        // Mean, not sum: two follow-ups are still one follow-up's worth.
        let two = ReplyOutcome::FollowUpQuestion.delayed_value() * 2.0;
        assert!(approx(blend(0.0, two, 2), 0.4));
        // Mixed evidence pulls toward the middle.
        let mixed =
            ReplyOutcome::ExplicitThanks.delayed_value() + ReplyOutcome::Correction.delayed_value();
        assert!(approx(blend(0.5, mixed, 2), 0.5));
    }

    #[test]
    fn thanks_is_recognised_as_token_and_as_phrase() {
        for t in [
            "thanks!",
            "Thanks",
            "thank you so much",
            "thx",
            "ty",
            "that worked",
            "that helped a lot",
            "perfect",
            "appreciate it",
            "good bot",
            "makes sense now",
        ] {
            assert_eq!(
                classify(t, None),
                Some(ReplyOutcome::ExplicitThanks),
                "{t:?}"
            );
        }
    }

    #[test]
    fn thanks_tokens_do_not_fire_inside_longer_words() {
        // "pretty" contains "ty", "perfectly" contains "perfect".
        assert_eq!(classify("pretty quiet in here", None), None);
        assert_eq!(classify("perfectly ordinary sentence", None), None);
    }

    #[test]
    fn correction_is_recognised_and_beats_a_courtesy_thanks() {
        for t in [
            "no",
            "nope",
            "that's wrong",
            "that is not what happens",
            "it doesn't work",
            "you're wrong about the port",
            "incorrect",
            "still broken",
        ] {
            assert_eq!(classify(t, None), Some(ReplyOutcome::Correction), "{t:?}");
        }
        assert_eq!(
            classify("thanks, but that's wrong", None),
            Some(ReplyOutcome::Correction),
            "a correction wearing a courtesy is still a correction"
        );
    }

    #[test]
    fn correction_tokens_do_not_fire_inside_longer_words() {
        assert_eq!(classify("wrongdoing aside", None), None);
        assert_eq!(classify("nobody minds", None), None);
    }

    #[test]
    fn bare_no_only_corrects_when_it_opens_the_message() {
        assert_eq!(classify("no", None), Some(ReplyOutcome::Correction));
        assert_eq!(
            classify("no, the port is 8181", None),
            Some(ReplyOutcome::Correction)
        );
        assert_eq!(classify("nah", None), Some(ReplyOutcome::Correction));
        assert_eq!(
            classify("there is no way around it", None),
            None,
            "`no` mid-sentence is a statement, not pushback"
        );
    }

    #[test]
    fn friendly_no_idioms_are_not_corrections() {
        for t in ["no problem", "no worries!", "no rush", "no need, got it"] {
            assert_ne!(classify(t, None), Some(ReplyOutcome::Correction), "{t:?}");
        }
        // "no need, got it" still reads positive on its own merits.
        assert_eq!(
            classify("no need, got it", None),
            None,
            "friendly, but carrying no thanks marker either"
        );
    }

    #[test]
    fn a_typographic_apostrophe_matches_the_same_phrases() {
        assert_eq!(
            classify("that\u{2019}s not what I meant", None),
            Some(ReplyOutcome::Correction)
        );
        assert_eq!(
            classify("that\u{2019}s not right", None),
            Some(ReplyOutcome::Correction)
        );
    }

    #[test]
    fn a_rephrase_of_the_same_ask_outscores_a_new_topical_question() {
        let ask = "how do I configure the voice gateway timeout?";
        assert_eq!(
            classify(
                "how can the voice gateway timeout be configured?",
                Some(ask)
            ),
            Some(ReplyOutcome::RephrasedSameAsk),
            "same content words, different wording"
        );
        assert_eq!(
            classify("does the gateway retry after a timeout?", Some(ask)),
            Some(ReplyOutcome::FollowUpQuestion),
            "shares topic, asks something new"
        );
    }

    #[test]
    fn an_unrelated_question_is_not_evidence_about_the_reply() {
        let ask = "how do I configure the voice gateway timeout?";
        assert_eq!(classify("anyone up for lunch?", Some(ask)), None);
    }

    #[test]
    fn topical_inference_needs_a_question_and_a_prior_ask() {
        let ask = "how do I configure the voice gateway timeout?";
        assert_eq!(
            classify("the voice gateway timeout is configured", Some(ask)),
            None,
            "a statement is not a follow-up question"
        );
        assert_eq!(
            classify("how do I configure the voice gateway timeout?", None),
            None,
            "with nothing to compare against, topic is unknowable"
        );
        assert_eq!(
            classify("how do I configure the voice gateway timeout?", Some("")),
            None,
            "an empty prior ask has no content words"
        );
        assert_eq!(
            classify("how do I configure it?", Some("the the a an")),
            None,
            "a prior ask that is all stopwords has no content words"
        );
    }

    #[test]
    fn empty_and_punctuation_only_text_classifies_as_nothing() {
        assert_eq!(classify("", None), None);
        assert_eq!(classify("   ", None), None);
        assert_eq!(classify("!!! ...", None), None);
        assert_eq!(classify("?", Some("how do I build it?")), None);
    }

    #[test]
    fn classification_is_deterministic_and_case_insensitive() {
        let ask = "how do I configure the voice gateway timeout?";
        let q = "HOW CAN THE VOICE GATEWAY TIMEOUT BE CONFIGURED?";
        assert_eq!(classify(q, Some(ask)), classify(q, Some(ask)));
        assert_eq!(
            classify(q, Some(ask)),
            Some(ReplyOutcome::RephrasedSameAsk),
            "shouting is still the same ask"
        );
    }

    #[test]
    fn overlap_is_measured_against_the_prior_ask_not_the_new_question() {
        // A long new question that happens to contain the whole short ask is a
        // rephrase by this measure — the human is still asking the same thing.
        assert_eq!(
            classify(
                "sorry, restating: what is the gateway timeout, in seconds?",
                Some("what is the gateway timeout?")
            ),
            Some(ReplyOutcome::RephrasedSameAsk)
        );
    }

    #[test]
    fn a_word_repeated_in_the_prior_ask_does_not_inflate_the_overlap() {
        // Counted with duplicates the shared share would be 3/4 and this would
        // read as a rephrase; deduplicated it is 1/2 — merely topical.
        assert_eq!(
            classify("timeout?", Some("timeout timeout timeout config?")),
            Some(ReplyOutcome::FollowUpQuestion)
        );
    }
}
