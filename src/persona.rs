//! Persona routing.
//!
//! The `discord-abbey` skill defines three registers and exactly one rule for
//! choosing between them: switch when Donald specifies, or when the task type is
//! *unambiguous*. That word is load-bearing, and it is why this module refuses to
//! guess: when a request pulls toward two personas at once, it falls back to Abbey
//! rather than picking a winner on a tiebreak nobody specified.
//!
//! Routing is deliberately deterministic and model-free. Discord kills an
//! interaction token after 3 seconds, so the path that decides *who answers* must
//! not itself be the thing that blows the budget.

use std::fmt;

/// One of Abbey's three registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persona {
    /// Direct, street-smart, reads people fast. The default.
    Abbey,
    /// Analytical, structured, system-builder.
    Aviva,
    /// Warm, adaptive, rapport-builder.
    Abi,
}

impl Persona {
    /// How this persona writes. Shown to the caller so the routing decision is
    /// legible rather than magic.
    pub const fn register(self) -> &'static str {
        match self {
            Self::Abbey => "direct, street-smart, reads people fast",
            Self::Aviva => "analytical, structured, system-builder",
            Self::Abi => "warm, adaptive, rapport-builder",
        }
    }

    /// What this persona is for.
    pub const fn handles(self) -> &'static str {
        match self {
            Self::Abbey => "social help, DM drafts, navigation, general Q&A",
            Self::Aviva => "server architecture, permission design, bot specs, mod policy, code",
            Self::Abi => "welcome messages, community tone-setting, de-escalation",
        }
    }

    /// Parse an explicit persona name. Case-insensitive; leading `@` tolerated
    /// because people type `@aviva` out of Discord habit.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().trim_start_matches('@').to_lowercase().as_str() {
            "abbey" => Some(Self::Abbey),
            "aviva" => Some(Self::Aviva),
            "abi" => Some(Self::Abi),
            _ => None,
        }
    }
}

impl fmt::Display for Persona {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Abbey => "Abbey",
            Self::Aviva => "Aviva",
            Self::Abi => "Abi",
        })
    }
}

/// Why a particular persona was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The caller named the persona outright.
    Explicit,
    /// The task type was unambiguous; carries the cue that decided it.
    Cue(&'static str),
    /// Nothing pulled hard enough, or two personas pulled equally.
    Default,
}

/// A routing decision: who answers, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub persona: Persona,
    pub reason: Reason,
}

/// Cues that mark a request as Aviva's. Matched as substrings against
/// lowercased input, so `permissions` matches on `permission`.
const AVIVA_CUES: &[&str] = &[
    "permission",
    "role hierarchy",
    "channel structure",
    "architecture",
    "server setup",
    "set up a server",
    "mod policy",
    "moderation policy",
    "bot spec",
    "schema",
    "webhook",
    "audit log",
    "rate limit",
    "slash command",
];

/// Cues that mark a request as Abi's.
const ABI_CUES: &[&str] = &[
    "welcome",
    "onboard",
    "de-escalate",
    "deescalate",
    "calm ",
    "defuse",
    "icebreaker",
    "tone-setting",
    "introduce",
];

fn count_hits(haystack: &str, cues: &[&'static str]) -> (usize, Option<&'static str>) {
    let mut hits = 0;
    let mut first = None;
    for cue in cues {
        if haystack.contains(cue) {
            hits += 1;
            if first.is_none() {
                first = Some(*cue);
            }
        }
    }
    (hits, first)
}

/// Route a request to a persona.
///
/// Precedence is explicit > unambiguous cue > default. A request that hits both
/// Aviva and Abi cues equally is *ambiguous by definition*, so it goes to Abbey
/// rather than to whichever list happened to be checked first — that ordering
/// would be an implementation detail masquerading as a decision.
pub fn route(request: &str, explicit: Option<Persona>) -> Route {
    if let Some(persona) = explicit {
        return Route {
            persona,
            reason: Reason::Explicit,
        };
    }

    let lowered = request.to_lowercase();
    let (aviva, aviva_cue) = count_hits(&lowered, AVIVA_CUES);
    let (abi, abi_cue) = count_hits(&lowered, ABI_CUES);

    match (aviva, abi) {
        (0, 0) => Route {
            persona: Persona::Abbey,
            reason: Reason::Default,
        },
        (a, b) if a > b => Route {
            persona: Persona::Aviva,
            // `a > b >= 0` means at least one Aviva cue matched, so the cue is present.
            reason: Reason::Cue(aviva_cue.expect("aviva hit implies a recorded cue")),
        },
        (a, b) if b > a => Route {
            persona: Persona::Abi,
            reason: Reason::Cue(abi_cue.expect("abi hit implies a recorded cue")),
        },
        // Equal and non-zero: pulled both ways, so not unambiguous.
        _ => Route {
            persona: Persona::Abbey,
            reason: Reason::Default,
        },
    }
}

/// Render a routing decision for Discord. Kept out of the command body so the
/// wording is testable without a gateway connection.
pub fn describe(route: &Route) -> String {
    let why = match &route.reason {
        Reason::Explicit => "you named her".to_string(),
        Reason::Cue(cue) => format!("unambiguous cue: \"{cue}\""),
        Reason::Default => "nothing pulled hard enough either way".to_string(),
    };
    format!(
        "**{}** — {}\nHandles: {}\nWhy: {}",
        route.persona,
        route.persona.register(),
        route.persona.handles(),
        why
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_abbey_when_nothing_matches() {
        let r = route("what's going on in here", None);
        assert_eq!(r.persona, Persona::Abbey);
        assert_eq!(r.reason, Reason::Default);
    }

    #[test]
    fn explicit_choice_beats_every_cue() {
        // The text is saturated with Aviva cues; the explicit pick must still win.
        let r = route("permission architecture schema webhook", Some(Persona::Abi));
        assert_eq!(r.persona, Persona::Abi);
        assert_eq!(r.reason, Reason::Explicit);
    }

    #[test]
    fn routes_system_work_to_aviva() {
        let r = route("help me design the permission matrix", None);
        assert_eq!(r.persona, Persona::Aviva);
        assert_eq!(r.reason, Reason::Cue("permission"));
    }

    #[test]
    fn routes_rapport_work_to_abi() {
        let r = route("write a welcome message for new members", None);
        assert_eq!(r.persona, Persona::Abi);
        assert_eq!(r.reason, Reason::Cue("welcome"));
    }

    #[test]
    fn an_even_pull_is_ambiguous_and_falls_back_to_abbey() {
        // One cue each: "permission" (Aviva) and "welcome" (Abi).
        let r = route("set the welcome channel's permission overwrites", None);
        assert_eq!(r.persona, Persona::Abbey);
        assert_eq!(r.reason, Reason::Default);
    }

    #[test]
    fn a_stronger_pull_still_wins_when_both_match() {
        // Two Aviva cues against one Abi cue.
        let r = route(
            "welcome channel: permission matrix and role hierarchy",
            None,
        );
        assert_eq!(r.persona, Persona::Aviva);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(route("PERMISSION design", None).persona, Persona::Aviva);
    }

    #[test]
    fn parses_names_leniently_but_not_loosely() {
        assert_eq!(Persona::parse("  AVIVA "), Some(Persona::Aviva));
        assert_eq!(Persona::parse("@abi"), Some(Persona::Abi));
        assert_eq!(Persona::parse("abbey"), Some(Persona::Abbey));
        assert_eq!(Persona::parse("clyde"), None);
    }

    #[test]
    fn describe_names_the_cue_that_decided_it() {
        let text = describe(&route("audit log retention", None));
        assert!(text.contains("Aviva"), "{text}");
        assert!(text.contains("audit log"), "{text}");
    }
}
