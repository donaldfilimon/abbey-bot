//! Conversation-scoped production tool host.
use super::*;

impl crate::tools::ToolHost for ToolScope<'_> {
    fn remember_fact(&mut self, fact: &str, supersedes: Option<&str>) -> String {
        // A model may PROPOSE that a new fact replaces an old one, but never
        // apply it. `remember_proposing` stores the new fact and queues the
        // proposal; the old fact survives until a human confirms. There is no
        // model-callable path to `remember_replacing` — a model must not be
        // able to confirm its own contested claim.
        // With the episode gate configured every memory write is proposed to
        // the ledger first, and this trait is synchronous, so the model path
        // cannot propose here. It queues instead (Donald's choice 2026-09-06):
        // the write is proposed and stored by the next drain, and the model is
        // told nothing is on record yet. A supersession stays a proposal the
        // person confirms; the old fact is never removed by the model.
        if self.state.gate_for(&self.scoped_guild).is_some() {
            return match crate::memory_gate::enqueue(
                self.state,
                &self.scoped_guild,
                &self.scoped_user,
                fact,
                supersedes,
                self.now,
            ) {
                Ok(message) | Err(message) => message,
            };
        }
        let service = self.state.memory_service();
        let outcome = match supersedes {
            Some(old) => service.remember_proposing(
                &self.scoped_guild,
                &self.scoped_user,
                fact,
                old,
                self.now,
            ),
            None => service.remember(&self.scoped_guild, &self.scoped_user, fact, self.now),
        };
        match outcome {
            Ok(RememberOutcome::Stored(fact)) => format!("Stored: {fact}"),
            Ok(RememberOutcome::Proposed { stored, proposed }) => format!(
                "Stored: {stored}. Proposed to replace {proposed:?}, which is unchanged until the person confirms."
            ),
            Ok(RememberOutcome::Superseded { stored, removed }) => {
                format!("Stored: {stored}. Replaced: {removed}")
            }
            Ok(RememberOutcome::Unchanged) => {
                "Already on record (or the fact list is full).".to_string()
            }
            Err(message) => message.to_string(),
        }
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

    fn recent_messages(&mut self, limit: usize) -> String {
        let text = AppState::lock(&self.state.stores)
            .memory
            .channel_mut(&self.scoped_channel)
            .render_recent(limit);
        if text.trim().is_empty() {
            "No recent messages on record for this channel.".to_string()
        } else {
            text
        }
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
        let (facts, pending) = service.subject_snapshot(&self.scoped_guild, &self.scoped_user);
        crate::inspect::render_facts(&facts, &pending)
    }
}
