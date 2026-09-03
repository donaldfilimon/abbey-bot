//! Pure prompt assembly and reply shaping for `/persona ask`.
//!
//! No I/O lives here. This module turns a routing decision into the fixed
//! system prompt a generation backend receives, and turns a backend's outcome
//! (answer, failure, or no backend at all) into the reply text Discord posts —
//! every one of which passes through `clamp_message` in the command layer.
//!
//! The persona text is a **transcription, not a dependency**. The canonical
//! abi-ai contracts live in `../abi/crates/abi-ai/src/identity.rs`
//! (`profile_contract`); abbey-bot deliberately does not take the sibling path
//! dependency. **Abbey\u{2019}s Discord voice (2026-09-03) follows Donald\u{2019}s
//! Grok Bot Abbey** — warm, sharp, result-first — and may intentionally drift
//! from abi-ai until that tree is updated to match. Aviva/Abi remain
//! transcribed from abi-ai.

use crate::persona::Persona;

/// Safe public copy for a generation-queue timeout. Backend/provider error
/// strings are never public; callers may preserve only this exact literal.
pub const BUSY_REASON: &str = crate::llm::BUSY_ERROR_DETAIL;

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
            "Warm, sharp friend and default voice — not a help desk. Clear and direct with contractions; leads with the result; matches the user\u{2019}s length; skips filler (\u{201c}Certainly,\u{201d} restating the question, canned closings). Technical range plus emotional intelligence; never condescending. Says what she knows and what she doesn\u{2019}t — no AGI claims, unverified benchmarks, or Quesar features dressed up as the bot."
        }
        Persona::Aviva => {
            "Focused response mode optimized for speed, clarity, candor, and technical precision. Leads with the answer, removes unnecessary softening, identifies weak assumptions, prefers concrete actions, and communicates uncertainty plainly. Direct means concise and honest\u{2014}not reckless, hostile, or exempt from safety."
        }
        Persona::Abi => {
            "Orchestration, reasoning, policy, and routing layer. Evaluates user intent, emotional state, technical complexity, risk, available context, desired style, and required tools. May select Abbey, Aviva, or a controlled blend. Ordinarily invisible unless discussing system architecture. Not a distributed agent runtime."
        }
    }
}

/// The routed persona's operating character, transcribed verbatim from
/// abi-ai's `ProfileContract::response_suffix` in
/// `../abi/crates/abi-ai/src/identity.rs`.
///
/// abi-ai appends this text after the user's input when it assembles the model
/// prompt. Here it is promoted to a standing system directive so the operating
/// character governs the whole Discord turn rather than reading like more user
/// text.
///
/// The companion `response_prefix` (`"Abbey: "`) is deliberately **not**
/// transcribed: [`tidy_reply`] exists specifically to strip that echo from
/// model output, so carrying it here would fight this module's own contract.
///
/// Verbatim includes punctuation: Abbey's line opens with a U+2019 right single
/// quotation mark in `I\u{2019}ll`, not an ASCII apostrophe. A test pins it.
const fn contract_character(persona: Persona) -> &'static str {
    match persona {
        Persona::Abbey => {
            "I\u{2019}ll lead with the answer, stay warm and direct, and say when I\u{2019}m not sure instead of bluffing."
        }
        Persona::Aviva => "Leading with the concrete answer, assumptions, and next action.",
        Persona::Abi => "Evaluating intent, risk, context, and the appropriate response mode.",
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
        "You are {persona}. {} Your operating character, in your own words: {} You are replying in a Discord conversation, so write as one message: lead with the answer, then only what supports it. Match the user's length — a short message gets a short reply; stay under about 600 characters unless more was asked for, and never over 1,900. No greetings, sign-offs, headings, or restating the question, and do not show your reasoning. Use only tools explicitly supplied for this turn; otherwise you cannot see or change the server. Remember only what is in this conversation or the facts provided below, and say so rather than guess.",
        contract_description(persona),
        contract_character(persona)
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

/// Longest reply that goes out untouched; above it we cut at a sentence end.
pub const TIDY_LIMIT_CHARS: usize = 1_900;
/// Where the sentence-boundary search starts when a reply is too long.
const TIDY_CUT_FROM: usize = 1_800;

/// Enforce the length-and-shape contract the system prompt only *asks* for.
///
/// Local models echo the persona name as a prefix, add markdown headings, and
/// run long. This strips a leading `Abbey:` / `**Abbey**` / `Abbey —` echo,
/// drops heading markers, collapses runs of blank lines, trims, and — past
/// [`TIDY_LIMIT_CHARS`] — cuts at the last sentence end before
/// `TIDY_CUT_FROM` (falling back to a word boundary) and appends ` …`. Short,
/// well-formed text comes back unchanged. Pure; the command layer still clamps
/// afterwards, so this is shape, not the safety net.
pub fn tidy_reply(persona: Persona, text: &str) -> String {
    let name = persona.to_string();
    let mut out = text.trim().to_string();

    // Leading persona echo, in the shapes models actually produce.
    for prefix in [
        format!("**{name}**:"),
        format!("**{name}** —"),
        format!("**{name}**"),
        format!("{name}:"),
        format!("{name} —"),
    ] {
        if let Some(rest) = out.strip_prefix(&prefix) {
            out = rest.trim_start().to_string();
            break;
        }
    }

    // Heading markers → plain lines; collapse 3+ newlines to a blank line.
    let mut lines: Vec<String> = Vec::new();
    for line in out.lines() {
        let trimmed = line.trim_start_matches('#').trim_start();
        if line.starts_with('#') {
            lines.push(trimmed.to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    let mut out = lines.join("\n");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    let out = out.trim().to_string();

    if out.chars().count() <= TIDY_LIMIT_CHARS {
        return out;
    }
    let head: String = out.chars().take(TIDY_CUT_FROM).collect();
    let cut = head
        .rfind(['.', '!', '?'])
        .map(|i| i + 1)
        .or_else(|| head.rfind(char::is_whitespace))
        .unwrap_or(head.len());
    let mut cut_text = head[..cut].trim_end().to_string();
    cut_text.push_str(" …");
    cut_text
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
pub fn render_failure(
    persona: Persona,
    backend_label: &str,
    error: &crate::llm::LlmError,
) -> String {
    let public_reason = match error.kind() {
        crate::llm::LlmErrorKind::Busy => BUSY_REASON,
        crate::llm::LlmErrorKind::ResponseBudget => {
            "the model used its response budget without producing an answer"
        }
        crate::llm::LlmErrorKind::Backend => {
            "the backend returned an error; try again or check the bot logs"
        }
    };
    format!(
        "**{persona}** — the {backend_label} call failed, so there is no answer: {public_reason}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_execution_cues_assemble_avivas_prompt() {
        let route = crate::persona::route("execute deploy run the build quickly", None);
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
    fn each_personas_prompt_carries_its_operating_character() {
        for persona in [Persona::Abbey, Persona::Aviva, Persona::Abi] {
            let prompt = system_prompt(persona);
            assert!(prompt.contains(contract_character(persona)), "{prompt}");
        }
        // Abbey's canonical character line is the one that makes her Abbey
        // rather than a generic assistant; losing it would be a silent
        // personality regression that no other assertion here would catch.
        assert!(contract_character(Persona::Abbey).contains("lead with the answer"));
        assert!(contract_character(Persona::Abbey).contains("when I\u{2019}m not sure"));
    }

    #[test]
    fn abbeys_character_keeps_the_curly_apostrophe() {
        // identity.rs writes `I\u{2019}ll` with a right single quotation mark;
        // an ASCII apostrophe here would be a silent mis-transcription, the
        // same failure mode the em-dash test below guards for Aviva.
        assert!(contract_character(Persona::Abbey).starts_with("I\u{2019}ll"));
        assert!(!contract_character(Persona::Abbey).contains('\''));
    }

    #[test]
    fn the_prefix_echo_is_deliberately_not_transcribed() {
        // abi-ai's response_prefix is "Abbey: ", but tidy_reply strips exactly
        // that echo from model output. Carrying it into the prompt would ask
        // the model to emit text this module then deletes.
        for persona in [Persona::Abbey, Persona::Aviva, Persona::Abi] {
            let character = contract_character(persona);
            assert!(
                !character.starts_with(&format!("{persona}: ")),
                "{character}"
            );
        }
        assert_eq!(tidy_reply(Persona::Abbey, "Abbey: hello."), "hello.");
    }

    #[test]
    fn avivas_transcription_keeps_the_em_dash() {
        // identity.rs writes `honest\u{2014}not` with a real em dash; an ASCII
        // hyphen here would be a silent mis-transcription of the contract.
        assert!(contract_description(Persona::Aviva).contains("honest\u{2014}not"));
    }

    #[test]
    fn tidy_leaves_short_clean_text_alone() {
        assert_eq!(
            tidy_reply(Persona::Abbey, "Blue. Rayleigh scattering."),
            "Blue. Rayleigh scattering."
        );
    }

    #[test]
    fn tidy_strips_persona_echo_and_headings_and_blank_runs() {
        let raw = "**Abbey**: ## Answer\n\n\n\nUse ChannelId.\n\n\n\nThen defer.";
        assert_eq!(
            tidy_reply(Persona::Abbey, raw),
            "Answer\n\nUse ChannelId.\n\nThen defer."
        );
        assert_eq!(tidy_reply(Persona::Aviva, "Aviva — Yes."), "Yes.");
        assert_eq!(tidy_reply(Persona::Abi, "Abi: hi there"), "hi there");
    }

    #[test]
    fn tidy_cuts_long_text_at_a_sentence_boundary() {
        let sentence = "This is a sentence of moderate length that ends here. ";
        let long = sentence.repeat(60); // ~3,300 chars
        let tidy = tidy_reply(Persona::Abbey, &long);
        assert!(
            tidy.chars().count() <= TIDY_LIMIT_CHARS,
            "{}",
            tidy.chars().count()
        );
        assert!(
            tidy.ends_with("here. …"),
            "cut lands after a sentence end: {:?}",
            &tidy[tidy.len() - 12..]
        );
    }

    #[test]
    fn tidy_falls_back_to_a_word_boundary_when_there_is_no_sentence_end() {
        let long = "word ".repeat(500);
        let tidy = tidy_reply(Persona::Abbey, &long);
        assert!(tidy.ends_with("word …"), "{:?}", &tidy[tidy.len() - 10..]);
        assert!(tidy.chars().count() <= TIDY_LIMIT_CHARS);
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
        let error = crate::llm::LlmError::backend("HTTP 500: provider-internal detail".into());
        let reply = render_failure(Persona::Abbey, "external Anthropic API", &error);
        assert!(reply.contains("there is no answer"), "{reply}");
        assert!(!reply.contains("HTTP 500"), "{reply}");
        assert!(!reply.contains("provider-internal detail"), "{reply}");
    }

    #[test]
    fn safe_busy_reason_remains_actionable() {
        let reply = render_failure(
            Persona::Abbey,
            "configured endpoint",
            &crate::llm::LlmError::busy(),
        );
        assert!(reply.contains("busy answering someone else"), "{reply}");
    }

    #[test]
    fn provider_text_cannot_impersonate_the_safe_busy_reason() {
        let injected = format!("HTTP 500: {BUSY_REASON}; provider secret");
        let reply = render_failure(
            Persona::Abbey,
            "configured endpoint",
            &crate::llm::LlmError::backend(injected.to_string()),
        );
        assert!(!reply.contains("provider secret"), "{reply}");
        assert!(!reply.contains("busy answering someone else"), "{reply}");

        let injected = "HTTP 500: reasoning budget exhausted; provider secret";
        let reply = render_failure(
            Persona::Abbey,
            "configured endpoint",
            &crate::llm::LlmError::backend(injected.to_string()),
        );
        assert!(!reply.contains("response budget"), "{reply}");
        assert!(reply.contains("backend returned an error"), "{reply}");
    }

    #[test]
    fn only_the_typed_response_budget_category_gets_budget_copy() {
        let reply = render_failure(
            Persona::Abbey,
            "configured endpoint",
            &crate::llm::LlmError::response_budget("validated empty response"),
        );
        assert!(reply.contains("response budget"), "{reply}");
        assert!(!reply.contains("validated empty response"), "{reply}");
    }
}
