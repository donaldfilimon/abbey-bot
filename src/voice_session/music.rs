//! Output ownership is independent of the listening epoch. All buffers and handles
//! are synchronously invalidated on stop; a late task can never publish a replacement.
use super::{PlaybackTermination, SessionEvent, VoicePhase};
use crate::{audio_tap::PcmBuffer, player_control::Player};
use songbird::tracks::TrackHandle;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

struct Output {
    generation: u64,
    buffer: PcmBuffer,
    track: Option<TrackHandle>,
}
struct State {
    output: Option<Output>,
    volume: u8,
    phase: VoicePhase,
    player: Option<Player>,
    status: String,
    last_event: Option<SessionEvent>,
}
pub struct MusicController {
    generation: AtomicU64,
    state: Mutex<State>,
}
impl Default for MusicController {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            state: Mutex::new(State {
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
    pub fn begin(&self, player: Player) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        self.generation.load(Ordering::SeqCst) == generation
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
    pub fn stop(&self, status: &str, reason: PlaybackTermination) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    pub fn finish(&self, generation: u64, status: &str, reason: PlaybackTermination) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.current(generation) {
            return;
        }
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
