//! Offline speech primitives and the loopback-only MLX-Audio client.
//!
//! This module deliberately knows nothing about Discord or Songbird. It turns
//! fixed 20 ms PCM frames into bounded utterances, speaks the OpenAI-compatible
//! MLX-Audio HTTP surface, and validates audio before the Discord shell can play
//! it. Raw audio and transcripts are never written to disk here.

use std::collections::VecDeque;
use std::time::Duration;

use serde::Deserialize;

use crate::vad::{EnergyVad, Vad};

pub const INPUT_SAMPLE_RATE: u32 = 24_000;
pub const FRAME_SAMPLES: usize = 480;
const PRE_ROLL_FRAMES: usize = 10;
const START_FRAMES: usize = 2;
const END_SILENCE_FRAMES: usize = 25;
const KEPT_TAIL_FRAMES: usize = 5;
const MIN_SPEECH_FRAMES: usize = 15;
const MAX_UTTERANCE_FRAMES: usize = 1_000;
const MAX_STT_RESPONSE_BYTES: u64 = 64 * 1024;
const MAX_TTS_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TTS_SECONDS: u64 = 45;
const MAX_SPOKEN_CHARS: usize = 1_200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfflineVoiceConfig {
    endpoint: reqwest::Url,
    pub stt_model: String,
    pub tts_model: String,
    pub voice: String,
    pub stt_language: String,
    tts_language_code: &'static str,
}

impl OfflineVoiceConfig {
    pub const DEFAULT_ENDPOINT: &'static str = "http://127.0.0.1:8181";
    pub const DEFAULT_STT_MODEL: &'static str = "mlx-community/whisper-large-v3-turbo-asr-fp16";
    pub const DEFAULT_TTS_MODEL: &'static str = "mlx-community/Kokoro-82M-bf16";
    pub const DEFAULT_VOICE: &'static str = "af_heart";

    pub fn from_values(
        endpoint: Option<String>,
        stt_model: Option<String>,
        tts_model: Option<String>,
        voice: Option<String>,
        language: Option<String>,
    ) -> Result<Self, String> {
        let endpoint = endpoint
            .as_deref()
            .and_then(crate::text::non_blank)
            .map(|s| s.to_string())
            .unwrap_or_else(|| Self::DEFAULT_ENDPOINT.to_string());
        let endpoint = validate_loopback_endpoint(&endpoint)?;
        let voice = voice
            .as_deref()
            .and_then(crate::text::non_blank)
            .map(|s| s.to_string())
            .unwrap_or_else(|| Self::DEFAULT_VOICE.to_string());
        let tts_language_code = kokoro_language_code(&voice)?;
        Ok(Self {
            endpoint,
            stt_model: safe_identifier(stt_model, Self::DEFAULT_STT_MODEL, "STT model")?,
            tts_model: safe_identifier(tts_model, Self::DEFAULT_TTS_MODEL, "TTS model")?,
            voice,
            stt_language: safe_identifier(language, "en", "STT language")?,
            tts_language_code,
        })
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_values(
            std::env::var("ABBEY_VOICE_LOCAL_ENDPOINT").ok(),
            std::env::var("ABBEY_VOICE_LOCAL_STT_MODEL").ok(),
            std::env::var("ABBEY_VOICE_LOCAL_TTS_MODEL").ok(),
            std::env::var("ABBEY_VOICE_LOCAL_TTS_VOICE").ok(),
            std::env::var("ABBEY_VOICE_LOCAL_LANGUAGE").ok(),
        )
    }

    #[must_use]
    pub fn endpoint_display(&self) -> &str {
        self.endpoint.as_str()
    }

    fn url(&self, path: &str) -> Result<reqwest::Url, String> {
        self.endpoint
            .join(path)
            .map_err(|e| format!("building the local speech URL failed: {e}"))
    }
}

fn safe_identifier(value: Option<String>, default: &str, label: &str) -> Result<String, String> {
    let value = value
        .as_deref()
        .and_then(crate::text::non_blank)
        .map(|s| s.to_string())
        .unwrap_or_else(|| default.to_string());
    if value.len() <= 200
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        Ok(value)
    } else {
        Err(format!(
            "{label} may contain only ASCII letters, digits, slash, dot, dash, underscore, or colon"
        ))
    }
}

/// MLX-Audio's Kokoro pipeline keys language selection from the first
/// character of its `<language><gender>_<name>` voice-pack convention. Keep
/// this independent from Whisper's STT language: a British or Japanese voice
/// must select its matching phonemizer even when recognition remains English.
fn kokoro_language_code(voice: &str) -> Result<&'static str, String> {
    let bytes = voice.as_bytes();
    let valid_shape = (4..=200).contains(&bytes.len())
        && matches!(bytes[1], b'f' | b'm')
        && bytes[2] == b'_'
        && bytes[3..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'));
    let code = valid_shape.then(|| match bytes[0] {
        b'a' => Some("a"), // American English
        b'b' => Some("b"), // British English
        b'e' => Some("e"), // Spanish
        b'f' => Some("f"), // French
        b'h' => Some("h"), // Hindi
        b'i' => Some("i"), // Italian
        b'p' => Some("p"), // Brazilian Portuguese
        b'j' => Some("j"), // Japanese
        b'z' => Some("z"), // Mandarin Chinese
        _ => None,
    });
    code.flatten().ok_or_else(|| {
        "TTS voice must use a supported Kokoro voice prefix (af_/am_, bf_/bm_, ef_/em_, ff_/fm_, hf_/hm_, if_/im_, pf_/pm_, jf_/jm_, or zf_/zm_)"
            .to_string()
    })
}

fn validate_loopback_endpoint(raw: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(raw)
        .map_err(|e| format!("ABBEY_VOICE_LOCAL_ENDPOINT is invalid: {e}"))?;
    if url.scheme() != "http"
        || !matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        )
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "ABBEY_VOICE_LOCAL_ENDPOINT must be credential-free loopback HTTP with no query or fragment"
                .into(),
        );
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceFrame {
    pub sequence: u64,
    /// `None` means Songbird has not supplied an SSRC-to-user mapping yet.
    pub speaker_id: Option<u64>,
    /// Exactly 20 ms of 24 kHz mono PCM16.
    pub samples: Vec<i16>,
    /// More than one participant carried meaningful speech energy in this
    /// tick. The dominant speaker is still transcribed, but person-specific
    /// memory and side-effecting tools must be withheld for the turn.
    pub overlap: bool,
}

impl VoiceFrame {
    #[must_use]
    pub fn silence(sequence: u64) -> Self {
        Self {
            sequence,
            speaker_id: None,
            samples: vec![0; FRAME_SAMPLES],
            overlap: false,
        }
    }
}

/// Continuity guard shared by both local segmentation and the direct Realtime
/// actor. The first frame establishes an arbitrary starting point; every later
/// frame must be its exact successor.
#[derive(Debug, Default)]
pub(crate) struct FrameSequence {
    last: Option<u64>,
}

impl FrameSequence {
    pub(crate) fn observe(&mut self, actual: u64) -> Result<(), SequenceGap> {
        let expected = self.last.map(|last| last.saturating_add(1));
        self.last = Some(actual);
        if let Some(expected) = expected
            && expected != actual
        {
            return Err(SequenceGap { expected, actual });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SequenceGap {
    pub(crate) expected: u64,
    pub(crate) actual: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
    pub speaker_id: Option<u64>,
    pub pcm: Vec<i16>,
    /// True when another mapped speaker became dominant during the turn.
    pub overlap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentEvent {
    SpeechStarted { speaker_id: Option<u64> },
    Completed(Utterance),
    AbortedOverrun,
}

#[derive(Debug)]
struct ActiveUtterance {
    speaker_id: Option<u64>,
    pcm: Vec<i16>,
    speech_frames: usize,
    silent_frames: usize,
    total_frames: usize,
    overlap: bool,
}

/// Deterministic, allocation-bounded turn segmentation for Songbird's 20 ms
/// callback frames. MLX/Whisper still performs transcription; this immediate
/// energy gate exists so local playback can stop before a transcript arrives.
/// The local MLX path uses [`EnergyVad`] only; thresholds are unified in
/// [`crate::vad`] so the offline and Realtime pre-filter cannot drift.
#[derive(Debug, Default)]
pub struct Segmenter {
    pre_roll: VecDeque<VoiceFrame>,
    candidate_frames: usize,
    candidate_speaker: Option<u64>,
    active: Option<ActiveUtterance>,
    sequence: FrameSequence,
    vad: EnergyVad,
}

impl Segmenter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_speaking(&self) -> bool {
        // A first voiced frame must defer prepared playback too, before the
        // second frame confirms SpeechStarted and would interrupt that reply.
        self.active.is_some() || self.candidate_frames > 0
    }

    pub fn push(&mut self, mut frame: VoiceFrame) -> Vec<SegmentEvent> {
        if frame.samples.len() != FRAME_SAMPLES {
            frame.samples.resize(FRAME_SAMPLES, 0);
        }
        let sequence_gap = self.sequence.observe(frame.sequence).is_err();
        if sequence_gap {
            let interrupted = self.active.take().is_some() || self.candidate_frames > 0;
            self.pre_roll.clear();
            self.candidate_frames = 0;
            self.candidate_speaker = None;
            if interrupted {
                return vec![SegmentEvent::AbortedOverrun];
            }
        }

        let voiced = self.vad.is_voice(&frame.samples);
        if let Some(active) = &mut self.active {
            active.total_frames += 1;
            active.overlap |= frame.overlap;
            if voiced {
                // Attribution is safe only when every voiced frame maps to the
                // same known Discord user. Unknown and changed mappings fail
                // closed even if there was no simultaneous energy overlap.
                active.overlap |= frame.speaker_id.is_none()
                    || active.speaker_id.is_none()
                    || active.speaker_id != frame.speaker_id;
                active.speech_frames += 1;
                active.silent_frames = 0;
                if active.speaker_id.is_some()
                    && frame.speaker_id.is_some()
                    && active.speaker_id != frame.speaker_id
                {
                    active.overlap = true;
                }
            } else {
                active.silent_frames += 1;
            }
            active.pcm.extend_from_slice(&frame.samples);

            if active.silent_frames >= END_SILENCE_FRAMES
                || active.total_frames >= MAX_UTTERANCE_FRAMES
            {
                let mut completed = self.active.take().expect("active checked above");
                let removable = completed
                    .silent_frames
                    .saturating_sub(KEPT_TAIL_FRAMES)
                    .saturating_mul(FRAME_SAMPLES);
                completed
                    .pcm
                    .truncate(completed.pcm.len().saturating_sub(removable));
                self.pre_roll.clear();
                self.candidate_frames = 0;
                self.candidate_speaker = None;
                if completed.speech_frames >= MIN_SPEECH_FRAMES {
                    return vec![SegmentEvent::Completed(Utterance {
                        speaker_id: completed.speaker_id,
                        pcm: completed.pcm,
                        overlap: completed.overlap,
                    })];
                }
            }
            return Vec::new();
        }

        self.pre_roll.push_back(frame.clone());
        while self.pre_roll.len() > PRE_ROLL_FRAMES {
            self.pre_roll.pop_front();
        }
        if voiced {
            if self.candidate_frames == 0 || self.candidate_speaker == frame.speaker_id {
                self.candidate_frames += 1;
            } else {
                self.candidate_frames = 1;
            }
            self.candidate_speaker = frame.speaker_id;
        } else {
            self.candidate_frames = 0;
            self.candidate_speaker = None;
        }

        if self.candidate_frames < START_FRAMES {
            return Vec::new();
        }
        let mut speaker_id = None;
        let mut attribution_uncertain = false;
        let mut pcm = Vec::with_capacity(MAX_UTTERANCE_FRAMES.min(100) * FRAME_SAMPLES);
        for buffered in &self.pre_roll {
            pcm.extend_from_slice(&buffered.samples);
            if self.vad.is_voice(&buffered.samples) {
                attribution_uncertain |= buffered.overlap || buffered.speaker_id.is_none();
                match (speaker_id, buffered.speaker_id) {
                    (None, Some(id)) => speaker_id = Some(id),
                    (Some(expected), Some(actual)) if expected != actual => {
                        attribution_uncertain = true;
                    }
                    _ => {}
                }
            }
        }
        let total_frames = self.pre_roll.len();
        self.pre_roll.clear();
        self.candidate_frames = 0;
        self.candidate_speaker = None;
        self.active = Some(ActiveUtterance {
            speaker_id,
            pcm,
            speech_frames: START_FRAMES,
            silent_frames: 0,
            total_frames,
            overlap: attribution_uncertain,
        });
        vec![SegmentEvent::SpeechStarted { speaker_id }]
    }
}

#[allow(dead_code)]
pub(crate) fn frame_is_voice(samples: &[i16]) -> bool {
    // Unified threshold source; delegating here keeps the historic call sites
    // in `commands_voice::receive` and tests correct while guaranteeing the
    // two voice paths cannot drift.
    EnergyVad::default().is_voice(samples)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: u8,
    /// Interleaved native-endian f32 samples for Songbird `RawAdapter`.
    pub pcm_f32: Vec<u8>,
}

pub fn encode_mono_pcm16_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>, String> {
    if sample_rate == 0 || samples.len() > sample_rate as usize * 30 {
        return Err("STT audio must be at most 30 seconds with a valid sample rate".into());
    }
    let data_len = samples
        .len()
        .checked_mul(2)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| "STT WAV is too large".to_string())?;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(wav)
}

pub fn decode_pcm16_wav(wav: &[u8]) -> Result<DecodedAudio, String> {
    if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("local TTS returned something other than a RIFF/WAVE file".into());
    }
    let declared_size = u32::from_le_bytes(wav[4..8].try_into().unwrap_or([0; 4])) as usize;
    if declared_size.checked_add(8) != Some(wav.len()) {
        return Err("local TTS WAV has an inconsistent RIFF length".into());
    }
    let mut cursor = 12_usize;
    let mut format = None;
    let mut data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= wav.len()) {
        let id = &wav[cursor..cursor + 4];
        let len =
            u32::from_le_bytes(wav[cursor + 4..cursor + 8].try_into().unwrap_or([0; 4])) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "local TTS WAV chunk length overflowed".to_string())?;
        if end > wav.len() {
            return Err("local TTS WAV contains a truncated chunk".into());
        }
        match id {
            b"fmt " if len >= 16 => {
                let audio_format = u16::from_le_bytes([wav[start], wav[start + 1]]);
                let channels = u16::from_le_bytes([wav[start + 2], wav[start + 3]]);
                let sample_rate =
                    u32::from_le_bytes(wav[start + 4..start + 8].try_into().unwrap_or([0; 4]));
                let block_align = u16::from_le_bytes([wav[start + 12], wav[start + 13]]);
                let bits = u16::from_le_bytes([wav[start + 14], wav[start + 15]]);
                format = Some((audio_format, channels, sample_rate, block_align, bits));
            }
            b"data" => data = Some((start, end)),
            _ => {}
        }
        cursor = end + (len & 1);
    }
    let (audio_format, channels, sample_rate, block_align, bits) =
        format.ok_or_else(|| "local TTS WAV has no format chunk".to_string())?;
    if audio_format != 1
        || bits != 16
        || !(channels == 1 || channels == 2)
        || block_align != channels * 2
    {
        return Err("local TTS WAV must contain mono or stereo PCM16".into());
    }
    if !(8_000..=96_000).contains(&sample_rate) {
        return Err("local TTS WAV sample rate is outside 8-96 kHz".into());
    }
    let (start, end) = data.ok_or_else(|| "local TTS WAV has no audio data".to_string())?;
    let frame_bytes = usize::from(channels) * 2;
    if (end - start) % frame_bytes != 0 {
        return Err("local TTS WAV ends in a partial PCM frame".into());
    }
    let frames = (end - start) / frame_bytes;
    if frames == 0 {
        return Err("local TTS WAV contains no audio frames".into());
    }
    if frames as u64 > u64::from(sample_rate) * MAX_TTS_SECONDS {
        return Err(format!("local TTS audio exceeds {MAX_TTS_SECONDS} seconds"));
    }
    let mut pcm_f32 = Vec::with_capacity(frames * usize::from(channels) * 4);
    let (pairs, remainder) = wav[start..end].as_chunks::<2>();
    debug_assert!(
        remainder.is_empty(),
        "PCM frame validation rejects remainders"
    );
    for pair in pairs {
        let sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
        pcm_f32.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(DecodedAudio {
        sample_rate,
        channels: u8::try_from(channels).unwrap_or(1),
        pcm_f32,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSpeechFailureKind {
    NotListening,
    TimedOut,
    Other,
}

fn looks_like_connect_failure(lower: &str) -> bool {
    lower.contains("connection refused")
        || lower.contains("error trying to connect")
        || lower.contains("tcp connect error")
        || lower.contains("connect error")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
}

fn looks_like_timeout(lower: &str) -> bool {
    lower.contains("timed out") || lower.contains("timeout")
}

/// Classify a local-speech HTTP/transport failure without leaking request bodies.
#[must_use]
pub fn classify_local_speech_failure(raw: &str) -> LocalSpeechFailureKind {
    let lower = raw.to_ascii_lowercase();
    if looks_like_connect_failure(&lower) {
        LocalSpeechFailureKind::NotListening
    } else if looks_like_timeout(&lower) {
        LocalSpeechFailureKind::TimedOut
    } else {
        LocalSpeechFailureKind::Other
    }
}

/// Connection-refused / reset: the sidecar is down. Timeouts are *not* included
/// because a live Whisper/Kokoro load can exceed one turn without meaning :8181
/// is gone.
#[must_use]
pub fn sidecar_is_unavailable(raw: &str) -> bool {
    classify_local_speech_failure(raw) == LocalSpeechFailureKind::NotListening
}

#[must_use]
pub fn local_speech_operator_message(
    kind: LocalSpeechFailureKind,
    endpoint: &str,
    stage: &str,
    raw: &str,
) -> String {
    match kind {
        LocalSpeechFailureKind::NotListening => format!(
            "MLX-Audio is not listening at {endpoint} ({stage}). Run deploy/install-mlx-audio-launchd.sh (setuptools 83; webrtcvad via importlib.metadata; readiness GET /v1/models). Log: ~/Library/Logs/abbey-bot/mlx-audio.log."
        ),
        LocalSpeechFailureKind::TimedOut => format!(
            "MLX-Audio at {endpoint} timed out during {stage} (starting or loading Whisper/Kokoro). Retry /voice status; do not wait on this command."
        ),
        LocalSpeechFailureKind::Other => format!("{stage} at {endpoint} failed: {raw}"),
    }
}

fn map_speech_http_error(endpoint: &str, stage: &str, error: reqwest::Error) -> String {
    let raw = error.to_string();
    let kind = if error.is_connect() {
        LocalSpeechFailureKind::NotListening
    } else if error.is_timeout() {
        LocalSpeechFailureKind::TimedOut
    } else {
        classify_local_speech_failure(&raw)
    };
    local_speech_operator_message(kind, endpoint, stage, &raw)
}

#[derive(Clone)]
pub struct MlxAudioClient {
    config: OfflineVoiceConfig,
    http: reqwest::Client,
}

impl MlxAudioClient {
    pub fn new(config: OfflineVoiceConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            // Local speech audio must never inherit HTTP(S)_PROXY. The URL is
            // loopback-validated above; bypassing system proxies keeps that
            // privacy boundary true even on a managed workstation.
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("building the local speech client failed: {e}"))?;
        Ok(Self { config, http })
    }

    fn map_error(&self, stage: &'static str, error: reqwest::Error) -> String {
        map_speech_http_error(self.config.endpoint_display(), stage, error)
    }

    pub async fn health(&self) -> Result<(), String> {
        let response = self
            .http
            .get(self.config.url("v1/models")?)
            .send()
            .await
            .map_err(|e| self.map_error("health", e))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "local speech service health check returned HTTP {}",
                response.status()
            ))
        }
    }

    /// Load both selected models before Discord decoding is enabled. In
    /// production Hugging Face is offline-only, so this is a deterministic
    /// cache/readiness check rather than an implicit download path.
    pub async fn prepare(&self) -> Result<(), String> {
        self.health().await?;
        let mut models = vec![
            self.config.stt_model.as_str(),
            self.config.tts_model.as_str(),
        ];
        models.sort_unstable();
        models.dedup();
        for model in models {
            let response = self
                .http
                .post(self.config.url("v1/models")?)
                .query(&[("model_name", model)])
                .send()
                .await
                .map_err(|e| self.map_error("model load", e))?;
            if !response.status().is_success() {
                return Err(format!(
                    "loading local speech model {model} returned HTTP {}",
                    response.status()
                ));
            }
        }
        // Kokoro loads its phonemizer lazily. A successful model load alone can
        // report ready when `misaki` is absent, so validate one discarded
        // synthesis before Discord capture can be enabled.
        let _ = self.synthesize("Abbey voice ready.").await?;
        Ok(())
    }

    pub async fn transcribe(&self, pcm: &[i16]) -> Result<String, String> {
        let wav = encode_mono_pcm16_wav(pcm, INPUT_SAMPLE_RATE)?;
        self.transcribe_wav(&wav).await
    }

    /// Transcribe a validated PCM16 WAV. The ordinary Discord path first
    /// wraps its bounded mono samples with [`encode_mono_pcm16_wav`]; the
    /// offline voice self-test uses this entry point to feed synthesized test
    /// speech through the exact same recognizer without a microphone.
    pub async fn transcribe_wav(&self, wav: &[u8]) -> Result<String, String> {
        // Reject malformed, empty, unexpectedly encoded, or overlong input
        // before allocating a multipart body or involving the model server.
        let _ = decode_pcm16_wav(wav)?;
        let part = reqwest::multipart::Part::bytes(wav.to_vec())
            .file_name("utterance.wav")
            .mime_str("audio/wav")
            .map_err(|e| format!("building the local STT request failed: {e}"))?;
        let form = reqwest::multipart::Form::new()
            .text("model", self.config.stt_model.clone())
            .text("language", self.config.stt_language.clone())
            .text("response_format", "json")
            .part("file", part);
        let response = self
            .http
            .post(self.config.url("v1/audio/transcriptions")?)
            .multipart(form)
            .send()
            .await
            .map_err(|e| self.map_error("speech recognition", e))?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_STT_RESPONSE_BYTES)
        {
            return Err("local speech recognition returned an oversized response".into());
        }
        let body = crate::http_body::read_capped(response, MAX_STT_RESPONSE_BYTES as usize)
            .await
            .map_err(|error| {
                if error.is_too_large() {
                    "local speech recognition returned an oversized response".to_string()
                } else {
                    format!("reading the local STT response failed: {error}")
                }
            })?;
        if !status.is_success() {
            return Err(format!("local speech recognition returned HTTP {status}"));
        }
        #[derive(Deserialize)]
        struct Transcript {
            text: String,
        }
        let text = serde_json::from_slice::<Transcript>(&body)
            .map_err(|e| format!("local speech recognition returned invalid JSON: {e}"))?
            .text;
        let text = text.trim();
        if text.is_empty() {
            return Err("local speech recognition returned an empty transcript".into());
        }
        if text.chars().count() > 8_000 {
            return Err("local speech recognition returned an oversized transcript".into());
        }
        Ok(text.to_string())
    }

    pub async fn synthesize(&self, text: &str) -> Result<DecodedAudio, String> {
        let (audio, _) = self.synthesize_response(text).await?;
        Ok(audio)
    }

    /// Synthesize and return the provider's validated PCM16 WAV. This is
    /// intentionally separate from Discord playback so an operator can audit
    /// Abbey's complete local voice without joining or recording a call.
    pub async fn synthesize_wav(&self, text: &str) -> Result<Vec<u8>, String> {
        let (_, wav) = self.synthesize_response(text).await?;
        Ok(wav)
    }

    async fn synthesize_response(&self, text: &str) -> Result<(DecodedAudio, Vec<u8>), String> {
        let text = spoken_text(text);
        if text.is_empty() {
            return Err("Abbey's response had no speakable text".into());
        }
        let response = self
            .http
            .post(self.config.url("v1/audio/speech")?)
            .json(&serde_json::json!({
                "model": self.config.tts_model,
                "input": text,
                "voice": self.config.voice,
                "lang_code": self.config.tts_language_code,
                "response_format": "wav",
                "stream": false,
            }))
            .send()
            .await
            .map_err(|e| self.map_error("speech synthesis", e))?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_TTS_RESPONSE_BYTES)
        {
            return Err("local speech synthesis returned oversized audio".into());
        }
        let body = crate::http_body::read_capped(response, MAX_TTS_RESPONSE_BYTES as usize)
            .await
            .map_err(|error| {
                if error.is_too_large() {
                    "local speech synthesis returned oversized audio".to_string()
                } else {
                    format!("reading local speech audio failed: {error}")
                }
            })?;
        if !status.is_success() {
            return Err(format!("local speech synthesis returned HTTP {status}"));
        }
        let audio = decode_pcm16_wav(&body)?;
        Ok((audio, body.to_vec()))
    }
}

/// Convert a model answer into compact speech without inventing content.
#[must_use]
pub fn spoken_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len().min(MAX_SPOKEN_CHARS));
    let mut in_url = false;
    for token in input.split_whitespace() {
        let is_url = token.starts_with("http://") || token.starts_with("https://");
        if is_url {
            if !in_url && !output.is_empty() {
                output.push_str(" link");
            }
            in_url = true;
            continue;
        }
        in_url = false;
        let cleaned: String = token
            .chars()
            .filter(|c| !matches!(c, '`' | '*' | '_' | '#' | '|' | '<' | '>'))
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push(' ');
        }
        for c in cleaned.chars() {
            if output.chars().count() >= MAX_SPOKEN_CHARS {
                break;
            }
            output.push(c);
        }
        if output.chars().count() >= MAX_SPOKEN_CHARS {
            break;
        }
    }
    output.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_speech_candidate_blocks_playback_until_cleared() {
        let mut segmenter = Segmenter::new();
        assert!(!segmenter.is_speaking());
        assert!(segmenter.push(frame(1, Some(9), 2_000)).is_empty());
        assert!(segmenter.is_speaking());
        assert!(segmenter.push(VoiceFrame::silence(2)).is_empty());
        assert!(!segmenter.is_speaking());

        assert!(segmenter.push(frame(3, Some(9), 2_000)).is_empty());
        assert!(segmenter.is_speaking());
        assert!(matches!(
            segmenter.push(frame(4, Some(9), 2_000)).as_slice(),
            [SegmentEvent::SpeechStarted { .. }]
        ));
        assert!(segmenter.is_speaking());
        for sequence in 5..30 {
            assert!(segmenter.push(VoiceFrame::silence(sequence)).is_empty());
        }
        assert!(!segmenter.is_speaking());
    }

    fn frame(sequence: u64, speaker: Option<u64>, value: i16) -> VoiceFrame {
        VoiceFrame {
            sequence,
            speaker_id: speaker,
            samples: vec![value; FRAME_SAMPLES],
            overlap: false,
        }
    }

    #[test]
    fn local_endpoint_is_loopback_only() {
        assert!(OfflineVoiceConfig::from_values(None, None, None, None, None).is_ok());
        assert!(
            OfflineVoiceConfig::from_values(
                Some("http://[::1]:8181".into()),
                None,
                None,
                None,
                None,
            )
            .is_ok()
        );
        for endpoint in [
            "http://example.com:8181",
            "https://127.0.0.1:8181",
            "http://user@127.0.0.1:8181",
            "http://127.0.0.1:8181?token=x",
        ] {
            let error =
                OfflineVoiceConfig::from_values(Some(endpoint.into()), None, None, None, None)
                    .unwrap_err();
            assert!(error.contains("loopback HTTP"), "{endpoint}: {error}");
        }
    }

    #[test]
    fn kokoro_language_is_derived_from_voice_not_stt_language() {
        for (voice, expected_code) in [
            ("af_heart", "a"),
            ("bf_emma", "b"),
            ("jf_alpha", "j"),
            ("zf_xiaobei", "z"),
            ("pf_dora", "p"),
        ] {
            let config = OfflineVoiceConfig::from_values(
                None,
                None,
                None,
                Some(voice.into()),
                Some("de".into()),
            )
            .unwrap();
            assert_eq!(config.tts_language_code, expected_code, "{voice}");
            assert_eq!(config.stt_language, "de", "STT stays independent");
        }
    }

    #[test]
    fn invalid_kokoro_voice_language_mapping_fails_at_configuration() {
        for voice in [
            "xf_unknown",
            "a_heart",
            "af",
            "alloy",
            "af_../../foo",
            "bf_emma.wav",
            "jf_日本語",
        ] {
            let error = OfflineVoiceConfig::from_values(None, None, None, Some(voice.into()), None)
                .unwrap_err();
            assert!(
                error.contains("supported Kokoro voice prefix"),
                "{voice}: {error}"
            );
        }
    }

    #[test]
    fn two_voiced_frames_start_and_silence_finishes_a_turn() {
        let mut segmenter = Segmenter::new();
        assert!(segmenter.push(frame(1, Some(9), 2_000)).is_empty());
        assert_eq!(
            segmenter.push(frame(2, Some(9), 2_000)),
            vec![SegmentEvent::SpeechStarted {
                speaker_id: Some(9)
            }]
        );
        for sequence in 3..=20 {
            assert!(segmenter.push(frame(sequence, Some(9), 2_000)).is_empty());
        }
        let mut completed = None;
        for sequence in 21..=45 {
            for event in segmenter.push(VoiceFrame::silence(sequence)) {
                if let SegmentEvent::Completed(value) = event {
                    completed = Some(value);
                }
            }
        }
        let completed = completed.expect("turn completed");
        assert_eq!(completed.speaker_id, Some(9));
        assert!(!completed.overlap);
        assert!(completed.pcm.len() >= 20 * FRAME_SAMPLES);
    }

    #[test]
    fn sequence_gap_aborts_instead_of_transcribing_corrupt_audio() {
        let mut segmenter = Segmenter::new();
        segmenter.push(frame(1, Some(9), 2_000));
        segmenter.push(frame(2, Some(9), 2_000));
        assert_eq!(
            segmenter.push(frame(4, Some(9), 2_000)),
            vec![SegmentEvent::AbortedOverrun]
        );
    }

    #[test]
    fn sequence_gap_during_start_candidate_resets_and_reports_overrun() {
        let mut segmenter = Segmenter::new();
        assert!(segmenter.push(frame(1, Some(9), 2_000)).is_empty());
        assert_eq!(
            segmenter.push(frame(3, Some(9), 2_000)),
            vec![SegmentEvent::AbortedOverrun]
        );
        assert!(segmenter.push(frame(4, Some(9), 2_000)).is_empty());
        assert_eq!(
            segmenter.push(frame(5, Some(9), 2_000)),
            vec![SegmentEvent::SpeechStarted {
                speaker_id: Some(9)
            }]
        );
    }

    #[test]
    fn pre_roll_overlap_and_speaker_changes_fail_attribution_closed() {
        let mut segmenter = Segmenter::new();
        let mut first = frame(1, Some(1), 2_000);
        first.overlap = true;
        segmenter.push(first);
        segmenter.push(frame(2, Some(2), 2_000));
        segmenter.push(frame(3, Some(2), 2_000));
        for sequence in 4..=18 {
            segmenter.push(frame(sequence, Some(2), 2_000));
        }
        let mut completed = None;
        for sequence in 19..=43 {
            for event in segmenter.push(VoiceFrame::silence(sequence)) {
                if let SegmentEvent::Completed(value) = event {
                    completed = Some(value);
                }
            }
        }
        let completed = completed.expect("completed");
        assert!(completed.overlap);
        assert_eq!(completed.speaker_id, Some(1));
    }

    #[test]
    fn missing_mapping_mid_utterance_fails_attribution_closed() {
        let mut segmenter = Segmenter::new();
        segmenter.push(frame(1, Some(1), 2_000));
        segmenter.push(frame(2, Some(1), 2_000));
        for sequence in 3..=17 {
            segmenter.push(frame(sequence, Some(1), 2_000));
        }
        segmenter.push(frame(18, None, 2_000));
        let mut completed = None;
        for sequence in 19..=43 {
            for event in segmenter.push(VoiceFrame::silence(sequence)) {
                if let SegmentEvent::Completed(value) = event {
                    completed = Some(value);
                }
            }
        }
        assert!(completed.expect("completed").overlap);
    }

    #[test]
    fn speaker_change_marks_overlap() {
        let mut segmenter = Segmenter::new();
        segmenter.push(frame(1, Some(1), 2_000));
        segmenter.push(frame(2, Some(1), 2_000));
        for sequence in 3..=17 {
            segmenter.push(frame(sequence, Some(1), 2_000));
        }
        segmenter.push(frame(18, Some(2), 3_000));
        let mut completed = None;
        for sequence in 19..=43 {
            for event in segmenter.push(VoiceFrame::silence(sequence)) {
                if let SegmentEvent::Completed(value) = event {
                    completed = Some(value);
                }
            }
        }
        assert!(completed.expect("completed").overlap);
    }

    #[test]
    fn wav_round_trip_is_pcm16_to_native_f32() {
        let source = [i16::MIN, 0, i16::MAX];
        let wav = encode_mono_pcm16_wav(&source, 24_000).unwrap();
        let decoded = decode_pcm16_wav(&wav).unwrap();
        assert_eq!(decoded.sample_rate, 24_000);
        assert_eq!(decoded.channels, 1);
        let (values, remainder) = decoded.pcm_f32.as_chunks::<4>();
        assert!(remainder.is_empty());
        let values: Vec<f32> = values
            .iter()
            .map(|bytes| f32::from_ne_bytes(*bytes))
            .collect();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], -1.0);
        assert_eq!(values[1], 0.0);
        assert!((values[2] - (32767.0 / 32768.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn malformed_or_long_wav_is_rejected() {
        assert!(decode_pcm16_wav(b"not a wav").is_err());
        let long = vec![0_i16; 24_000 * 46];
        let wav = encode_mono_pcm16_wav(&long, 24_000).unwrap_err();
        assert!(wav.contains("30 seconds"));

        let mut empty = encode_mono_pcm16_wav(&[], 24_000).unwrap();
        assert!(decode_pcm16_wav(&empty).unwrap_err().contains("no audio"));
        empty[32..34].copy_from_slice(&4_u16.to_le_bytes());
        assert!(decode_pcm16_wav(&empty).is_err());
    }

    #[test]
    fn spoken_copy_drops_formatting_and_raw_urls() {
        let spoken = spoken_text("**Abbey:** see https://example.com/a and `cargo test`");
        assert_eq!(spoken, "Abbey: see link and cargo test");
    }

    #[tokio::test]
    async fn wav_transcription_rejects_malformed_input_before_network() {
        let config = OfflineVoiceConfig::from_values(None, None, None, None, None).unwrap();
        let client = MlxAudioClient::new(config).unwrap();
        let error = client.transcribe_wav(b"not a wave file").await.unwrap_err();
        assert!(error.contains("RIFF/WAVE"), "{error}");
    }

    #[test]
    fn connect_failures_are_operator_not_listening_copy() {
        let raw = "error sending request for url (http://127.0.0.1:8181/v1/models): error trying to connect: tcp connect error: Connection refused (os error 61)";
        assert_eq!(
            classify_local_speech_failure(raw),
            LocalSpeechFailureKind::NotListening
        );
        assert!(sidecar_is_unavailable(raw));
        let message = local_speech_operator_message(
            LocalSpeechFailureKind::NotListening,
            "http://127.0.0.1:8181/",
            "health",
            raw,
        );
        assert!(message.contains("not listening"));
        assert!(message.contains("install-mlx-audio-launchd.sh"));
        assert!(message.contains("setuptools"));
        assert!(message.contains("importlib.metadata"));
        assert!(!message.contains("pkg_resources"));
        assert!(!message.contains("os error 61"));
        assert!(
            message.chars().count() <= 240,
            "{}",
            message.chars().count()
        );
    }

    #[test]
    fn timeouts_tell_the_operator_to_retry_status_not_wait_on_discord() {
        let raw = "operation timed out";
        assert_eq!(
            classify_local_speech_failure(raw),
            LocalSpeechFailureKind::TimedOut
        );
        assert!(
            !sidecar_is_unavailable(raw),
            "a slow Whisper/Kokoro load must not tear down an active session"
        );
        let message = local_speech_operator_message(
            LocalSpeechFailureKind::TimedOut,
            "http://127.0.0.1:8181/",
            "health",
            raw,
        );
        assert!(message.contains("timed out"));
        assert!(message.contains("/voice status"));
        assert!(
            message.chars().count() <= 240,
            "{}",
            message.chars().count()
        );
    }
}
