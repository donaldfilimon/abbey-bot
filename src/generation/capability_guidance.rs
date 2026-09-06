//! Operational context is separate from persona contracts and conversation evidence.

pub(super) fn system_prompt(
    base: &str,
    scope: &str,
    tool_names: &[&str],
    suffix: Option<&str>,
) -> String {
    let tools = if tool_names.is_empty() {
        "No callable tools are offered in this round. Answer from the available conversation and evidence; do not claim to have inspected or changed external state.".to_owned()
    } else {
        format!(
            "Callable tools in this round: {}. Only these offered tools can perform actions during this answer. Use inspect_status for current operational facts when it is offered, and distinguish an unavailable tool from an unavailable provider.",
            tool_names.join(", ")
        )
    };
    let mut system = format!(
        "{base}\n\nOperational capability context (not conversation evidence):\n{tools}\nDo not claim an action completed without its successful result. Pending, refused, partial and uncertain effects are not completed actions. Do not invent commands or promise future work that has not been scheduled."
    );
    if scope.starts_with("discord:") {
        system.push_str(
            "\nYou are integrated with Discord. The private /help task home offers conversation, memory, image instructions, voice/music status and authorized administration. Direct people there for supported workflows; each action still checks its inputs and authority. Voice presence is not listening: starting requires the current participants' agreement and an authorized member in the configured channel. These chat tools do not create arbitrary roles, custom slash commands, or unsolicited member DMs. Explain the specific supported next step instead of denying that you are a bot or claiming every Discord action is available.",
        );
    }
    if let Some(suffix) = suffix.filter(|text| !text.trim().is_empty()) {
        system.push_str("\n\n");
        system.push_str(suffix.trim());
    }
    system
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_guidance_names_real_tools_and_supported_recovery_without_claiming_effects() {
        let system = system_prompt("Persona", "discord:42", &["recall", "inspect_status"], None);
        assert!(system.starts_with("Persona\n\n"));
        assert!(system.contains("recall, inspect_status"));
        assert!(system.contains("/help"));
        assert!(system.contains("Do not claim"));
        assert!(!system.contains("remember_fact"));
    }

    #[test]
    fn disabled_round_does_not_offer_actions_or_discord_controls_on_other_transports() {
        let system = system_prompt("Persona", "slack:42", &[], Some("Private audition"));
        assert!(system.contains("No callable tools"));
        assert!(!system.contains("/help"));
        assert!(system.ends_with("Private audition"));
    }

    #[test]
    fn operational_context_never_copies_private_scope_identifiers() {
        let system = system_prompt("Persona", "discord:private:SECRET-SCOPE", &["recall"], None);
        assert!(!system.contains("SECRET-SCOPE"));
        assert!(system.len() < 1800);
    }
}
