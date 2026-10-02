//! Conversation-scoped production tool host.
use super::*;

impl crate::tools::ToolHost for ToolScope<'_> {
    fn remember_fact(&mut self, _fact: &str, _supersedes: Option<&str>) -> String {
        // Retain the tool and turn interface for in-flight provider compatibility.
        // A model invocation is never the member's explicit storage consent.
        let _turn = self.memory_turn;
        "Personal memory was not stored or queued. Ask the member to use `/remember` themselves to save a self-authored fact; use `/recall` to review existing facts.".into()
    }

    fn lookup_reputation(&mut self, user_id: Option<&str>) -> String {
        let user = match user_id {
            Some(id) => crate::guild::scoped_user_id(
                self.network.as_str(),
                id.trim_start_matches(['<', '@', '!']).trim_end_matches('>'),
            ),
            None => self.scoped_user.clone(),
        };
        let rep = self.state.reputation_snapshot(&self.scoped_guild, &user);
        format!("Reputation {rep:.2} (0 = poor, 1 = excellent).")
    }

    fn recall(&mut self, query: &str) -> String {
        let facts =
            self.state
                .memory_service()
                .recall(&self.scoped_guild, &self.scoped_user, query, 5);
        if facts.is_empty() {
            return "Nothing on record.".to_string();
        }
        facts
            .into_iter()
            .map(|f| format!("• {}", f.text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn switch_persona(&mut self, persona: crate::persona::Persona) -> String {
        self.persona = persona;
        format!("Switched to {persona}; continue the conversation as {persona}.")
    }

    fn recent_messages(&mut self, _limit: usize) -> String {
        "Inherited channel messages are retained for authorized inspection and are unavailable to generated conversation.".into()
    }

    fn inspect_status(&mut self, aspect: crate::tools::InspectAspect) -> String {
        let runtime = crate::inspect::RuntimeInspect {
            generation_configured: self.state.generation_label().is_some(),
            tools_on: self.state.providers.tools_enabled(),
            vision_on: self.state.providers.vision_available(),
            quiet: self.state.quiet,
            data: self.state.data_dir.is_some(),
            gate: self.state.episode_gate.as_ref().map(|gate| gate.counters()),
        };
        let guild_line = if matches!(
            aspect,
            crate::tools::InspectAspect::Guild | crate::tools::InspectAspect::All
        ) {
            let stores = AppState::lock(&self.state.stores);
            let guilds = AppState::lock(&self.state.guilds);
            let settings = guilds.lookup(&self.scoped_guild, &*stores);
            drop(guilds);
            drop(stores);
            settings.map(|settings| {
                let left = AppState::lock(&self.state.budget).tokens_left(
                    &self.scoped_guild,
                    settings.unsolicited_per_hour,
                    self.now,
                );
                crate::inspect::render_guild_body(&settings, left)
            })
        } else {
            None
        };
        let voice = if matches!(
            aspect,
            crate::tools::InspectAspect::Voice | crate::tools::InspectAspect::All
        ) {
            self.state.voice_inspect.state_for(&self.scoped_guild)
        } else {
            crate::inspect::VoiceInspectState::Off
        };
        let providers = if matches!(
            aspect,
            crate::tools::InspectAspect::Provider | crate::tools::InspectAspect::All
        ) {
            self.state.provider_inspect()
        } else {
            Vec::new()
        };
        crate::inspect::render_status(aspect, &runtime, guild_line.as_deref(), voice, &providers)
    }

    fn list_facts(&mut self) -> String {
        let service = self.state.memory_service();
        let context = service.context_for(
            &self.scoped_guild,
            &self.scoped_user,
            &self.scoped_channel,
            "",
            0,
            0.5,
        );
        crate::inspect::render_facts(&context.user_facts, &[])
    }
}

#[cfg(test)]
#[path = "tool_scope/memory_consent_tests.rs"]
mod memory_consent_tests;

#[cfg(test)]
#[path = "tool_scope/generated_use_tests.rs"]
mod generated_use_tests;
