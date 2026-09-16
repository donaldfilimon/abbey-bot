//! Operational context is separate from persona contracts and conversation evidence.

pub(super) fn system_prompt(
    base: &str,
    scope: &str,
    tool_names: &[&str],
    suffix: Option<&str>,
    user_input: &str,
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
            "\nYou are integrated with Discord. Prefer `/help` for the private task home. Supported guild mutations go through `/server …` with permission-mirroring (both the asking member and Abbey must hold the Discord permission)—e.g. create/rename/slowmode/delete-channel, assign/remove-role, move-member, purge with confirm. Voice/music uses `/voice …` (listening needs explicit consent; music never grants consent). Memory/persona/admin use `/remember`, `/persona`, `/admin …`. Chat tools still do not invent custom slash commands or send unsolicited member DMs. Point people at the exact supported slash next step instead of claiming every Discord action is available or denying that you are a bot. Voice presence is not listening.",
        );
        if learning_topic(user_input) {
            system.push_str(
                "\nWhen the topic is learning, the guild policy loop, DQN, epsilon, act/budget, or self-improvement: point operators at `/admin act`, `/admin learning`, `/admin brain`, and `/admin budget`. Learning updates the in-process per-guild DQN from settled rewards and does not rewrite Abbey's source code or promise autonomous self-rewrite.",
            );
        }
    }
    if let Some(suffix) = suffix.filter(|text| !text.trim().is_empty()) {
        system.push_str("\n\n");
        system.push_str(suffix.trim());
    }
    system
}

/// True when the user is asking about adaptive learning / guild policy controls.
fn learning_topic(user_input: &str) -> bool {
    let t = user_input.to_ascii_lowercase();
    const KEYS: &[&str] = &[
        "learning",
        "self-improv",
        "self improv",
        "dqn",
        "epsilon",
        "replay",
        "step_count",
        "step count",
        "unsolicited",
        "/admin act",
        "/admin learning",
        "/admin brain",
        "/admin budget",
        "guild policy",
        "policy loop",
    ];
    KEYS.iter().any(|k| t.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_guidance_names_real_tools_and_supported_recovery_without_claiming_effects() {
        let system = system_prompt(
            "Persona",
            "discord:42",
            &["recall", "inspect_status"],
            None,
            "how do I rename a channel?",
        );
        assert!(system.starts_with("Persona\n\n"));
        assert!(system.contains("recall, inspect_status"));
        assert!(system.contains("/help"));
        assert!(system.contains("Do not claim"));
        assert!(!system.contains("remember_fact"));
        assert!(
            !system.contains("/admin act"),
            "learning controls stay off-topic until learning is mentioned"
        );
    }

    #[test]
    fn discord_learning_topic_points_at_admin_learning_controls_without_code_rewrite_claims() {
        let system = system_prompt(
            "Persona",
            "discord:42",
            &["recall"],
            None,
            "Is Abbey self-improving / learning from the guild DQN?",
        );
        assert!(system.contains("/admin act"));
        assert!(system.contains("/admin learning"));
        assert!(system.contains("/admin brain"));
        assert!(system.contains("/admin budget"));
        assert!(system.contains("does not rewrite Abbey's source code"));
        assert!(!system.contains("autonomous self-rewrite of code that already happened"));
    }

    #[test]
    fn disabled_round_does_not_offer_actions_or_discord_controls_on_other_transports() {
        let system = system_prompt(
            "Persona",
            "slack:42",
            &[],
            Some("Private audition"),
            "tell me about learning",
        );
        assert!(system.contains("No callable tools"));
        assert!(!system.contains("/help"));
        assert!(!system.contains("/admin act"));
        assert!(system.ends_with("Private audition"));
    }

    #[test]
    fn operational_context_never_copies_private_scope_identifiers() {
        let system = system_prompt(
            "Persona",
            "discord:private:SECRET-SCOPE",
            &["recall"],
            None,
            "hi",
        );
        assert!(!system.contains("SECRET-SCOPE"));
        assert!(system.len() < 1800);
    }

    #[test]
    fn learning_topic_detector_is_case_insensitive() {
        assert!(learning_topic("EPSILON override?"));
        assert!(learning_topic("show /Admin Brain"));
        assert!(!learning_topic("what's the weather"));
    }
}
