//! Canonical ABI persona routing, transcribed from `abi-ai`.
//!
//! Abbey is the empathetic polymath and neutral default, Aviva is the concise
//! direct expert, and ABI/Abi is the orchestration and governance layer. The
//! weights, keyword scores, prefix matching, exact leading-name selector, and
//! tie order mirror `../abi/crates/abi-ai/src/{identity,keywords,router}.rs`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persona {
    Abbey,
    Aviva,
    Abi,
}

impl Persona {
    pub const fn register(self) -> &'static str {
        match self {
            Self::Abbey => "empathetic polymath with technical depth and human awareness",
            Self::Aviva => "concise direct expert: answer, assumptions, next action",
            Self::Abi => "orchestration, reasoning, policy, risk, and routing",
        }
    }

    pub const fn handles(self) -> &'static str {
        match self {
            Self::Abbey => "most technical, emotional, creative, and teaching conversations",
            Self::Aviva => "urgent execution, precise fixes, and deliberately terse answers",
            Self::Abi => "explicit system orchestration, governance, routing, and risk review",
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileWeights {
    pub abbey: f32,
    pub aviva: f32,
    pub abi: f32,
}

impl ProfileWeights {
    pub const fn prior() -> Self {
        Self {
            abbey: 0.40,
            aviva: 0.30,
            abi: 0.30,
        }
    }

    pub const fn only(persona: Persona) -> Self {
        match persona {
            Persona::Abbey => Self {
                abbey: 1.0,
                aviva: 0.0,
                abi: 0.0,
            },
            Persona::Aviva => Self {
                abbey: 0.0,
                aviva: 1.0,
                abi: 0.0,
            },
            Persona::Abi => Self {
                abbey: 0.0,
                aviva: 0.0,
                abi: 1.0,
            },
        }
    }

    fn normalize(&mut self) {
        let total = self.abbey + self.aviva + self.abi;
        if total > 0.0 {
            self.abbey /= total;
            self.aviva /= total;
            self.abi /= total;
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Keyword {
    word: &'static str,
    abbey: f32,
    aviva: f32,
    abi: f32,
}

macro_rules! keyword {
    ($word:literal, $abbey:literal, $aviva:literal, $abi:literal) => {
        Keyword {
            word: $word,
            abbey: $abbey,
            aviva: $aviva,
            abi: $abi,
        }
    };
}

const KEYWORDS: [Keyword; 29] = [
    keyword!("analyze", 0.9, 0.5, 0.2),
    keyword!("structure", 0.9, 0.6, 0.2),
    keyword!("logical", 0.85, 0.7, 0.2),
    keyword!("compare", 0.8, 0.7, 0.2),
    keyword!("explain", 0.95, 0.4, 0.2),
    keyword!("creative", 0.95, 0.3, 0.1),
    keyword!("imagine", 0.95, 0.3, 0.1),
    keyword!("explore", 0.9, 0.4, 0.2),
    keyword!("brainstorm", 0.95, 0.3, 0.1),
    keyword!("help", 0.95, 0.4, 0.2),
    keyword!("learn", 0.95, 0.3, 0.2),
    keyword!("frustrated", 0.95, 0.2, 0.1),
    keyword!("run", 0.3, 0.95, 0.2),
    keyword!("execute", 0.3, 0.95, 0.2),
    keyword!("deploy", 0.3, 0.95, 0.2),
    keyword!("build", 0.5, 0.9, 0.2),
    keyword!("fix", 0.5, 0.95, 0.2),
    keyword!("quick", 0.3, 0.95, 0.1),
    keyword!("direct", 0.3, 0.95, 0.1),
    keyword!("concise", 0.3, 0.95, 0.1),
    keyword!("orchestrate", 0.2, 0.2, 0.95),
    keyword!("routing", 0.2, 0.2, 0.95),
    keyword!("governance", 0.3, 0.3, 0.95),
    keyword!("policy", 0.3, 0.4, 0.9),
    keyword!("profile", 0.3, 0.3, 0.9),
    keyword!("safe", 0.8, 0.5, 0.5),
    keyword!("risk", 0.8, 0.6, 0.6),
    keyword!("design", 0.9, 0.5, 0.3),
    keyword!("pattern", 0.85, 0.5, 0.3),
];

#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    Explicit,
    Weighted(ProfileWeights),
    Default,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub persona: Persona,
    pub reason: Reason,
}

/// Recognize only an exact leading Abbey, Aviva, or ABI token, optionally
/// prefixed with `@` and followed by whitespace, comma, or colon.
#[must_use]
pub fn explicit_selector(input: &str) -> Option<Persona> {
    let mut remaining = input.trim_matches([' ', '\t', '\r', '\n']);
    remaining = remaining.strip_prefix('@').unwrap_or(remaining);
    let bytes = remaining.as_bytes();
    let end = bytes
        .iter()
        .position(|byte| !byte.is_ascii_alphabetic())
        .unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    if end < bytes.len() {
        let separator = bytes[end];
        if !matches!(
            separator,
            b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C | b',' | b':'
        ) {
            return None;
        }
    }
    match &remaining[..end] {
        name if name.eq_ignore_ascii_case("abbey") => Some(Persona::Abbey),
        name if name.eq_ignore_ascii_case("aviva") => Some(Persona::Aviva),
        name if name.eq_ignore_ascii_case("abi") => Some(Persona::Abi),
        _ => None,
    }
}

#[must_use]
pub fn analyze(input: &str) -> ProfileWeights {
    if let Some(persona) = explicit_selector(input) {
        return ProfileWeights::only(persona);
    }
    let mut weights = ProfileWeights::prior();
    for word in input.split_ascii_whitespace() {
        let trimmed = word.trim_end_matches(['.', ',', '!', '?', ':', ';', '"', '\'']);
        for entry in KEYWORDS {
            if starts_with_ignore_case(trimmed, entry.word) {
                weights.abbey += entry.abbey * 0.1;
                weights.aviva += entry.aviva * 0.1;
                weights.abi += entry.abi * 0.1;
            }
        }
    }
    weights.normalize();
    weights
}

fn select(weights: ProfileWeights) -> Persona {
    if weights.abbey >= weights.aviva && weights.abbey >= weights.abi {
        Persona::Abbey
    } else if weights.aviva >= weights.abi {
        Persona::Aviva
    } else {
        Persona::Abi
    }
}

fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack.len() >= needle.len()
        && haystack.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
}

#[must_use]
pub fn route(request: &str, explicit: Option<Persona>) -> Route {
    if let Some(persona) = explicit {
        return Route {
            persona,
            reason: Reason::Explicit,
        };
    }
    if let Some(persona) = explicit_selector(request) {
        return Route {
            persona,
            reason: Reason::Explicit,
        };
    }
    let weights = analyze(request);
    let persona = select(weights);
    let reason = if weights == ProfileWeights::prior() {
        Reason::Default
    } else {
        Reason::Weighted(weights)
    };
    Route { persona, reason }
}

#[must_use]
pub fn describe(route: &Route) -> String {
    let why = match route.reason {
        Reason::Explicit => "explicit leading name or command choice".to_string(),
        Reason::Default => "neutral ABI prior favors Abbey (0.40 / 0.30 / 0.30)".to_string(),
        Reason::Weighted(weights) => format!(
            "ABI weights: Abbey {:.2} · Aviva {:.2} · ABI {:.2}",
            weights.abbey, weights.aviva, weights.abi
        ),
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
    fn neutral_prior_and_ties_favor_abbey() {
        let route = route("hello world", None);
        assert_eq!(route.persona, Persona::Abbey);
        assert_eq!(route.reason, Reason::Default);
        assert_eq!(
            select(ProfileWeights {
                abbey: 1.0,
                aviva: 1.0,
                abi: 1.0
            }),
            Persona::Abbey
        );
    }

    #[test]
    fn leading_names_are_exact_overrides() {
        assert_eq!(route("Aviva, be direct", None).persona, Persona::Aviva);
        assert_eq!(route("@ABI: orchestrate", None).persona, Persona::Abi);
        assert_eq!(route("Abbey help", None).persona, Persona::Abbey);
        assert_eq!(route("avivacious prose", None).persona, Persona::Abbey);
        assert_eq!(route("Please ask Aviva", None).persona, Persona::Abbey);
    }

    #[test]
    fn explicit_command_choice_beats_text() {
        assert_eq!(
            route("Aviva execute quickly", Some(Persona::Abi)).persona,
            Persona::Abi
        );
    }

    #[test]
    fn execution_routes_aviva_and_governance_routes_abi() {
        assert_eq!(
            route("execute deploy run the build quickly", None).persona,
            Persona::Aviva
        );
        assert_eq!(
            route("orchestrate routing governance policy", None).persona,
            Persona::Abi
        );
    }

    #[test]
    fn stems_match_only_token_prefixes() {
        assert!(analyze("running").aviva > ProfileWeights::prior().aviva);
        assert_eq!(analyze("overrun"), ProfileWeights::prior());
        assert_eq!(route("unsafe", None).reason, Reason::Default);
    }

    #[test]
    fn descriptions_match_canonical_roles() {
        assert!(Persona::Abbey.register().contains("empathetic polymath"));
        assert!(Persona::Aviva.register().contains("direct expert"));
        assert!(Persona::Abi.register().contains("orchestration"));
    }
}
