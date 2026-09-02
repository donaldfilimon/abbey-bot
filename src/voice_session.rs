//! Serialized lifecycle and observability for one Discord voice session.
//!
//! Provider tasks carry an epoch. Rejoin, pause, leave, bot moves, and shutdown
//! advance it before cancellation, so late work cannot overwrite status or
//! publish stale audio. The process owns exactly one provider task and one
//! playback handle at a time.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::Duration;

use songbird::tracks::TrackHandle;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::inspect::{VoiceInspectRegistry, VoiceInspectState};
use crate::voice::VoiceConfig;

mod control;
mod playback;
mod verification;

pub use control::{authoritative_text_reply, requests_consent_withdrawal};
pub use playback::{PlaybackTermination, register_playback_termination};
pub use verification::VerificationActivation;
use verification::VerificationState;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    PlaybackTerminated {
        turn: u64,
        termination: PlaybackTermination,
    },
}

impl VoiceRuntime {
    /// Invalidate a failed provider from inside its own task without awaiting
    /// the JoinHandle that represents that same task.
    pub async fn actor_failed(&self, epoch: u64, status: impl Into<String>) -> bool {
        self.actor_stop_to(epoch, VoicePhase::Failed, status).await
    }

    /// Close consent from inside the local actor after an attributed speaker
    /// explicitly withdraws. This mirrors `actor_failed`'s self-JoinHandle
    /// handling but preserves the recoverable AwaitingConsent phase.
    pub async fn actor_awaiting_consent(&self, epoch: u64, status: impl Into<String>) -> bool {
        self.actor_stop_to(epoch, VoicePhase::AwaitingConsent, status)
            .await
    }

    async fn actor_stop_to(
        &self,
        epoch: u64,
        phase: VoicePhase,
        status: impl Into<String>,
    ) -> bool {
        let control = {
            let mut inner = self.inner.lock().await;
            let _activation = self
                .activation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inner.epoch != epoch || self.current_epoch.load(Ordering::SeqCst) != epoch {
                return false;
            }
            let next_epoch = epoch.saturating_add(1);
            self.media_epoch.store(0, Ordering::SeqCst);
            let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
            self.pending_start_generation.store(0, Ordering::SeqCst);
            self.start_changes.send_replace(generation);
            let mut sessions = self
                .discord_sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            sessions.retire_current();
            sessions.attested_epoch = next_epoch;
            sessions.attested.clear();
            inner.epoch = next_epoch;
            inner.phase = phase;
            inner.status = bounded_status(status.into());
            self.current_epoch.store(next_epoch, Ordering::SeqCst);
            self.publish_inspect_phase(phase, false);
            inner.control.take()
        };
        if let Some(control) = control {
            let _ = control.cancel.send(true);
            // This path runs inside `control.task` itself. Never await a lock
            // the actor may currently hold; take and stop playback when the
            // lock is immediately available, and let the actor's normal exit
            // cleanup handle the rare contended case.
            if let Ok(mut playback) = control.playback.try_lock()
                && let Some(track) = playback.take()
            {
                let _ = track.stop();
            }
            // Actor-triggered teardown is called by `control.task` itself.
            // Dropping its handle detaches it; the caller returns naturally.
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

/// Actor resources detached from an epoch whose media gate and public phase
/// are already closed. Discord shells can stop the exact Decode driver first,
/// then reap these resources without a state-truth race.
pub struct ConsentPause {
    control: Option<SessionControl>,
}

impl ConsentPause {
    pub async fn finish(self) {
        if let Some(control) = self.control {
            stop_control(control).await;
        }
    }
}

struct RuntimeState {
    epoch: u64,
    phase: VoicePhase,
    status: String,
    consent_epoch: u64,
    participants: HashSet<u64>,
    control: Option<SessionControl>,
}

struct VoiceInspectBinding {
    registry: Arc<VoiceInspectRegistry>,
    scoped_guild_id: String,
}

impl VoiceInspectBinding {
    fn publish(&self, state: VoiceInspectState) {
        self.registry.publish(&self.scoped_guild_id, state);
    }

    fn mark_media_revoked(&self) {
        self.registry.mark_media_revoked(&self.scoped_guild_id);
    }

    fn mark_session_adverse(&self) {
        self.registry.mark_session_adverse(&self.scoped_guild_id);
    }
}

const RETIRED_DISCORD_SESSIONS: usize = 8;

#[derive(Default)]
struct DiscordSessions {
    current: Option<(u64, String)>,
    retired: VecDeque<String>,
    attested_epoch: u64,
    attested: HashSet<u64>,
}

impl DiscordSessions {
    fn retire_current(&mut self) {
        if let Some((_, session_id)) = self.current.take() {
            self.remember_retired(session_id);
        }
    }

    fn remember_retired(&mut self, session_id: String) {
        if self.retired.iter().any(|retired| retired == &session_id) {
            return;
        }
        self.retired.push_back(session_id);
        while self.retired.len() > RETIRED_DISCORD_SESSIONS {
            self.retired.pop_front();
        }
    }
}

/// Result of atomically correlating an adverse bot VoiceStateUpdate payload
/// with the Discord session bound to the current runtime epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordSessionEvent {
    Current { epoch: u64, media_was_enabled: bool },
    Retired,
    Unknown { epoch: u64, media_was_enabled: bool },
}

#[derive(Debug, Clone)]
pub struct VoiceSnapshot {
    pub epoch: u64,
    pub phase: VoicePhase,
    pub media_enabled: bool,
    pub start_pending: bool,
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
    pending_start_generation: AtomicU64,
    // Lock order is `inner` (when needed) -> `activation_gate` ->
    // `discord_sessions`. Synchronous gateway/data-plane paths take only the
    // latter two and release them before awaiting lifecycle cleanup.
    activation_gate: SyncMutex<()>,
    start_changes: watch::Sender<u64>,
    discord_sessions: SyncMutex<DiscordSessions>,
    dropped_input: AtomicU64,
    aborted_overruns: AtomicU64,
    barge_ins: AtomicU64,
    completed_turns: AtomicU64,
    verification: SyncMutex<VerificationState>,
    inspect: Option<VoiceInspectBinding>,
    inner: Mutex<RuntimeState>,
}

impl VoiceRuntime {
    #[must_use]
    pub fn new(config: VoiceConfig) -> Self {
        Self::build(config, None)
    }

    #[must_use]
    pub fn new_with_inspect(config: VoiceConfig, registry: Arc<VoiceInspectRegistry>) -> Self {
        let guild_id = config.guild_id.to_string();
        let inspect = VoiceInspectBinding {
            registry,
            scoped_guild_id: crate::guild::scoped_guild_id("discord", Some(&guild_id)),
        };
        let runtime = Self::build(config, Some(inspect));
        runtime.publish_inspect(VoiceInspectState::Off);
        runtime
    }

    fn build(config: VoiceConfig, inspect: Option<VoiceInspectBinding>) -> Self {
        let (start_changes, _) = watch::channel(0);
        Self {
            config,
            transition: Mutex::new(()),
            current_epoch: AtomicU64::new(0),
            media_epoch: AtomicU64::new(0),
            start_generation: AtomicU64::new(0),
            pending_start_generation: AtomicU64::new(0),
            activation_gate: SyncMutex::new(()),
            start_changes,
            discord_sessions: SyncMutex::new(DiscordSessions::default()),
            dropped_input: AtomicU64::new(0),
            aborted_overruns: AtomicU64::new(0),
            barge_ins: AtomicU64::new(0),
            completed_turns: AtomicU64::new(0),
            verification: SyncMutex::new(VerificationState::default()),
            inspect,
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

    fn publish_inspect(&self, state: VoiceInspectState) {
        if let Some(inspect) = &self.inspect {
            inspect.publish(state);
        }
    }

    fn mark_inspect_media_revoked(&self) {
        if let Some(inspect) = &self.inspect {
            inspect.mark_media_revoked();
        }
    }

    fn mark_inspect_session_adverse(&self) {
        if let Some(inspect) = &self.inspect {
            inspect.mark_session_adverse();
        }
    }

    fn publish_inspect_phase(&self, phase: VoicePhase, media_enabled: bool) {
        let state = match phase {
            VoicePhase::Disconnected => VoiceInspectState::Off,
            VoicePhase::PresenceOnly => VoiceInspectState::Presence,
            VoicePhase::AwaitingConsent => VoiceInspectState::AwaitingConsent,
            VoicePhase::Listening | VoicePhase::Thinking | VoicePhase::Speaking
                if media_enabled =>
            {
                VoiceInspectState::Active
            }
            VoicePhase::Connecting
            | VoicePhase::Listening
            | VoicePhase::Thinking
            | VoicePhase::Speaking
            | VoicePhase::Failed => VoiceInspectState::Paused,
        };
        self.publish_inspect(state);
    }

    #[must_use]
    pub fn is_current(&self, epoch: u64) -> bool {
        self.current_epoch.load(Ordering::SeqCst) == epoch
    }

    #[must_use]
    pub fn current_epoch(&self) -> u64 {
        self.current_epoch.load(Ordering::SeqCst)
    }

    /// True only after public disclosure and final membership verification for
    /// this exact session. Songbird's self-deafen flag is cosmetic, so receive
    /// callbacks and playback use this software gate as the authority.
    #[must_use]
    pub fn media_enabled(&self, epoch: u64) -> bool {
        epoch != 0 && self.is_current(epoch) && self.media_epoch.load(Ordering::SeqCst) == epoch
    }

    /// Timing-critical receive callbacks use this synchronous compare/exchange
    /// to close the media gate, then schedule slower task/call cleanup.
    pub fn revoke_media(&self, epoch: u64) -> bool {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let revoked = self
            .media_epoch
            .compare_exchange(epoch, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if revoked {
            self.mark_inspect_media_revoked();
        }
        revoked
    }

    /// Linearize a short, non-async media side effect with activation and
    /// revocation. Callers may prepare or acquire their own resources first,
    /// but playback start and durable commit must occur inside this closure so
    /// neither can begin after the consent gate closes.
    pub fn with_media_enabled<T>(&self, epoch: u64, action: impl FnOnce() -> T) -> Option<T> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if epoch == 0
            || self.current_epoch.load(Ordering::SeqCst) != epoch
            || self.media_epoch.load(Ordering::SeqCst) != epoch
        {
            return None;
        }
        Some(action())
    }

    /// Reserve one potentially slow start attempt. Leave/pause/replacement
    /// invalidates this token without having to wait for model preflight.
    pub fn reserve_start(&self) -> u64 {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation
            .store(generation, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        generation
    }

    /// Capture the lifecycle generation before a validated start performs its
    /// first await. This is deliberately not a pending-start reservation:
    /// channel and permission validation may still reject the request without
    /// superseding a legitimate preflight already in progress.
    #[must_use]
    pub fn start_operation_token(&self) -> u64 {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.start_generation.load(Ordering::SeqCst)
    }

    /// Publish a pending start only if no stop, withdrawal, safety event, or
    /// newer start crossed the caller's pre-await operation token. The check
    /// and reservation share the activation lock with cancellation, so an
    /// older request cannot resume after `/voice leave` and become a new start.
    pub fn reserve_start_if_unchanged(&self, operation_token: u64) -> Option<u64> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.start_generation.load(Ordering::SeqCst) != operation_token {
            return None;
        }
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation
            .store(generation, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        Some(generation)
    }

    /// Cancel a pending start and synchronously close any media gate it may
    /// have just opened. The same non-async mutex guards activation's final
    /// generation check and media store, so a stop request can never be
    /// followed by a stale start reopening media.
    pub fn cancel_pending_start(&self) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.media_epoch.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_media_revoked();
    }

    /// Clear a completed/failed start reservation without invalidating a newer
    /// attempt that replaced it.
    pub fn finish_start_attempt(&self, generation: u64) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self
            .pending_start_generation
            .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.start_changes.send_replace(0);
        }
    }

    #[must_use]
    pub fn start_is_current(&self, generation: u64) -> bool {
        self.start_generation.load(Ordering::SeqCst) == generation
            && self.pending_start_generation.load(Ordering::SeqCst) == generation
    }

    /// Resolve as soon as replacement, leave, a gateway safety event, or any
    /// other transition invalidates this slow start attempt.
    pub async fn wait_for_start_cancellation(&self, generation: u64) {
        let mut changes = self.start_changes.subscribe();
        while self.start_is_current(generation) {
            if changes.changed().await.is_err() {
                break;
            }
        }
    }

    /// Bind Discord's opaque session id to the exact connecting runtime epoch.
    /// A later epoch automatically retires the binding so delayed disconnects
    /// can be distinguished from adverse events for the live call.
    pub async fn bind_discord_session(&self, epoch: u64, session_id: String) -> bool {
        let inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.epoch != epoch
            || inner.phase != VoicePhase::Connecting
            || self.current_epoch.load(Ordering::SeqCst) != epoch
        {
            return false;
        }
        let mut sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((bound_epoch, bound_id)) = sessions.current.as_ref() {
            if *bound_epoch == epoch && bound_id == &session_id {
                return true;
            }
            sessions.retire_current();
        }
        sessions.current = Some((epoch, session_id));
        true
    }

    /// Record a session Discord is retiring even when it belonged to
    /// no-audio presence rather than a conversational runtime epoch.
    pub fn remember_retired_discord_session(&self, session_id: String) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remember_retired(session_id);
    }

    /// Preserve and classify the actual adverse gateway payload, and close the
    /// media/start gates in the same synchronous critical section. The caller
    /// may then await Discord/actor cleanup without a transient recovery in the
    /// cache erasing the event that required revocation.
    #[must_use]
    pub fn revoke_for_discord_session(&self, session_id: &str) -> DiscordSessionEvent {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        let sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let relation = if sessions
            .current
            .as_ref()
            .is_some_and(|(bound_epoch, bound_id)| *bound_epoch == epoch && bound_id == session_id)
        {
            0_u8
        } else if sessions.retired.iter().any(|retired| retired == session_id) {
            1
        } else {
            2
        };
        if relation == 1 {
            return DiscordSessionEvent::Retired;
        }
        let media_was_enabled = epoch != 0 && self.media_epoch.swap(0, Ordering::SeqCst) == epoch;
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_session_adverse();
        if relation == 0 {
            DiscordSessionEvent::Current {
                epoch,
                media_was_enabled,
            }
        } else {
            DiscordSessionEvent::Unknown {
                epoch,
                media_was_enabled,
            }
        }
    }

    /// Synchronously close the current media/start gates for a payload-backed
    /// permission or participant event that has no bot session id of its own.
    #[must_use]
    pub fn revoke_for_external_event(&self) -> u64 {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        self.media_epoch.store(0, Ordering::SeqCst);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_session_adverse();
        epoch
    }

    /// Atomically decide whether a participant join belongs to the immutable
    /// attestation for the current epoch. A delayed event for someone already
    /// attested to a replacement is ignored; every other join closes that
    /// replacement's gates before gateway cleanup awaits.
    #[must_use]
    pub fn revoke_for_unattested_participant(&self, user_id: u64) -> Option<u64> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        let sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sessions.attested_epoch == epoch && sessions.attested.contains(&user_id) {
            return None;
        }
        self.media_epoch.store(0, Ordering::SeqCst);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_media_revoked();
        Some(epoch)
    }

    /// Advance an exact active epoch while atomically closing media and any
    /// pending start. Sharing `activation_gate` with `activate` makes the
    /// epoch check and media transition indivisible with respect to a final
    /// activation attempt.
    pub async fn begin(&self, participants: HashSet<u64>) -> u64 {
        let mut inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst).saturating_add(1);
        self.media_epoch.store(0, Ordering::SeqCst);
        let mut sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions.retire_current();
        sessions.attested_epoch = epoch;
        sessions.attested = participants.clone();
        self.dropped_input.store(0, Ordering::Relaxed);
        self.aborted_overruns.store(0, Ordering::Relaxed);
        self.barge_ins.store(0, Ordering::Relaxed);
        self.completed_turns.store(0, Ordering::Relaxed);
        inner.epoch = epoch;
        inner.phase = VoicePhase::Connecting;
        inner.status = "joining Discord voice safely".into();
        inner.consent_epoch = inner.consent_epoch.saturating_add(1);
        inner.participants = participants;
        self.current_epoch.store(epoch, Ordering::SeqCst);
        self.publish_inspect_phase(VoicePhase::Connecting, false);
        epoch
    }

    pub async fn activate(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
    ) -> bool {
        self.activate_inner(epoch, start_generation, status, None)
            .await
    }

    /// Open media and publish content-free verifier activation evidence in
    /// the same critical section as the lifecycle transition.
    pub async fn activate_verified(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
        evidence: VerificationActivation,
    ) -> bool {
        self.activate_inner(epoch, start_generation, status, Some(evidence))
            .await
    }

    async fn activate_inner(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
        verification: Option<VerificationActivation>,
    ) -> bool {
        let status = bounded_status(status.into());
        let mut inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.epoch != epoch
            || inner.phase != VoicePhase::Connecting
            || !self.is_current(epoch)
            || !self.start_is_current(start_generation)
        {
            return false;
        }
        inner.phase = VoicePhase::Listening;
        inner.status = status;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.media_epoch.store(epoch, Ordering::SeqCst);
        self.publish_inspect_phase(VoicePhase::Listening, true);
        if let Some(evidence) = verification {
            self.record_verification_activation(evidence, inner.consent_epoch);
        }
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
        let Some(pause) = self
            .begin_pause_epoch_for_consent(epoch, participants, status)
            .await
        else {
            return false;
        };
        pause.finish().await;
        true
    }

    /// Close the exact epoch and publish AwaitingConsent before a Discord Call
    /// is left. This split keeps the bot's own disconnect callback truthful and
    /// lets the shell stop Decode before bounded actor/task reaping.
    pub async fn begin_pause_epoch_for_consent(
        &self,
        epoch: u64,
        participants: HashSet<u64>,
        status: impl Into<String>,
    ) -> Option<ConsentPause> {
        self.begin_pause_epoch_for_consent_inner(epoch, participants, status, false)
            .await
    }

    /// Close an exact epoch and publish a participant-change milestone in the
    /// same critical section. This cannot race a verified activation.
    pub async fn begin_participant_pause_epoch_for_consent(
        &self,
        epoch: u64,
        participants: HashSet<u64>,
        status: impl Into<String>,
    ) -> Option<ConsentPause> {
        self.begin_pause_epoch_for_consent_inner(epoch, participants, status, true)
            .await
    }

    async fn begin_pause_epoch_for_consent_inner(
        &self,
        epoch: u64,
        participants: HashSet<u64>,
        status: impl Into<String>,
        participant_change: bool,
    ) -> Option<ConsentPause> {
        let control = {
            let mut inner = self.inner.lock().await;
            let _activation = self
                .activation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inner.epoch != epoch || self.current_epoch.load(Ordering::SeqCst) != epoch {
                return None;
            }
            if participant_change {
                self.record_verification_participant_pause(participants.len());
            }
            let next_epoch = epoch.saturating_add(1);
            self.media_epoch.store(0, Ordering::SeqCst);
            let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
            self.pending_start_generation.store(0, Ordering::SeqCst);
            self.start_changes.send_replace(generation);
            let mut sessions = self
                .discord_sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            sessions.retire_current();
            sessions.attested_epoch = next_epoch;
            sessions.attested = participants.clone();
            inner.epoch = next_epoch;
            inner.phase = VoicePhase::AwaitingConsent;
            inner.status = bounded_status(status.into());
            inner.participants = participants;
            self.current_epoch.store(next_epoch, Ordering::SeqCst);
            self.publish_inspect_phase(VoicePhase::AwaitingConsent, false);
            inner.control.take()
        };
        // Stop in-flight STT/LLM/TTS/provider work as soon as the consent
        // epoch closes. `finish` still owns bounded playback stop and task
        // reaping after the shell has left the exact Decode call.
        if let Some(control) = control.as_ref() {
            let _ = control.cancel.send(true);
        }
        Some(ConsentPause { control })
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
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.epoch == epoch && self.current_epoch.load(Ordering::SeqCst) == epoch {
            inner.phase = phase;
            inner.status = bounded_status(status.into());
            self.publish_inspect_phase(phase, self.media_epoch.load(Ordering::SeqCst) == epoch);
        }
    }

    /// Provider readiness can arrive before command-side membership and
    /// permission checks finish. Record that readiness without claiming the
    /// session is listening; only `activate` may open media and leave
    /// `Connecting`.
    pub async fn set_prepared_status(&self, epoch: u64, status: impl Into<String>) {
        if !self.is_current(epoch) {
            return;
        }
        let mut inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.epoch == epoch
            && inner.phase == VoicePhase::Connecting
            && self.media_epoch.load(Ordering::SeqCst) != epoch
        {
            inner.status = bounded_status(status.into());
            self.publish_inspect_phase(VoicePhase::Connecting, false);
        }
    }

    pub async fn set_presence_with_discord_session(
        &self,
        session_id: String,
        status: impl Into<String>,
    ) {
        self.stop_to_inner(
            VoicePhase::PresenceOnly,
            status,
            true,
            None,
            Some(session_id),
        )
        .await;
    }

    pub async fn pause_for_consent(&self, participants: HashSet<u64>) {
        self.stop_to_inner(
            VoicePhase::AwaitingConsent,
            "voice disconnected; renewed participant consent is required",
            true,
            Some(participants),
            None,
        )
        .await;
    }

    pub async fn disconnect(&self, status: impl Into<String>) {
        self.stop_to(VoicePhase::Disconnected, status).await;
    }

    /// Stop the installed actor/call state while preserving the caller's
    /// already-reserved start token. Must be used only under `transition`.
    pub async fn disconnect_for_replace(&self, status: impl Into<String>) {
        self.stop_to_inner(VoicePhase::Disconnected, status, false, None, None)
            .await;
    }

    pub async fn fail_safe(&self, status: impl Into<String>) {
        self.stop_to(VoicePhase::Failed, status).await;
    }

    async fn stop_to(&self, phase: VoicePhase, status: impl Into<String>) {
        self.stop_to_inner(phase, status, true, None, None).await;
    }

    async fn stop_to_inner(
        &self,
        phase: VoicePhase,
        status: impl Into<String>,
        cancel_pending_start: bool,
        participants: Option<HashSet<u64>>,
        discord_session_id: Option<String>,
    ) {
        let control = {
            let mut inner = self.inner.lock().await;
            let _activation = self
                .activation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.media_epoch.store(0, Ordering::SeqCst);
            if cancel_pending_start {
                let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
                self.pending_start_generation.store(0, Ordering::SeqCst);
                self.start_changes.send_replace(generation);
            }
            let epoch = self.current_epoch.load(Ordering::SeqCst).saturating_add(1);
            let mut sessions = self
                .discord_sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(session_id) = discord_session_id {
                let same_session = sessions
                    .current
                    .as_ref()
                    .is_some_and(|(_, current)| current == &session_id);
                if !same_session {
                    sessions.retire_current();
                }
                sessions.current = Some((epoch, session_id));
            } else {
                sessions.retire_current();
            }
            sessions.attested_epoch = epoch;
            sessions.attested = participants.clone().unwrap_or_default();
            inner.epoch = epoch;
            inner.phase = phase;
            inner.status = bounded_status(status.into());
            if let Some(participants) = participants {
                inner.participants = participants;
            }
            self.current_epoch.store(epoch, Ordering::SeqCst);
            self.publish_inspect_phase(phase, false);
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
            media_enabled: self.media_enabled(inner.epoch),
            start_pending: self.pending_start_generation.load(Ordering::SeqCst) != 0,
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
    pub fn epoch_attests_now(&self, epoch: u64, user_id: u64) -> bool {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.current_epoch.load(Ordering::SeqCst) == epoch
            && sessions.attested_epoch == epoch
            && sessions.attested.contains(&user_id)
    }

    pub async fn epoch_attests(&self, epoch: u64, user_id: u64) -> bool {
        self.epoch_attests_now(epoch, user_id)
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
mod tests;
