//! Confirm sustained input before cancelling an audible local reply.
//!
//! Capture starts promptly in `Segmenter`, but a short energy spike must not
//! destroy playback. This independent, allocation-free gate requires 300 ms of
//! voiced audio from one known speaker within 400 ms. It tolerates brief quiet
//! gaps without accumulating scattered clicks or uncertain speaker evidence.

use crate::offline_voice::{FRAME_SAMPLES, FrameSequence, VoiceFrame};
use crate::vad::{EnergyVad, Vad};

const MIN_VOICED_FRAMES: u32 = 15;
const WINDOW_FRAMES: u32 = 20;
const WINDOW_MASK: u32 = (1 << WINDOW_FRAMES) - 1;

#[derive(Debug, Default)]
pub(super) struct InterruptionGate {
    sequence: FrameSequence,
    speaker: Option<u64>,
    voiced_window: u32,
    confirmed: bool,
    vad: EnergyVad,
}

impl InterruptionGate {
    /// A new output track must receive fresh interruption evidence; input from
    /// before playback must neither cancel it early nor leave the gate latched.
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn clear_run(&mut self) {
        self.speaker = None;
        self.voiced_window = 0;
        self.confirmed = false;
    }

    pub(super) fn observe(&mut self, frame: &VoiceFrame) -> Option<u64> {
        if self.sequence.observe(frame.sequence).is_err()
            || frame.samples.len() != FRAME_SAMPLES
            || frame.overlap
        {
            self.clear_run();
            return None;
        }

        let voiced = self.vad.is_voice(&frame.samples);
        if voiced {
            let Some(speaker) = frame.speaker_id else {
                self.clear_run();
                return None;
            };
            if self.speaker != Some(speaker) {
                self.clear_run();
                self.speaker = Some(speaker);
            }
        }

        self.voiced_window = ((self.voiced_window << 1) | u32::from(voiced)) & WINDOW_MASK;
        if self.voiced_window == 0 {
            self.clear_run();
        } else if voiced && !self.confirmed && self.voiced_window.count_ones() >= MIN_VOICED_FRAMES
        {
            self.confirmed = true;
            return self.speaker;
        }
        None
    }
}

#[cfg(test)]
mod tests;
