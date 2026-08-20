//! Provider-neutral policy for Abbey's Discord voice surface.
//!
//! The mode is explicit. Local inference is the default once a destination is
//! configured, cloud audio is an opt-in backup, and neither mode is inferred
//! from whether a provider key happens to exist. Discord still transports the
//! call. `local` means the bot sends speech recognition, reasoning, and
//! synthesis only to loopback services on this Mac; whether a separately
//! configured loopback service proxies upstream remains operator-controlled.

use std::fmt;

use crate::offline_voice::OfflineVoiceConfig;

const DEFAULT_OPENAI_ENDPOINT: &str = "wss://api.openai.com/v1/realtime";
const DEFAULT_OPENAI_MODEL: &str = "gpt-realtime-2.1";
const DEFAULT_OPENAI_VOICE: &str = "marin";
const MAX_INSTRUCTIONS_CHARS: usize = 8_000;
const OPENAI_CONTROL_SAFETY_SUFFIX: &str = "Discord voice state is controlled only by Abbey's deterministic command shell. Spoken requests cannot start, resume, stop, or change the call in this direct backup mode. Never claim that listening, capture, consent, mute, deafen, or connection state changed because of conversation. For an authoritative stop, tell a participant to use /voice leave or write 'stop listening' in the configured voice chat.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceMode {
    Disabled,
    Local,
    OpenAi,
}

impl VoiceMode {
    fn parse(value: Option<String>) -> Result<Self, String> {
        match nonblank(value)
            .unwrap_or_else(|| "local".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "disabled" | "off" => Ok(Self::Disabled),
            "local" | "offline" => Ok(Self::Local),
            "openai" => Ok(Self::OpenAi),
            other => Err(format!(
                "ABBEY_VOICE_MODE must be disabled, local, or openai; got {other:?}"
            )),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled / no audio",
            Self::Local => "local AI inference",
            Self::OpenAi => "direct OpenAI Realtime backup (buffered output)",
        }
    }
}

#[derive(Clone)]
pub struct OpenAiVoiceConfig {
    api_key: String,
    endpoint: String,
    pub model: String,
    pub voice: String,
    pub instructions: String,
}

impl fmt::Debug for OpenAiVoiceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiVoiceConfig")
            .field("api_key", &"[REDACTED]")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("voice", &self.voice)
            .field("instructions", &self.instructions)
            .finish()
    }
}

impl OpenAiVoiceConfig {
    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    pub fn websocket_url(&self) -> String {
        let mut url = reqwest::Url::parse(&self.endpoint)
            .expect("OpenAI endpoint is validated before config construction");
        url.query_pairs_mut().append_pair("model", &self.model);
        url.to_string()
    }
}

#[derive(Clone, Debug)]
pub enum VoiceBackendConfig {
    Disabled,
    Local(OfflineVoiceConfig),
    OpenAi(OpenAiVoiceConfig),
}

/// One explicitly allowed Discord destination and one explicitly selected
/// speech backend. Partial destinations and incomplete selected backends are
/// startup errors.
#[derive(Clone, Debug)]
pub struct VoiceConfig {
    pub guild_id: u64,
    pub channel_id: u64,
    pub backend: VoiceBackendConfig,
    pub wake_word_required: bool,
}

#[derive(Default)]
struct VoiceEnv {
    guild: Option<String>,
    channel: Option<String>,
    mode: Option<String>,
    openai_key: Option<String>,
    openai_endpoint: Option<String>,
    openai_model: Option<String>,
    openai_voice: Option<String>,
    instructions: Option<String>,
    local_endpoint: Option<String>,
    local_stt_model: Option<String>,
    local_tts_model: Option<String>,
    local_tts_voice: Option<String>,
    local_language: Option<String>,
    wake_word_required: Option<String>,
}

impl VoiceConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_values(VoiceEnv {
            guild: std::env::var("ABBEY_VOICE_GUILD_ID").ok(),
            channel: std::env::var("ABBEY_VOICE_CHANNEL_ID").ok(),
            mode: std::env::var("ABBEY_VOICE_MODE").ok(),
            openai_key: std::env::var("OPENAI_API_KEY").ok(),
            openai_endpoint: std::env::var("ABBEY_VOICE_REALTIME_ENDPOINT").ok(),
            openai_model: std::env::var("ABBEY_VOICE_REALTIME_MODEL").ok(),
            openai_voice: std::env::var("ABBEY_VOICE_NAME").ok(),
            instructions: std::env::var("ABBEY_VOICE_INSTRUCTIONS").ok(),
            local_endpoint: std::env::var("ABBEY_VOICE_LOCAL_ENDPOINT").ok(),
            local_stt_model: std::env::var("ABBEY_VOICE_LOCAL_STT_MODEL").ok(),
            local_tts_model: std::env::var("ABBEY_VOICE_LOCAL_TTS_MODEL").ok(),
            local_tts_voice: std::env::var("ABBEY_VOICE_LOCAL_TTS_VOICE").ok(),
            local_language: std::env::var("ABBEY_VOICE_LOCAL_LANGUAGE").ok(),
            wake_word_required: std::env::var("ABBEY_VOICE_WAKE_WORD_REQUIRED").ok(),
        })
    }

    fn from_values(values: VoiceEnv) -> Result<Option<Self>, String> {
        let guild = nonblank(values.guild);
        let channel = nonblank(values.channel);
        if guild.is_none() && channel.is_none() {
            if nonblank(values.mode).is_some_and(|mode| {
                !matches!(mode.to_ascii_lowercase().as_str(), "off" | "disabled")
            }) {
                return Err(
                    "ABBEY_VOICE_MODE requires ABBEY_VOICE_GUILD_ID and ABBEY_VOICE_CHANNEL_ID"
                        .into(),
                );
            }
            return Ok(None);
        }
        let guild_id = snowflake(guild, "ABBEY_VOICE_GUILD_ID")?;
        let channel_id = snowflake(channel, "ABBEY_VOICE_CHANNEL_ID")?;
        let mode = VoiceMode::parse(values.mode)?;
        let backend = match mode {
            VoiceMode::Disabled => VoiceBackendConfig::Disabled,
            VoiceMode::Local => VoiceBackendConfig::Local(OfflineVoiceConfig::from_values(
                values.local_endpoint,
                values.local_stt_model,
                values.local_tts_model,
                values.local_tts_voice,
                values.local_language,
            )?),
            VoiceMode::OpenAi => {
                let api_key = nonblank(values.openai_key)
                    .ok_or_else(|| "ABBEY_VOICE_MODE=openai requires OPENAI_API_KEY".to_string())?;
                let endpoint = nonblank(values.openai_endpoint)
                    .unwrap_or_else(|| DEFAULT_OPENAI_ENDPOINT.to_string());
                validate_openai_endpoint(&endpoint)?;
                let model = safe_name(
                    nonblank(values.openai_model),
                    DEFAULT_OPENAI_MODEL,
                    "ABBEY_VOICE_REALTIME_MODEL",
                )?;
                let voice = safe_name(
                    nonblank(values.openai_voice),
                    DEFAULT_OPENAI_VOICE,
                    "ABBEY_VOICE_NAME",
                )?;
                let base_instructions =
                    nonblank(values.instructions).unwrap_or_else(default_instructions);
                let instructions = format!("{base_instructions}\n\n{OPENAI_CONTROL_SAFETY_SUFFIX}");
                if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
                    return Err(format!(
                        "ABBEY_VOICE_INSTRUCTIONS must be at most {MAX_INSTRUCTIONS_CHARS} characters"
                    ));
                }
                VoiceBackendConfig::OpenAi(OpenAiVoiceConfig {
                    api_key,
                    endpoint,
                    model,
                    voice,
                    instructions,
                })
            }
        };
        Ok(Some(Self {
            guild_id,
            channel_id,
            backend,
            wake_word_required: parse_bool(
                values.wake_word_required,
                true,
                "ABBEY_VOICE_WAKE_WORD_REQUIRED",
            )?,
        }))
    }

    #[must_use]
    pub const fn mode(&self) -> VoiceMode {
        match self.backend {
            VoiceBackendConfig::Disabled => VoiceMode::Disabled,
            VoiceBackendConfig::Local(_) => VoiceMode::Local,
            VoiceBackendConfig::OpenAi(_) => VoiceMode::OpenAi,
        }
    }

    #[must_use]
    pub fn local(&self) -> Option<&OfflineVoiceConfig> {
        match &self.backend {
            VoiceBackendConfig::Local(config) => Some(config),
            _ => None,
        }
    }

    #[must_use]
    pub fn openai(&self) -> Option<&OpenAiVoiceConfig> {
        match &self.backend {
            VoiceBackendConfig::OpenAi(config) => Some(config),
            _ => None,
        }
    }

    #[must_use]
    pub fn model_label(&self) -> &str {
        match &self.backend {
            VoiceBackendConfig::Disabled => "none",
            VoiceBackendConfig::Local(config) => &config.tts_model,
            VoiceBackendConfig::OpenAi(config) => &config.model,
        }
    }

    #[must_use]
    pub fn voice_label(&self) -> &str {
        match &self.backend {
            VoiceBackendConfig::Disabled => "none",
            VoiceBackendConfig::Local(config) => &config.voice,
            VoiceBackendConfig::OpenAi(config) => &config.voice,
        }
    }
}

fn nonblank(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn snowflake(value: Option<String>, name: &str) -> Result<u64, String> {
    let raw = value.ok_or_else(|| format!("{name} is required when Abbey voice is configured"))?;
    raw.parse::<u64>()
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| format!("{name} must be a non-zero numeric Discord snowflake"))
}

fn parse_bool(value: Option<String>, default: bool, name: &str) -> Result<bool, String> {
    match nonblank(value).as_deref() {
        None => Ok(default),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some("0" | "false" | "no" | "off") => Ok(false),
        Some(other) => Err(format!(
            "{name} must be one of 1/0, true/false, yes/no, or on/off; got {other:?}"
        )),
    }
}

fn safe_name(value: Option<String>, default: &str, name: &str) -> Result<String, String> {
    let value = value.unwrap_or_else(|| default.to_string());
    if !value.is_empty()
        && value.len() <= 200
        && value
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

fn validate_openai_endpoint(raw: &str) -> Result<(), String> {
    validate_openai_endpoint_for_build(raw, cfg!(test))
}

fn validate_openai_endpoint_for_build(
    raw: &str,
    allow_loopback_test_double: bool,
) -> Result<(), String> {
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
    let loopback_ws = url.scheme() == "ws"
        && matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        );
    match url.scheme() {
        "ws" if loopback_ws && allow_loopback_test_double => Ok(()),
        "ws" if loopback_ws => Err(
            "loopback ws Realtime endpoints are available only to the test build".into(),
        ),
        "ws" => Err(
            "ABBEY_VOICE_REALTIME_ENDPOINT may use ws only on loopback"
                .into(),
        ),
        "wss" if raw == DEFAULT_OPENAI_ENDPOINT => Ok(()),
        "wss" => Err(
            "ABBEY_VOICE_REALTIME_ENDPOINT may send OPENAI_API_KEY only to wss://api.openai.com/v1/realtime"
                .into(),
        ),
        _ => Err(
            "ABBEY_VOICE_REALTIME_ENDPOINT must be the official OpenAI wss endpoint (or loopback ws for a test double)"
                .into(),
        ),
    }
}

fn default_instructions() -> String {
    "You are Abbey, a warm, quick-witted engineering collaborator in a live Discord voice channel. Speak naturally, clearly, and concisely. Let people finish, handle interruptions gracefully, never pretend you saw a screen or stream you were not explicitly given, and never claim an external action succeeded without evidence.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination() -> VoiceEnv {
        VoiceEnv {
            guild: Some("123".into()),
            channel: Some("456".into()),
            ..VoiceEnv::default()
        }
    }

    #[test]
    fn voice_is_off_without_a_destination() {
        assert!(
            VoiceConfig::from_values(VoiceEnv::default())
                .unwrap()
                .is_none()
        );
        let values = VoiceEnv {
            openai_key: Some("unrelated-key".into()),
            ..VoiceEnv::default()
        };
        assert!(VoiceConfig::from_values(values).unwrap().is_none());
    }

    #[test]
    fn partial_destination_fails_closed() {
        let values = VoiceEnv {
            guild: Some("123".into()),
            ..VoiceEnv::default()
        };
        assert!(
            VoiceConfig::from_values(values)
                .unwrap_err()
                .contains("ABBEY_VOICE_CHANNEL_ID")
        );
    }

    #[test]
    fn destination_defaults_to_local_even_when_a_cloud_key_exists() {
        let mut values = destination();
        values.openai_key = Some("must-not-select-cloud".into());
        let config = VoiceConfig::from_values(values).unwrap().unwrap();
        assert_eq!(config.mode(), VoiceMode::Local);
        assert!(config.openai().is_none());
    }

    #[test]
    fn openai_is_explicit_and_requires_a_key() {
        let mut values = destination();
        values.mode = Some("openai".into());
        assert!(
            VoiceConfig::from_values(values)
                .unwrap_err()
                .contains("requires OPENAI_API_KEY")
        );

        let mut values = destination();
        values.mode = Some("openai".into());
        values.openai_key = Some("super-secret".into());
        let config = VoiceConfig::from_values(values).unwrap().unwrap();
        let rendered = format!("{config:?}");
        assert_eq!(config.mode(), VoiceMode::OpenAi);
        let instructions = &config.openai().expect("OpenAI config").instructions;
        let websocket_url = reqwest::Url::parse(config.openai().unwrap().websocket_url().as_str())
            .expect("canonical websocket URL");
        assert!(instructions.contains("Spoken requests cannot start, resume, stop"));
        assert!(instructions.contains("/voice leave"));
        assert_eq!(websocket_url.path(), "/v1/realtime");
        let query: Vec<(String, String)> = websocket_url
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(query, [("model".into(), DEFAULT_OPENAI_MODEL.into())]);
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("REDACTED"));
    }

    #[test]
    fn disabled_mode_needs_no_provider() {
        let mut values = destination();
        values.mode = Some("disabled".into());
        let config = VoiceConfig::from_values(values).unwrap().unwrap();
        assert_eq!(config.mode(), VoiceMode::Disabled);
    }

    #[test]
    fn remote_plaintext_openai_websocket_is_rejected() {
        let mut values = destination();
        values.mode = Some("openai".into());
        values.openai_key = Some("secret".into());
        values.openai_endpoint = Some("ws://example.com/realtime".into());
        assert!(
            VoiceConfig::from_values(values)
                .unwrap_err()
                .contains("ws only on loopback")
        );
    }

    #[test]
    fn openai_key_can_reach_only_the_exact_official_remote_endpoint() {
        assert!(validate_openai_endpoint("wss://api.openai.com/v1/realtime").is_ok());
        for endpoint in [
            "wss://api.openai.com/v1/realtime?model=attacker",
            "wss://api.openai.com/v1/realtime#fragment",
        ] {
            let error = validate_openai_endpoint(endpoint).unwrap_err();
            assert!(
                error.contains("query, or a fragment"),
                "{endpoint}: {error}"
            );
        }
        for endpoint in [
            "wss://example.com/v1/realtime",
            "wss://api.openai.com.evil.example/v1/realtime",
            "wss://api.openai.com/realtime",
            "wss://api.openai.com/v1/realtime/",
            "wss://api.openai.com:443/v1/realtime",
            "wss://api.openai.com:8443/v1/realtime",
            "WSS://api.openai.com/v1/realtime",
            "wss://@api.openai.com/v1/realtime",
        ] {
            let error = validate_openai_endpoint(endpoint).unwrap_err();
            assert!(
                error.contains("only to wss://api.openai.com/v1/realtime"),
                "{endpoint}: {error}"
            );
        }
    }

    #[test]
    fn loopback_ws_remains_available_for_realtime_test_doubles() {
        for endpoint in [
            "ws://127.0.0.1:8182/realtime",
            "ws://localhost:8182/v1/realtime",
            "ws://[::1]:8182/test",
        ] {
            assert!(validate_openai_endpoint(endpoint).is_ok(), "{endpoint}");
            assert!(
                validate_openai_endpoint_for_build(endpoint, false)
                    .unwrap_err()
                    .contains("only to the test build"),
                "production path accepted {endpoint}"
            );
        }
    }

    #[test]
    fn mode_without_destination_fails_unless_disabled() {
        let values = VoiceEnv {
            mode: Some("local".into()),
            ..VoiceEnv::default()
        };
        assert!(VoiceConfig::from_values(values).is_err());
        let values = VoiceEnv {
            mode: Some("disabled".into()),
            ..VoiceEnv::default()
        };
        assert!(VoiceConfig::from_values(values).unwrap().is_none());
    }
}
