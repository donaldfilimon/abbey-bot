//! Serialized lifecycle and observability for one Discord voice session.
//!
//! Provider tasks carry an epoch. Rejoin, pause, leave, bot moves, and shutdown
//! advance it before cancellation, so late work cannot overwrite status or
//! publish stale audio. The process owns exactly one provider task and one
//! playback handle at a time.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use songbird::tracks::TrackHandle;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::voice::VoiceConfig;

pub type SharedPlayback = Arc<Mutex<Option<TrackHandle>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicePhase {
    Disconnected,
    PresenceOnly,
    Connecting,
    Listening,
    Thinking,
    Speaking,
    AwaitingConsent,
    Failed,
}

impl VoicePhase {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::PresenceOnly => "muted/self-deafened presence",
            Self::Connecting => "connecting",
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::Speaking => "speaking",
            Self::AwaitingConsent => "paused for renewed consent",
            Self::Failed => "failed safe",
        }
    }

    #[must_use]
    pub const fn processes_audio(self) -> bool {
        matches!(self, Self::Listening | Self::Thinking | Self::Speaking)
    }
}

#[derive(Debug)]
pub enum SessionEvent {
    PlaybackEnded(u64),
}

impl VoiceRuntime {
    /// Invalidate a failed provider from inside its own task without awaiting
    /// the JoinHandle that represents that same task.
    pub async fn actor_failed(&self, epoch: u64, status: impl Into<String>) -> bool {
        if self
            .current_epoch
            .compare_exchange(
                epoch,
                epoch.saturating_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return false;
        }
        let _ = self
            .media_epoch
            .compare_exchange(epoch, 0, Ordering::SeqCst, Ordering::SeqCst);
        self.start_generation.fetch_add(1, Ordering::SeqCst);
        let control = {
            let mut inner = self.inner.lock().await;
            if inner.epoch != epoch {
                return false;
            }
            inner.epoch = epoch.saturating_add(1);
            inner.phase = VoicePhase::Failed;
            inner.status = bounded_status(status.into());
            inner.control.take()
        };
        if let Some(control) = control {
            let _ = control.cancel.send(true);
            let track = { control.playback.lock().await.take() };
            if let Some(track) = track {
                let _ = track.stop();
            }
            // This method is called by `control.task` itself. Dropping its
            // handle detaches it; the caller immediately returns naturally.
            drop(control.task);
        }
        true
    }
}

pub struct SessionControl {
    pub cancel: watch::Sender<bool>,
    pub task: JoinHandle<()>,
    pub playback: SharedPlayback,
}

struct RuntimeState {
    epoch: u64,
    phase: VoicePhase,
    status: String,
    consent_epoch: u64,
    participants: HashSet<u64>,
    control: Option<SessionControl>,
}

#[derive(Debug, Clone)]
pub struct VoiceSnapshot {
    pub epoch: u64,
    pub phase: VoicePhase,
    pub status: String,
    pub consent_epoch: u64,
    pub participant_count: usize,
    pub dropped_input: u64,
    pub aborted_overruns: u64,
    pub barge_ins: u64,
    pub completed_turns: u64,
}

pub struct VoiceRuntime {
    pub config: VoiceConfig,
    pub transition: Mutex<()>,
    current_epoch: AtomicU64,
    media_epoch: AtomicU64,
    start_generation: AtomicU64,
    dropped_input: AtomicU64,
    aborted_overruns: AtomicU64,
    barge_ins: AtomicU64,
    completed_turns: AtomicU64,
    inner: Mutex<RuntimeState>,
}

impl VoiceRuntime {
    #[must_use]
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            transition: Mutex::new(()),
            current_epoch: AtomicU64::new(0),
            media_epoch: AtomicU64::new(0),
            start_generation: AtomicU64::new(0),
            dropped_input: AtomicU64::new(0),
            aborted_overruns: AtomicU64::new(0),
            barge_ins: AtomicU64::new(0),
            completed_turns: AtomicU64::new(0),
            inner: Mutex::new(RuntimeState {
                epoch: 0,
                phase: VoicePhase::Disconnected,
                status: "configured; disconnected".into(),
                consent_epoch: 0,
                participants: HashSet::new(),
                control: None,
            }),
        }
    }

    #[must_use]
    pub fn is_current(&self, epoch: u64) -> bool {
        self.current_epoch.load(Ordering::SeqCst) == epoch
    }

    /// True only after public disclosure and final membership verification for
    /// this exact session. Songbird's self-deafen flag is cosmetic, so receive
    /// callbacks and playback use this software gate as the authority.
    #[must_use]
    pub fn media_enabled(&self, epoch: u64) -> bool {
        self.is_current(epoch) && self.media_epoch.load(Ordering::SeqCst) == epoch
    }

    /// Timing-critical receive callbacks use this synchronous compare/exchange
    /// to close the media gate, then schedule slower task/call cleanup.
    pub fn revoke_media(&self, epoch: u64) -> bool {
        self.media_epoch
            .compare_exchange(epoch, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Reserve one potentially slow start attempt. Leave/pause/replacement
    /// invalidates this token without having to wait for model preflight.
    pub fn reserve_start(&self) -> u64 {
        self.start_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn cancel_pending_start(&self) {
        self.start_generation.fetch_add(1, Ordering::SeqCst);
    }

    #[must_use]
    pub fn start_is_current(&self, generation: u64) -> bool {
        self.start_generation.load(Ordering::SeqCst) == generation
    }

    pub async fn begin(&self, participants: HashSet<u64>) -> u64 {
        self.media_epoch.store(0, Ordering::SeqCst);
        let epoch = self.current_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.dropped_input.store(0, Ordering::Relaxed);
        self.aborted_overruns.store(0, Ordering::Relaxed);
        self.barge_ins.store(0, Ordering::Relaxed);
        self.completed_turns.store(0, Ordering::Relaxed);
        let mut inner = self.inner.lock().await;
        inner.epoch = epoch;
        inner.phase = VoicePhase::Connecting;
        inner.status = "joining Discord voice safely".into();
        inner.consent_epoch = inner.consent_epoch.saturating_add(1);
        inner.participants = participants;
        epoch
    }

    pub async fn activate(&self, epoch: u64, status: impl Into<String>) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.epoch != epoch || !self.is_current(epoch) {
            return false;
        }
        inner.phase = VoicePhase::Listening;
        inner.status = bounded_status(status.into());
        self.media_epoch.store(epoch, Ordering::SeqCst);
        true
    }

    /// Close an active consent epoch before any caller waits on the Discord
    /// transition lock. The epoch CAS prevents a stale gateway/audio event from
    /// pausing a replacement session.
    pub async fn pause_epoch_for_consent(
        &self,
        epoch: u64,
        participants: HashSet<u64>,
        status: impl Into<String>,
    ) -> bool {
        if self
            .current_epoch
            .compare_exchange(
                epoch,
                epoch.saturating_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return false;
        }
        let _ = self
            .media_epoch
            .compare_exchange(epoch, 0, Ordering::SeqCst, Ordering::SeqCst);
        self.start_generation.fetch_add(1, Ordering::SeqCst);
        let control = {
            let mut inner = self.inner.lock().await;
            if inner.epoch != epoch {
                return false;
            }
            inner.epoch = epoch.saturating_add(1);
            inner.phase = VoicePhase::AwaitingConsent;
            inner.status = bounded_status(status.into());
            inner.participants = participants;
            inner.control.take()
        };
        if let Some(control) = control {
            stop_control(control).await;
        }
        true
    }

    pub async fn install_control(&self, epoch: u64, control: SessionControl) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.epoch != epoch || !self.is_current(epoch) {
            drop(inner);
            stop_control(control).await;
            return false;
        }
        inner.control = Some(control);
        true
    }

    pub async fn set_status(&self, epoch: u64, phase: VoicePhase, status: impl Into<String>) {
        if !self.is_current(epoch) {
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.epoch == epoch && self.is_current(epoch) {
            inner.phase = phase;
            inner.status = bounded_status(status.into());
        }
    }

    pub async fn set_presence(&self, status: impl Into<String>) {
        self.stop_to(VoicePhase::PresenceOnly, status).await;
    }

    pub async fn pause_for_consent(&self, participants: HashSet<u64>) {
        self.stop_to(
            VoicePhase::AwaitingConsent,
            "audio stopped; a new participant requires renewed consent",
        )
        .await;
        let mut inner = self.inner.lock().await;
        inner.participants = participants;
    }

    pub async fn disconnect(&self, status: impl Into<String>) {
        self.stop_to(VoicePhase::Disconnected, status).await;
    }

    /// Stop the installed actor/call state while preserving the caller's
    /// already-reserved start token. Must be used only under `transition`.
    pub async fn disconnect_for_replace(&self, status: impl Into<String>) {
        self.stop_to_inner(VoicePhase::Disconnected, status, false)
            .await;
    }

    pub async fn fail_safe(&self, status: impl Into<String>) {
        self.stop_to(VoicePhase::Failed, status).await;
    }

    async fn stop_to(&self, phase: VoicePhase, status: impl Into<String>) {
        self.stop_to_inner(phase, status, true).await;
    }

    async fn stop_to_inner(
        &self,
        phase: VoicePhase,
        status: impl Into<String>,
        cancel_pending_start: bool,
    ) {
        self.media_epoch.store(0, Ordering::SeqCst);
        if cancel_pending_start {
            self.start_generation.fetch_add(1, Ordering::SeqCst);
        }
        let epoch = self.current_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let control = {
            let mut inner = self.inner.lock().await;
            inner.epoch = epoch;
            inner.phase = phase;
            inner.status = bounded_status(status.into());
            inner.control.take()
        };
        if let Some(control) = control {
            stop_control(control).await;
        }
    }

    pub async fn snapshot(&self) -> VoiceSnapshot {
        let inner = self.inner.lock().await;
        VoiceSnapshot {
            epoch: inner.epoch,
            phase: inner.phase,
            status: inner.status.clone(),
            consent_epoch: inner.consent_epoch,
            participant_count: inner.participants.len(),
            dropped_input: self.dropped_input.load(Ordering::Relaxed),
            aborted_overruns: self.aborted_overruns.load(Ordering::Relaxed),
            barge_ins: self.barge_ins.load(Ordering::Relaxed),
            completed_turns: self.completed_turns.load(Ordering::Relaxed),
        }
    }

    pub async fn phase(&self) -> VoicePhase {
        self.inner.lock().await.phase
    }

    /// Gateway callbacks are independently spawned and may arrive after a new
    /// command/session has already attested the same member. Correlate both the
    /// epoch and immutable participant set before treating an old join event as
    /// a new-consent boundary.
    pub async fn epoch_attests(&self, epoch: u64, user_id: u64) -> bool {
        let inner = self.inner.lock().await;
        inner.epoch == epoch && self.is_current(epoch) && inner.participants.contains(&user_id)
    }

    pub fn note_dropped_input(&self) {
        self.dropped_input.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_overrun(&self) {
        self.aborted_overruns.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_barge_in(&self) {
        self.barge_ins.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_completed_turn(&self) {
        self.completed_turns.fetch_add(1, Ordering::Relaxed);
    }
}

async fn stop_control(control: SessionControl) {
    let _ = control.cancel.send(true);
    let mut task = control.task;
    let mut task_reaped = false;
    let track =
        match tokio::time::timeout(Duration::from_millis(250), control.playback.lock()).await {
            Ok(mut playback) => playback.take(),
            Err(_) => {
                task.abort();
                let _ = (&mut task).await;
                task_reaped = true;
                tokio::time::timeout(Duration::from_millis(250), control.playback.lock())
                    .await
                    .ok()
                    .and_then(|mut playback| playback.take())
            }
        };
    if let Some(track) = track {
        let _ = track.stop();
    }
    if !task_reaped
        && tokio::time::timeout(Duration::from_secs(2), &mut task)
            .await
            .is_err()
    {
        task.abort();
        let _ = task.await;
    }
}

fn bounded_status(status: String) -> String {
    let flattened = status.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut bounded: String = flattened.chars().take(240).collect();
    if flattened.chars().count() > 240 {
        bounded.push('…');
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::{VoiceBackendConfig, VoiceConfig};

    fn runtime() -> VoiceRuntime {
        VoiceRuntime::new(VoiceConfig {
            guild_id: 1,
            channel_id: 2,
            backend: VoiceBackendConfig::Disabled,
            wake_word_required: true,
        })
    }

    #[tokio::test]
    async fn stale_epoch_cannot_overwrite_disconnect() {
        let runtime = runtime();
        let epoch = runtime.begin(HashSet::new()).await;
        runtime.disconnect("left").await;
        runtime
            .set_status(epoch, VoicePhase::Speaking, "stale")
            .await;
        let status = runtime.snapshot().await;
        assert_eq!(status.phase, VoicePhase::Disconnected);
        assert_eq!(status.status, "left");
        assert!(!runtime.media_enabled(epoch));
    }

    #[tokio::test]
    async fn media_gate_requires_activation_for_the_current_epoch() {
        let runtime = runtime();
        let epoch = runtime.begin(HashSet::new()).await;
        assert!(!runtime.media_enabled(epoch));
        assert!(runtime.activate(epoch, "ready").await);
        assert!(runtime.media_enabled(epoch));
        runtime.pause_for_consent(HashSet::new()).await;
        assert!(!runtime.media_enabled(epoch));
    }

    #[test]
    fn status_is_flattened_and_bounded() {
        let status = bounded_status(format!("line one\n{}", "x".repeat(500)));
        assert!(!status.contains('\n'));
        assert!(status.chars().count() <= 241);
    }
}
