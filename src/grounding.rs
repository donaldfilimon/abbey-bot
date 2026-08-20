//! The honest-answer guard: a **lexical grounding check** over a candidate
//! reply, plus a policy helper that turns its verdict into an action.
//!
//! # What this is, and what it is not
//!
//! This is **not** a hallucination detector, and nothing here should ever be
//! described as one. It cannot tell whether a claim is true. All it does is
//! extract a small set of *concrete lexical shapes* from a reply — version-like
//! tokens, dates, bare years, percentages, other statistics, and quoted strings
//! — and ask a purely mechanical question: does this exact shape appear
//! anywhere in the grounding that was available for this turn?
//!
//! A shape that appears nowhere in the grounding is reported as an *ungrounded
//! specific*. That is a defensible, checkable claim. "This reply is a
//! hallucination" is not, and a semantic detector built out of string matching
//! would be exactly the kind of dressed-up guess [`crate::ask`]'s copy exists to
//! prevent.
//!
//! # Composition with [`crate::ask`]
//!
//! [`crate::ask::tidy_reply`] owns reply *shape* (persona echo, headings,
//! length) and the persona system prompt *asks* the model not to guess. This
//! module measures whether it did, after the fact. It duplicates neither: run
//! `tidy_reply` first, then [`hedged`], which budgets its note against
//! [`crate::ask::TIDY_LIMIT_CHARS`] so the honest part cannot be the thing that
//! gets truncated by the command layer's clamp.
//!
//! # Honest limits
//!
//! Everything below is a **non-detection**, by design or by construction:
//!
//! - A fabricated API, product, function, or person name that carries no
//!   version, date, statistic, or quotation shape. `Guild::fetch_flags()` is
//!   invisible here unless the model quotes it.
//! - A specific that *is* present in the grounding but is being used to say
//!   something false. Presence is not correctness.
//! - A paraphrased fabrication ("released a couple of years ago") — no shape.
//! - Bare integers under any length with no separator, unit, or percent sign
//!   (`16`, `3 options`, a Discord snowflake). Suppressed deliberately: this is
//!   the single largest false-positive class, and a guard that cries wolf is
//!   worse than no guard.
//! - Number *words* (`one message`, `three`) are never scanned at all.
//! - Non-Latin digits, and years outside 1900–2099.
//! - Over-grounding is chosen on purpose wherever the rules are ambiguous: when
//!   in doubt this module says "grounded" and stays silent, because a false
//!   accusation in a user-visible reply costs more than a miss.
//!
//! # Wiring status — read this before quoting the module in a claim
//!
//! **Nothing calls this module in the reply path yet.** It is a pure, tested
//! library registered in `main.rs`, and no user-visible behaviour changes
//! because it exists. Saying "the bot declines to invent" on the strength of
//! this file would be exactly the kind of unearned claim it was written to
//! catch.
//!
//! The intended seam is [`crate::generation::generate_with_backend`], at the
//! point where a round returns text: the prepared transcript (`turns`) and the
//! [`crate::memory::PersonaContext`] are both in scope there and have not yet
//! been polluted by the answer itself, which is the only place the *full*
//! grounding for a turn exists. Two things must be settled before that edit is
//! honest rather than convenient:
//!
//! 1. The streaming path has already posted the message by the time text is
//!    final, so attaching a hedge means an edit, not a different return value.
//! 2. `commands.rs` alone cannot supply the transcript — `Engine` exposes no
//!    read-only accessor for it, and re-`prepare`ing after `commit` would
//!    ground the reply in itself.

// Scoped to this module, and deliberate. The crate is a binary, so every item
// here is `dead_code` until the seam above is wired; the alternative was to
// edit the live generation path in the same change that introduces the guard,
// which is a behaviour change wearing a library's clothes. The tests exercise
// the whole surface.
#![allow(dead_code)]

use std::collections::BTreeSet;

/// The concrete shapes this module is willing to assert something about.
///
/// Each variant is a shape a deterministic scanner can recognize without
/// guessing at meaning. There is deliberately no `Fact`, `Entity`, or `Claim`
/// variant — those would require judgement this module does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpecificKind {
    /// `v4`, `1.2.3`, `v1.2.3-rc1`.
    Version,
    /// An ISO calendar date, `2021-03-04`.
    Date,
    /// A bare four-digit year in 1900–2099.
    Year,
    /// `40%`, `12.5 percent`.
    Percentage,
    /// A number specific enough to read as a measurement: thousands-separated
    /// (`1,200`), scale-suffixed (`12k`), scale-worded (`3 million`), or
    /// decimal (`3.5`).
    Statistic,
    /// A double-quoted run of text containing at least one letter.
    Quotation,
}

impl SpecificKind {
    /// Short human label, used in the hedge note and in test assertions.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Date => "date",
            Self::Year => "year",
            Self::Percentage => "percentage",
            Self::Statistic => "statistic",
            Self::Quotation => "quotation",
        }
    }
}

/// One specific asserted in a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Specific {
    pub kind: SpecificKind,
    /// Exactly as it was written in the reply.
    pub text: String,
    /// The normalized form used for comparison (lowercased, thousands
    /// separators removed, `percent` folded to `%`, whitespace collapsed).
    pub key: String,
}

/// Everything the turn was allowed to know: conversation turns, remembered
/// facts, a channel summary — whatever the caller supplied.
///
/// Built from plain `&str` fragments rather than from `ChatTurn` or
/// `PersonaContext` so this module stays pure and testable with string
/// literals, and so it does not couple to the memory or engine layers.
///
/// **Do not push the system prompt in as a source.** It carries its own
/// numbers (the length budget), which would silently ground unrelated
/// specifics in the reply.
#[derive(Debug, Clone, Default)]
pub struct Grounding {
    /// Normalized keys of every specific-shaped token found in the sources,
    /// generously expanded (see [`Grounding::push_source`]).
    keys: BTreeSet<String>,
    /// All sources, lowercased with whitespace runs collapsed, for the
    /// token-boundary-aware raw fallback and for quotation substring lookup.
    haystack: String,
}

impl Grounding {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from a sequence of grounding fragments.
    ///
    /// Typical caller: the prepared transcript's turn texts, plus the channel
    /// summary and each remembered user fact.
    pub fn from_sources<'a, I>(sources: I) -> Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut g = Self::new();
        for s in sources {
            g.push_source(s);
        }
        g
    }

    /// Add one grounding fragment.
    ///
    /// The grounding side is scanned **more generously** than the reply side:
    /// bare integers count as keys here (so a reply's `v4` is grounded by a
    /// source's "version 4"), and each key is expanded — a date also grounds
    /// its year, a version also grounds its prefixes, a percentage also grounds
    /// its bare number. Generosity on this side only ever produces silence,
    /// which is the safe direction for this guard.
    pub fn push_source(&mut self, text: &str) {
        let mut found = Vec::new();
        scan_into(&text.chars().collect::<Vec<char>>(), true, true, &mut found);
        for s in found {
            expand_grounding_key(&s, &mut self.keys);
        }
        if !self.haystack.is_empty() {
            self.haystack.push('\n');
        }
        self.haystack.push_str(&normalize_haystack(text));
    }

    /// True when no source has been pushed. A caller with no grounding at all
    /// should generally not hedge — every specific would be "ungrounded",
    /// which is noise rather than signal.
    pub fn is_empty(&self) -> bool {
        self.haystack.is_empty()
    }

    /// Whether this grounding contains `s`.
    fn grounds(&self, s: &Specific) -> bool {
        if self.keys.contains(&s.key) {
            return true;
        }
        match s.kind {
            // A quoted phrase need not be quoted in the source, so a plain
            // substring match over the normalized haystack is the right test.
            SpecificKind::Quotation => self.haystack.contains(&s.key),
            // Numeric shapes fall back to a token-boundary-aware raw match, so
            // `2019` is not grounded by `20190` or by a Discord snowflake.
            _ => {
                boundary_contains(&self.haystack, &s.key)
                    || boundary_contains(&self.haystack, &s.text.to_lowercase())
            }
        }
    }
}

/// The structured result of a check: what was examined, and what of it the
/// grounding did not contain.
///
/// Deliberately not a boolean — the caller decides policy, and a caller that
/// wants to log, count, or route on the specifics needs to see them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Verdict {
    /// Every specific the scanner recognized in the reply, deduplicated.
    pub examined: Vec<Specific>,
    /// The subset that appears nowhere in the grounding.
    pub ungrounded: Vec<Specific>,
    /// The grounding had no sources at all, so `ungrounded` is necessarily
    /// everything `examined` found. Recorded rather than hidden: the
    /// measurement is still true, but it carries no information, and
    /// [`Verdict::should_hedge`] refuses to act on it.
    pub grounding_empty: bool,
}

impl Verdict {
    /// True when nothing the scanner recognized was missing from the grounding.
    /// Note the honest reading: "no *ungrounded specific of a shape this module
    /// checks*", not "true" and not "not a hallucination".
    pub fn is_grounded(&self) -> bool {
        self.ungrounded.is_empty()
    }

    /// The policy predicate, which is deliberately *not* `!is_grounded()`.
    ///
    /// With no grounding at all, every specific is trivially ungrounded and a
    /// hedge would fire on every reply carrying a number — the cry-wolf mode
    /// that makes a guard worse than none. That case is enforced here rather
    /// than left to the caller's discipline.
    pub fn should_hedge(&self) -> bool {
        !self.grounding_empty && !self.is_grounded()
    }
}

/// Extract the specifics asserted in `reply` and test each against `grounding`.
///
/// Pure and deterministic: no model call, no I/O, no clock, no randomness.
/// The same `(reply, grounding)` always yields the same [`Verdict`].
pub fn check(reply: &str, grounding: &Grounding) -> Verdict {
    let mut examined = Vec::new();
    scan_into(
        &reply.chars().collect::<Vec<char>>(),
        false,
        true,
        &mut examined,
    );
    dedup_specifics(&mut examined);
    let ungrounded = examined
        .iter()
        .filter(|s| !grounding.grounds(s))
        .cloned()
        .collect();
    Verdict {
        examined,
        ungrounded,
        grounding_empty: grounding.is_empty(),
    }
}

/// What a caller should do with a reply, given its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Post the reply unchanged.
    PassThrough,
    /// Post the reply with this note appended. The note is fixed copy with a
    /// bounded list of specifics substituted in; see [`hedge_note`].
    Hedge(String),
}

/// Opening of the hedge. Fixed copy — pinned verbatim by test.
const HEDGE_PREFIX: &str = "Heads up — treat these as unsupported: ";
/// Closing of the hedge. Fixed copy — pinned verbatim by test.
///
/// Every clause here is a statement about *absence*. It must never say or
/// imply that anything ran, was fetched, was remembered, or was written: this
/// module performs one string comparison and has no such capability.
const HEDGE_SUFFIX: &str = ". Nothing in this conversation or the facts I was given contains them, and I have no source \
     for them here.";
/// How many specifics the note names before it says "and N more".
const HEDGE_MAX_LISTED: usize = 3;
/// Longest rendering of a single specific inside the note.
const HEDGE_ITEM_CHARS: usize = 40;

/// Render one ungrounded specific for display inside the note.
///
/// The text is model output being quoted back into a message Discord will
/// render, so it is treated as hostile: whitespace collapsed to keep the note
/// one line, backticks stripped, and the result wrapped in inline code so any
/// `**`, `_`, `|| ||`, `#` or `>` it still contains is inert rather than live
/// markup in the middle of an honesty sentence.
fn hedge_item(s: &Specific) -> String {
    let flat = collapse_ws(&s.text).replace('`', "");
    format!("`{}`", clip(&flat, HEDGE_ITEM_CHARS))
}

/// Render the fixed hedge copy for a non-empty set of ungrounded specifics.
///
/// Bounded regardless of how many specifics there are: at most
/// [`HEDGE_MAX_LISTED`] are named, each clipped to [`HEDGE_ITEM_CHARS`].
fn hedge_note(ungrounded: &[Specific]) -> String {
    let listed: Vec<String> = ungrounded
        .iter()
        .take(HEDGE_MAX_LISTED)
        .map(hedge_item)
        .collect();
    let mut body = listed.join(", ");
    let extra = ungrounded.len().saturating_sub(listed.len());
    if extra > 0 {
        body.push_str(&format!(" and {extra} more"));
    }
    format!("{HEDGE_PREFIX}{body}{HEDGE_SUFFIX}")
}

/// Turn a verdict into an action. Policy lives here so [`check`] stays a pure
/// measurement and a caller can choose a different policy over the same data.
///
/// See [`Verdict::should_hedge`] for why this is not simply
/// `!verdict.is_grounded()`.
pub fn action(verdict: &Verdict) -> Action {
    if verdict.should_hedge() {
        Action::Hedge(hedge_note(&verdict.ungrounded))
    } else {
        Action::PassThrough
    }
}

/// Apply [`action`] to a reply, guaranteeing the result fits
/// [`crate::ask::TIDY_LIMIT_CHARS`].
///
/// The hedge is the part that must survive: if body plus note would exceed the
/// budget, the **body** is cut at a word boundary (with an ellipsis, matching
/// `tidy_reply`'s own convention) rather than letting the command layer's clamp
/// eat the honest sentence off the end.
pub fn hedged(reply: &str, verdict: &Verdict) -> String {
    match action(verdict) {
        Action::PassThrough => reply.trim().to_string(),
        Action::Hedge(note) => {
            const SEP: &str = "\n\n";
            let budget = crate::ask::TIDY_LIMIT_CHARS
                .saturating_sub(note.chars().count() + SEP.chars().count());
            let body = trim_to_words(reply.trim(), budget);
            format!("{body}{SEP}{note}")
        }
    }
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

/// Shortest quoted run worth reporting. Below this, quotes are overwhelmingly
/// emphasis or scare quotes rather than a claimed excerpt.
const MIN_QUOTE_CHARS: usize = 6;

/// Characters allowed to sit *inside* a token between two alphanumerics.
/// This is what keeps `2021-03-04`, `3:30`, `10-20` and `v1.2.3-rc1` each a
/// single token, so the classifier sees the whole shape instead of fragments.
const fn is_connector(c: char) -> bool {
    matches!(c, '.' | ',' | ':' | '-' | '/' | '_')
}

/// Walk `chars`, appending every recognized specific to `out`.
///
/// `bare_integers` enables the grounding-side generosity (plain integers become
/// keys). `quotes` is cleared when recursing into a quotation's own contents,
/// which bounds recursion at one level while still letting a version inside a
/// quoted string be classified as a version.
fn scan_into(chars: &[char], bare_integers: bool, quotes: bool, out: &mut Vec<Specific>) {
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let c = chars[i];

        if quotes && (c == '"' || c == '\u{201C}') {
            if let Some(j) = (i + 1..n).find(|&j| chars[j] == '"' || chars[j] == '\u{201D}') {
                let inner = &chars[i + 1..j];
                let content: String = inner.iter().collect();
                let trimmed = content.trim();
                if trimmed.chars().count() >= MIN_QUOTE_CHARS
                    && trimmed.chars().any(char::is_alphabetic)
                {
                    out.push(Specific {
                        kind: SpecificKind::Quotation,
                        text: trimmed.to_string(),
                        key: collapse_ws(&trimmed.to_lowercase()),
                    });
                }
                // A fabricated `serenity = "0.12.3"` must still register as a
                // version, so the quote's contents are scanned too.
                scan_into(inner, bare_integers, false, out);
                i = j + 1;
                continue;
            }
            i += 1;
            continue;
        }

        if c.is_alphanumeric() {
            let mut j = i;
            while j < n
                && (chars[j].is_alphanumeric()
                    || (is_connector(chars[j]) && j + 1 < n && chars[j + 1].is_alphanumeric()))
            {
                j += 1;
            }
            let mut token: String = chars[i..j].iter().collect();
            let mut end = j;
            if end < n && chars[end] == '%' {
                token.push('%');
                end += 1;
            }
            let unit = next_word(chars, end);
            let unit_str = unit.as_ref().map(|(w, _)| w.as_str());
            if let Some((kind, key, consumes_unit)) = classify(&token, unit_str, bare_integers) {
                let text = match (consumes_unit, &unit) {
                    (true, Some((w, _))) => format!("{token} {w}"),
                    _ => token.clone(),
                };
                out.push(Specific { kind, text, key });
                if consumes_unit && let Some((_, after)) = unit {
                    i = after;
                    continue;
                }
            }
            i = end;
            continue;
        }

        i += 1;
    }
}

/// The next space-separated alphabetic word after `from`, lowercased, with the
/// index just past it. Used only to fold `40 percent` and `3 million` into the
/// preceding number.
fn next_word(chars: &[char], from: usize) -> Option<(String, usize)> {
    let mut k = from;
    while k < chars.len() && chars[k] == ' ' {
        k += 1;
    }
    if k == from {
        return None;
    }
    let start = k;
    while k < chars.len() && chars[k].is_alphabetic() {
        k += 1;
    }
    if k == start {
        return None;
    }
    Some((chars[start..k].iter().collect::<String>().to_lowercase(), k))
}

/// Classify one token, returning its kind, its normalized key, and whether the
/// following unit word (`percent`, `million`, …) was folded into it.
///
/// Returns `None` for every shape this module deliberately refuses to judge:
/// times, ratios, ranges, scores, list markers, identifiers, and — unless
/// `bare_integers` — plain integers.
fn classify(
    token: &str,
    unit: Option<&str>,
    bare_integers: bool,
) -> Option<(SpecificKind, String, bool)> {
    let (core, has_percent) = match token.strip_suffix('%') {
        Some(rest) => (rest, true),
        None => (token, false),
    };
    let (vprefix, rest) = match core.strip_prefix(['v', 'V']) {
        Some(r) if r.starts_with(|c: char| c.is_ascii_digit()) => (true, r),
        _ => (false, core),
    };
    if !rest.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    // Percentages, written either way.
    if has_percent
        && !vprefix
        && let Some(n) = plain_number(rest)
    {
        return Some((SpecificKind::Percentage, format!("{n}%"), false));
    }
    if !has_percent
        && !vprefix
        && matches!(unit, Some("percent" | "percentage" | "pct"))
        && let Some(n) = plain_number(rest)
    {
        return Some((SpecificKind::Percentage, format!("{n}%"), true));
    }
    if has_percent {
        return None;
    }

    let segments: Vec<&str> = rest.split('.').collect();
    let numeric_lead = segments.len() >= 2
        && !segments[0].is_empty()
        && segments[0].chars().all(|c| c.is_ascii_digit())
        && segments[1].starts_with(|c: char| c.is_ascii_digit());

    // Versions: anything `v`-prefixed, or three-plus dotted numeric segments.
    // Two bare segments (`3.5`) are left to the decimal rule below, because a
    // decimal number is by far the commoner reading.
    if vprefix && (numeric_lead || rest.chars().all(|c| c.is_ascii_digit())) {
        return Some((SpecificKind::Version, rest.to_ascii_lowercase(), false));
    }
    if numeric_lead && segments.len() >= 3 {
        return Some((SpecificKind::Version, rest.to_ascii_lowercase(), false));
    }
    if vprefix {
        return None;
    }

    if let Some(date) = iso_date(rest) {
        return Some((SpecificKind::Date, date, false));
    }

    if rest.len() == 4
        && rest.chars().all(|c| c.is_ascii_digit())
        && let Ok(year) = rest.parse::<u32>()
        && (1900..=2099).contains(&year)
    {
        return Some((SpecificKind::Year, rest.to_string(), false));
    }

    // Thousands-separated, e.g. `1,200`.
    if rest.contains(',')
        && let Some(n) = plain_number(rest)
    {
        return Some((SpecificKind::Statistic, n, false));
    }

    // Decimal, e.g. `3.5`.
    if segments.len() == 2
        && !segments[0].is_empty()
        && !segments[1].is_empty()
        && segments
            .iter()
            .all(|s| s.chars().all(|c| c.is_ascii_digit()))
    {
        return Some((SpecificKind::Statistic, rest.to_string(), false));
    }

    // Scale suffix, e.g. `12k`.
    if rest.len() >= 2 {
        let (head, tail) = rest.split_at(rest.len() - 1);
        if head.chars().all(|c| c.is_ascii_digit())
            && matches!(tail, "k" | "K" | "m" | "M" | "b" | "B" | "t" | "T")
        {
            return Some((SpecificKind::Statistic, rest.to_ascii_lowercase(), false));
        }
    }

    let is_integer = rest.chars().all(|c| c.is_ascii_digit());

    // Scale word, e.g. `3 million`.
    if is_integer
        && matches!(unit, Some("thousand" | "million" | "billion" | "trillion"))
        && let Some(word) = unit
    {
        return Some((SpecificKind::Statistic, format!("{rest} {word}"), true));
    }

    // Bare integers are suppressed on the reply side. This is the single
    // biggest false-positive class — list counts, item counts, snowflakes,
    // "Postgres 16" — and no deterministic rule separates a claim from a
    // count. Only the grounding side keeps them, where over-matching is safe.
    if bare_integers && is_integer {
        return Some((SpecificKind::Statistic, rest.to_string(), false));
    }

    None
}

/// `dddd-dd-dd` with a plausible month and day, returned as-is.
fn iso_date(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    if parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return None;
    }
    if !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(s.to_string())
}

/// Digits with optional thousands separators and at most one decimal point,
/// returned with the separators removed. `None` for anything else.
fn plain_number(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let mut dots = 0;
    for c in s.chars() {
        match c {
            '0'..='9' | ',' => {}
            '.' => dots += 1,
            _ => return None,
        }
    }
    if dots > 1 {
        return None;
    }
    Some(s.replace(',', ""))
}

/// Add `s`'s key to `keys`, plus the looser forms it should also ground.
fn expand_grounding_key(s: &Specific, keys: &mut BTreeSet<String>) {
    keys.insert(s.key.clone());
    match s.kind {
        // A source saying `2019-03-04` grounds a reply saying `2019`.
        SpecificKind::Date => {
            if let Some(year) = s.key.split('-').next() {
                keys.insert(year.to_string());
            }
        }
        // A source saying `4.2.1` grounds a reply saying `4.2` or `v4`.
        SpecificKind::Version => {
            let segments: Vec<&str> = s.key.split('.').collect();
            for take in 1..segments.len() {
                keys.insert(segments[..take].join("."));
            }
        }
        // A source saying `40%` grounds a reply saying `40`.
        SpecificKind::Percentage => {
            if let Some(n) = s.key.strip_suffix('%') {
                keys.insert(n.to_string());
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_haystack(s: &str) -> String {
    collapse_ws(&s.to_lowercase())
}

/// `haystack.contains(needle)`, but only at token boundaries, so `2019` is not
/// matched inside `20190`, inside `1.2019.3`, or inside a Discord snowflake.
///
/// A trailing `.`/`,`/`:` counts as a boundary unless a digit follows it, which
/// is what lets a sentence-final `4.2.1.` still ground `4.2.1`.
fn boundary_contains(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(needle).any(|(idx, _)| {
        let before_ok = match haystack[..idx].chars().next_back() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | ',')),
        };
        let mut after = haystack[idx + needle.len()..].chars();
        let after_ok = match after.next() {
            None => true,
            Some(c) if c.is_alphanumeric() || matches!(c, '_' | '-' | '/') => false,
            Some('.' | ',' | ':') => !after.next().is_some_and(|d| d.is_ascii_digit()),
            Some(_) => true,
        };
        before_ok && after_ok
    })
}

/// Keep the first occurrence of each `(kind, key)` pair, preserving order.
fn dedup_specifics(items: &mut Vec<Specific>) {
    let mut seen: BTreeSet<(SpecificKind, String)> = BTreeSet::new();
    items.retain(|s| seen.insert((s.kind, s.key.clone())));
}

/// Clip to `max` characters at a character boundary, appending `…` when cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// Cut `s` to at most `max` characters at a word boundary, appending ` …`.
/// Mirrors `tidy_reply`'s convention so a hedged reply does not look different
/// from a long one.
fn trim_to_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 2 {
        return String::new();
    }
    let head: String = s.chars().take(max - 2).collect();
    let cut = head
        .rfind(char::is_whitespace)
        .unwrap_or_else(|| head.trim_end().len());
    format!("{} …", head[..cut].trim_end())
}

#[cfg(test)]
mod tests {
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
}
