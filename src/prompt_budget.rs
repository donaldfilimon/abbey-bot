//! Pure prompt budgeting for providers with small context windows.
//!
//! A prompt is split into [`PromptParts`]: the static persona `core` and
//! `addenda` (template text only, safe for argv), the operational `guidance`,
//! the asker's `standing` (authorization context), ranked `facts`, and the
//! transcript `turns`. [`fit`] trims deterministically
//! to a [`Budget`]: oldest turns first while keeping the last user turn and
//! everything after it, then the least relevant facts. It never trims the
//! core, guidance, standing or addenda; when those plus the last user turn still
//! overflow, the parts are returned untrimmed so the provider's own context
//! error produces the honest degraded reply. No wall clock, no randomness.

use std::borrow::Cow;

use crate::llm::{ChatTurn, Role};

/// Conservative characters per token. Measured with `fm count-tokens`
/// (macOS 27.2, AFM 3 Core Advanced, 2026-09-29): prose 4.12, JSON 3.57,
/// code 4.78, CJK 2.08, emoji 1.09. Non-ASCII text is covered by
/// [`budget_len`] charging three characters per non-ASCII scalar.
pub const CHARS_PER_TOKEN: usize = 3;

/// `fm --model system` context window, shared by input and output. Measured
/// on macOS 27.2, AFM 3 Core Advanced, 2026-09-29: 7,917 input tokens
/// answered; 8,167 failed with "The session's transcript exceeded the model's
/// context size".
pub const FM_SYSTEM_CONTEXT_TOKENS: usize = 8192;
/// Tokens held back for the model's answer.
pub const FM_SYSTEM_OUTPUT_RESERVE_TOKENS: usize = 1024;
/// Tokens held back for framing the parts do not carry: the decision schema,
/// the stdin JSON keys, tool vocabulary and the static instruction sentence.
pub const FM_SYSTEM_FRAMING_RESERVE_TOKENS: usize = 512;

/// A prompt budget in [`budget_len`] units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub max_chars: usize,
}

impl Budget {
    pub const fn from_tokens(tokens: usize) -> Self {
        Self {
            max_chars: tokens * CHARS_PER_TOKEN,
        }
    }

    /// The input budget for the on-device `fm --model system` window.
    pub const fn fm_system() -> Self {
        Self::from_tokens(
            FM_SYSTEM_CONTEXT_TOKENS
                - FM_SYSTEM_OUTPUT_RESERVE_TOKENS
                - FM_SYSTEM_FRAMING_RESERVE_TOKENS,
        )
    }
}

/// Budget cost of `text`: one per ASCII character and [`CHARS_PER_TOKEN`] per
/// non-ASCII scalar, so a non-ASCII scalar never costs less than one token.
pub fn budget_len(text: &str) -> usize {
    text.chars()
        .map(|c| if c.is_ascii() { 1 } else { CHARS_PER_TOKEN })
        .sum()
}

/// One prompt, split by what may be trimmed and where it may travel.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptParts {
    /// The persona's static system prompt. Never trimmed.
    pub core: String,
    /// Operational capability context. Never trimmed.
    pub guidance: String,
    /// Rendered style addenda (static template text). Never trimmed.
    pub addenda: String,
    /// The asker's standing line: authorization context, not a fact. Never
    /// trimmed; rendered after the facts.
    pub standing: String,
    /// Context lines with their relevance; higher is more relevant.
    pub facts: Vec<(f32, String)>,
    pub turns: Vec<ChatTurn>,
}

impl PromptParts {
    /// Split a rendered persona context into its trailing standing line and
    /// ranked facts. The context already orders its lines by relevance to the
    /// message being answered, so earlier lines rank higher. Lines rejoin
    /// byte-identically with `"\n"`.
    pub fn new(core: String, context: &str, guidance: String, turns: Vec<ChatTurn>) -> Self {
        let mut lines: Vec<&str> = if context.is_empty() {
            Vec::new()
        } else {
            context.split('\n').collect()
        };
        let standing = match lines.last() {
            Some(line) if line.starts_with(crate::memory::STANDING_PREFIX) => {
                lines.pop().unwrap_or_default().to_string()
            }
            _ => String::new(),
        };
        let count = lines.len();
        let facts = lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| ((count - index) as f32 / count as f32, line.to_string()))
            .collect();
        Self {
            core,
            guidance,
            addenda: String::new(),
            standing,
            facts,
            turns,
        }
    }

    /// Facts then standing, one per line, as the persona context rendered them.
    fn context_text(&self) -> String {
        self.facts
            .iter()
            .map(|(_, line)| line.as_str())
            .chain((!self.standing.is_empty()).then_some(self.standing.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The single system prompt for providers that take one. With no addenda
    /// and untrimmed facts this is byte-identical to the historical
    /// `core + "\n\n" + context + "\n\n" + guidance`.
    pub fn system(&self) -> String {
        join_sections(&[
            &self.core,
            &self.addenda,
            &self.context_text(),
            &self.guidance,
        ])
    }

    /// Static template text only (persona core and addenda): safe for argv.
    pub fn instructions(&self) -> String {
        join_sections(&[&self.core, &self.addenda])
    }

    /// The per-request policy that must stay off argv: facts, standing and
    /// guidance.
    pub fn policy(&self) -> String {
        join_sections(&[&self.context_text(), &self.guidance])
    }

    /// Budget cost of everything the model receives from these parts.
    pub(crate) fn cost(&self) -> usize {
        self.fixed_cost()
            + self
                .facts
                .iter()
                .map(|(_, line)| budget_len(line))
                .sum::<usize>()
            + self.turns.iter().map(turn_cost).sum::<usize>()
    }

    fn fixed_cost(&self) -> usize {
        budget_len(&self.core)
            + budget_len(&self.guidance)
            + budget_len(&self.addenda)
            + budget_len(&self.standing)
    }

    fn last_user(&self) -> Option<usize> {
        self.turns.iter().rposition(|turn| turn.role == Role::User)
    }
}

fn join_sections(sections: &[&str]) -> String {
    sections
        .iter()
        .filter(|section| !section.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn turn_cost(turn: &ChatTurn) -> usize {
    budget_len(&turn.text)
        + turn
            .tool_calls
            .iter()
            .map(|call| {
                budget_len(&call.id)
                    + budget_len(&call.name)
                    + budget_len(&call.arguments.to_string())
            })
            .sum::<usize>()
        + turn.tool_call_id.as_deref().map_or(0, budget_len)
}

/// Trim `parts` to `budget`. Deterministic and idempotent.
pub fn fit(parts: &PromptParts, budget: Budget) -> PromptParts {
    if parts.cost() <= budget.max_chars {
        return parts.clone();
    }
    let keep_from = parts.last_user().unwrap_or(0);
    let floor = parts.fixed_cost()
        + parts.turns[keep_from..]
            .iter()
            .map(turn_cost)
            .sum::<usize>();
    if floor > budget.max_chars {
        return parts.clone();
    }
    let mut fitted = parts.clone();
    let mut droppable = keep_from;
    while droppable > 0 && fitted.cost() > budget.max_chars {
        fitted.turns.remove(0);
        droppable -= 1;
    }
    // Never open on an assistant or tool turn.
    while droppable > 0 && fitted.turns[0].role != Role::User {
        fitted.turns.remove(0);
        droppable -= 1;
    }
    while fitted.cost() > budget.max_chars {
        let Some(weakest) = fitted
            .facts
            .iter()
            .enumerate()
            .min_by(|(a_index, (a, _)), (b_index, (b, _))| {
                a.total_cmp(b).then_with(|| b_index.cmp(a_index))
            })
            .map(|(index, _)| index)
        else {
            break;
        };
        fitted.facts.remove(weakest);
    }
    fitted
}

/// The parts a provider should receive: fitted to its budget, or untouched
/// when it declares none.
pub fn fitted(parts: &PromptParts, budget: Option<Budget>) -> Cow<'_, PromptParts> {
    match budget {
        Some(budget) => Cow::Owned(fit(parts, budget)),
        None => Cow::Borrowed(parts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(turns: Vec<ChatTurn>, facts: &[(f32, &str)]) -> PromptParts {
        PromptParts {
            core: "CORE persona".into(),
            guidance: "GUIDANCE".into(),
            addenda: "ADDENDA".into(),
            standing: "User standing: 0.50".into(),
            facts: facts
                .iter()
                .map(|(score, line)| (*score, (*line).to_string()))
                .collect(),
            turns,
        }
    }

    fn history() -> Vec<ChatTurn> {
        vec![
            ChatTurn::user("q1 ".repeat(20)),
            ChatTurn::assistant("a1 ".repeat(20)),
            ChatTurn::user("q2 ".repeat(20)),
            ChatTurn::assistant("a2 ".repeat(20)),
            ChatTurn::user("last question"),
        ]
    }

    #[test]
    fn budget_len_charges_non_ascii_as_one_token_each() {
        assert_eq!(budget_len("hello"), 5);
        assert_eq!(budget_len("héllo"), 7);
        // Emoji-heavy text: every scalar (including the skin-tone modifier)
        // costs a full token, far above its UTF-8-agnostic char count.
        let emoji = "👍🏽🎉🔥🔥🔥";
        assert_eq!(emoji.chars().count(), 6);
        assert_eq!(budget_len(emoji), 6 * CHARS_PER_TOKEN);
        assert_eq!(budget_len("ok 👍"), 3 + CHARS_PER_TOKEN);
    }

    #[test]
    fn fm_system_budget_matches_the_measured_window() {
        assert_eq!(Budget::fm_system().max_chars, 6656 * CHARS_PER_TOKEN);
    }

    #[test]
    fn under_budget_is_identity() {
        let p = parts(history(), &[(1.0, "fact one"), (0.5, "fact two")]);
        assert_eq!(
            fit(
                &p,
                Budget {
                    max_chars: p.cost()
                }
            ),
            p
        );
        assert_eq!(*fitted(&p, None), p);
    }

    #[test]
    fn drops_oldest_turns_first_keeps_last_user_turn() {
        let p = parts(history(), &[(1.0, "fact one")]);
        let without_first_pair = p.cost() - turn_cost(&p.turns[0]) - turn_cost(&p.turns[1]);
        let out = fit(
            &p,
            Budget {
                max_chars: without_first_pair,
            },
        );
        assert_eq!(out.turns, p.turns[2..].to_vec());
        assert_eq!(out.facts, p.facts, "facts survive while turns can go");

        let out = fit(
            &p,
            Budget {
                max_chars: without_first_pair - 1,
            },
        );
        assert_eq!(out.turns, vec![ChatTurn::user("last question")]);
        assert_eq!(out.facts, p.facts);
    }

    #[test]
    fn keeps_the_current_tool_round_after_the_last_user_turn() {
        let call = crate::tools::ToolCall {
            id: "c".into(),
            name: "recall".into(),
            arguments: serde_json::json!({"query": "x"}),
        };
        let mut turns = history();
        turns.push(ChatTurn::assistant_calls("", vec![call]));
        turns.push(ChatTurn {
            role: Role::Tool,
            text: "result".into(),
            tool_calls: Vec::new(),
            tool_call_id: Some("c".into()),
        });
        let p = parts(turns.clone(), &[]);
        let floor = p.fixed_cost() + turns[4..].iter().map(turn_cost).sum::<usize>();
        let out = fit(&p, Budget { max_chars: floor });
        assert_eq!(out.turns, turns[4..].to_vec());
    }

    #[test]
    fn drops_low_relevance_facts_after_turns() {
        let p = parts(
            history(),
            &[
                (1.0, "most relevant"),
                (0.2, "least relevant"),
                (0.6, "middle"),
            ],
        );
        let no_history = PromptParts {
            turns: vec![ChatTurn::user("last question")],
            ..p.clone()
        };
        let out = fit(
            &p,
            Budget {
                max_chars: no_history.cost() - 1,
            },
        );
        assert_eq!(out.turns, vec![ChatTurn::user("last question")]);
        assert_eq!(
            out.facts,
            vec![
                (1.0, "most relevant".to_string()),
                (0.6, "middle".to_string())
            ]
        );

        let floor = no_history.cost()
            - no_history
                .facts
                .iter()
                .map(|(_, l)| budget_len(l))
                .sum::<usize>();
        let out = fit(
            &p,
            Budget {
                max_chars: floor + budget_len("most relevant"),
            },
        );
        assert_eq!(out.facts, vec![(1.0, "most relevant".to_string())]);
    }

    #[test]
    fn never_trims_core_or_addenda() {
        let p = parts(history(), &[(1.0, "fact")]);
        for max_chars in 0..p.cost() {
            let out = fit(&p, Budget { max_chars });
            assert_eq!(out.core, p.core);
            assert_eq!(out.addenda, p.addenda);
            assert_eq!(out.guidance, p.guidance);
            assert_eq!(out.standing, p.standing, "authorization context survives");
            assert_eq!(out.turns.last(), p.turns.last());
        }
        // Below the irreducible floor nothing is trimmed: the provider's own
        // context error produces the honest degraded reply.
        assert_eq!(fit(&p, Budget { max_chars: 1 }), p);
    }

    #[test]
    fn standing_survives_a_heavy_trim_of_a_real_context() {
        let context = crate::memory::PersonaContext {
            channel_summary: "busy channel ".repeat(40),
            user_facts: vec!["likes rust".into(), "lives by the sea".into()],
            reputation: 0.9,
            addenda: String::new(),
            personal_memory_permits: Default::default(),
        };
        let p = PromptParts::new(
            "CORE".into(),
            &context.render("rust"),
            "GUIDE".into(),
            history(),
        );
        assert!(p.standing.starts_with(crate::memory::STANDING_PREFIX));
        assert_eq!(
            p.system(),
            format!("CORE\n\n{}\n\nGUIDE", context.render("rust"))
        );
        let floor = p.fixed_cost() + turn_cost(&ChatTurn::user("last question"));
        let out = fit(&p, Budget { max_chars: floor });
        assert!(out.facts.is_empty(), "every ranked fact line is trimmed");
        assert_eq!(out.turns, vec![ChatTurn::user("last question")]);
        assert_eq!(out.standing, p.standing);
        assert!(out.policy().starts_with(crate::memory::STANDING_PREFIX));
    }

    #[test]
    fn fit_is_idempotent() {
        let p = parts(history(), &[(1.0, "fact one"), (0.5, "fact two")]);
        for max_chars in 0..=p.cost() + 1 {
            let budget = Budget { max_chars };
            let once = fit(&p, budget);
            assert_eq!(fit(&once, budget), once, "max_chars {max_chars}");
        }
    }

    #[test]
    fn rendered_context_round_trips_through_ranked_lines() {
        let context =
            "Recent channel context: a\nb\nKnown about this user: x; y\nUser standing: 0.50";
        let p = PromptParts::new("CORE".into(), context, "GUIDE".into(), Vec::new());
        assert_eq!(p.facts.len(), 3);
        assert_eq!(p.standing, "User standing: 0.50");
        assert!(p.facts.windows(2).all(|pair| pair[0].0 > pair[1].0));
        assert_eq!(p.system(), format!("CORE\n\n{context}\n\nGUIDE"));
        assert_eq!(p.instructions(), "CORE");
        assert_eq!(p.policy(), format!("{context}\n\nGUIDE"));
    }
}
