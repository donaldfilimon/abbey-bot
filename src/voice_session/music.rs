//! Output ownership is independent of the listening epoch. All buffers and handles
//! are synchronously invalidated on stop; a late task can never publish a replacement.
use super::{PlaybackTermination, SessionEvent, VoicePhase};
use crate::{audio_tap::PcmBuffer, player_control::Player};
use songbird::tracks::TrackHandle;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Output {
    generation: u64,
    buffer: PcmBuffer,
    track: Option<TrackHandle>,
}
struct State {
    cancel: tokio_util::sync::CancellationToken,
    output: Option<Output>,
    volume: u8,
    phase: VoicePhase,
    player: Option<Player>,
    status: String,
    last_event: Option<SessionEvent>,
}
pub struct MusicController {
    closed: AtomicBool,
    generation: AtomicU64,
    state: Mutex<State>,
}
impl Default for MusicController {
    fn default() -> Self {
        Self {
            closed: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            state: Mutex::new(State {
                cancel: tokio_util::sync::CancellationToken::new(),
                output: None,
                volume: 100,
                phase: VoicePhase::Disconnected,
                player: None,
                status: "stopped".into(),
                last_event: None,
            }),
        }
    }
}
impl MusicController {
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop("service stopping", PlaybackTermination::Stopped);
    }
    pub fn begin(&self, player: Player) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.closed.load(Ordering::SeqCst) {
            return 0;
        }
        state.cancel.cancel();
        state.cancel = tokio_util::sync::CancellationToken::new();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(old) = state.output.take() {
            old.buffer.close();
            if let Some(track) = old.track {
                let _ = track.stop();
            }
        }
        state.player = Some(player);
        state.status = "starting".into();
        generation
    }
    pub fn current(&self, generation: u64) -> bool {
        !self.closed.load(Ordering::SeqCst)
            && generation != 0
            && self.generation.load(Ordering::SeqCst) == generation
    }
    /// Serialize native launch with generation invalidation. The returned token
    /// cancels an already-launched child when leave, stop, or replacement wins.
    pub fn launch_current<T>(
        &self,
        generation: u64,
        launch: impl FnOnce() -> T,
    ) -> Option<(T, tokio_util::sync::CancellationToken)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.current(generation)
            .then(|| (launch(), state.cancel.clone()))
    }
    pub fn install(&self, generation: u64, buffer: PcmBuffer, track: TrackHandle) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.current(generation) {
            buffer.close();
            let _ = track.stop();
            return false;
        }
        if let Some(old) = state.output.take() {
            old.buffer.close();
            if let Some(track) = old.track {
                let _ = track.stop();
            }
        }
        let _ = track.set_volume(crate::music::volume(state.volume, state.phase));
        state.output = Some(Output {
            generation,
            buffer,
            track: Some(track),
        });
        state.status = "playing".into();
        true
    }
    /// Cancel playback and report whether output **was** active, decided under the same
    /// lock acquisition that performs the cancellation.
    ///
    /// Returning it matters: a caller that consults [`Self::is_output_active`] first takes
    /// the lock twice, and a concurrent `begin` landing in that window is cancelled by
    /// this call while the caller still reports that nothing was playing.
    pub fn stop(&self, status: &str, reason: PlaybackTermination) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let was_active = Self::active(&state);
        state.cancel.cancel();
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(old) = state.output.take() {
            old.buffer.close();
            if let Some(track) = old.track {
                let _ = track.stop();
            }
        }
        state.status = status.into();
        state.last_event = Some(SessionEvent::MusicTerminated { reason });
        was_active
    }
    pub fn finish(&self, generation: u64, status: &str, reason: PlaybackTermination) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.current(generation) {
            return;
        }
        state.cancel.cancel();
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(old) = state.output.take() {
            old.buffer.close();
            if let Some(track) = old.track {
                let _ = track.stop();
            }
        }
        state.status = status.into();
        state.last_event = Some(SessionEvent::MusicTerminated { reason });
    }
    /// Discard the old transport during a consent-driven call replacement without
    /// changing the independent music operation token.
    pub fn detach(&self, generation: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .output
            .as_ref()
            .is_some_and(|o| o.generation == generation)
        {
            let old = state.output.take().unwrap();
            old.buffer.close();
            if let Some(track) = old.track {
                let _ = track.stop();
            }
        }
    }
    pub fn phase(&self, phase: VoicePhase) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.phase = phase;
        if let Some(track) = state.output.as_ref().and_then(|o| o.track.as_ref()) {
            let _ = track.set_volume(crate::music::volume(state.volume, phase));
        }
    }
    pub fn set_volume(&self, volume: u8) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.volume = volume.min(100);
        if let Some(track) = state.output.as_ref().and_then(|o| o.track.as_ref()) {
            let _ = track.set_volume(crate::music::volume(state.volume, state.phase));
        }
    }
    pub fn player(&self) -> Option<Player> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .player
    }
    /// True while a track handle is installed or music is mid-start.
    #[must_use]
    pub fn is_output_active(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::active(&state)
    }

    /// The single definition of "output is active", so [`Self::stop`] and
    /// [`Self::is_output_active`] can never drift apart.
    fn active(state: &State) -> bool {
        state.output.is_some() || state.status == "starting" || state.status == "playing"
    }
    pub fn status(&self) -> String {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        format!("Music: {}; volume {}%", state.status, state.volume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use songbird::input::RawAdapter;
    use std::io::Cursor;

    #[tokio::test]
    async fn music_handles_keep_separate_ownership_and_reject_stale_install() {
        let config = songbird::Config::default().scheduler(songbird::driver::Scheduler::new(
            songbird::driver::SchedulerConfig::default(),
        ));
        let mut driver = songbird::Driver::new(config);
        let controller = MusicController::default();
        let generation = controller.begin(Player::Spotify);
        // Synthetic, finite PCM; standalone driver never joins Discord.
        let track =
            driver.play_input(RawAdapter::new(Cursor::new(vec![0; 48000 * 8]), 48000, 2).into());
        assert!(controller.install(generation, PcmBuffer::new(), track.clone()));
        controller.set_volume(80);
        controller.phase(VoicePhase::Speaking);
        let speech =
            driver.play_input(RawAdapter::new(Cursor::new(vec![0; 48000 * 4]), 48000, 1).into());
        assert_ne!(track.uuid(), speech.uuid());
        assert_eq!(
            controller
                .state
                .lock()
                .unwrap()
                .output
                .as_ref()
                .unwrap()
                .track
                .as_ref()
                .unwrap()
                .uuid(),
            track.uuid()
        );
        assert_eq!(controller.state.lock().unwrap().phase, VoicePhase::Speaking);
        controller.phase(VoicePhase::Listening);
        assert_eq!(controller.state.lock().unwrap().volume, 80);
        let next = controller.begin(Player::Music);
        let stale =
            driver.play_input(RawAdapter::new(Cursor::new(vec![0; 48000 * 8]), 48000, 2).into());
        assert!(!controller.install(generation, PcmBuffer::new(), stale));
        assert!(controller.current(next));
        controller.finish(next, "source ended", PlaybackTermination::Errored);
        assert!(matches!(
            controller.state.lock().unwrap().last_event,
            Some(SessionEvent::MusicTerminated {
                reason: PlaybackTermination::Errored
            })
        ));
        let _ = speech.stop();
    }
}
