//! Serialized lifecycle and observability for one Discord voice session.
//!
//! Provider tasks carry an epoch. Rejoin, pause, leave, bot moves, and shutdown
//! advance it before cancellation, so late work cannot overwrite status or
//! publish stale audio. The process owns exactly one provider task and one
//! playback handle at a time.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex, OnceLock};
use std::time::Duration;

use songbird::tracks::TrackHandle;
use tokio::sync::{Mutex, watch};

use crate::inspect::{VoiceInspectRegistry, VoiceInspectState};
use crate::voice::{VoiceBackendConfig, VoiceConfig, VoiceMode};

mod activation;
mod control;
mod music;
mod ownership;
mod playback;
mod verification;
pub use ownership::VoiceTask;

pub use control::{authoritative_text_reply, requests_consent_withdrawal, withdrawal_requested};
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

    /// No media epoch can be open and no join is in flight: `Disconnected`,
    /// the muted/self-deafened autojoin presence (which never decodes,
    /// whatever the backend), and `Failed` (media closed by `fail_safe`).
    /// `AwaitingConsent` is excluded on purpose: a paused session still owns
    /// the consent it was granted under, and a resume must honour it.
    #[must_use]
    pub const fn accepts_backend_change(self) -> bool {
        matches!(self, Self::Disconnected | Self::PresenceOnly | Self::Failed)
    }

    #[must_use]
    pub const fn processes_audio(self) -> bool {
        matches!(self, Self::Listening | Self::Thinking | Self::Speaking)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    MusicTerminated {
        reason: PlaybackTermination,
    },
    PlaybackTerminated {
        turn: u64,
        termination: PlaybackTermination,
    },
}

/// Closed-vocabulary outcome for a voice phase.
///
/// `PresenceOnly` and `Listening` are deliberately distinct: presence-only is
/// connected but deaf and mute, while `Listening` is the only phase that can
/// actually hear a participant. Reporting both as `Ready` hid exactly the
/// difference an operator asks about when checking whether voice is on.
#[must_use]
pub(crate) fn phase_outcome(phase: VoicePhase) -> crate::observability::EventOutcome {
    use crate::observability::EventOutcome;
    match phase {
        VoicePhase::Disconnected => EventOutcome::Stopped,
        VoicePhase::PresenceOnly => EventOutcome::Degraded,
        VoicePhase::Listening => EventOutcome::Ready,
        VoicePhase::Connecting | VoicePhase::Thinking | VoicePhase::Speaking => {
            EventOutcome::Started
        }
        VoicePhase::AwaitingConsent => EventOutcome::Skipped,
        VoicePhase::Failed => EventOutcome::Failed,
    }
}

/// An error category is evidence about a failure, so it is carried only by the
/// `Failed` phase; any other phase reports none even if a caller supplied one.
#[must_use]
pub(crate) fn phase_error(
    phase: VoicePhase,
    error: Option<crate::observability::OperationalErrorCategory>,
) -> Option<crate::observability::OperationalErrorCategory> {
    match phase {
        VoicePhase::Failed => error,
        _ => None,
    }
}

impl VoiceRuntime {
    /// Invalidate a failed provider from inside its own task without awaiting
    /// the JoinHandle that represents that same task.
    pub async fn actor_failed(
        &self,
        epoch: u64,
        status: impl Into<String>,
        error: crate::observability::OperationalErrorCategory,
    ) -> bool {
        self.actor_stop_to(epoch, VoicePhase::Failed, status, Some(error))
            .await
    }

    /// Close consent from inside the local actor after an attributed speaker
    /// explicitly withdraws. This mirrors `actor_failed`'s self-JoinHandle
    /// handling but preserves the recoverable AwaitingConsent phase.
    pub async fn actor_awaiting_consent(&self, epoch: u64, status: impl Into<String>) -> bool {
        self.actor_stop_to(epoch, VoicePhase::AwaitingConsent, status, None)
            .await
    }

    async fn actor_stop_to(
        &self,
        epoch: u64,
        phase: VoicePhase,
        status: impl Into<String>,
        error: Option<crate::observability::OperationalErrorCategory>,
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
            self.publish_inspect_phase(phase, false, error);
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
    pub task: VoiceTask,
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
    processing_mode: VoiceMode,
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

/// Why a `/voice mode` switch was refused. Carrying the reason as a value
/// rather than a rendered sentence keeps one copy of each message and lets
/// tests assert on the decision instead of on prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSwitchRefusal {
    Starting,
    MediaOpen,
    Active(VoicePhase),
    VerificationArmed,
}

impl ModeSwitchRefusal {
    #[must_use]
    pub fn message(self) -> String {
        let leave = "Stop it with `/voice leave` before changing the backend.";
        match self {
            Self::Starting => format!("Voice is starting right now. {leave}"),
            Self::MediaOpen => format!("Voice is active right now. {leave}"),
            Self::Active(phase) => format!("Voice is {} right now. {leave}", phase.label()),
            Self::VerificationArmed => "A live voice verification run is armed, and it observes local inference only. Finish it with `/voice leave` before switching away from local.".to_string(),
        }
    }
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
    service: OnceLock<crate::service::OperationRegistry>,
    telemetry: OnceLock<crate::service::telemetry::TelemetryRequests>,
    draining: AtomicBool,
    pub music: music::MusicController,
    pub config: VoiceConfig,
    pub consent: Arc<crate::voice_consent_store::ConsentStore>,
    /// The mode in force right now, which `/voice mode` may change while the
    /// call is disconnected. Deliberately an atomic rather than a lock: every
    /// read is a copy-out, so this can never participate in the
    /// `inner -> activation_gate -> discord_sessions` order documented below,
    /// and no actor can await while "holding" it.
    selected_mode: AtomicU8,
    pub transition: Mutex<()>,
    current_epoch: AtomicU64,
    music_ready_epoch: AtomicU64,
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

pub struct ConsentChange {
    pub epoch_to_stop: Option<u64>,
    pub saved: tokio::sync::oneshot::Receiver<Result<bool, &'static str>>,
}

/// `VoiceMode` as a stable byte, so the mode in force can live in an atomic
/// instead of a lock. The mapping is private and only ever round-tripped
/// through the two functions below.
const fn mode_code(mode: VoiceMode) -> u8 {
    match mode {
        VoiceMode::Disabled => 0,
        VoiceMode::Local => 1,
        VoiceMode::OpenAi => 2,
    }
}

/// Inverse of [`mode_code`]. Only [`mode_code`] ever writes the byte, so an
/// unknown value is impossible; it degrades to `Disabled` rather than panicking
/// because failing closed is the right answer for a voice gate.
const fn mode_from_code(code: u8) -> VoiceMode {
    match code {
        1 => VoiceMode::Local,
        2 => VoiceMode::OpenAi,
        _ => VoiceMode::Disabled,
    }
}

impl VoiceRuntime {
    #[cfg(test)]
    #[must_use]
    pub fn new(config: VoiceConfig) -> Self {
        let consent = Arc::new(crate::voice_consent_store::ConsentStore::load(
            None,
            config.guild_id,
        ));
        Self::build(config, None, consent)
    }

    #[must_use]
    pub fn new_with_inspect(
        config: VoiceConfig,
        registry: Arc<VoiceInspectRegistry>,
        consent: Arc<crate::voice_consent_store::ConsentStore>,
    ) -> Self {
        let guild_id = config.guild_id.to_string();
        let inspect = VoiceInspectBinding {
            registry,
            scoped_guild_id: crate::guild::scoped_guild_id("discord", Some(&guild_id)),
        };
        let runtime = Self::build(config, Some(inspect), consent);
        runtime.publish_inspect(VoiceInspectState::Off);
        runtime
    }

    fn build(
        config: VoiceConfig,
        inspect: Option<VoiceInspectBinding>,
        consent: Arc<crate::voice_consent_store::ConsentStore>,
    ) -> Self {
        let (start_changes, _) = watch::channel(0);
        let selected_mode = AtomicU8::new(mode_code(config.mode()));
        Self {
            service: OnceLock::new(),
            telemetry: OnceLock::new(),
            draining: AtomicBool::new(false),
            music: music::MusicController::default(),
            consent,
            selected_mode,
            config,
            transition: Mutex::new(()),
            current_epoch: AtomicU64::new(0),
            music_ready_epoch: AtomicU64::new(0),
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
                processing_mode: VoiceMode::Disabled,
                control: None,
            }),
        }
    }

    /// The mode in force now — the startup selection unless `/voice mode`
    /// changed it. Prefer [`VoiceRuntime::effective_backend`] when starting a
    /// call: a join must decide once and carry that decision, never re-read.
    #[must_use]
    pub fn effective_mode(&self) -> VoiceMode {
        mode_from_code(self.selected_mode.load(Ordering::SeqCst))
    }

    /// Snapshot the mode *and* its backend together, so everything one join
    /// does — the Songbird decode mode, the consent disclosure, the actor that
    /// connects, and the reply — describes the same backend. Re-reading
    /// [`VoiceRuntime::effective_mode`] at each of those points would allow a
    /// concurrent switch to tell participants "local, stays on this Mac" while
    /// the cloud actor connects.
    ///
    /// `None` means the mode in force has no usable backend, which
    /// `/voice mode` refuses to create.
    #[must_use]
    pub fn effective_backend(&self) -> Option<VoiceBackendConfig> {
        self.config.backend_for(self.effective_mode())
    }

    /// Change the mode in force. Callers must hold `transition` and must have
    /// established that no call is running or pending.
    pub fn set_effective_mode(&self, mode: VoiceMode) {
        self.selected_mode.store(mode_code(mode), Ordering::SeqCst);
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

    /// `error` is carried only on the `Failed` phase. Managed runs disable
    /// tracing entirely (`EnvFilter::new("off")`), so this closed event is the
    /// sole operational record of why voice stopped; dropping the category
    /// left an operator with a bare `failed` and no cause.
    fn publish_inspect_phase(
        &self,
        phase: VoicePhase,
        media_enabled: bool,
        error: Option<crate::observability::OperationalErrorCategory>,
    ) {
        if let Some(events) = self.telemetry.get() {
            use crate::observability::{EventCode, EventComponent};
            let _ = events.record(
                EventComponent::Voice,
                EventCode::VoiceState,
                phase_outcome(phase),
                phase_error(phase, error),
            );
        }
        self.music.phase(phase);
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
        self.accepting_work()
            && epoch != 0
            && self.is_current(epoch)
            && self.media_epoch.load(Ordering::SeqCst) == epoch
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
        if !self.accepting_work()
            || epoch == 0
            || self.current_epoch.load(Ordering::SeqCst) != epoch
            || self.media_epoch.load(Ordering::SeqCst) != epoch
        {
            return None;
        }
        Some(action())
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
            self.publish_inspect_phase(VoicePhase::AwaitingConsent, false, None);
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
        if !self.accepting_work() || inner.epoch != epoch || !self.is_current(epoch) {
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
            self.publish_inspect_phase(
                phase,
                self.media_epoch.load(Ordering::SeqCst) == epoch,
                None,
            );
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
            self.publish_inspect_phase(VoicePhase::Connecting, false, None);
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
            None,
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
            None,
        )
        .await;
    }

    pub async fn disconnect(&self, status: impl Into<String>) {
        self.music
            .stop("voice disconnected", PlaybackTermination::Stopped);
        self.stop_to(VoicePhase::Disconnected, status, None).await;
    }

    /// Stop the installed actor/call state while preserving the caller's
    /// already-reserved start token. Must be used only under `transition`.
    pub async fn disconnect_for_replace(&self, status: impl Into<String>) {
        self.stop_to_inner(VoicePhase::Disconnected, status, false, None, None, None)
            .await;
    }

    pub async fn fail_safe(
        &self,
        status: impl Into<String>,
        error: crate::observability::OperationalErrorCategory,
    ) {
        self.music
            .stop("voice failed", PlaybackTermination::Errored);
        self.stop_to(VoicePhase::Failed, status, Some(error)).await;
    }

    async fn stop_to(
        &self,
        phase: VoicePhase,
        status: impl Into<String>,
        error: Option<crate::observability::OperationalErrorCategory>,
    ) {
        self.stop_to_inner(phase, status, true, None, None, error)
            .await;
    }

    async fn stop_to_inner(
        &self,
        phase: VoicePhase,
        status: impl Into<String>,
        cancel_pending_start: bool,
        participants: Option<HashSet<u64>>,
        discord_session_id: Option<String>,
        error: Option<crate::observability::OperationalErrorCategory>,
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
            self.publish_inspect_phase(phase, false, error);
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

    /// Published only after the exact revoked Decode call is physically torn down.
    pub async fn music_consent_teardown_complete(&self, epoch: u64) {
        let inner = self.inner.lock().await;
        if inner.epoch == epoch && inner.phase == VoicePhase::AwaitingConsent {
            self.music_ready_epoch.store(epoch, Ordering::SeqCst);
        }
    }
    pub fn music_may_restore_output(&self, epoch: u64) -> bool {
        self.current_epoch() == epoch && self.music_ready_epoch.load(Ordering::SeqCst) == epoch
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
    let track = tokio::time::timeout(Duration::from_millis(250), control.playback.lock())
        .await
        .ok()
        .and_then(|mut playback| playback.take());
    if let Some(track) = track {
        let _ = track.stop();
    }
    // A provider owns nested recognition/turn tasks. Aborting the outer actor
    // would drop those JoinSets before observing their joins. The service's
    // shared shutdown deadline bounds waiting, while retaining the real actor.
    control.task.join().await;
    if let Some(track) = control.playback.lock().await.take() {
        let _ = track.stop();
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
