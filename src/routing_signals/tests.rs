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
