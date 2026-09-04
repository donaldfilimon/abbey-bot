//! Consent-free local voice acceptance for operators.
//!
//! This mode never starts a Serenity gateway, Songbird voice driver, or call.
//! It synthesizes a fixed wake phrase locally, transcribes it through the
//! configured Whisper model, sends that transcript through the same canonical
//! Abbey generation path used by local Discord voice, validates the exact
//! Songbird input adapter, and writes the final Kokoro reply to a new WAV file.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Cursor;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::generation;
use crate::memory::PersonaContext;
use crate::offline_voice::{MlxAudioClient, OfflineVoiceConfig, decode_pcm16_wav, spoken_text};
use crate::persona;
use crate::runtime::{self, AppState};
use songbird::input::RawAdapter;

const TEST_UTTERANCE: &str =
    "Abbey, explain in one short sentence how your local voice protects privacy.";
const SELF_TEST_SYSTEM_SUFFIX: &str = "You are producing a private, local Abbey voice audition. Respond in one to three short, natural sentences unless the user explicitly asks for detail. Avoid Markdown, raw URLs, emoji, tables, headings, and unspoken formatting. Pronounce code, symbols, and acronyms clearly. This test cannot perform external actions or durable memory changes, so never claim that either happened.";

#[derive(Debug)]
pub struct VoiceSelfTestReport {
    pub output: PathBuf,
    pub round_trip_word_recall: f32,
    pub sample_rate: u32,
    pub channels: u8,
    pub duration_millis: u64,
}

/// Run the complete local voice chain without a Discord token or call.
///
/// The output uses create-new semantics so a typo or repeated invocation can
/// never replace an existing recording.
pub async fn run(output: &Path) -> Result<VoiceSelfTestReport, String> {
    if output.as_os_str().is_empty() {
        return Err("the voice self-test output path is empty".into());
    }
    if output.exists() {
        return Err(format!(
            "the voice self-test refuses to overwrite {}",
            output.display()
        ));
    }

    let client = MlxAudioClient::new(OfflineVoiceConfig::from_env()?)?;
    client.prepare().await?;

    // A synthetic stimulus validates TTS -> STT deterministically without
    // opening a microphone or capturing another person.
    let stimulus_wav = client.synthesize_wav(TEST_UTTERANCE).await?;
    let transcript = client.transcribe_wav(&stimulus_wav).await?;
    if !crate::voice::contains_wake_name(
        &transcript,
        &crate::voice::VoiceConfig::default_wake_words(),
    ) {
        return Err("local Whisper did not preserve an Abbey/Abi/Aviva wake name".into());
    }

    let configured = crate::llm::Backend::from_env();
    let fallback = matches!(
        configured.as_ref(),
        Some(crate::llm::Backend::Anthropic { .. })
    )
    .then(|| {
        crate::llm::Backend::from_values(
            None,
            std::env::var("ABBEY_BOT_LLM_ENDPOINT").ok(),
            std::env::var("ABBEY_BOT_LLM_MODEL").ok(),
        )
    })
    .flatten();
    let backend = configured
        .as_ref()
        .into_iter()
        .chain(fallback.as_ref())
        .find(|backend| backend.is_loopback_openai_compatible())
        .cloned()
        .ok_or_else(|| {
            "the voice self-test requires a loopback ABBEY_BOT_LLM_ENDPOINT; it will not send the transcript to a remote text provider"
                .to_string()
        })?;
    // The audition deliberately starts with empty in-memory stores. It does
    // not load the production state file, WDBX segment, guild settings,
    // rewards, or conversation sessions merely because ABBEY_DATA_DIR is in
    // the operator's service environment.
    let state = AppState::in_memory();
    let selected_persona = persona::route(&transcript, None).persona;
    let scope = "discord:voice:self-test";
    let context = PersonaContext::empty();
    let _slot = state
        .acquire_generation_for_voice()
        .await
        .map_err(|error| error.to_string())?;
    let (answer, _) = generation::generate_without_delivery(
        &state,
        &backend,
        selected_persona,
        &generation::Ask {
            scope,
            context: &context,
            user_input: &transcript,
            now: runtime::now(),
        },
        Some(SELF_TEST_SYSTEM_SUFFIX),
    )
    .await
    .map_err(|error| error.to_string())?;
    let spoken_answer = spoken_text(&answer);
    if spoken_answer.is_empty() {
        return Err("Abbey's local reasoning returned no speakable answer".into());
    }
    let output_wav = client.synthesize_wav(&spoken_answer).await?;
    let reply_transcript = client.transcribe_wav(&output_wav).await?;
    let round_trip_word_recall = word_recall(&spoken_answer, &reply_transcript);
    if round_trip_word_recall < 0.60 {
        return Err(format!(
            "Abbey's synthesized reply did not survive the local TTS-to-STT quality check ({:.0}% word recall)",
            round_trip_word_recall * 100.0
        ));
    }
    let decoded = decode_pcm16_wav(&output_wav)?;
    let sample_frames = decoded
        .pcm_f32
        .len()
        .checked_div(4 * usize::from(decoded.channels))
        .ok_or_else(|| "Abbey's synthesized audio has invalid channel metadata".to_string())?;
    let duration_millis = u64::try_from(sample_frames)
        .unwrap_or(u64::MAX)
        .saturating_mul(1_000)
        / u64::from(decoded.sample_rate);
    let songbird_input: songbird::input::Input = RawAdapter::new(
        Cursor::new(decoded.pcm_f32),
        decoded.sample_rate,
        u32::from(decoded.channels),
    )
    .into();
    let playable = songbird_input
        .make_playable_async(
            songbird::input::codecs::get_codec_registry(),
            songbird::input::codecs::get_probe(),
        )
        .await
        .map_err(|error| format!("Songbird could not prepare Abbey's local PCM: {error}"))?;
    if !playable.is_playable() {
        return Err("Songbird did not promote Abbey's local PCM to playable audio".into());
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(output)
        .map_err(|error| format!("creating {} failed: {error}", output.display()))?;
    file.write_all(&output_wav)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("writing {} failed: {error}", output.display()))?;

    Ok(VoiceSelfTestReport {
        output: output.to_path_buf(),
        round_trip_word_recall,
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        duration_millis,
    })
}

fn word_recall(expected: &str, actual: &str) -> f32 {
    let expected = normalized_words(expected);
    if expected.is_empty() {
        return 0.0;
    }
    let mut actual_counts = HashMap::<String, usize>::new();
    for word in normalized_words(actual) {
        *actual_counts.entry(word).or_default() += 1;
    }
    let mut matched = 0_usize;
    for word in &expected {
        if let Some(count) = actual_counts.get_mut(word)
            && *count > 0
        {
            *count -= 1;
            matched += 1;
        }
    }
    matched as f32 / expected.len() as f32
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_score_is_case_punctuation_and_duplicate_tolerant() {
        assert_eq!(
            word_recall("Abbey says hello, hello!", "abbey says hello."),
            0.75
        );
        assert_eq!(word_recall("one two three four", "one three"), 0.5);
        assert_eq!(word_recall("one two", "unrelated"), 0.0);
        assert_eq!(word_recall("", "anything"), 0.0);
    }
}
