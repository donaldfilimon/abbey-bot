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
        OfflineVoiceConfig::from_values(Some("http://[::1]:8181".into()), None, None, None, None,)
            .is_ok()
    );
    for endpoint in [
        "http://example.com:8181",
        "https://127.0.0.1:8181",
        "http://user@127.0.0.1:8181",
        "http://127.0.0.1:8181?token=x",
    ] {
        let error = OfflineVoiceConfig::from_values(Some(endpoint.into()), None, None, None, None)
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
