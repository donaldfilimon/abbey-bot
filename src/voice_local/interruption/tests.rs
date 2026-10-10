//! Synthetic frame sequences pin interruption confidence independently of STT.
use super::*;

fn frame(sequence: u64, speaker_id: Option<u64>, voiced: bool) -> VoiceFrame {
    VoiceFrame {
        sequence,
        speaker_id,
        samples: vec![if voiced { 2_000 } else { 0 }; 480],
        overlap: false,
    }
}

fn next_frame(
    gate: &mut InterruptionGate,
    sequence: &mut u64,
    speaker_id: Option<u64>,
    voiced: bool,
) -> Option<u64> {
    *sequence += 1;
    gate.observe(&frame(*sequence, speaker_id, voiced))
}

fn expect_no_confirmation(
    gate: &mut InterruptionGate,
    sequence: &mut u64,
    speaker_id: Option<u64>,
    voiced: bool,
    count: usize,
) {
    for _ in 0..count {
        assert_eq!(
            next_frame(gate, sequence, speaker_id, voiced),
            None,
            "unexpected interruption at frame {sequence}"
        );
    }
}

#[test]
fn forty_millisecond_noise_never_confirms() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 2);
    expect_no_confirmation(&mut gate, &mut sequence, None, false, 25);
}

#[test]
fn fourteen_frames_are_insufficient_and_fifteen_confirm() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
}

#[test]
fn five_quiet_frames_allow_a_brief_gap_without_speaker_attribution() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 7);
    expect_no_confirmation(&mut gate, &mut sequence, None, false, 5);
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 7);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
}

#[test]
fn a_longer_gap_keeps_old_voiced_frames_out_of_confirmation() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 7);
    expect_no_confirmation(&mut gate, &mut sequence, None, false, 6);
    // Seven earlier frames plus eight later ones total fifteen, but never
    // coexist inside the 20-frame window. Fresh speech must establish itself.
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
}

#[test]
fn scattered_clicks_cannot_accumulate_into_an_interruption() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    for _ in 0..40 {
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 2);
        expect_no_confirmation(&mut gate, &mut sequence, None, false, 2);
    }
}

#[test]
fn an_unknown_voiced_frame_discards_prior_confidence() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, None, true), None);
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
}

#[test]
fn overlapping_frames_reset_confidence_even_when_quiet() {
    for voiced in [true, false] {
        let mut gate = InterruptionGate::default();
        let mut sequence = 0;
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
        sequence += 1;
        let mut overlapping = frame(sequence, Some(7), voiced);
        overlapping.overlap = true;
        assert_eq!(gate.observe(&overlapping), None);
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
        assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
    }
}

#[test]
fn malformed_frame_lengths_reset_instead_of_counting_partial_audio() {
    for sample_count in [0, 479, 481, 960] {
        let mut gate = InterruptionGate::default();
        let mut sequence = 0;
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
        sequence += 1;
        let mut malformed = frame(sequence, Some(7), true);
        malformed.samples.resize(sample_count, 2_000);
        assert_eq!(gate.observe(&malformed), None);
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
        assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
    }
}

#[test]
fn different_speakers_cannot_pool_their_short_segments() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    for speaker in [7, 8, 7, 8] {
        expect_no_confirmation(&mut gate, &mut sequence, Some(speaker), true, 14);
    }
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(8), true), Some(8));
}

#[test]
fn an_earlier_speaker_does_not_taint_sustained_current_speech() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 1);
    expect_no_confirmation(&mut gate, &mut sequence, None, false, 7);
    // STT's ten-frame pre-roll can still contain speaker 7 when speaker 8
    // starts. Interruption confidence must follow the current speaker instead.
    expect_no_confirmation(&mut gate, &mut sequence, Some(8), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(8), true), Some(8));
}

#[test]
fn sequence_discontinuities_reject_the_frame_and_discard_prior_confidence() {
    // Forward gap, duplicate sequence, and backwards sequence respectively.
    for discontinuous_sequence in [19, 14, 13] {
        let mut gate = InterruptionGate::default();
        let mut sequence = 0;
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
        assert_eq!(
            gate.observe(&frame(discontinuous_sequence, Some(7), true)),
            None
        );
        sequence = discontinuous_sequence;
        expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
        assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
    }
}

#[test]
fn confirmation_is_once_per_run_including_brief_pauses() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 60);
    expect_no_confirmation(&mut gate, &mut sequence, None, false, 19);
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 30);
}

#[test]
fn a_fully_quiet_window_rearms_confirmation() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
    expect_no_confirmation(&mut gate, &mut sequence, None, false, 20);
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
}

#[test]
fn new_playback_discards_earlier_partial_confidence() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    gate.reset();
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
}

#[test]
fn new_playback_rearms_and_starts_a_fresh_sequence() {
    let mut gate = InterruptionGate::default();
    let mut sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(7), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(7), true), Some(7));
    gate.reset();
    sequence = 0;
    expect_no_confirmation(&mut gate, &mut sequence, Some(8), true, 14);
    assert_eq!(next_frame(&mut gate, &mut sequence, Some(8), true), Some(8));
}
