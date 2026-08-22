//! Typed Songbird playback termination at the provider/runtime boundary.
//!
//! Songbird emits `TrackEvent::End` for natural completion, manual stop, and
//! as a secondary event after a playback error. Local and backup providers use
//! this single observer so those outcomes cannot drift into different notions
//! of completion.

use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::tracks::{PlayMode, TrackHandle};
use tokio::sync::mpsc;

use super::SessionEvent;

/// Content-free provenance for a terminal Songbird track event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackTermination {
    Natural,
    Stopped,
    Errored,
    Unclassified,
}

impl PlaybackTermination {
    #[must_use]
    fn from_event_context(context: &EventContext<'_>) -> Self {
        let EventContext::Track(tracks) = context else {
            return Self::Unclassified;
        };
        tracks.first().map_or(Self::Unclassified, |(state, _)| {
            Self::from_play_mode(&state.playing)
        })
    }

    #[must_use]
    fn from_play_mode(mode: &PlayMode) -> Self {
        match mode {
            PlayMode::End => Self::Natural,
            PlayMode::Stop => Self::Stopped,
            PlayMode::Errored(_) => Self::Errored,
            _ => Self::Unclassified,
        }
    }
}

struct PlaybackTerminationObserver {
    tx: mpsc::UnboundedSender<SessionEvent>,
    turn: u64,
}

#[serenity::async_trait]
impl EventHandler for PlaybackTerminationObserver {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        let _ = self.tx.send(SessionEvent::PlaybackTerminated {
            turn: self.turn,
            termination: PlaybackTermination::from_event_context(context),
        });
        None
    }
}

/// Attach the canonical typed terminal observer to a Songbird track.
pub fn register_playback_termination(
    handle: &TrackHandle,
    events: &mpsc::UnboundedSender<SessionEvent>,
    turn: u64,
) -> Result<(), String> {
    handle
        .add_event(
            Event::Track(TrackEvent::End),
            PlaybackTerminationObserver {
                tx: events.clone(),
                turn,
            },
        )
        .map_err(|error| format!("registering the playback completion event failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn songbird_terminal_provenance_is_preserved() {
        assert_eq!(
            PlaybackTermination::from_play_mode(&PlayMode::End),
            PlaybackTermination::Natural
        );
        assert_eq!(
            PlaybackTermination::from_play_mode(&PlayMode::Stop),
            PlaybackTermination::Stopped
        );
        assert_eq!(
            PlaybackTermination::from_play_mode(&PlayMode::Errored(
                songbird::tracks::PlayError::Decode(Arc::new(
                    symphonia::core::errors::Error::DecodeError("test playback failure"),
                )),
            )),
            PlaybackTermination::Errored
        );
        assert_eq!(
            PlaybackTermination::from_play_mode(&PlayMode::Play),
            PlaybackTermination::Unclassified
        );
    }
}
