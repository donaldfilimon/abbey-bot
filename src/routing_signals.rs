//! Conversational signals the canonical keyword bag cannot see, composed on
//! top of [`crate::persona::route`] rather than inside it.
//!
//! `persona.rs` is a verbatim transcription of
//! `../abi/crates/abi-ai/src/{identity,keywords,router}.rs`: its 29 keywords,
//! their weights, the 0.40 / 0.30 / 0.30 prior, the prefix matching, and the
//! Abbey-first tie order are canonical and are not edited here. That table is
//! matched per token with `starts_with_ignore_case` over
//! `split_ascii_whitespace`, so **no multi-word phrase can ever match it, by
//! construction** — "losing my mind", "cannot figure it out", "no idea what",
//! "right now" are all invisible to it, and so are shape facts like a trailing
//! `!!!`, a shouted line, or a three-word imperative. That is the gap this
//! module fills, and it fills it *beside* the canonical router:
//!
//! 1. [`route`] always asks [`crate::persona::route`] first and keeps its
//!    [`crate::persona::Route`] verbatim in [`ComposedRoute::base`].
//! 2. An explicit command choice or an exact leading name (`Aviva, ...`) comes
//!    back as [`crate::persona::Reason::Explicit`] and is **absolute** — the
//!    signal layer reports what it saw and changes nothing.
//! 3. Canonical keyword evidence ([`crate::persona::Reason::Weighted`]) also
//!    wins outright. Signals are still computed so `/persona route` can say
//!    "distress seen, canonical keywords won", but they never flip it.
//! 4. Only when the canonical router lands on the neutral prior
//!    ([`crate::persona::Reason::Default`]) — the exact case where its answer
//!    carries no evidence — may a signal decide.
//!
//! That Default-only rule is what makes "canonical behavior is unchanged" a
//! structural fact rather than a hope: every input the canonical router has an
//! opinion about keeps that opinion, byte for byte.
//!
//! The composition happens on the *decision*, not on
//! [`crate::persona::ProfileWeights`]: `select` and `normalize` are private to
//! `persona.rs`, and re-implementing the tie order here would be a second copy
//! of canonical logic — precisely the drift the transcription guards against.
//!
//! Scoring is integer-only over fixed-order arrays, so the same string always
//! produces the same [`Signals`]. No model call, no clock, no I/O.
//!
//! Deliberate non-goals: no negation handling ("not urgent" still scores as
//! urgency), no sarcasm, no per-user calibration, and no history — a signal is
//! read off one message.

use crate::persona::{self, Persona};

/// Score at which a signal family is strong enough to decide a neutral route.
const FIRE: i32 = 2;

/// A message longer than this is not "terse", so terse-urgency cannot fire on
/// it. Long urgent prose is an explanation, and explanation is Abbey's lane.
const TERSE_WORDS: usize = 12;

/// Multi-word cues of emotional distress — the family the keyword bag misses
/// most, because every entry here is longer than one token.
const DISTRESS_PHRASES: [(&str, i32); 34] = [
    ("losing my mind", 3),
    ("lost my mind", 3),
    ("out of my mind", 3),
    ("at my wits end", 3),
    ("wits end", 3),
    ("about to cry", 3),
    ("want to cry", 3),
    ("about to lose it", 3),
    ("give up", 3),
    ("giving up", 3),
    ("cant take it anymore", 3),
    ("cant take this anymore", 3),
    ("fed up", 3),
    ("burned out", 3),
    ("burnt out", 3),
    ("driving me crazy", 3),
    ("driving me nuts", 3),
    ("driving me insane", 3),
    ("pulling my hair out", 3),
    ("tearing my hair out", 3),
    ("i hate this", 3),
    ("im done", 3),
    ("i am done", 3),
    ("so done", 3),
    ("nothing works", 3),
    ("nothing is working", 3),
    ("nothing i try", 3),
    ("been at this for hours", 3),
    ("been stuck", 3),
    ("breaking point", 3),
    // First-person scoping. "stuck" and "struggling" are machine words as
    // often as human ones — "the job is stuck", "the query is struggling
    // under load" — so they score 1 on their own and only reach FIRE when a
    // person says it about themselves.
    ("im stuck", 2),
    ("i am stuck", 2),
    ("im struggling", 2),
    ("i am struggling", 2),
];

/// Single-token distress cues. `frustrated` is deliberately absent: it is
/// already canonical at 0.95 Abbey. Its inflections are not — the canonical
/// prefix match tests whether the token starts with `frustrated`, which
/// "frustrating" and "frustration" do not.
const DISTRESS_WORDS: [(&str, i32); 26] = [
    ("stuck", 1),
    ("overwhelmed", 2),
    ("hopeless", 2),
    ("exhausted", 2),
    ("demoralized", 2),
    ("defeated", 2),
    ("desperate", 2),
    ("despair", 2),
    ("panicking", 2),
    ("panicked", 2),
    ("miserable", 2),
    ("frustrating", 2),
    ("frustration", 2),
    ("infuriating", 2),
    ("maddening", 2),
    ("agonizing", 2),
    ("dreading", 2),
    ("upset", 2),
    ("furious", 2),
    ("livid", 2),
    ("struggling", 1),
    ("suffering", 2),
    ("anxious", 2),
    ("stressed", 2),
    ("crying", 2),
    ("sobbing", 2),
];

/// "I do not know what is happening" — distinct from distress, same
/// destination. Confusion is a teaching problem, and teaching is Abbey's.
const CONFUSION_PHRASES: [(&str, i32); 22] = [
    ("no idea", 2),
    ("not sure why", 2),
    ("not sure how", 2),
    ("no clue", 2),
    ("cant figure it out", 2),
    ("cant figure out", 2),
    ("cant work out", 2),
    ("cannot figure out", 2),
    ("dont understand", 2),
    ("do not understand", 2),
    ("dont get it", 2),
    ("dont get why", 2),
    ("makes no sense", 2),
    ("doesnt make sense", 2),
    ("does not make sense", 2),
    ("what am i doing wrong", 2),
    ("im lost", 2),
    ("i am lost", 2),
    ("completely lost", 2),
    ("totally lost", 2),
    ("over my head", 2),
    ("in over my head", 2),
];

const CONFUSION_WORDS: [(&str, i32); 9] = [
    ("confused", 2),
    ("confusing", 2),
    ("baffled", 2),
    ("baffling", 2),
    ("puzzled", 2),
    ("bewildered", 2),
    ("unclear", 2),
    ("clueless", 2),
    ("mystified", 2),
];

/// Soft confusion — a why/how opener only reaches [`FIRE`] once the question
/// shape confirms it, which is what stops a bare "and what about tomorrow?"
/// from being read as confusion.
const CONFUSION_SOFT: [(&str, i32); 6] = [
    ("why is", 1),
    ("why does", 1),
    ("why did", 1),
    ("why cant", 1),
    ("how do i", 1),
    ("what does", 1),
];

/// Unambiguous urgency: an outage, a deadline, or an explicit "now".
const URGENCY_PHRASES: [(&str, i32); 10] = [
    ("right now", 2),
    ("as soon as possible", 2),
    ("on fire", 2),
    ("is down", 2),
    ("are down", 2),
    ("went down", 2),
    ("no time", 2),
    ("out of time", 2),
    ("need this now", 3),
    ("hurry up", 3),
];

const URGENCY_WORDS: [(&str, i32); 15] = [
    ("asap", 2),
    ("urgent", 2),
    ("urgently", 2),
    ("immediately", 2),
    ("emergency", 2),
    ("hurry", 2),
    ("outage", 2),
    ("downtime", 2),
    ("blocker", 2),
    ("escalate", 2),
    ("escalating", 2),
    ("pager", 2),
    ("paging", 2),
    ("sev1", 2),
    ("p0", 2),
];

/// Soft urgency — reaches [`FIRE`] only when the sentence is also shaped as a
/// command, so "what now" stays neutral while "restart it now" does not.
/// `in prod` and `critical` live here rather than above because both are
/// ordinary technical vocabulary: "this happens in production" is a bug report,
/// not a page, and only the imperative shape makes it a page.
const URGENCY_SOFT: [(&str, i32); 6] = [
    ("now", 1),
    ("fast", 1),
    ("rush", 1),
    ("critical", 1),
    ("in prod", 1),
    ("in production", 1),
];

/// Interrogative openers. Auxiliaries (`do`, `can`, `is`) are deliberately
/// excluded: "do it now" is an imperative, not a question, and a real question
/// built on one almost always carries the `?`.
const INTERROGATIVES: [&str; 9] = [
    "what", "why", "how", "when", "where", "who", "whom", "whose", "which",
];

const IMPERATIVES: [&str; 45] = [
    "fix", "run", "deploy", "restart", "reboot", "ship", "push", "pull", "revert", "rollback",
    "roll", "kill", "stop", "start", "check", "send", "make", "do", "get", "update", "merge",
    "retry", "cancel", "pause", "resume", "mute", "unmute", "join", "leave", "add", "remove",
    "delete", "set", "give", "tell", "show", "open", "close", "clear", "reset", "bump", "patch",
    "publish", "release", "page",
];

/// Openers that carry no shape of their own and are skipped before the first
/// real token is classified.
const POLITE: [&str; 11] = [
    "please", "pls", "plz", "hey", "hi", "hello", "ok", "okay", "yo", "just", "and",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Ends with `?`, or opens with an interrogative.
    Question,
    /// Opens with a bare imperative verb, optionally behind a politeness or
    /// "could you" wrapper.
    Command,
    Statement,
}

impl Shape {
    const fn label(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Command => "command",
            Self::Statement => "statement",
        }
    }
}

/// What one message looks like, independent of the canonical keyword table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signals {
    pub distress: i32,
    pub confusion: i32,
    pub urgency: i32,
    pub shape: Shape,
    pub words: usize,
    /// Repeated terminal punctuation (`!!`, `??`, `?!`) or a shouted line.
    pub emphasis: bool,
}

impl Signals {
    /// Read the signals off one message. Pure, allocation-bounded, and stable:
    /// every table is a fixed-order array and every score is an integer.
    #[must_use]
    pub fn detect(input: &str) -> Self {
        let normalized = normalize(input);
        let words = normalized.split_ascii_whitespace().count();
        let shape = shape_of(input, &normalized);
        let emphasis = has_emphasis(input);

        let mut distress =
            score(&normalized, &DISTRESS_PHRASES) + score(&normalized, &DISTRESS_WORDS);
        let mut confusion =
            score(&normalized, &CONFUSION_PHRASES) + score(&normalized, &CONFUSION_WORDS);
        let mut urgency = score(&normalized, &URGENCY_PHRASES) + score(&normalized, &URGENCY_WORDS);
        let soft_confusion = score(&normalized, &CONFUSION_SOFT);
        let soft_urgency = score(&normalized, &URGENCY_SOFT);
        confusion += soft_confusion;
        urgency += soft_urgency;

        // Shape and emphasis confirm evidence; they never create it. A message
        // with no lexical cue at all scores zero however it is punctuated,
        // which is what keeps a neutral follow-up neutral.
        if emphasis {
            if distress > 0 {
                distress += 1;
            }
            if urgency > 0 {
                urgency += 1;
            }
        }
        match shape {
            Shape::Question if confusion > 0 => confusion += 1,
            Shape::Command if urgency > 0 => urgency += 1,
            _ => {}
        }

        Self {
            distress,
            confusion,
            urgency,
            shape,
            words,
            emphasis,
        }
    }

    /// Any lexical evidence at all — used to report "seen but not acted on".
    #[must_use]
    pub fn any(&self) -> bool {
        self.distress > 0 || self.confusion > 0 || self.urgency > 0
    }

    /// Which way these signals would push a route that carries no canonical
    /// evidence. Distress outranks confusion outranks urgency: someone who is
    /// both upset and in a hurry is upset first.
    #[must_use]
    pub fn adjustment(&self) -> Adjustment {
        if self.distress >= FIRE {
            Adjustment::Distress
        } else if self.confusion >= FIRE {
            Adjustment::Confusion
        } else if self.urgency >= FIRE
            && self.words <= TERSE_WORDS
            && self.shape != Shape::Question
            && self.distress == 0
            && self.confusion == 0
        {
            Adjustment::TerseUrgency
        } else {
            Adjustment::None
        }
    }

    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "distress {} · confusion {} · urgency {} · {} · {} word{}{}",
            self.distress,
            self.confusion,
            self.urgency,
            self.shape.label(),
            self.words,
            if self.words == 1 { "" } else { "s" },
            if self.emphasis { " · emphatic" } else { "" }
        )
    }
}

/// Which signal family, if any, decided a route the canonical router left on
/// the neutral prior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjustment {
    None,
    Distress,
    Confusion,
    TerseUrgency,
}

impl Adjustment {
    const fn why(self) -> &'static str {
        match self {
            Self::None => "canonical routing",
            Self::Distress => {
                "distress cues the canonical keyword table cannot see, over the neutral prior"
            }
            Self::Confusion => {
                "confusion cues the canonical keyword table cannot see, over the neutral prior"
            }
            Self::TerseUrgency => {
                "terse urgency cues the canonical keyword table cannot see, over the neutral prior"
            }
        }
    }
}

/// A canonical route plus what the signal layer made of the same message.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedRoute {
    /// The persona to actually use.
    pub persona: Persona,
    /// The canonical route, untouched.
    pub base: persona::Route,
    pub signals: Signals,
    pub adjustment: Adjustment,
}

impl ComposedRoute {
    /// True when the signal layer, not the canonical router, chose the persona.
    #[must_use]
    pub fn adjusted(&self) -> bool {
        self.adjustment != Adjustment::None
    }

    /// True when this route stands on its own evidence — canonical keywords, an
    /// explicit selector, or a fired signal — so a caller must **not** fall
    /// back to a session or guild default. False means the message was neutral
    /// to both layers, which is exactly when stickiness should win.
    #[must_use]
    pub fn is_decisive(&self) -> bool {
        !matches!(self.base.reason, persona::Reason::Default) || self.adjusted()
    }
}

/// Canonical routing first, signals only over the neutral prior.
///
/// `explicit` is the command-level override and is passed straight through, so
/// `/persona ask ... as:Abi` remains absolute — as does a leading `Aviva,`.
#[must_use]
pub fn route(request: &str, explicit: Option<Persona>) -> ComposedRoute {
    let base = persona::route(request, explicit);
    let signals = Signals::detect(request);
    let adjustment = if matches!(base.reason, persona::Reason::Default) {
        signals.adjustment()
    } else {
        Adjustment::None
    };
    let persona = match adjustment {
        Adjustment::None => base.persona,
        Adjustment::Distress | Adjustment::Confusion => Persona::Abbey,
        Adjustment::TerseUrgency => Persona::Aviva,
    };
    ComposedRoute {
        persona,
        base,
        signals,
        adjustment,
    }
}

/// Explain a composed route. When the canonical router decided, this is
/// [`persona::describe`] verbatim plus an honest note about signals that were
/// seen and not acted on.
#[must_use]
pub fn describe(route: &ComposedRoute) -> String {
    if !route.adjusted() {
        let mut text = persona::describe(&route.base);
        if route.signals.any() {
            text.push_str("\nSignals: ");
            text.push_str(&route.signals.summary());
            text.push_str(" (canonical routing kept)");
        }
        return text;
    }
    format!(
        "**{}** — {}\nHandles: {}\nWhy: {}\nSignals: {}",
        route.persona,
        route.persona.register(),
        route.persona.handles(),
        route.adjustment.why(),
        route.signals.summary()
    )
}

/// Lowercase, drop apostrophes so "can't" and "cant" score alike, turn every
/// other non-alphanumeric run into a single space, and pad both ends so phrase
/// lookups can test word boundaries without a special case.
fn normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push(' ');
    let mut at_space = true;
    for ch in input.chars() {
        if ch == '\'' || ch == '\u{2019}' {
            continue;
        }
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            at_space = false;
        } else if !at_space {
            out.push(' ');
            at_space = true;
        }
    }
    if !at_space {
        out.push(' ');
    }
    out
}

/// Whole-word `contains` over normalized text. Byte comparison against `b' '`
/// is UTF-8 safe: a space byte never occurs inside a multi-byte sequence.
fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    haystack.match_indices(phrase).any(|(start, _)| {
        let end = start + phrase.len();
        let before = start == 0 || haystack.as_bytes()[start - 1] == b' ';
        let after = end == haystack.len() || haystack.as_bytes()[end] == b' ';
        before && after
    })
}

/// Each entry contributes at most once, so a repeated word cannot inflate a
/// score past its family's weight.
fn score(normalized: &str, table: &[(&str, i32)]) -> i32 {
    let mut total = 0;
    for (phrase, weight) in table {
        if contains_phrase(normalized, phrase) {
            total += weight;
        }
    }
    total
}

fn has_emphasis(input: &str) -> bool {
    let bytes = input.as_bytes();
    let repeated = bytes
        .windows(2)
        .any(|pair| matches!(pair[0], b'!' | b'?') && matches!(pair[1], b'!' | b'?'));
    if repeated {
        return true;
    }
    let mut upper = 0usize;
    let mut letters = 0usize;
    for ch in input.chars() {
        if ch.is_alphabetic() {
            letters += 1;
            if ch.is_uppercase() {
                upper += 1;
            }
        }
    }
    letters >= 6 && upper * 5 >= letters * 3
}

/// The first token that carries shape, with politeness and "could you"
/// wrappers stripped.
fn leading_token(normalized: &str) -> Option<&str> {
    let tokens: Vec<&str> = normalized.split_ascii_whitespace().collect();
    let mut index = 0;
    loop {
        while index < tokens.len() && POLITE.contains(&tokens[index]) {
            index += 1;
        }
        let modal = index + 1 < tokens.len()
            && matches!(tokens[index], "can" | "could" | "would" | "will")
            && matches!(tokens[index + 1], "you" | "u");
        if modal {
            index += 2;
            continue;
        }
        break;
    }
    tokens.get(index).copied()
}

fn shape_of(original: &str, normalized: &str) -> Shape {
    let leading = leading_token(normalized);
    if original.trim_end().ends_with('?') || leading.is_some_and(|t| INTERROGATIVES.contains(&t)) {
        return Shape::Question;
    }
    if leading.is_some_and(|t| IMPERATIVES.contains(&t)) {
        return Shape::Command;
    }
    Shape::Statement
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every input the existing canonical tests assert on, plus the shapes most
    /// likely to trip a naive signal layer. The composed persona must equal the
    /// canonical persona for all of them.
    const CANONICAL_CORPUS: [&str; 16] = [
        "hello world",
        "hello there",
        "Aviva, be direct",
        "@ABI: orchestrate",
        "Abbey help",
        "avivacious prose",
        "Please ask Aviva",
        "Aviva execute quickly",
        "execute deploy run the build quickly",
        "orchestrate routing governance policy",
        "execute the deploy quickly",
        "ABI: review governance risk",
        "Abbey, help me",
        "and what about tomorrow?",
        "Abbey, take this one",
        "unsafe",
    ];

    #[test]
    fn canonical_routes_are_untouched() {
        for input in CANONICAL_CORPUS {
            let canonical = persona::route(input, None);
            let composed = route(input, None);
            assert_eq!(
                composed.persona, canonical.persona,
                "signal layer changed a canonical route for {input:?}"
            );
            assert_eq!(
                composed.base, canonical,
                "the canonical route must survive verbatim for {input:?}"
            );
            assert!(
                !composed.adjusted(),
                "no signal should fire on canonical corpus entry {input:?}"
            );
        }
    }

    #[test]
    fn distress_invisible_to_keywords_routes_abbey() {
        for input in [
            "I'm completely stuck and losing my mind on this",
            "been at this for hours and nothing works",
            "I am at my wits end with this thing",
            "this is so frustrating, I want to cry",
            "I'm overwhelmed",
            "I'm stuck",
            "honestly I'm about to give up",
        ] {
            let canonical = persona::route(input, None);
            let composed = route(input, None);
            assert_eq!(
                canonical.reason,
                persona::Reason::Default,
                "precondition: the keyword bag must be blind to {input:?}"
            );
            assert_eq!(composed.persona, Persona::Abbey, "{input:?}");
            assert_eq!(composed.adjustment, Adjustment::Distress, "{input:?}");
            assert!(composed.is_decisive(), "{input:?}");
        }
    }

    #[test]
    fn confusion_routes_abbey() {
        for input in [
            "I have no idea what is going on here",
            "this makes no sense to me",
            "I'm confused",
            "why does this keep happening?",
            "I cannot figure out where the value comes from",
        ] {
            let composed = route(input, None);
            assert_eq!(composed.persona, Persona::Abbey, "{input:?}");
            assert!(
                matches!(
                    composed.adjustment,
                    Adjustment::Confusion | Adjustment::Distress
                ),
                "{input:?} produced {:?}",
                composed.adjustment
            );
        }
    }

    #[test]
    fn terse_urgency_routes_aviva() {
        for input in [
            "restart it now",
            "need this asap",
            "prod is down",
            "escalate, this is critical",
            "can you page the on-call now",
        ] {
            let canonical = persona::route(input, None);
            let composed = route(input, None);
            assert_eq!(
                canonical.reason,
                persona::Reason::Default,
                "precondition: the keyword bag must be blind to {input:?}"
            );
            assert_eq!(composed.persona, Persona::Aviva, "{input:?}");
            assert_eq!(composed.adjustment, Adjustment::TerseUrgency, "{input:?}");
        }
    }

    #[test]
    fn distress_outranks_urgency() {
        let composed = route(
            "everything is on fire and I am losing my mind right now",
            None,
        );
        assert_eq!(composed.persona, Persona::Abbey);
        assert_eq!(composed.adjustment, Adjustment::Distress);
        assert!(composed.signals.urgency > 0, "urgency was still observed");
    }

    #[test]
    fn explicit_selectors_stay_absolute_under_distress() {
        // A leading name outranks every signal, in both directions.
        let aviva = route("Aviva, I am completely stuck and losing my mind", None);
        assert_eq!(aviva.persona, Persona::Aviva);
        assert_eq!(aviva.base.reason, persona::Reason::Explicit);
        assert!(!aviva.adjusted());

        let abi = route("@ABI: prod is down, escalate now", None);
        assert_eq!(abi.persona, Persona::Abi);
        assert!(!abi.adjusted());

        // And so does the command-level choice.
        let forced = route(
            "I am completely stuck and losing my mind",
            Some(Persona::Aviva),
        );
        assert_eq!(forced.persona, Persona::Aviva);
        assert!(!forced.adjusted());

        let forced_abi = route("restart it now", Some(Persona::Abi));
        assert_eq!(forced_abi.persona, Persona::Abi);
        assert!(!forced_abi.adjusted());
    }

    #[test]
    fn canonical_keyword_evidence_outranks_signals() {
        // "explain"/"design" are canonical Abbey; "deploy"/"quick" canonical
        // Aviva. Weighted routes are never flipped, only annotated.
        let weighted = route("deploy this quickly, I am losing my mind", None);
        assert!(matches!(weighted.base.reason, persona::Reason::Weighted(_)));
        assert_eq!(weighted.persona, weighted.base.persona);
        assert!(!weighted.adjusted());
        assert!(
            weighted.signals.distress >= FIRE,
            "the distress was still measured and reported"
        );
        assert!(describe(&weighted).contains("canonical routing kept"));
    }

    #[test]
    fn shape_and_emphasis_confirm_but_never_create() {
        // Punctuation and shape alone must not move a neutral message.
        for input in [
            "and what about tomorrow?",
            "!!!",
            "WHAT ABOUT TOMORROW???",
            "ok",
        ] {
            let composed = route(input, None);
            assert!(
                !composed.adjusted(),
                "{input:?} fired {:?} on shape alone",
                composed.adjustment
            );
        }
        // A question opener alone is not confusion.
        assert_eq!(Signals::detect("what about tomorrow?").confusion, 0);
        // But it does lift a soft cue to the firing threshold.
        assert!(Signals::detect("why does this happen?").confusion >= FIRE);
    }

    /// The cost of a signal layer is false positives, so the everyday traffic
    /// it will mostly see must stay exactly where the canonical router put it.
    #[test]
    fn ordinary_chatter_stays_neutral() {
        for input in [
            "the build finished",
            "I pushed the branch, let me know",
            "what's the plan for the release?",
            "thanks, that worked",
            "I'll take a look tomorrow",
            "meeting in 10, brb",
            "we should probably rename this now",
            "restart the pod when you get a chance",
            "this only happens in production",
            "the critical section needs a lock",
            "the job is stuck",
            "the queue is stuck in pending",
            "the query is struggling under load",
            "morning",
        ] {
            let composed = route(input, None);
            assert!(
                !composed.adjusted(),
                "{input:?} fired {:?} ({})",
                composed.adjustment,
                composed.signals.summary()
            );
        }
    }

    #[test]
    fn long_urgent_prose_is_not_terse_urgency() {
        let input = "we need this now because the release window closes tonight \
                     and the customer has been waiting on the migration since Monday";
        let composed = route(input, None);
        assert!(composed.signals.urgency >= FIRE);
        assert!(composed.signals.words > TERSE_WORDS);
        assert!(!composed.adjusted(), "long prose must not read as terse");
    }

    #[test]
    fn urgent_questions_do_not_route_aviva() {
        // A question is a request for understanding even when it names prod.
        let composed = route("how does the retry path behave in production?", None);
        assert_eq!(composed.signals.shape, Shape::Question);
        assert_ne!(composed.adjustment, Adjustment::TerseUrgency);
    }

    #[test]
    fn shapes_are_classified_past_politeness_wrappers() {
        assert_eq!(
            Signals::detect("please just restart it").shape,
            Shape::Command
        );
        assert_eq!(
            Signals::detect("could you restart it").shape,
            Shape::Command
        );
        assert_eq!(Signals::detect("do it now").shape, Shape::Command);
        assert_eq!(Signals::detect("why is it broken").shape, Shape::Question);
        assert_eq!(Signals::detect("it broke again").shape, Shape::Statement);
        assert_eq!(Signals::detect("I fixed it").shape, Shape::Statement);
    }

    #[test]
    fn detection_is_total_and_deterministic() {
        for input in [
            "",
            "   ",
            "\n\t",
            "🙂🙃",
            "———",
            "café naïve",
            "line one\nline two\n\nline four",
            "?!",
            "a",
        ] {
            let first = Signals::detect(input);
            let second = Signals::detect(input);
            assert_eq!(
                first, second,
                "detection must be deterministic for {input:?}"
            );
            let composed = route(input, None);
            assert_eq!(
                composed.persona,
                persona::route(input, None).persona,
                "degenerate input {input:?} must not be re-routed"
            );
        }
    }

    #[test]
    fn normalization_folds_apostrophes_and_punctuation() {
        assert_eq!(normalize("Can't—stop!"), " cant stop ");
        assert_eq!(normalize(""), " ");
        assert!(contains_phrase(
            &normalize("I don't understand this"),
            "dont understand"
        ));
        assert!(
            !contains_phrase(&normalize("misunderstanding"), "dont understand"),
            "phrases must match on word boundaries"
        );
        assert!(
            !contains_phrase(&normalize("nowhere"), "now"),
            "a soft cue must not match inside a longer word"
        );
    }

    #[test]
    fn descriptions_name_the_deciding_layer() {
        let adjusted = describe(&route("I am completely stuck and losing my mind", None));
        assert!(adjusted.starts_with("**Abbey**"), "{adjusted}");
        assert!(adjusted.contains("distress cues"), "{adjusted}");
        assert!(adjusted.contains("Signals: distress"), "{adjusted}");

        let untouched = describe(&route("hello world", None));
        assert_eq!(
            untouched,
            persona::describe(&persona::route("hello world", None)),
            "a neutral message must be explained exactly as before"
        );
    }
}
