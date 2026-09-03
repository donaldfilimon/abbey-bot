//! Action space and the 18-dimension state encoder
//! (`docs/spec/adaptivelearning.md`, "The action space" and "StateEncoder").
//!
//! Deterministic and pure: the hour of day is injected rather than read from a
//! clock, so the same input always yields the same state and stored replays stay
//! valid across restarts.

use std::ops::{
    Deref, DerefMut, Index, IndexMut, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo,
    RangeToInclusive,
};

use crate::brain::intent::Intent;

/// What the bot may do in response to a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BotAction {
    /// Don't reply — the most important action to learn.
    Stay = 0,
    /// Reply as the routed persona.
    Reply = 1,
    /// Emoji-react only (low-cost acknowledgment).
    React = 2,
}

impl BotAction {
    /// Every action, in index order.
    pub const ALL: [BotAction; 3] = [BotAction::Stay, BotAction::Reply, BotAction::React];

    /// The action with this network-output index, if any.
    pub fn from_index(index: usize) -> Option<BotAction> {
        BotAction::ALL.get(index).copied()
    }

    /// Network-output index of this action.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Length of the state vector.
pub const STATE_DIMENSIONS: usize = 18;

/// Characters of message text beyond which the length feature saturates.
const LENGTH_CAP: usize = 400;

/// Messages in the last five minutes beyond which channel heat saturates.
const HEAT_CAP: u32 = 30;

/// Named indices for the non-intent dimensions — replaces magic numbers.
///
/// Intent occupies `[0..9)` via [`Intent::index`]; the remaining nine slots are
/// typed here so `s[9]` becomes `s[StateField::Reputation]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateField {
    Reputation = 9,
    Length = 10,
    MentionsBot = 11,
    IsQuestion = 12,
    HasImage = 13,
    Sin = 14,
    Cos = 15,
    Heat = 16,
    Sentiment = 17,
}

/// Newtype around the 18-float state vector — typed indexing and named accessors.
///
/// Derefs to `[f32]` so `&StateVector` coerces to `&[f32]` for `q_values` and
/// `select_action`, and `to_vec()` / `len()` / slicing work without a wrapper
/// method. `Index<StateField>` removes the `s[9]` magic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateVector(pub [f32; STATE_DIMENSIONS]);

impl StateVector {
    /// View as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// View as a mutable slice.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.0
    }

    /// Consume into the inner array.
    #[must_use]
    pub fn into_array(self) -> [f32; STATE_DIMENSIONS] {
        self.0
    }

    #[must_use]
    pub fn reputation(&self) -> f32 {
        self[StateField::Reputation]
    }

    #[must_use]
    pub fn length(&self) -> f32 {
        self[StateField::Length]
    }

    #[must_use]
    pub fn mentions_bot(&self) -> f32 {
        self[StateField::MentionsBot]
    }

    #[must_use]
    pub fn is_question(&self) -> f32 {
        self[StateField::IsQuestion]
    }

    #[must_use]
    pub fn has_image(&self) -> f32 {
        self[StateField::HasImage]
    }

    #[must_use]
    pub fn sin(&self) -> f32 {
        self[StateField::Sin]
    }

    #[must_use]
    pub fn cos(&self) -> f32 {
        self[StateField::Cos]
    }

    #[must_use]
    pub fn heat(&self) -> f32 {
        self[StateField::Heat]
    }

    #[must_use]
    pub fn sentiment(&self) -> f32 {
        self[StateField::Sentiment]
    }
}

impl Default for StateVector {
    fn default() -> Self {
        Self([0.0; STATE_DIMENSIONS])
    }
}

impl Deref for StateVector {
    type Target = [f32];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StateVector {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<[f32]> for StateVector {
    fn as_ref(&self) -> &[f32] {
        &self.0
    }
}

impl AsMut<[f32]> for StateVector {
    fn as_mut(&mut self) -> &mut [f32] {
        &mut self.0
    }
}

impl From<[f32; STATE_DIMENSIONS]> for StateVector {
    fn from(arr: [f32; STATE_DIMENSIONS]) -> Self {
        Self(arr)
    }
}

impl From<StateVector> for [f32; STATE_DIMENSIONS] {
    fn from(v: StateVector) -> Self {
        v.0
    }
}

impl Index<StateField> for StateVector {
    type Output = f32;
    fn index(&self, field: StateField) -> &Self::Output {
        &self.0[field as usize]
    }
}

impl IndexMut<StateField> for StateVector {
    fn index_mut(&mut self, field: StateField) -> &mut Self::Output {
        &mut self.0[field as usize]
    }
}

impl Index<usize> for StateVector {
    type Output = f32;
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for StateVector {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

impl Index<Range<usize>> for StateVector {
    type Output = [f32];
    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl IndexMut<Range<usize>> for StateVector {
    fn index_mut(&mut self, range: Range<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

impl Index<RangeFrom<usize>> for StateVector {
    type Output = [f32];
    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl IndexMut<RangeFrom<usize>> for StateVector {
    fn index_mut(&mut self, range: RangeFrom<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

impl Index<RangeTo<usize>> for StateVector {
    type Output = [f32];
    fn index(&self, range: RangeTo<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl IndexMut<RangeTo<usize>> for StateVector {
    fn index_mut(&mut self, range: RangeTo<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

impl Index<RangeFull> for StateVector {
    type Output = [f32];
    fn index(&self, _: RangeFull) -> &Self::Output {
        &self.0[..]
    }
}

impl IndexMut<RangeFull> for StateVector {
    fn index_mut(&mut self, _: RangeFull) -> &mut Self::Output {
        &mut self.0[..]
    }
}

impl Index<RangeInclusive<usize>> for StateVector {
    type Output = [f32];
    fn index(&self, range: RangeInclusive<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl IndexMut<RangeInclusive<usize>> for StateVector {
    fn index_mut(&mut self, range: RangeInclusive<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

impl Index<RangeToInclusive<usize>> for StateVector {
    type Output = [f32];
    fn index(&self, range: RangeToInclusive<usize>) -> &Self::Output {
        &self.0[range]
    }
}

impl IndexMut<RangeToInclusive<usize>> for StateVector {
    fn index_mut(&mut self, range: RangeToInclusive<usize>) -> &mut Self::Output {
        &mut self.0[range]
    }
}

/// Everything the encoder needs, already extracted from the platform event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateInput<'a> {
    pub text: &'a str,
    pub intent: Intent,
    /// Author reputation, 0…1.
    pub reputation: f64,
    /// Messages in the channel over the last five minutes.
    pub channel_heat: u32,
    pub mentions_bot: bool,
    pub has_image: bool,
    /// Local hour of day, 0…23.
    pub hour_of_day: u32,
}

/// Encode a message into the state vector.
///
/// Layout:
/// - `[0..8]` intent one-hot, in [`Intent::ALL`] order
/// - `[9]` author reputation, 0…1
/// - `[10]` message length capped at 400 chars, 0…1
/// - `[11]` mentions the bot, 0|1
/// - `[12]` is a question (ends with `?`), 0|1
/// - `[13]` has image attachment(s), 0|1
/// - `[14]` hour-of-day sin, −1…1
/// - `[15]` hour-of-day cos, −1…1
/// - `[16]` channel heat capped at 30, 0…1
/// - `[17]` deterministic sentiment, −1…1
pub fn encode(input: &StateInput) -> StateVector {
    let mut s = StateVector([0.0f32; STATE_DIMENSIONS]);
    s.0[input.intent.index()] = 1.0;
    s[StateField::Reputation] = input.reputation as f32;
    s[StateField::Length] = input.text.chars().count().min(LENGTH_CAP) as f32 / LENGTH_CAP as f32;
    s[StateField::MentionsBot] = if input.mentions_bot { 1.0 } else { 0.0 };
    s[StateField::IsQuestion] = if input.text.ends_with('?') { 1.0 } else { 0.0 };
    s[StateField::HasImage] = if input.has_image { 1.0 } else { 0.0 };
    let angle = 2.0 * std::f64::consts::PI * f64::from(input.hour_of_day) / 24.0;
    s[StateField::Sin] = angle.sin() as f32;
    s[StateField::Cos] = angle.cos() as f32;
    s[StateField::Heat] = input.channel_heat.min(HEAT_CAP) as f32 / HEAT_CAP as f32;
    s[StateField::Sentiment] = sentiment_score(input.text);
    s
}

const POSITIVE_WORDS: [&str; 20] = [
    "love", "great", "awesome", "nice", "good", "thanks", "thank", "cool", "amazing", "best",
    "happy", "lol", "lmao", "haha", "w", "based", "fire", "goat", "pog", "clean",
];

const NEGATIVE_WORDS: [&str; 18] = [
    "hate", "bad", "awful", "terrible", "worst", "sucks", "trash", "angry", "sad", "annoying",
    "stupid", "dumb", "l", "mid", "cringe", "broken", "ugh", "wtf",
];

/// Unicode scalars counted +1. `'❤'` is U+2764 alone, so the emoji-presentation
/// form `❤️` (U+2764 U+FE0F) still matches on its first scalar.
const POSITIVE_EMOJI: [char; 5] = ['❤', '😂', '🔥', '👍', '😍'];
const NEGATIVE_EMOJI: [char; 4] = ['💀', '👎', '😡', '🤮'];

/// Deterministic lexicon sentiment, −1…1 — the 18th state dimension.
///
/// `(pos − neg) / max(tokens, 4)`, clamped, with light emoji weighting. Tokens
/// are the lowercased text split on non-alphanumeric characters. Intentionally
/// not an ML model: reproducibility beats accuracy for a reward-shaping feature.
pub fn sentiment_score(text: &str) -> f32 {
    let mut token_count = 0usize;
    let mut score: i32 = 0;
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        token_count += 1;
        if POSITIVE_WORDS.iter().any(|w| token.eq_ignore_ascii_case(w)) {
            score += 1;
        }
        if NEGATIVE_WORDS.iter().any(|w| token.eq_ignore_ascii_case(w)) {
            score -= 1;
        }
    }
    if token_count == 0 {
        return 0.0;
    }
    for c in text.chars() {
        if POSITIVE_EMOJI.contains(&c) {
            score += 1;
        } else if NEGATIVE_EMOJI.contains(&c) {
            score -= 1;
        }
    }
    (score as f32 / token_count.max(4) as f32).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> StateInput<'_> {
        StateInput {
            text,
            intent: Intent::SmallTalk,
            reputation: 0.5,
            channel_heat: 0,
            mentions_bot: false,
            has_image: false,
            hour_of_day: 0,
        }
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn actions_round_trip_through_index() {
        assert_eq!(BotAction::ALL.len(), 3);
        for (i, a) in BotAction::ALL.iter().enumerate() {
            assert_eq!(a.index(), i);
            assert_eq!(BotAction::from_index(i), Some(*a));
        }
        assert_eq!(BotAction::Stay.index(), 0);
        assert_eq!(BotAction::Reply.index(), 1);
        assert_eq!(BotAction::React.index(), 2);
        assert_eq!(BotAction::from_index(3), None);
    }

    #[test]
    fn encode_is_eighteen_wide_and_deterministic() {
        let a = encode(&input("hello there"));
        let b = encode(&input("hello there"));
        assert_eq!(a.len(), STATE_DIMENSIONS);
        assert_eq!(a, b);
    }

    #[test]
    fn one_hot_slot_matches_intent_all_position() {
        for (i, intent) in Intent::ALL.iter().enumerate() {
            let mut inp = input("");
            inp.intent = *intent;
            let s = encode(&inp);
            for (slot, v) in s[..9].iter().enumerate() {
                let expected = if slot == i { 1.0 } else { 0.0 };
                assert_eq!(*v, expected, "{intent:?} slot {slot}");
            }
        }
    }

    #[test]
    fn reputation_and_flags_land_in_their_slots() {
        let mut inp = input("ping");
        inp.reputation = 0.75;
        inp.mentions_bot = true;
        inp.has_image = true;
        let s = encode(&inp);
        assert!(approx(s[9], 0.75));
        assert_eq!(s[11], 1.0);
        assert_eq!(s[12], 0.0);
        assert_eq!(s[13], 1.0);
        let s = encode(&input("ping"));
        assert_eq!(s[11], 0.0);
        assert_eq!(s[13], 0.0);
    }

    #[test]
    fn length_caps_at_four_hundred_chars() {
        let s = encode(&input(&"a".repeat(200)));
        assert!(approx(s[10], 0.5));
        let s = encode(&input(&"a".repeat(400)));
        assert!(approx(s[10], 1.0));
        let s = encode(&input(&"a".repeat(1000)));
        assert!(approx(s[10], 1.0));
        // Chars, not bytes: 400 multi-byte characters saturate exactly.
        let s = encode(&input(&"é".repeat(400)));
        assert!(approx(s[10], 1.0));
        assert_eq!(encode(&input(""))[10], 0.0);
    }

    #[test]
    fn question_flag_is_trailing_question_mark() {
        assert_eq!(encode(&input("why?"))[12], 1.0);
        assert_eq!(encode(&input("why? because"))[12], 0.0);
        assert_eq!(encode(&input("what"))[12], 0.0);
    }

    #[test]
    fn hour_sin_cos_at_zero_six_twelve() {
        let mut inp = input("");
        inp.hour_of_day = 0;
        let s = encode(&inp);
        assert!(approx(s[14], 0.0));
        assert!(approx(s[15], 1.0));
        inp.hour_of_day = 6;
        let s = encode(&inp);
        assert!(approx(s[14], 1.0));
        assert!(approx(s[15], 0.0));
        inp.hour_of_day = 12;
        let s = encode(&inp);
        assert!(approx(s[14], 0.0));
        assert!(approx(s[15], -1.0));
    }

    #[test]
    fn heat_caps_at_thirty() {
        let mut inp = input("");
        inp.channel_heat = 15;
        assert!(approx(encode(&inp)[16], 0.5));
        inp.channel_heat = 30;
        assert!(approx(encode(&inp)[16], 1.0));
        inp.channel_heat = 300;
        assert!(approx(encode(&inp)[16], 1.0));
    }

    #[test]
    fn sentiment_sits_in_slot_seventeen() {
        let s = encode(&input("this is great"));
        assert!(approx(s[17], sentiment_score("this is great")));
        assert!(s[17] > 0.0);
    }

    #[test]
    fn sentiment_positive_negative_neutral_empty() {
        assert!(sentiment_score("I love this, great work") > 0.0);
        assert!(sentiment_score("this is awful and bad") < 0.0);
        assert_eq!(sentiment_score("the table is brown"), 0.0);
        assert_eq!(sentiment_score(""), 0.0);
        assert_eq!(sentiment_score("!!! ... "), 0.0);
    }

    #[test]
    fn sentiment_divides_by_at_least_four_tokens() {
        // One positive token, one token total → 1/4, not 1/1.
        assert!(approx(sentiment_score("great"), 0.25));
        // Ten tokens, one positive → 1/10.
        assert!(approx(sentiment_score("great a b c d e f g h i"), 0.1));
    }

    #[test]
    fn sentiment_emoji_weighting() {
        // Emoji are not tokens, so the divisor floors at 4: one 🔥 on a neutral
        // word scores 1/4.
        assert!(approx(sentiment_score("ok 🔥"), 0.25));
        // ❤️ with the variation selector still counts via its first scalar.
        assert!(approx(sentiment_score("ok ❤️"), 0.25));
        assert!(approx(sentiment_score("ok 💀"), -0.25));
        // Emoji-only text has no tokens, so scores 0 regardless.
        assert_eq!(sentiment_score("🔥🔥🔥"), 0.0);
    }

    #[test]
    fn sentiment_is_case_insensitive_and_token_bounded() {
        assert!(approx(sentiment_score("GREAT"), 0.25));
        // "greatest" is not "great".
        assert_eq!(sentiment_score("greatest"), 0.0);
    }

    #[test]
    fn sentiment_clamps_to_unit_range() {
        assert_eq!(sentiment_score("love 🔥🔥🔥🔥🔥🔥🔥🔥🔥🔥"), 1.0);
        assert_eq!(sentiment_score("hate 💀💀💀💀💀💀💀💀💀💀"), -1.0);
    }

    #[test]
    fn state_vector_named_accessors_match_indexed_slots() {
        let mut inp = input("ping");
        inp.reputation = 0.9;
        inp.mentions_bot = true;
        inp.has_image = true;
        inp.hour_of_day = 6;
        inp.channel_heat = 15;
        let s = encode(&inp);
        assert!(approx(s.reputation(), s[StateField::Reputation]));
        assert!(approx(s.length(), s[StateField::Length]));
        assert_eq!(s.mentions_bot(), s[StateField::MentionsBot]);
        assert_eq!(s.is_question(), s[StateField::IsQuestion]);
        assert_eq!(s.has_image(), s[StateField::HasImage]);
        assert!(approx(s.sin(), s[StateField::Sin]));
        assert!(approx(s.cos(), s[StateField::Cos]));
        assert!(approx(s.heat(), s[StateField::Heat]));
        assert!(approx(s.sentiment(), s[StateField::Sentiment]));
        // Typed index and legacy usize index point at the same slot.
        assert!(approx(
            s[StateField::Reputation],
            s[StateField::Reputation as usize]
        ));
    }

    #[test]
    fn state_field_indices_match_spec_layout() {
        assert_eq!(StateField::Reputation as usize, 9);
        assert_eq!(StateField::Length as usize, 10);
        assert_eq!(StateField::MentionsBot as usize, 11);
        assert_eq!(StateField::IsQuestion as usize, 12);
        assert_eq!(StateField::HasImage as usize, 13);
        assert_eq!(StateField::Sin as usize, 14);
        assert_eq!(StateField::Cos as usize, 15);
        assert_eq!(StateField::Heat as usize, 16);
        assert_eq!(StateField::Sentiment as usize, 17);
    }

    #[test]
    fn state_vector_derefs_to_slice_and_converts() {
        let s = encode(&input("hello"));
        let slice: &[f32] = &s;
        assert_eq!(slice.len(), STATE_DIMENSIONS);
        let vec = s.to_vec();
        assert_eq!(vec.len(), STATE_DIMENSIONS);
        let arr: [f32; STATE_DIMENSIONS] = s.into_array();
        assert_eq!(arr.len(), STATE_DIMENSIONS);
        let from_arr = StateVector::from(arr);
        assert_eq!(from_arr, s);
    }
}
