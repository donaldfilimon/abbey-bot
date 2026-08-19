//! Pure prompt assembly and reply shaping for `/persona ask`.
//!
//! No I/O lives here. This module turns a routing decision into the fixed
//! system prompt a generation backend receives, and turns a backend's outcome
//! (answer, failure, or no backend at all) into the reply text Discord posts —
//! every one of which passes through `clamp_message` in the command layer.
//!
//! The persona text is a **transcription, not a dependency**. The canonical
//! persona contracts live in abi-ai (`../abi/crates/abi-ai/src/identity.rs`,
//! `profile_contract`), and abbey-bot deliberately does not take the sibling
//! path dependency — it would break the second live clone's build (see the
//! 2026-08-10 AI-backend proposal). The cost of that choice is drift: when the
//! contracts change over there, this table must be updated from that file by
//! hand.

use crate::persona::Persona;

/// The routed persona's operating description, transcribed verbatim from
/// abi-ai's `ProfileContract::description` in
/// `../abi/crates/abi-ai/src/identity.rs`.
///
/// Verbatim includes punctuation: Aviva's text carries a U+2014 em dash
/// (`honest\u{2014}not`), written here with the same escape the source uses so
/// it cannot be silently "fixed" into ASCII. A test pins it.
const fn contract_description(persona: Persona) -> &'static str {
    match persona {
        Persona::Abbey => {
            "Primary user-facing personality combining technical expertise, emotional intelligence, creativity, clear teaching, thoughtful judgment, and collaborative problem-solving. Used for most conversations when both human awareness and technical depth matter."
        }
        Persona::Aviva => {
            "Focused response mode optimized for speed, clarity, candor, and technical precision. Leads with the answer, removes unnecessary softening, identifies weak assumptions, prefers concrete actions, and communicates uncertainty plainly. Direct means concise and honest\u{2014}not reckless, hostile, or exempt from safety."
        }
        Persona::Abi => {
            "Orchestration, reasoning, policy, and routing layer. Evaluates user intent, emotional state, technical complexity, risk, available context, desired style, and required tools. May select Abbey, Aviva, or a controlled blend. Ordinarily invisible unless discussing system architecture. Not a distributed agent runtime."
        }
    }
}

/// Assemble the system prompt for the routed persona.
///
/// Deliberately static: the transcribed contract plus fixed Discord framing
/// (answer-first, length-matched, no shown reasoning — the lines that keep a
/// local reasoning model from rambling; wording reviewed by the persona
/// 2026-08-19) (the reply is clamped at 2,000 codepoints, so the model is
/// told the budget rather than generating text destined for truncation).
/// Nothing dynamic — no timestamps, ids, or guild data — so each persona's
/// prompt is one fixed string, pinned by test.
pub fn system_prompt(persona: Persona) -> String {
    format!(
        "You are {persona}. {} You are replying in a Discord conversation, so write as one message: lead with the answer, then only what supports it. Match the user's length — a short message gets a short reply; stay under about 600 characters unless more was asked for, and never over 1,900. No greetings, sign-offs, headings, or restating the question, and do not show your reasoning. You have no tools, cannot see the server, and remember only what is in this conversation or the facts provided below — say so rather than guess.",
        contract_description(persona)
    )
}

/// The reply when no generation backend is configured.
///
/// This copy is the honesty contract: it names the routed persona and states
/// plainly that nothing here can answer — a template echo dressed up as AI is
/// exactly what the proposal forbids. Pinned verbatim by test; change it only
/// deliberately, with the test.
pub fn degraded_reply(persona: Persona) -> String {
    format!(
        "**{persona}** was routed this, but no generation backend is configured, so there is no model to answer — nothing here is a canned reply. Whoever runs the bot enables answers by setting ANTHROPIC_API_KEY (Anthropic API) or ABBEY_BOT_LLM_ENDPOINT (local OpenAI-compatible server, e.g. Ollama)."
    )
}

/// Frame a backend's answer for Discord.
///
/// The label names what actually answered (`Backend::label`), because the bot
/// itself generates nothing and the copy must never suggest otherwise.
pub fn render_answer(persona: Persona, backend_label: &str, answer: &str) -> String {
    format!("**{persona}** — answered via {backend_label}:\n\n{answer}")
}

/// Frame a failed backend call.
///
/// Honest about what happened: a backend was configured, the call produced no
/// answer, and no text is invented to fill the gap.
pub fn render_failure(persona: Persona, backend_label: &str, reason: &str) -> String {
    format!("**{persona}** — the {backend_label} call failed, so there is no answer: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_the_permission_matrix_question_assembles_avivas_prompt() {
        // The proposal's worked example: this request routes to Aviva, and the
        // assembled system prompt carries her transcribed contract text —
        // asserted by string, pinned to the transcription.
        let route = crate::persona::route("help me design the permission matrix", None);
        assert_eq!(route.persona, Persona::Aviva);

        let prompt = system_prompt(route.persona);
        assert!(prompt.starts_with("You are Aviva. "), "{prompt}");
        assert!(
            prompt.contains(
                "Focused response mode optimized for speed, clarity, candor, and technical precision."
            ),
            "the prompt must carry the abi-ai ProfileContract transcription: {prompt}"
        );
    }

    #[test]
    fn each_personas_prompt_carries_its_own_transcription() {
        for persona in [Persona::Abbey, Persona::Aviva, Persona::Abi] {
            let prompt = system_prompt(persona);
            assert!(
                prompt.starts_with(&format!("You are {persona}. ")),
                "{prompt}"
            );
            assert!(prompt.contains(contract_description(persona)), "{prompt}");
        }
    }

    #[test]
    fn avivas_transcription_keeps_the_em_dash() {
        // identity.rs writes `honest\u{2014}not` with a real em dash; an ASCII
        // hyphen here would be a silent mis-transcription of the contract.
        assert!(contract_description(Persona::Aviva).contains("honest\u{2014}not"));
    }

    #[test]
    fn degraded_reply_is_the_verbatim_honesty_copy() {
        // Asserted verbatim, because this copy *is* the honesty contract: it
        // names the routed persona and states no generation backend is
        // configured. Any edit to the wording must be a deliberate one, here.
        assert_eq!(
            degraded_reply(Persona::Abbey),
            "**Abbey** was routed this, but no generation backend is configured, so there is no model to answer — nothing here is a canned reply. Whoever runs the bot enables answers by setting ANTHROPIC_API_KEY (Anthropic API) or ABBEY_BOT_LLM_ENDPOINT (local OpenAI-compatible server, e.g. Ollama)."
        );
        // The persona slot is live, not baked into the literal.
        assert!(degraded_reply(Persona::Aviva).starts_with("**Aviva** was routed"));
    }

    #[test]
    fn rendered_answers_attribute_the_backend_honestly() {
        let reply = render_answer(Persona::Abi, "external Anthropic API", "the answer");
        assert_eq!(
            reply,
            "**Abi** — answered via external Anthropic API:\n\nthe answer"
        );
    }

    #[test]
    fn rendered_failures_state_there_is_no_answer() {
        let reply = render_failure(Persona::Abbey, "external Anthropic API", "HTTP 500");
        assert!(reply.contains("there is no answer"), "{reply}");
        assert!(reply.contains("HTTP 500"), "{reply}");
    }
}
