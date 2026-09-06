//! Pure, privacy-bounded voice status projections.

use crate::voice::VoiceMode;
use crate::voice_session::VoicePhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberVoiceState {
    Off,
    Presence,
    AwaitingConsent,
    Active,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingCategory {
    Off,
    Local,
    OpenAi,
}

impl From<VoiceMode> for ProcessingCategory {
    fn from(value: VoiceMode) -> Self {
        match value {
            VoiceMode::Disabled => Self::Off,
            VoiceMode::Local => Self::Local,
            VoiceMode::OpenAi => Self::OpenAi,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberVoiceInput {
    pub configured: bool,
    pub phase: Option<VoicePhase>,
    pub mode: VoiceMode,
    pub caller_agrees: bool,
    pub channel_id: Option<u64>,
    pub caller_can_view_channel: bool,
    pub caller_present: bool,
    pub caller_can_manage: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberVoiceView {
    pub state: MemberVoiceState,
    pub processing: ProcessingCategory,
    pub caller_agrees: bool,
    pub channel: MemberChannel,
    pub next_action: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberChannel {
    NotConfigured,
    Hidden,
    Visible(u64),
}

impl MemberVoiceView {
    #[must_use]
    pub fn project(input: MemberVoiceInput) -> Self {
        let state = if !input.configured || input.mode == VoiceMode::Disabled {
            MemberVoiceState::Off
        } else {
            match input.phase.unwrap_or(VoicePhase::Disconnected) {
                VoicePhase::PresenceOnly | VoicePhase::Connecting => MemberVoiceState::Presence,
                VoicePhase::AwaitingConsent => MemberVoiceState::AwaitingConsent,
                VoicePhase::Listening | VoicePhase::Thinking | VoicePhase::Speaking => {
                    MemberVoiceState::Active
                }
                VoicePhase::Disconnected | VoicePhase::Failed => MemberVoiceState::Paused,
            }
        };
        let channel = match (input.channel_id, input.caller_can_view_channel) {
            (None, _) => MemberChannel::NotConfigured,
            (Some(id), true) => MemberChannel::Visible(id),
            (Some(_), false) => MemberChannel::Hidden,
        };
        let next_action = match state {
            MemberVoiceState::Off => "A manager can configure or select a processing mode.",
            MemberVoiceState::AwaitingConsent if !input.caller_agrees => {
                "Review `/voice consent` and save your choice."
            }
            MemberVoiceState::AwaitingConsent
                if input.caller_can_manage && !input.caller_present =>
            {
                "Join the configured voice channel before using `/voice resume consent:true`."
            }
            MemberVoiceState::AwaitingConsent if input.caller_can_manage => {
                "After everyone present agrees, use `/voice resume consent:true`."
            }
            MemberVoiceState::Active if input.caller_present => {
                "Use `/voice leave` whenever you want processing to stop."
            }
            MemberVoiceState::Presence if input.caller_can_manage => {
                if input.caller_present {
                    "After everyone present agrees, use `/voice join consent:true`."
                } else {
                    "Join the configured voice channel before using `/voice join consent:true`."
                }
            }
            MemberVoiceState::Paused if input.caller_can_manage => {
                if input.caller_present {
                    "Review consent, then use `/voice join consent:true`."
                } else {
                    "Join the configured voice channel before using `/voice join consent:true`."
                }
            }
            _ => "Review `/voice consent`; a manager controls starting the call.",
        };
        Self {
            state,
            processing: input.mode.into(),
            caller_agrees: input.caller_agrees,
            channel,
            next_action,
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let state = match self.state {
            MemberVoiceState::Off => "off",
            MemberVoiceState::Presence => "presence",
            MemberVoiceState::AwaitingConsent => "awaiting consent",
            MemberVoiceState::Active => "active",
            MemberVoiceState::Paused => "paused",
        };
        let processing = match self.processing {
            ProcessingCategory::Off => "Off",
            ProcessingCategory::Local => "Local",
            ProcessingCategory::OpenAi => "OpenAI",
        };
        let channel = match self.channel {
            MemberChannel::NotConfigured => "not configured".into(),
            MemberChannel::Hidden => "configured channel hidden".into(),
            MemberChannel::Visible(id) => format!("<#{id}>"),
        };
        format!(
            "Abbey voice: **{state}**\nProcessing: **{processing}**\nYour saved choice covers this mode: **{}**\nChannel: {channel}\nNext: {}",
            if self.caller_agrees { "yes" } else { "no" },
            self.next_action
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminVoiceInput {
    pub music_status: String,
    pub phase: String,
    pub media_gate_open: bool,
    pub pending_start: bool,
    pub selected_mode: String,
    pub configured_modes: Vec<String>,
    pub consent_epoch: u64,
    pub session_epoch: u64,
    pub participant_count: usize,
    pub dropped_input: u64,
    pub aborted_overruns: u64,
    pub barge_ins: u64,
    pub completed_turns: u64,
    pub speech_models: String,
    pub sidecar_readiness: String,
    pub text_backend_readiness: String,
    pub verifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminVoiceView(pub String);

impl AdminVoiceView {
    #[must_use]
    pub fn project(input: AdminVoiceInput) -> Self {
        Self(format!(
            "Voice diagnostics\n{}\nPhase: {}\nMedia gate: {}\nPending start: {}\nSelected mode: {}\nConfigured modes: {}\nConsent epoch: {} · session epoch: {}\nParticipants attested: {}\n{}\nSidecar: {}\nText backend: {}\nVerifier: {}\nQueue drops: {} · overrun-aborted turns: {} · barge-ins: {} · completed turns: {}",
            input.music_status,
            input.phase,
            if input.media_gate_open {
                "open"
            } else {
                "closed"
            },
            if input.pending_start { "yes" } else { "no" },
            input.selected_mode,
            input.configured_modes.join(", "),
            input.consent_epoch,
            input.session_epoch,
            input.participant_count,
            input.speech_models,
            input.sidecar_readiness,
            input.text_backend_readiness,
            input.verifier,
            input.dropped_input,
            input.aborted_overruns,
            input.barge_ins,
            input.completed_turns,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_channel_never_renders_its_id_or_diagnostics() {
        let view = MemberVoiceView::project(MemberVoiceInput {
            configured: true,
            phase: Some(VoicePhase::Listening),
            mode: VoiceMode::Local,
            caller_agrees: true,
            channel_id: Some(987_654_321),
            caller_can_view_channel: false,
            caller_present: true,
            caller_can_manage: false,
        });
        let rendered = view.render();
        assert!(rendered.contains("configured channel hidden"));
        assert!(!rendered.contains("987654321"));
        for forbidden in ["epoch", "model", "endpoint", "queue", "participant"] {
            assert!(
                !rendered.to_ascii_lowercase().contains(forbidden),
                "{rendered}"
            );
        }
    }

    #[test]
    fn member_projection_has_exact_field_allowlist() {
        let fields = std::mem::size_of::<MemberVoiceView>();
        assert!(fields > 0);
        let rendered = MemberVoiceView::project(MemberVoiceInput {
            configured: true,
            phase: Some(VoicePhase::AwaitingConsent),
            mode: VoiceMode::OpenAi,
            caller_agrees: false,
            channel_id: Some(7),
            caller_can_view_channel: true,
            caller_present: false,
            caller_can_manage: false,
        })
        .render();
        assert_eq!(rendered.lines().count(), 5, "{rendered}");
        assert!(rendered.contains("awaiting consent"));
        assert!(rendered.contains("OpenAI"));
    }

    #[test]
    fn manager_next_action_requires_presence_before_join_or_resume() {
        for phase in [
            VoicePhase::PresenceOnly,
            VoicePhase::Disconnected,
            VoicePhase::AwaitingConsent,
        ] {
            let base = MemberVoiceInput {
                configured: true,
                phase: Some(phase),
                mode: VoiceMode::Local,
                caller_agrees: true,
                channel_id: Some(7),
                caller_can_view_channel: false,
                caller_present: false,
                caller_can_manage: true,
            };
            let absent = MemberVoiceView::project(base.clone());
            assert!(
                absent
                    .next_action
                    .starts_with("Join the configured voice channel"),
                "{}",
                absent.next_action
            );
            let present = MemberVoiceView::project(MemberVoiceInput {
                caller_present: true,
                ..base
            });
            assert!(
                !present
                    .next_action
                    .starts_with("Join the configured voice channel"),
                "{}",
                present.next_action
            );
        }
    }
}
