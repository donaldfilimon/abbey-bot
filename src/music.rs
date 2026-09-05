//! Pure music policy. Music never grants or renews Discord input consent.
use crate::voice_session::VoicePhase;

/// Validate the invocation location before any player or guild REST operation.
pub fn command_channel_gate(
    configured_guild: u64,
    configured_channel: Option<u64>,
    guild: Option<u64>,
    channel: u64,
) -> Result<(), String> {
    if guild != Some(configured_guild) {
        return Err("Music is available only in Abbey's configured voice server.".into());
    }
    if let Some(destination) = configured_channel
        && channel != destination
    {
        return Err(format!(
            "Use music commands in <#{destination}>. Music was not changed."
        ));
    }
    Ok(())
}

pub fn gate(
    configured_guild: bool,
    manager: bool,
    present: bool,
    macos: bool,
) -> Result<(), &'static str> {
    if !configured_guild {
        Err("Music is available only in Abbey's configured voice server.")
    } else if !manager {
        Err("Music controls require Manage Server.")
    } else if !present {
        Err("Join Abbey's configured voice channel before controlling music.")
    } else if !macos {
        Err("Local music capture requires a macOS host.")
    } else {
        Ok(())
    }
}
pub fn volume(level: u8, phase: VoicePhase) -> f32 {
    f32::from(level.min(100)) / 100.0
        * if phase == VoicePhase::Speaking {
            0.25
        } else {
            1.0
        }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn music_commands_require_the_configured_guild_and_optional_exact_channel() {
        assert!(command_channel_gate(1, Some(2), Some(1), 2).is_ok());
        assert_eq!(
            command_channel_gate(1, Some(2), Some(1), 3).unwrap_err(),
            "Use music commands in <#2>. Music was not changed."
        );
        for channel in [2, 3] {
            for guild in [None, Some(4)] {
                let error = command_channel_gate(1, Some(2), guild, channel).unwrap_err();
                assert!(
                    !error.contains("<#"),
                    "do not disclose another guild's destination"
                );
                assert!(command_channel_gate(1, None, guild, channel).is_err());
            }
            assert!(command_channel_gate(1, None, Some(1), channel).is_ok());
        }
    }
    #[test]
    fn every_music_gate_is_required() {
        for bits in 0..16 {
            assert_eq!(
                gate(bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0).is_ok(),
                bits == 15
            );
        }
    }
    #[test]
    fn ducking_preserves_user_volume_and_does_not_depend_on_consent() {
        for level in 0..=100 {
            for phase in [
                VoicePhase::Disconnected,
                VoicePhase::PresenceOnly,
                VoicePhase::Connecting,
                VoicePhase::Listening,
                VoicePhase::Thinking,
                VoicePhase::AwaitingConsent,
                VoicePhase::Failed,
            ] {
                assert_eq!(volume(level, phase), f32::from(level) / 100.0);
            }
            assert_eq!(
                volume(level, VoicePhase::Speaking),
                f32::from(level) / 400.0
            );
        }
    }
}
