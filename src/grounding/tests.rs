use super::*;

fn kinds(v: &Verdict) -> Vec<SpecificKind> {
    v.ungrounded.iter().map(|s| s.kind).collect()
}

fn texts(v: &Verdict) -> Vec<&str> {
    v.ungrounded.iter().map(|s| s.text.as_str()).collect()
}

// -- true positives ----------------------------------------------------

#[test]
fn an_invented_version_is_ungrounded() {
    let g = Grounding::from_sources(["how do I open a voice session?"]);
    let v = check("Use the connect helper added in 4.2.1.", &g);
    assert_eq!(texts(&v), vec!["4.2.1"]);
    assert_eq!(kinds(&v), vec![SpecificKind::Version]);
    assert!(!v.is_grounded());
}

#[test]
fn an_invented_year_date_percentage_and_quote_are_all_ungrounded() {
    let g = Grounding::from_sources(["tell me about the rollout"]);
    let v = check(
        "It shipped in 2019, was patched on 2020-06-01, cut latency 40%, and the changelog \
         calls it \"the fastest path yet\".",
        &g,
    );
    assert_eq!(
        texts(&v),
        vec!["2019", "2020-06-01", "40%", "the fastest path yet"]
    );
    assert_eq!(
        kinds(&v),
        vec![
            SpecificKind::Year,
            SpecificKind::Date,
            SpecificKind::Percentage,
            SpecificKind::Quotation,
        ]
    );
}

#[test]
fn statistics_with_separators_suffixes_and_scale_words_are_caught() {
    let g = Grounding::from_sources(["how big is it?"]);
    let v = check("About 1,200 rows, 12k events, and 3 million tokens.", &g);
    assert_eq!(texts(&v), vec!["1,200", "12k", "3 million"]);
    assert!(kinds(&v).iter().all(|k| *k == SpecificKind::Statistic));
}

#[test]
fn a_version_invented_inside_a_code_block_is_still_caught() {
    // The failure mode this exists for: a plausible-looking dependency
    // line with a version nobody supplied.
    let g = Grounding::from_sources(["what do I put in Cargo.toml?"]);
    let v = check("```toml\nserenity = \"0.12.3\"\n```", &g);
    assert!(
        v.ungrounded
            .iter()
            .any(|s| s.kind == SpecificKind::Version && s.text == "0.12.3"),
        "{:?}",
        v.ungrounded
    );
}

#[test]
fn percent_written_as_a_word_is_caught_and_normalized() {
    let g = Grounding::from_sources(["did it help?"]);
    let v = check("Throughput rose 40 percent.", &g);
    assert_eq!(texts(&v), vec!["40 percent"]);
    assert_eq!(v.ungrounded[0].key, "40%");
}

// -- false-positive direction -----------------------------------------
//
// These are the tests that matter most: a guard that cries wolf is worse
// than no guard, so every ordinary shape a correct reply contains must
// pass through silently.

#[test]
fn a_version_the_user_supplied_is_not_flagged() {
    let g = Grounding::from_sources(["I'm on serenity v4.2.1, is that current?"]);
    let v = check("v4.2.1 is current; nothing to do.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(!v.examined.is_empty(), "the scanner still saw the version");
}

#[test]
fn a_bare_v_version_is_grounded_by_a_spelled_out_source() {
    let g = Grounding::from_sources(["we're still on version 4 of the schema"]);
    let v = check("v4 is the one you want.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn a_year_the_user_asked_about_is_not_flagged() {
    let g = Grounding::from_sources(["what changed in 2019?"]);
    let v = check("2019 brought the gateway rewrite.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn a_year_is_grounded_by_a_full_date_in_the_source() {
    let g = Grounding::from_sources(["the incident report is dated 2019-03-04"]);
    let v = check("That was 2019.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn a_fact_from_memory_grounds_the_reply() {
    // The grounding is not only the transcript: remembered facts and the
    // channel summary are sources too.
    let g = Grounding::from_sources(["what should I upgrade?", "runs Postgres 16"]);
    let v = check("Postgres 16 is fine; leave it.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn small_counts_and_list_language_never_trip_the_guard() {
    let g = Grounding::from_sources(["what are my options?"]);
    let v = check(
        "There are 3 options, and one message is enough to start. 5 items fit on a page; \
         the other 42 do not.",
        &g,
    );
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(v.examined.is_empty(), "{:?}", v.examined);
}

#[test]
fn a_markdown_ordered_list_is_not_a_set_of_claims() {
    let g = Grounding::from_sources(["walk me through it"]);
    let v = check("1. Join the channel.\n2. Consent.\n3. Speak.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(v.examined.is_empty(), "{:?}", v.examined);
}

#[test]
fn times_ratios_ranges_and_scores_are_not_versions_or_dates() {
    let g = Grounding::from_sources(["when and how much?"]);
    let v = check(
        "At 3:30, a 3:1 ratio, 10-20 per batch, and it ended 2-1.",
        &g,
    );
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(v.examined.is_empty(), "{:?}", v.examined);
}

#[test]
fn the_numbers_that_live_in_ordinary_technical_prose_are_not_claims() {
    // The realistic false-positive set for a Discord answer about this
    // very codebase: type names, status codes, ports, RFC numbers, hex
    // colours, durations, and architecture triples. None of these is a
    // specific anyone invented, and flagging them would make the guard
    // unusable in exactly the conversations it is meant for.
    let g = Grounding::from_sources(["why is the gateway erroring?"]);
    let v = check(
        "It is an i64 cast on x86_64: HTTP 404 from port 8080, per RFC 6455, after a 20ms \
         backoff. The embed colour is #3a7bd5.",
        &g,
    );
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(v.examined.is_empty(), "{:?}", v.examined);
}

#[test]
fn a_snowflake_id_is_never_a_year_or_a_statistic() {
    let g = Grounding::from_sources(["who is that?"]);
    let v = check("That is user 123456789012345678.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(v.examined.is_empty(), "{:?}", v.examined);
}

#[test]
fn a_year_is_not_grounded_by_a_longer_number_that_merely_contains_it() {
    // The boundary rule earning its keep: 20190 and a snowflake both
    // *contain* "2019" and must not ground it.
    let g = Grounding::from_sources(["batch 20190 ran on channel 201912345678901234"]);
    let v = check("It happened in 2019.", &g);
    assert!(!v.is_grounded(), "{:?}", v.examined);
    assert_eq!(texts(&v), vec!["2019"]);
}

#[test]
fn quoting_the_users_own_words_back_is_not_a_fabrication() {
    let g = Grounding::from_sources(["it says the handshake timed out and I don't know why"]);
    let v = check(
        "You said \"the handshake timed out\" — that's a UDP discovery failure.",
        &g,
    );
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn short_scare_quotes_are_not_treated_as_excerpts() {
    let g = Grounding::from_sources(["is it done?"]);
    let v = check("It is \"done\", in the loosest sense.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
    assert!(v.examined.is_empty(), "{:?}", v.examined);
}

#[test]
fn a_sentence_final_version_is_still_grounded() {
    let g = Grounding::from_sources(["I pinned it to 4.2.1."]);
    let v = check("Keep 4.2.1.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn a_source_version_grounds_a_shorter_prefix_in_the_reply() {
    let g = Grounding::from_sources(["we ship 4.2.1 today"]);
    let v = check("The 4.2 line is what you want.", &g);
    assert!(v.is_grounded(), "{:?}", v.ungrounded);
}

#[test]
fn curly_quotes_and_em_dashes_do_not_panic_the_scanner() {
    // The scanner slices a char vector precisely so multibyte text is safe;
    // ask.rs's persona copy is full of U+2014 and U+2019.
    let g = Grounding::from_sources(["café — what did it say?"]);
    let v = check(
        "The café note said \u{201C}closed for the season\u{201D} — nothing else.",
        &g,
    );
    assert_eq!(texts(&v), vec!["closed for the season"]);
}

#[test]
fn an_empty_reply_and_empty_grounding_are_handled() {
    let v = check("", &Grounding::new());
    assert!(v.is_grounded());
    assert!(v.examined.is_empty());
    assert!(Grounding::new().is_empty());
}

// -- verdict shape -----------------------------------------------------

#[test]
fn the_verdict_reports_what_was_examined_not_just_a_boolean() {
    let g = Grounding::from_sources(["I'm on 4.2.1"]);
    let v = check("4.2.1 is fine, but 2019 was the rewrite.", &g);
    assert_eq!(v.examined.len(), 2, "{:?}", v.examined);
    assert_eq!(v.ungrounded.len(), 1);
    assert_eq!(v.ungrounded[0].text, "2019");
    assert_eq!(v.ungrounded[0].kind.label(), "year");
}

#[test]
fn repeated_specifics_are_reported_once() {
    let g = Grounding::from_sources(["hello"]);
    let v = check("2019 and 2019 and again 2019.", &g);
    assert_eq!(v.ungrounded.len(), 1);
}

// -- policy ------------------------------------------------------------

#[test]
fn a_grounded_verdict_passes_the_reply_through_unchanged() {
    let g = Grounding::from_sources(["I'm on 4.2.1"]);
    let v = check("4.2.1 is fine.", &g);
    assert_eq!(action(&v), Action::PassThrough);
    assert_eq!(hedged("4.2.1 is fine.", &v), "4.2.1 is fine.");
}

#[test]
fn the_hedge_copy_is_verbatim() {
    // Pinned like ask.rs's degraded_reply: this copy *is* the honesty
    // contract, so changing it must be a deliberate edit here.
    let g = Grounding::from_sources(["hello"]);
    let v = check("It shipped in 2019.", &g);
    let Action::Hedge(note) = action(&v) else {
        panic!("expected a hedge");
    };
    assert_eq!(
        note,
        "Heads up — treat these as unsupported: `2019`. Nothing in this conversation or the \
         facts I was given contains them, and I have no source for them here."
    );
}

#[test]
fn an_empty_grounding_never_hedges() {
    // With no sources, every specific is trivially "ungrounded" and the
    // hedge would fire on every reply containing a number. The measurement
    // stays honest; the policy refuses to act on it.
    let v = check("It shipped in 2019 at 40%.", &Grounding::new());
    assert!(v.grounding_empty);
    assert!(!v.is_grounded(), "the measurement is still reported");
    assert_eq!(v.ungrounded.len(), 2);
    assert!(!v.should_hedge());
    assert_eq!(action(&v), Action::PassThrough);
    assert_eq!(
        hedged("It shipped in 2019 at 40%.", &v),
        "It shipped in 2019 at 40%."
    );
}

#[test]
fn a_quoted_specific_cannot_inject_markup_into_the_honesty_sentence() {
    // The note quotes model output back into a message Discord renders, so
    // a quotation carrying newlines or markdown must not break the note's
    // shape or turn live inside it.
    let g = Grounding::from_sources(["what does the doc say?"]);
    let reply = "The doc says \"the **fast** path\nis || spoilered ||\".";
    let v = check(reply, &g);
    let Action::Hedge(note) = action(&v) else {
        panic!("expected a hedge");
    };
    assert!(!note.contains('\n'), "{note:?}");
    assert_eq!(note.lines().count(), 1, "{note:?}");
    assert!(
        note.contains("`the **fast** path is || spoilered ||`"),
        "{note:?}"
    );
}

#[test]
fn backticks_in_a_quoted_specific_cannot_escape_the_inline_code_span() {
    let g = Grounding::from_sources(["what does it say?"]);
    let reply = "It says \"run `rm -rf /` first\".";
    let v = check(reply, &g);
    let Action::Hedge(note) = action(&v) else {
        panic!("expected a hedge");
    };
    // Exactly the two backticks this module added, and no more.
    assert_eq!(note.matches('`').count(), 2, "{note:?}");
    assert!(note.contains("`run rm -rf / first`"), "{note:?}");
}

#[test]
fn the_hedge_never_claims_a_tool_ran_or_memory_changed() {
    // The negative half of the contract, in the style of ask.rs's
    // provider_text_cannot_impersonate_the_safe_busy_reason: this module
    // performs one string comparison and has no other capability, so the
    // copy must not imply lookup, retrieval, or persistence.
    let g = Grounding::from_sources(["hello"]);
    let v = check("It shipped in 2019 at 40%.", &g);
    let Action::Hedge(note) = action(&v) else {
        panic!("expected a hedge");
    };
    let lower = note.to_lowercase();
    for forbidden in [
        "checked",
        "looked up",
        "look up",
        "searched",
        "search",
        "verified",
        "verify",
        "confirmed",
        "remembered",
        "remember",
        "saved",
        "stored",
        "fetched",
        "queried",
        "i ran",
        "tool",
        "memory",
        "database",
    ] {
        assert!(
            !lower.contains(forbidden),
            "{note:?} contains {forbidden:?}"
        );
    }
}

#[test]
fn the_hedge_names_at_most_three_specifics_then_counts_the_rest() {
    let g = Grounding::from_sources(["hello"]);
    let v = check("1985, 1986, 1987, 1988 and 1989.", &g);
    assert_eq!(v.ungrounded.len(), 5);
    let Action::Hedge(note) = action(&v) else {
        panic!("expected a hedge");
    };
    assert!(note.contains("`1985`, `1986`, `1987` and 2 more"), "{note}");
}

#[test]
fn the_hedge_survives_a_maximum_length_reply() {
    // The whole point: the honest sentence must not be the part that gets
    // truncated. The body is cut instead, and the result still fits the
    // budget tidy_reply enforces.
    let g = Grounding::from_sources(["hello"]);
    let long = format!("{} It shipped in 2019.", "word ".repeat(600));
    let v = check(&long, &g);
    assert!(!v.is_grounded());
    let out = hedged(&long, &v);
    assert!(
        out.chars().count() <= crate::ask::TIDY_LIMIT_CHARS,
        "{}",
        out.chars().count()
    );
    assert!(out.ends_with("no source for them here."), "{out}");
    assert!(out.contains("2019"), "{out}");
}

#[test]
fn a_hedged_short_reply_keeps_its_body_intact() {
    let g = Grounding::from_sources(["hello"]);
    let reply = "It shipped in 2019.";
    let v = check(reply, &g);
    let out = hedged(reply, &v);
    assert!(
        out.starts_with("It shipped in 2019.\n\nHeads up — "),
        "{out}"
    );
}

#[test]
fn a_very_long_quotation_is_clipped_inside_the_note() {
    let g = Grounding::from_sources(["hello"]);
    let quote = "a".repeat(300);
    let reply = format!("The doc says \"{quote}\".");
    let v = check(&reply, &g);
    let Action::Hedge(note) = action(&v) else {
        panic!("expected a hedge");
    };
    assert!(note.chars().count() < 300, "{}", note.chars().count());
    assert!(note.contains('…'), "{note}");
}

// -- composition with ask ---------------------------------------------

#[test]
fn hedging_composes_with_tidy_reply_rather_than_duplicating_it() {
    // tidy_reply owns shape; this module owns grounding. Running both in
    // order must leave the persona echo stripped *and* the hedge attached.
    let g = Grounding::from_sources(["what shipped?"]);
    let raw = "**Abbey**: ## Answer\n\nIt shipped in 2019.";
    let tidy = crate::ask::tidy_reply(crate::persona::Persona::Abbey, raw);
    assert_eq!(tidy, "Answer\n\nIt shipped in 2019.");
    let out = hedged(&tidy, &check(&tidy, &g));
    assert!(out.starts_with("Answer\n\nIt shipped in 2019."), "{out}");
    assert!(out.contains("treat these as unsupported: `2019`"), "{out}");
}

// -- grounding construction -------------------------------------------

#[test]
fn sources_accumulate_and_any_one_of_them_can_ground_a_specific() {
    let mut g = Grounding::new();
    g.push_source("what should I do?");
    assert!(!check("Pin 4.2.1.", &g).is_grounded());
    g.push_source("the lockfile pins 4.2.1");
    assert!(check("Pin 4.2.1.", &g).is_grounded());
}
