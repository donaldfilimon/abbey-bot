//! The conversation engine: per-scope multi-turn sessions and the prompts
//! that feed a backend.
//!
//! This is the port of appleintelligence.md's `ABIEngine` seam, minus the
//! model: one session per scope key (normally the scoped channel id), so
//! multi-turn context is per-conversation rather than global; a `render`ed
//! [`PersonaContext`] appended to the persona's system prompt; and a budget
//! check standing in for `willOverflow`. Everything here is pure — the
//! command layer owns the [`crate::llm::Backend`] and the transport, calls
//! [`Engine::prepare`], posts the request, and hands the reply back through
//! [`Engine::commit`]. No test needs a network, and neither does this module.
//!
//! Two behaviours are deliberate rather than incidental: a persona switch
//! mid-conversation keeps the transcript (the spec's Dynamic Profiles point —
//! history survives a switch; only the instructions change), and trimming
//! always drops the *oldest* turns, by count and by character budget.

use std::collections::{HashMap, VecDeque};

use crate::llm::{ChatTurn, Role};
use crate::memory::PersonaContext;
use crate::persona::Persona;

/// Turns a session keeps before the oldest fall off.
pub const MAX_TURNS: usize = 20;
/// Characters of transcript a session keeps; older turns are dropped past it.
pub const CONTEXT_BUDGET_CHARS: usize = 6000;

/// One conversation's live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub persona: Persona,
    /// Oldest first, strictly alternating user/assistant.
    pub turns: VecDeque<ChatTurn>,
    pub last_used: u64,
}

impl Session {
    fn new(persona: Persona, now: u64) -> Self {
        Self {
            persona,
            turns: VecDeque::new(),
            last_used: now,
        }
    }

    fn chars(&self) -> usize {
        self.turns.iter().map(|t| t.text.chars().count()).sum()
    }

    /// Drop the oldest turns until both caps hold. Turns are always removed
    /// in pairs-or-better from the front, so the transcript never starts
    /// with an assistant turn (which the Anthropic API rejects).
    fn trim(&mut self) {
        while self.turns.len() > MAX_TURNS || self.chars() > CONTEXT_BUDGET_CHARS {
            if self.turns.pop_front().is_none() {
                break;
            }
        }
        while self
            .turns
            .front()
            .is_some_and(|t| t.role == Role::Assistant)
        {
            self.turns.pop_front();
        }
    }
}

/// What the command layer sends: the system prompt and the full transcript
/// ending in the new user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTurn {
    pub system_prompt: String,
    pub turns: Vec<ChatTurn>,
}

impl PreparedTurn {
    /// Characters across the system prompt and every turn.
    pub fn chars(&self) -> usize {
        self.system_prompt.chars().count()
            + self
                .turns
                .iter()
                .map(|t| t.text.chars().count())
                .sum::<usize>()
    }
}

/// Per-scope sessions. One per bot process; the command layer holds it
/// behind whatever lock it already uses for memory.
#[derive(Debug, Default)]
pub struct Engine {
    sessions: HashMap<String, Session>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assemble the next request for `scope`. Creates the session on first
    /// use; on a persona change switches the persona and keeps the transcript.
    /// The transcript is not mutated — [`Engine::commit`] does that once the
    /// backend has answered, so a failed call leaves no dangling user turn.
    pub fn prepare(
        &mut self,
        scope: &str,
        persona: Persona,
        context: &PersonaContext,
        user_input: &str,
        now: u64,
    ) -> PreparedTurn {
        let session = self
            .sessions
            .entry(scope.to_string())
            .or_insert_with(|| Session::new(persona, now));
        session.persona = persona;
        session.last_used = now;
        let mut turns: Vec<ChatTurn> = session.turns.iter().cloned().collect();
        turns.push(ChatTurn::user(user_input));
        PreparedTurn {
            system_prompt: format!(
                "{}\n\n{}",
                crate::ask::system_prompt(persona),
                context.render()
            ),
            turns,
        }
    }

    /// Record one exchange and trim to the caps.
    pub fn commit(&mut self, scope: &str, user_input: &str, assistant_reply: &str, now: u64) {
        let session = self
            .sessions
            .entry(scope.to_string())
            .or_insert_with(|| Session::new(Persona::Abbey, now));
        session.turns.push_back(ChatTurn::user(user_input));
        session
            .turns
            .push_back(ChatTurn::assistant(assistant_reply));
        session.last_used = now;
        session.trim();
    }

    /// Forget one scope's transcript entirely.
    pub fn reset(&mut self, scope: &str) -> bool {
        self.sessions.remove(scope).is_some()
    }

    /// Drop every session idle for longer than `max_idle_secs`. Returns how
    /// many were dropped.
    pub fn evict_idle(&mut self, now: u64, max_idle_secs: u64) -> usize {
        let before = self.sessions.len();
        self.sessions
            .retain(|_, s| now.saturating_sub(s.last_used) <= max_idle_secs);
        before - self.sessions.len()
    }

    /// Turns held for `scope` (0 when there is no session).
    pub fn session_len(&self, scope: &str) -> usize {
        self.sessions.get(scope).map_or(0, |s| s.turns.len())
    }

    pub fn session_persona(&self, scope: &str) -> Option<Persona> {
        self.sessions.get(scope).map(|s| s.persona)
    }

    /// The `willOverflow` stand-in: would this prompt exceed a character
    /// budget? Callers pass the budget their backend can take.
    pub fn will_overflow(prepared: &PreparedTurn, budget_chars: usize) -> bool {
        prepared.chars() > budget_chars
    }
}

/// The `/summarize` prompt pair: the persona's system prompt, and a user
/// message carrying the transcript to compress.
pub fn summarize_prompt(persona: Persona, transcript: &str, count: usize) -> (String, String) {
    let system = crate::ask::system_prompt(persona);
    let user = format!(
        "Summarize the last {count} messages from this channel in a few sentences, keeping who said what only where it matters. Messages are oldest first, one per line as `author: text`.\n\n{transcript}"
    );
    (system, user)
}

/// The member-joined welcome prompt. Always the Abi persona — platforms.md:
/// "welcome is always warm".
pub fn welcome_prompt(display_name: &str) -> String {
    format!(
        "{}\n\nA new member named {display_name} just joined the server. Write a short, warm welcome message addressed to them — two sentences at most, no questions about private details.",
        crate::ask::system_prompt(Persona::Abi)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Backend, build_chat_request};
    use serde_json::json;

    fn ctx() -> PersonaContext {
        PersonaContext::empty()
    }

    #[test]
    fn prepare_appends_context_to_the_persona_prompt() {
        let mut engine = Engine::new();
        let context = PersonaContext {
            channel_summary: "deploy talk".into(),
            user_facts: vec!["likes rust".into()],
            reputation: 0.5,
        };
        let prepared = engine.prepare("c", Persona::Aviva, &context, "hi", 1);
        assert_eq!(
            prepared.system_prompt,
            format!(
                "{}\n\nRecent channel context: deploy talk\nKnown about this user: likes rust\nUser standing: 0.50",
                crate::ask::system_prompt(Persona::Aviva)
            )
        );
        assert_eq!(prepared.turns, vec![ChatTurn::user("hi")]);
        // prepare alone records nothing — a failed call leaves no orphan turn.
        assert_eq!(engine.session_len("c"), 0);
    }

    #[test]
    fn multi_turn_request_alternates_on_both_backends() {
        let mut engine = Engine::new();
        engine.commit("c", "q1", "a1", 1);
        let prepared = engine.prepare("c", Persona::Abbey, &ctx(), "q2", 2);
        assert_eq!(
            prepared.turns,
            vec![
                ChatTurn::user("q1"),
                ChatTurn::assistant("a1"),
                ChatTurn::user("q2")
            ]
        );

        let anthropic = Backend::Anthropic {
            api_key: "k".into(),
        };
        let request = build_chat_request(&anthropic, &prepared.system_prompt, &prepared.turns);
        assert_eq!(request.body["system"], prepared.system_prompt);
        assert_eq!(
            request.body["messages"],
            json!([
                {"role": "user", "content": "q1"},
                {"role": "assistant", "content": "a1"},
                {"role": "user", "content": "q2"},
            ])
        );

        let local = Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8080".into(),
        };
        let request = build_chat_request(&local, &prepared.system_prompt, &prepared.turns);
        let messages = request.body["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], prepared.system_prompt);
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["role"], "user");
    }

    #[test]
    fn sessions_are_per_scope() {
        let mut engine = Engine::new();
        engine.commit("c1", "q", "a", 1);
        assert_eq!(engine.session_len("c1"), 2);
        assert_eq!(engine.session_len("c2"), 0);
        let prepared = engine.prepare("c2", Persona::Abbey, &ctx(), "fresh", 2);
        assert_eq!(prepared.turns.len(), 1);
    }

    #[test]
    fn trimming_by_turn_count_keeps_the_most_recent() {
        let mut engine = Engine::new();
        for i in 0..15 {
            engine.commit("c", &format!("q{i}"), &format!("a{i}"), i);
        }
        assert_eq!(engine.session_len("c"), MAX_TURNS);
        let prepared = engine.prepare("c", Persona::Abbey, &ctx(), "next", 99);
        assert_eq!(prepared.turns[0], ChatTurn::user("q5"));
        assert_eq!(prepared.turns[MAX_TURNS - 1], ChatTurn::assistant("a14"));
        assert_eq!(prepared.turns.last(), Some(&ChatTurn::user("next")));
    }

    #[test]
    fn trimming_by_char_budget_keeps_the_most_recent_and_starts_on_a_user_turn() {
        let mut engine = Engine::new();
        let big = "x".repeat(2500);
        engine.commit("c", "q0", &big, 1); // 2502 chars
        engine.commit("c", "q1", &big, 2); // 5004
        engine.commit("c", "q2", &big, 3); // 7506 → trim
        let prepared = engine.prepare("c", Persona::Abbey, &ctx(), "next", 4);
        // Oldest pair gone; transcript still opens on a user turn.
        assert_eq!(prepared.turns[0], ChatTurn::user("q1"));
        assert_eq!(prepared.turns.len(), 5);
        let total: usize = engine.sessions["c"]
            .turns
            .iter()
            .map(|t| t.text.chars().count())
            .sum();
        assert!(total <= CONTEXT_BUDGET_CHARS, "{total}");
    }

    #[test]
    fn persona_switch_keeps_the_transcript() {
        let mut engine = Engine::new();
        engine.prepare("c", Persona::Abbey, &ctx(), "q1", 1);
        engine.commit("c", "q1", "a1", 1);
        let prepared = engine.prepare("c", Persona::Aviva, &ctx(), "q2", 2);
        assert_eq!(engine.session_persona("c"), Some(Persona::Aviva));
        assert!(prepared.system_prompt.starts_with("You are Aviva. "));
        assert_eq!(prepared.turns.len(), 3, "history survived the switch");
        assert_eq!(prepared.turns[0], ChatTurn::user("q1"));
    }

    #[test]
    fn reset_forgets_one_scope_only() {
        let mut engine = Engine::new();
        engine.commit("c1", "q", "a", 1);
        engine.commit("c2", "q", "a", 1);
        assert!(engine.reset("c1"));
        assert!(!engine.reset("c1"));
        assert_eq!(engine.session_len("c1"), 0);
        assert_eq!(engine.session_len("c2"), 2);
    }

    #[test]
    fn evict_idle_drops_only_stale_sessions() {
        let mut engine = Engine::new();
        engine.commit("old", "q", "a", 100);
        engine.commit("fresh", "q", "a", 900);
        assert_eq!(engine.evict_idle(1000, 300), 1);
        assert_eq!(engine.session_len("old"), 0);
        assert_eq!(engine.session_len("fresh"), 2);
        // Exactly at the boundary is kept.
        assert_eq!(engine.evict_idle(1200, 300), 0);
        assert_eq!(engine.evict_idle(1201, 300), 1);
    }

    #[test]
    fn will_overflow_measures_prompt_plus_turns() {
        let mut engine = Engine::new();
        let prepared = engine.prepare("c", Persona::Abbey, &ctx(), "hello", 1);
        let size = prepared.chars();
        assert!(!Engine::will_overflow(&prepared, size));
        assert!(Engine::will_overflow(&prepared, size - 1));
    }

    #[test]
    fn summarize_prompt_carries_count_and_transcript() {
        let (system, user) = summarize_prompt(Persona::Abbey, "a: hi\nb: yo", 2);
        assert_eq!(system, crate::ask::system_prompt(Persona::Abbey));
        assert!(user.starts_with("Summarize the last 2 messages"), "{user}");
        assert!(user.ends_with("a: hi\nb: yo"), "{user}");
    }

    #[test]
    fn welcome_prompt_is_always_abi() {
        let prompt = welcome_prompt("Dana");
        assert!(prompt.starts_with("You are Abi. "), "{prompt}");
        assert!(prompt.contains("named Dana just joined"), "{prompt}");
    }
}
