//! Unified voice activity detection for Abbey's voice paths.
//!
//! The offline local path and the OpenAI Realtime path previously kept
//! independent silence counters and duplicated energy thresholds. This module
//! owns the single trait and the single threshold pair so the two consumers
//! cannot drift.
//!
//! * [`EnergyVad`] is the local energy gate: squared RMS >= 280^2 and a
//!   meaningful peak. It is the only gate used by the local MLX-Audio actor.
//! * [`SemanticVad`] is the provider semantic gate: it has no local decision
//!   and fires only when `VadCtx::provider_speech_started` is true.
//! * [`ComposedVad`] composes the two for the OpenAI Realtime actor: energy is
//!   the pre-filter (silent frames are dropped locally and never sent), semantic
//!   is the interruption decision (local barge-in never cancels a response).

/// Mean squared energy threshold (280^2). Unified source for offline and inline
/// paths so a future tuning cannot create a drift.
pub const MEAN_THRESHOLD: u64 = 78_400;
/// Peak amplitude threshold paired with [`MEAN_THRESHOLD`]. Both must pass.
pub const PEAK_THRESHOLD: i32 = 900;

/// Backward-compatible aliases for the names used in the task description.
#[allow(dead_code)]
pub const VOICE_MEAN_THRESHOLD: u64 = MEAN_THRESHOLD;
#[allow(dead_code)]
pub const VOICE_PEAK_THRESHOLD: i32 = PEAK_THRESHOLD;

/// Context supplied to [`Vad::should_interrupt`]. Local energy is already
/// reflected in [`Vad::is_voice`]; this context carries only the provider
/// signal so a local silence counter cannot drive an interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VadCtx {
    pub provider_speech_started: bool,
}

impl VadCtx {
    #[allow(dead_code)]
    #[must_use]
    pub const fn provider_started() -> Self {
        Self {
            provider_speech_started: true,
        }
    }

    #[allow(dead_code)]
    #[must_use]
    pub const fn none() -> Self {
        Self {
            provider_speech_started: false,
        }
    }
}

/// Unified VAD contract consumed by both voice actors.
pub trait Vad {
    fn is_voice(&self, frame: &[i16]) -> bool;
    fn should_interrupt(&self, ctx: &VadCtx) -> bool;
}

/// Energy gate for the local MLX path. Holds the single threshold pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnergyVad {
    pub mean_threshold: u64,
    pub peak_threshold: i32,
}

impl EnergyVad {
    #[allow(dead_code)]
    #[must_use]
    pub const fn new(mean_threshold: u64, peak_threshold: i32) -> Self {
        Self {
            mean_threshold,
            peak_threshold,
        }
    }

    /// Convenience constructor matching the `EnergyVad(threshold)` phrasing in
    /// the task: the mean threshold is provided, the peak stays unified.
    #[allow(dead_code)]
    #[must_use]
    pub const fn with_mean(mean_threshold: u64) -> Self {
        Self {
            mean_threshold,
            peak_threshold: PEAK_THRESHOLD,
        }
    }
}

impl Default for EnergyVad {
    fn default() -> Self {
        Self {
            mean_threshold: MEAN_THRESHOLD,
            peak_threshold: PEAK_THRESHOLD,
        }
    }
}

impl Vad for EnergyVad {
    fn is_voice(&self, frame: &[i16]) -> bool {
        if frame.is_empty() {
            return false;
        }
        let mut squared = 0_u64;
        let mut peak = 0_i32;
        for sample in frame {
            let value = i32::from(*sample).unsigned_abs();
            peak = peak.max(i32::try_from(value).unwrap_or(i32::MAX));
            squared = squared.saturating_add(u64::from(value) * u64::from(value));
        }
        let mean = squared / u64::try_from(frame.len()).unwrap_or(1);
        mean >= self.mean_threshold && peak >= self.peak_threshold
    }

    fn should_interrupt(&self, _ctx: &VadCtx) -> bool {
        // Local energy alone never interrupts; the offline actor uses
        // `Segmenter::SpeechStarted` for barge-in, and the composed actor
        // delegates the decision to the semantic gate.
        false
    }
}

/// Provider semantic gate. It has no local `is_voice` decision and interrupts
/// only on the provider's `speech_started` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SemanticVad;

impl Vad for SemanticVad {
    fn is_voice(&self, _frame: &[i16]) -> bool {
        false
    }

    fn should_interrupt(&self, ctx: &VadCtx) -> bool {
        ctx.provider_speech_started
    }
}

/// Composed gate for the OpenAI Realtime path: energy as pre-filter,
/// semantic as interruption decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComposedVad {
    pub energy: EnergyVad,
    pub semantic: SemanticVad,
}

impl ComposedVad {
    #[allow(dead_code)]
    #[must_use]
    pub const fn new(energy: EnergyVad, semantic: SemanticVad) -> Self {
        Self { energy, semantic }
    }
}

impl Vad for ComposedVad {
    fn is_voice(&self, frame: &[i16]) -> bool {
        self.energy.is_voice(frame)
    }

    fn should_interrupt(&self, ctx: &VadCtx) -> bool {
        self.semantic.should_interrupt(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_gate_rejects_silence_and_noise() {
        let vad = EnergyVad::default();
        assert!(!vad.is_voice(&[]));
        assert!(!vad.is_voice(&[0; 480]));
        assert!(!vad.is_voice(&[100; 480]));
        assert!(vad.is_voice(&[2_000; 480]));
        assert!(vad.is_voice(&[i16::MAX; 480]));
    }

    #[test]
    fn energy_gate_never_interrupts_on_provider_ctx() {
        let vad = EnergyVad::default();
        assert!(!vad.should_interrupt(&VadCtx::provider_started()));
        assert!(!vad.should_interrupt(&VadCtx::none()));
    }

    #[test]
    fn semantic_gate_only_interrupts_on_provider_event() {
        let vad = SemanticVad;
        assert!(!vad.is_voice(&[i16::MAX; 480]));
        assert!(vad.should_interrupt(&VadCtx::provider_started()));
        assert!(!vad.should_interrupt(&VadCtx::none()));
    }

    #[test]
    fn composed_is_energy_prefilter_and_semantic_decision() {
        let vad = ComposedVad::default();
        assert!(vad.is_voice(&[2_000; 480]));
        assert!(!vad.is_voice(&[0; 480]));
        assert!(vad.should_interrupt(&VadCtx::provider_started()));
        assert!(!vad.should_interrupt(&VadCtx::none()));
    }

    #[test]
    fn thresholds_are_unified() {
        assert_eq!(EnergyVad::default().mean_threshold, MEAN_THRESHOLD);
        assert_eq!(EnergyVad::default().peak_threshold, PEAK_THRESHOLD);
        assert_eq!(MEAN_THRESHOLD, 78_400);
        assert_eq!(PEAK_THRESHOLD, 900);
    }
}
