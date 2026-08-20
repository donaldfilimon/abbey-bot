//! Provider-neutral policy and PCM transforms for live voice.
//!
//! Discord/Songbird and WebSocket types stay in the command shell. This module
//! owns the fail-closed configuration and deterministic audio transforms, so
//! both can be tested without a gateway, voice server, or API key.

use std::fmt;

const DEFAULT_ENDPOINT: &str = "wss://api.openai.com/v1/realtime";
const DEFAULT_MODEL: &str = "gpt-realtime-2.1";
const DEFAULT_VOICE: &str = "marin";
const MAX_INSTRUCTIONS_CHARS: usize = 8_000;

/// A single explicitly allowed Discord destination and Realtime provider.
///
/// The API key is deliberately private and `Debug` is redacted. Voice is off
/// unless all three required values are present; a partial configuration is a
/// startup error rather than an unexpectedly permissive mode.
#[derive(Clone)]
pub struct VoiceConfig {
    pub guild_id: u64,
    pub channel_id: u64,
    api_key: String,
    pub endpoint: String,
    pub model: String,
    pub voice: String,
    pub instructions: String,
}

impl fmt::Debug for VoiceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoiceConfig")
            .field("guild_id", &self.guild_id)
            .field("channel_id", &self.channel_id)
            .field("api_key", &"[REDACTED]")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("voice", &self.voice)
            .field("instructions", &self.instructions)
            .finish()
    }
}

impl VoiceConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_values(
            std::env::var("ABBEY_VOICE_GUILD_ID").ok(),
            std::env::var("ABBEY_VOICE_CHANNEL_ID").ok(),
            std::env::var("OPENAI_API_KEY").ok(),
            std::env::var("ABBEY_VOICE_REALTIME_ENDPOINT").ok(),
            std::env::var("ABBEY_VOICE_REALTIME_MODEL").ok(),
            std::env::var("ABBEY_VOICE_NAME").ok(),
            std::env::var("ABBEY_VOICE_INSTRUCTIONS").ok(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_values(
        guild: Option<String>,
        channel: Option<String>,
        api_key: Option<String>,
        endpoint: Option<String>,
        model: Option<String>,
        voice: Option<String>,
        instructions: Option<String>,
    ) -> Result<Option<Self>, String> {
        let guild = nonblank(guild);
        let channel = nonblank(channel);
        let api_key = nonblank(api_key);
        // A general OpenAI key may exist for unrelated features. Only the two
        // voice-specific destination variables opt this subsystem in.
        if guild.is_none() && channel.is_none() {
            return Ok(None);
        }
        let guild_id = snowflake(guild, "ABBEY_VOICE_GUILD_ID")?;
        let channel_id = snowflake(channel, "ABBEY_VOICE_CHANNEL_ID")?;
        let api_key = api_key.ok_or_else(|| {
            "OPENAI_API_KEY is required when Abbey live voice is configured".to_string()
        })?;
        let endpoint = nonblank(endpoint).unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        validate_endpoint(&endpoint)?;
        let model = safe_name(nonblank(model), DEFAULT_MODEL, "ABBEY_VOICE_REALTIME_MODEL")?;
        let voice = safe_name(nonblank(voice), DEFAULT_VOICE, "ABBEY_VOICE_NAME")?;
        let instructions = nonblank(instructions).unwrap_or_else(default_instructions);
        if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
            return Err(format!(
                "ABBEY_VOICE_INSTRUCTIONS must be at most {MAX_INSTRUCTIONS_CHARS} characters"
            ));
        }

        Ok(Some(Self {
            guild_id,
            channel_id,
            api_key,
            endpoint,
            model,
            voice,
            instructions,
        }))
    }

    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    pub fn websocket_url(&self) -> String {
        format!("{}?model={}", self.endpoint, self.model)
    }
}

fn nonblank(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn snowflake(value: Option<String>, name: &str) -> Result<u64, String> {
    let raw =
        value.ok_or_else(|| format!("{name} is required when Abbey live voice is configured"))?;
    raw.parse::<u64>()
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| format!("{name} must be a non-zero numeric Discord snowflake"))
}

fn safe_name(value: Option<String>, default: &str, name: &str) -> Result<String, String> {
    let value = value.unwrap_or_else(|| default.to_string());
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        Ok(value)
    } else {
        Err(format!(
            "{name} may contain only ASCII letters, digits, dot, dash, underscore, or colon"
        ))
    }
}

fn validate_endpoint(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw)
        .map_err(|e| format!("ABBEY_VOICE_REALTIME_ENDPOINT is invalid: {e}"))?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "ABBEY_VOICE_REALTIME_ENDPOINT must not contain credentials, a query, or a fragment"
                .into(),
        );
    }
    match url.scheme() {
        "wss" => Ok(()),
        "ws" if matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1")) => Ok(()),
        "ws" => Err(
            "ABBEY_VOICE_REALTIME_ENDPOINT may use ws only on loopback; remote providers require wss"
                .into(),
        ),
        _ => Err("ABBEY_VOICE_REALTIME_ENDPOINT must use wss (or loopback ws)".into()),
    }
}

fn default_instructions() -> String {
    "You are Abbey, a warm, quick-witted engineering collaborator in a live Discord voice channel. Speak naturally, clearly, and concisely. Let people finish, handle interruptions gracefully, never pretend you saw a screen or stream you were not explicitly given, and never claim an external action succeeded without evidence.".to_string()
}

/// Mix synchronized Discord 48 kHz stereo PCM and downsample it to the
/// Realtime API's 24 kHz mono PCM16. Inputs shorter than a full frame simply
/// constrain the output; an empty tick becomes 20 ms of silence for VAD.
pub fn discord_to_realtime(speakers: &[&[i16]]) -> Vec<i16> {
    if speakers.is_empty() {
        return vec![0; 480];
    }
    let frames = speakers.iter().map(|s| s.len() / 2).min().unwrap_or(0);
    let mut output = Vec::with_capacity(frames.div_ceil(2));
    for frame in (0..frames).step_by(2) {
        let mixed = speakers
            .iter()
            .map(|samples| (i64::from(samples[frame * 2]) + i64::from(samples[frame * 2 + 1])) / 2)
            .sum::<i64>()
            / i64::try_from(speakers.len()).unwrap_or(1);
        output.push(mixed.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16);
    }
    output
}

/// Convert Realtime 24 kHz mono PCM16 to the interleaved native-endian f32
/// 48 kHz stereo stream expected by Songbird's `RawAdapter`.
pub fn realtime_to_discord(pcm: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(pcm.len() * 8);
    for pair in pcm.chunks_exact(2) {
        let sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
        // Duplicate once in time (24 -> 48 kHz) and once across channels.
        for _ in 0..4 {
            output.extend_from_slice(&sample.to_ne_bytes());
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_is_off_when_no_required_value_is_present() {
        assert!(
            VoiceConfig::from_values(None, None, None, None, None, None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            VoiceConfig::from_values(
                None,
                None,
                Some("other-use-key".into()),
                None,
                None,
                None,
                None
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn partial_voice_config_fails_closed() {
        let error = VoiceConfig::from_values(
            Some("123".into()),
            None,
            Some("secret".into()),
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("ABBEY_VOICE_CHANNEL_ID"));
    }

    #[test]
    fn remote_plaintext_websocket_is_rejected() {
        let error = VoiceConfig::from_values(
            Some("123".into()),
            Some("456".into()),
            Some("secret".into()),
            Some("ws://example.com/realtime".into()),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("remote providers require wss"));
    }

    #[test]
    fn endpoint_query_is_rejected_before_model_is_appended() {
        let error = VoiceConfig::from_values(
            Some("123".into()),
            Some("456".into()),
            Some("secret".into()),
            Some("wss://example.com/realtime?other=value".into()),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("query"));
    }

    #[test]
    fn debug_never_prints_the_api_key() {
        let config = VoiceConfig::from_values(
            Some("123".into()),
            Some("456".into()),
            Some("super-secret".into()),
            None,
            None,
            None,
            None,
        )
        .unwrap()
        .unwrap();
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("REDACTED"));
    }

    #[test]
    fn discord_audio_is_mixed_and_downsampled() {
        let left = [1000, 3000, 5000, 7000, 9000, 11000, 13000, 15000];
        let right = [-1000, 1000, 3000, 5000, 7000, 9000, 11000, 13000];
        assert_eq!(discord_to_realtime(&[&left, &right]), vec![1000, 9000]);
        assert_eq!(discord_to_realtime(&[]).len(), 480);
    }

    #[test]
    fn realtime_audio_is_upsampled_to_stereo_f32() {
        let output = realtime_to_discord(&32767_i16.to_le_bytes());
        assert_eq!(output.len(), 16);
        for bytes in output.chunks_exact(4) {
            let value = f32::from_ne_bytes(bytes.try_into().unwrap());
            assert!((value - (32767.0 / 32768.0)).abs() < f32::EPSILON);
        }
    }
}
