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
const OPENAI_CONTROL_SAFETY_SUFFIX: &str = "Discord voice state is controlled only by Abbey's deterministic command shell. Spoken requests cannot start, resume, stop, or change the call in this direct backup mode. Never claim that listening, capture, consent, mute, deafen, or connection state changed because of conversation. For an authoritative stop, tell a participant to use /voice leave or mention Abbey and write 'stop listening' in the configured voice chat.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceMode {
    Disabled,
    Local,
    OpenAi,
}

impl VoiceMode {
    /// Parse an operator-supplied mode name. Public so `/voice mode` accepts
    /// exactly the aliases `ABBEY_VOICE_MODE` accepts — a second, narrower
    /// parser in the command shell silently rejected `off` and `offline`.
    pub fn parse(value: Option<String>) -> Result<Self, String> {
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
            .field("instructions_chars", &self.instructions.chars().count())
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

impl VoiceBackendConfig {
    /// The mode this backend *is*. A snapshot carries its own mode so a join
    /// never has to ask shared state what it is doing halfway through.
    #[must_use]
    pub const fn mode(&self) -> VoiceMode {
        match self {
            Self::Disabled => VoiceMode::Disabled,
            Self::Local(_) => VoiceMode::Local,
            Self::OpenAi(_) => VoiceMode::OpenAi,
        }
    }
}

/// One explicitly allowed Discord destination and one explicitly selected
/// speech backend. Partial destinations and incomplete selected backends are
/// startup errors.
///
/// A backend whose environment happens to be complete but which was *not*
/// selected is retained here **inert**, reachable only through
/// [`VoiceConfig::available_local`] / [`VoiceConfig::available_openai`] so
/// `/voice mode` can validate a switch against something real. Retention is not
/// selection: [`VoiceConfig::mode`], [`VoiceConfig::local`] and
/// [`VoiceConfig::openai`] keep meaning *selected at startup*, so a present key
/// still never chooses cloud audio.
#[derive(Clone, Debug)]
pub struct VoiceConfig {
    pub guild_id: u64,
    pub channel_id: u64,
    pub music_command_channel_id: Option<u64>,
    pub backend: VoiceBackendConfig,
    pub wake_word_required: bool,
    pub wake_words: Vec<String>,
    /// Complete-but-unselected local backend, if its environment allowed one.
    retained_local: Option<OfflineVoiceConfig>,
    /// Complete-but-unselected OpenAI backend, if its environment allowed one.
    retained_openai: Option<OpenAiVoiceConfig>,
}

#[derive(Default)]
struct VoiceEnv {
    guild: Option<String>,
    channel: Option<String>,
    music_command_channel: Option<String>,
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
    wake_words: Option<String>,
}

impl VoiceConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_values(VoiceEnv {
            guild: std::env::var("ABBEY_VOICE_GUILD_ID").ok(),
            channel: std::env::var("ABBEY_VOICE_CHANNEL_ID").ok(),
            music_command_channel: std::env::var("ABBEY_MUSIC_COMMAND_CHANNEL_ID").ok(),
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
            wake_words: std::env::var("ABBEY_VOICE_WAKE_WORDS").ok(),
        })
    }

    fn from_values(values: VoiceEnv) -> Result<Option<Self>, String> {
        let music_command_channel_id = nonblank(values.music_command_channel.clone())
            .map(|value| snowflake(Some(value), "ABBEY_MUSIC_COMMAND_CHANNEL_ID"))
            .transpose()?;
        let guild = nonblank(values.guild.clone());
        let channel = nonblank(values.channel.clone());
        if guild.is_none() && channel.is_none() {
            if music_command_channel_id.is_some() {
                return Err("ABBEY_MUSIC_COMMAND_CHANNEL_ID requires ABBEY_VOICE_GUILD_ID and ABBEY_VOICE_CHANNEL_ID".into());
            }
            if nonblank(values.mode.clone()).is_some_and(|mode| {
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
        let mode = VoiceMode::parse(values.mode.clone())?;

        // The selected mode still fails closed: an incomplete or unusable
        // backend for `mode` is a startup error, exactly as before.
        let backend = match mode {
            VoiceMode::Disabled => VoiceBackendConfig::Disabled,
            VoiceMode::Local => VoiceBackendConfig::Local(build_local(&values)?),
            VoiceMode::OpenAi => VoiceBackendConfig::OpenAi(build_openai(&values)?),
        };

        // Unselected backends are retained only when their environment is
        // already complete, and any failure is swallowed: an unusable backend
        // you did not ask for must never break startup, and retaining one must
        // never select it.
        let retained_local = match mode {
            VoiceMode::Local => None,
            _ => build_local(&values).ok(),
        };
        let retained_openai = match mode {
            VoiceMode::OpenAi => None,
            _ => build_openai(&values).ok(),
        };

        Ok(Some(Self {
            guild_id,
            channel_id,
            music_command_channel_id,
            backend,
            wake_word_required: parse_bool(
                values.wake_word_required,
                true,
                "ABBEY_VOICE_WAKE_WORD_REQUIRED",
            )?,
            wake_words: parse_wake_words(values.wake_words.clone()),
            retained_local,
            retained_openai,
        }))
    }

    /// A config with one explicitly selected backend and nothing retained, so
    /// `/voice mode` can switch only to `disabled`. This is the shape the
    /// process had before unselected backends were retained, and it is what
    /// callers that build a config directly (rather than from the environment)
    /// should use.
    #[must_use]
    pub fn selected_only(
        guild_id: u64,
        channel_id: u64,
        backend: VoiceBackendConfig,
        wake_word_required: bool,
    ) -> Self {
        Self {
            guild_id,
            channel_id,
            music_command_channel_id: None,
            backend,
            wake_word_required,
            wake_words: DEFAULT_WAKE_WORDS
                .iter()
                .map(|w| (*w).to_string())
                .collect(),
            retained_local: None,
            retained_openai: None,
        }
    }

    /// A local backend usable right now — the selected one, or a retained
    /// complete-but-inert one. `/voice mode` validates against this; nothing
    /// else should, because being *available* is not being *selected*.
    #[must_use]
    pub fn available_local(&self) -> Option<&OfflineVoiceConfig> {
        self.local().or(self.retained_local.as_ref())
    }

    /// An OpenAI backend usable right now — selected or retained-but-inert.
    /// See [`VoiceConfig::available_local`]; a present key still never selects
    /// cloud audio on its own.
    #[must_use]
    pub fn available_openai(&self) -> Option<&OpenAiVoiceConfig> {
        self.openai().or(self.retained_openai.as_ref())
    }

    /// The backend for `mode`, if one is usable. Returns an owned value so a
    /// caller can snapshot it once and stop re-reading shared state mid-join.
    #[must_use]
    pub fn backend_for(&self, mode: VoiceMode) -> Option<VoiceBackendConfig> {
        match mode {
            VoiceMode::Disabled => Some(VoiceBackendConfig::Disabled),
            VoiceMode::Local => self
                .available_local()
                .cloned()
                .map(VoiceBackendConfig::Local),
            VoiceMode::OpenAi => self
                .available_openai()
                .cloned()
                .map(VoiceBackendConfig::OpenAi),
        }
    }

    /// The mode selected at startup. Runtime switching lives on
    /// `VoiceRuntime::effective_mode`; this stays the startup answer.
    #[must_use]
    pub const fn mode(&self) -> VoiceMode {
        match self.backend {
            VoiceBackendConfig::Disabled => VoiceMode::Disabled,
            VoiceBackendConfig::Local(_) => VoiceMode::Local,
            VoiceBackendConfig::OpenAi(_) => VoiceMode::OpenAi,
        }
    }

    #[must_use]
    pub fn default_wake_words() -> Vec<String> {
        DEFAULT_WAKE_WORDS
            .iter()
            .map(|w| (*w).to_string())
            .collect()
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

/// Names-only operator checklist for the env the running process actually loaded.
/// Values are never stored; `Debug` is booleans only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperatorEnvPresence {
    pub discord_token: bool,
    pub abbey_guild_id: bool,
    pub llm_endpoint: bool,
    pub llm_model: bool,
    pub vision_endpoint: bool,
    pub vision_model: bool,
    pub voice_guild_id: bool,
    pub voice_channel_id: bool,
    pub voice_mode: bool,
    pub voice_local_endpoint: bool,
}

impl OperatorEnvPresence {
    pub fn from_env() -> Self {
        Self::from_get(|name| std::env::var(name).ok())
    }

    pub fn from_get(get: impl Fn(&str) -> Option<String>) -> Self {
        let present = |name: &str| {
            get(name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .is_some()
        };
        Self {
            discord_token: present("DISCORD_TOKEN"),
            abbey_guild_id: present("ABBEY_GUILD_ID"),
            llm_endpoint: present("ABBEY_BOT_LLM_ENDPOINT"),
            llm_model: present("ABBEY_BOT_LLM_MODEL"),
            vision_endpoint: present("ABBEY_VISION_ENDPOINT"),
            vision_model: present("ABBEY_VISION_MODEL"),
            voice_guild_id: present("ABBEY_VOICE_GUILD_ID"),
            voice_channel_id: present("ABBEY_VOICE_CHANNEL_ID"),
            voice_mode: present("ABBEY_VOICE_MODE"),
            voice_local_endpoint: present("ABBEY_VOICE_LOCAL_ENDPOINT"),
        }
    }

    /// Local conversational voice cannot work without a loopback LLM. Returning
    /// `Some` means `/voice join` will fail closed for that reason.
    #[must_use]
    pub fn local_voice_llm_gap(
        self,
        voice_is_local: bool,
        has_loopback_llm: bool,
    ) -> Option<&'static str> {
        if voice_is_local && !has_loopback_llm {
            Some(
                "local voice is configured but ABBEY_BOT_LLM_ENDPOINT is missing or not loopback — /voice join will fail closed before sidecar prepare",
            )
        } else {
            None
        }
    }
}

fn validate_local_voice_platform() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(
            "ABBEY_VOICE_MODE=local is supported only on macOS; use disabled or explicitly configure openai on this platform"
                .into(),
        )
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

/// Parse a comma-separated wake-word list.
///
/// Words are lowercased and must be ASCII-alphabetic and at most 32 bytes, so a
/// configured word can always be produced by `contains_wake_name`'s tokenizer.
/// A blank, absent, or fully invalid value falls back to the default list
/// rather than leaving Abbey unaddressable.
/// Wake names Abbey answers to when `ABBEY_VOICE_WAKE_WORDS` is unset.
pub const DEFAULT_WAKE_WORDS: [&str; 4] = ["abbey", "abby", "aviva", "abi"];

/// Whether `text` addresses Abbey by one of `wake_words`.
///
/// Matching is token-bounded on ASCII-alphabetic runs, so "an abbeylike
/// building" is not an address. Callers own the word list; there is no implicit
/// default here.
#[must_use]
pub fn contains_wake_name(text: &str, wake_words: &[String]) -> bool {
    text.split(|character: char| !character.is_ascii_alphabetic())
        .any(|word| {
            let lower = word.to_ascii_lowercase();
            wake_words.iter().any(|candidate| candidate == &lower)
        })
}

fn parse_wake_words(value: Option<String>) -> Vec<String> {
    let default = VoiceConfig::default_wake_words();
    let Some(raw) = nonblank(value) else {
        return default;
    };
    let words: Vec<String> = raw
        .split(',')
        .map(|w| w.trim().to_ascii_lowercase())
        .filter(|w| !w.is_empty() && w.len() <= 32 && w.chars().all(|c| c.is_ascii_alphabetic()))
        .collect();
    if words.is_empty() { default } else { words }
}

/// Build a local backend from env values. Shared by the selected path (where a
/// failure is a startup error) and the retained path (where it is swallowed),
/// so a retained backend is validated exactly as strictly as a selected one.
fn build_local(values: &VoiceEnv) -> Result<OfflineVoiceConfig, String> {
    validate_local_voice_platform()?;
    OfflineVoiceConfig::from_values(
        values.local_endpoint.clone(),
        values.local_stt_model.clone(),
        values.local_tts_model.clone(),
        values.local_tts_voice.clone(),
        values.local_language.clone(),
    )
}

/// Build an OpenAI backend from env values. See [`build_local`] for why this is
/// shared rather than inlined per path.
fn build_openai(values: &VoiceEnv) -> Result<OpenAiVoiceConfig, String> {
    let api_key = nonblank(values.openai_key.clone())
        .ok_or_else(|| "ABBEY_VOICE_MODE=openai requires OPENAI_API_KEY".to_string())?;
    let endpoint = nonblank(values.openai_endpoint.clone())
        .unwrap_or_else(|| DEFAULT_OPENAI_ENDPOINT.to_string());
    validate_openai_endpoint(&endpoint)?;
    let model = safe_name(
        nonblank(values.openai_model.clone()),
        DEFAULT_OPENAI_MODEL,
        "ABBEY_VOICE_REALTIME_MODEL",
    )?;
    let voice = safe_name(
        nonblank(values.openai_voice.clone()),
        DEFAULT_OPENAI_VOICE,
        "ABBEY_VOICE_NAME",
    )?;
    let base_instructions =
        nonblank(values.instructions.clone()).unwrap_or_else(default_instructions);
    let instructions = format!("{base_instructions}\n\n{OPENAI_CONTROL_SAFETY_SUFFIX}");
    if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err(format!(
            "ABBEY_VOICE_INSTRUCTIONS must be at most {MAX_INSTRUCTIONS_CHARS} characters"
        ));
    }
    Ok(OpenAiVoiceConfig {
        api_key,
        endpoint,
        model,
        voice,
        instructions,
    })
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
    "You are Abbey — a warm, sharp friend in a live Discord voice channel, not a help desk. Lead with the point, keep it short, use contractions, and skip filler. Let people finish; handle interruptions gracefully. Never pretend you saw a screen or stream you were not given, never claim an external action succeeded without evidence, and say when you\u{2019}re not sure.".to_string()
}

#[cfg(test)]
mod tests;
