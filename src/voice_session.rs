//! Serialized lifecycle and observability for one Discord voice session.
//!
//! Provider tasks carry an epoch. Rejoin, pause, leave, bot moves, and shutdown
//! advance it before cancellation, so late work cannot overwrite status or
//! publish stale audio. The process owns exactly one provider task and one
//! playback handle at a time.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex};
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
        if !self.advance_epoch_and_cancel_start(epoch) {
            return false;
        }
        let control = {
            let mut inner = self.inner.lock().await;
            if inner.epoch != epoch {
                return false;
            }
            inner.epoch = epoch.saturating_add(1);
            inner.phase = phase;
            inner.status = bounded_status(status.into());
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

/// Return authoritative public copy for explicit voice-consent/control text,
/// or for a narrow standalone consent response while renewed consent is being
/// collected. All rendered state comes from `snapshot`; provider/model prose
/// and the free-form internal status string are deliberately excluded.
#[must_use]
pub fn authoritative_text_reply(text: &str, snapshot: &VoiceSnapshot) -> Option<String> {
    if !is_voice_control_text(text)
        && !is_explicit_withdrawal(text)
        && !(snapshot.phase == VoicePhase::AwaitingConsent && is_standalone_consent_response(text))
    {
        return None;
    }
    Some(voice_state_copy(snapshot))
}

/// A negative voice command may close an in-flight/active consent epoch, but
/// positive prose may never start one. Caller presence remains a Discord-shell
/// authorization check.
#[must_use]
pub fn requests_consent_withdrawal(text: &str, snapshot: &VoiceSnapshot) -> bool {
    (snapshot.start_pending
        || matches!(
            snapshot.phase,
            VoicePhase::Connecting
                | VoicePhase::Listening
                | VoicePhase::Thinking
                | VoicePhase::Speaking
        ))
        && is_explicit_withdrawal(text)
}

/// Normalize only for the small operational grammar below. Apostrophes are
/// removed so `don't listen` remains two useful words, while every other
/// punctuation mark becomes a word boundary.
fn normalized_voice_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut at_separator = true;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            at_separator = false;
        } else if matches!(character, '\'' | '\u{2019}') {
            // Keep contractions together: `don't` -> `dont`.
        } else if !at_separator {
            normalized.push(' ');
            at_separator = true;
        }
    }
    normalized.trim_end().to_string()
}

fn contains_phrase(normalized: &str, phrase: &str) -> bool {
    let mut haystack = String::with_capacity(normalized.len() + 2);
    haystack.push(' ');
    haystack.push_str(normalized);
    haystack.push(' ');

    let mut needle = String::with_capacity(phrase.len() + 2);
    needle.push(' ');
    needle.push_str(phrase);
    needle.push(' ');
    haystack.contains(&needle)
}

fn contains_any_phrase(normalized: &str, phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| contains_phrase(normalized, phrase))
}

/// Match only explicit voice-consent or voice-control language. This is not an
/// intent classifier: its deliberately small grammar is a safety boundary for
/// statements the generative path must never answer from imagination.
fn is_voice_control_text(text: &str) -> bool {
    const VOICE_CONTEXT: &[&str] = &[
        "voice",
        "listening",
        "listen",
        "audio",
        "microphone",
        "mic",
        "speech",
        "recognition",
        "transcription",
        "recording",
    ];
    const CONSENT_WORDS: &[&str] = &["consent", "consented", "consenting"];
    const CONSENT_PHRASES: &[&str] = &[
        "agree to audio processing",
        "agree to voice processing",
        "agree to local recognition",
        "agree to speech recognition",
        "agree to abbey listening",
        "agree to being recorded",
        "agree to be recorded",
        "agreed to audio processing",
        "agreed to voice processing",
        "agreed to local recognition",
        "agreed to speech recognition",
        "agreed to abbey listening",
        "agreed to being recorded",
        "permission to listen",
        "permission to process audio",
        "permission for voice processing",
        "give permission to listen",
        "grant permission to listen",
        "you may listen",
        "opt in to voice",
        "opt into voice",
        "opt in to listening",
        "opt in to speech recognition",
        "opted in to voice",
        "opted into voice",
    ];
    const CONTROL_PHRASES: &[&str] = &[
        "resume voice",
        "resume the voice",
        "voice resume",
        "voice resumed",
        "resume listening",
        "listening resumed",
        "restart voice",
        "restart the voice",
        "restart listening",
        "start voice",
        "start the voice",
        "voice started",
        "start listening",
        "started listening",
        "start audio processing",
        "stop voice",
        "stop the voice",
        "voice stopped",
        "stop listening",
        "stopped listening",
        "stop audio processing",
        "pause voice",
        "pause the voice",
        "voice paused",
        "pause listening",
        "listening paused",
        "pause audio processing",
        "join voice",
        "join the voice",
        "leave voice",
        "leave the voice",
        "enable voice",
        "enable the voice",
        "voice enabled",
        "enable listening",
        "disable voice",
        "disable the voice",
        "voice disabled",
        "disable listening",
        "turn voice on",
        "turn the voice on",
        "turn on voice",
        "turn on the voice",
        "turn listening on",
        "turn on listening",
        "turn voice off",
        "turn the voice off",
        "turn off voice",
        "turn off the voice",
        "turn listening off",
        "turn off listening",
        "voice is on",
        "voice is off",
        "voice is active",
        "voice is inactive",
        "voice is back on",
        "voice is back off",
        "voice back on",
        "voice back off",
        "listening is on",
        "listening is off",
        "listening is active",
        "listening is inactive",
        "listening is back on",
        "listening is back off",
        "audio processing is on",
        "audio processing is off",
        "audio processing is active",
        "audio processing is inactive",
        "audio processing resumed",
        "audio processing paused",
        "audio processing stopped",
        "speech components are back on",
        "speech components are back off",
        "speech recognition is on",
        "speech recognition is off",
        "turn microphone on",
        "turn microphone off",
        "turn mic on",
        "turn mic off",
        "do not listen",
        "dont listen",
        "are you listening",
        "is abbey listening",
        "voice status",
        "listening status",
        "has voice resumed",
        "did voice resume",
        "did listening resume",
        "is voice active",
        "is voice on",
        "is listening active",
        "is audio processing active",
    ];

    let normalized = normalized_voice_text(text);
    if normalized.is_empty() || !contains_any_phrase(&normalized, VOICE_CONTEXT) {
        return false;
    }
    contains_any_phrase(&normalized, CONSENT_WORDS)
        || contains_any_phrase(&normalized, CONSENT_PHRASES)
        || contains_any_phrase(&normalized, CONTROL_PHRASES)
}

/// During an explicit renewed-consent epoch, short standalone consent or
/// withdrawal statements need no repeated `voice` qualifier. Exact matching
/// keeps unrelated agreement and a bare `yes` on the ordinary social path.
fn is_standalone_consent_response(text: &str) -> bool {
    const RESPONSES: &[&str] = &[
        "i agree",
        "i consent",
        "we consent",
        "we all consent",
        "everyone consents",
        "everyone consent",
        "all consent",
        "i opt in",
        "we opt in",
        "i do not consent",
        "i dont consent",
        "we do not consent",
        "we dont consent",
    ];

    let normalized = normalized_voice_text(text);
    let normalized = normalized.strip_prefix("abbey ").unwrap_or(&normalized);
    RESPONSES.contains(&normalized)
}

fn is_explicit_withdrawal(text: &str) -> bool {
    const REFUSALS: &[&str] = &[
        "i do not consent",
        "i dont consent",
        "we do not consent",
        "we dont consent",
        "i withdraw my consent",
        "we withdraw our consent",
        "withdraw my consent",
        "withdraw our consent",
        "i revoke my consent",
        "we revoke our consent",
        "revoke my consent",
        "revoke our consent",
        "i no longer consent",
        "we no longer consent",
    ];
    const COMMANDS: &[&str] = &[
        "stop voice",
        "stop the voice",
        "stop listening",
        "stop audio processing",
        "stop recording",
        "stop recording me",
        "stop transcription",
        "stop transcribing",
        "stop transcribing me",
        "pause voice",
        "pause the voice",
        "pause listening",
        "pause audio processing",
        "leave voice",
        "leave the voice",
        "disable voice",
        "disable the voice",
        "disable listening",
        "turn voice off",
        "turn the voice off",
        "turn off voice",
        "turn off the voice",
        "turn listening off",
        "turn off listening",
        "do not listen",
        "dont listen",
        "do not record me",
        "dont record me",
        "do not transcribe me",
        "dont transcribe me",
    ];

    let normalized = normalized_voice_text(text);
    let without_name = normalized.strip_prefix("abbey ").unwrap_or(&normalized);
    let mut command = without_name.strip_prefix("please ").unwrap_or(without_name);
    // Accept courtesy words in either common order while keeping the actual
    // withdrawal phrase an exact match. Questions such as "how do I stop
    // voice?" therefore remain non-authoritative.
    for _ in 0..2 {
        command = command.strip_suffix(" now").unwrap_or(command);
        command = command.strip_suffix(" please").unwrap_or(command);
    }
    if COMMANDS.contains(&command) {
        return true;
    }

    let standalone_refusal = REFUSALS.contains(&command);
    let voice_context = [
        "voice",
        "listening",
        "listen",
        "audio",
        "microphone",
        "mic",
        "speech",
        "recognition",
        "transcription",
        "recording",
    ];
    standalone_refusal
        || (contains_any_phrase(command, &voice_context) && contains_any_phrase(command, REFUSALS))
}

fn voice_state_copy(snapshot: &VoiceSnapshot) -> String {
    if snapshot.phase.processes_audio() && snapshot.media_enabled {
        let replacement = if snapshot.start_pending {
            " A replacement start is pending, but the current consented session remains active until it is explicitly stopped or safely replaced."
        } else {
            ""
        };
        return format!(
            "Voice is active for the current consent epoch. Current phase: {} · consent epoch: {} · participants recorded: {}. Participant audio processing is enabled; `/voice leave` stops it.{replacement}",
            snapshot.phase.label(),
            snapshot.consent_epoch,
            snapshot.participant_count
        );
    }
    if snapshot.start_pending {
        return format!(
            "A voice start is pending, but the media gate is closed and no participant audio is being processed. An explicit withdrawal or `/voice leave` cancels it; only final membership, permission, and consent checks can activate it. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        );
    }
    match snapshot.phase {
        VoicePhase::Disconnected => format!(
            "Voice is disconnected, and no participant audio is being processed. Starting it requires consent from everyone currently present and a manager's `/voice join consent:true`. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::PresenceOnly => format!(
            "Abbey is present only in muted/self-deafened no-audio mode. Capture, recognition, reasoning, synthesis, and playback are disabled. Starting voice requires everyone-present consent and a manager's `/voice join consent:true`. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::Connecting => format!(
            "Voice is still connecting and has not become active. The software media gate is closed, so no participant audio is being processed. Check `/voice status` before treating voice as started. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::Listening | VoicePhase::Thinking | VoicePhase::Speaking => format!(
            "Voice is stopping or paused. Its media gate is closed, so no participant audio is being processed while the runtime leaves phase {}. Check `/voice status` for the settled state. Consent epoch: {} · participants recorded: {}.",
            snapshot.phase.label(),
            snapshot.consent_epoch,
            snapshot.participant_count
        ),
        VoicePhase::AwaitingConsent => format!(
            "Voice has not resumed. Abbey is paused with its media gate closed, so no participant audio is being processed; the pause procedure also tears down any existing conversational connection. Renewed consent from everyone currently present plus a manager's `/voice resume consent:true` are required before voice can restart. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::Failed => format!(
            "Voice failed safe, and no participant audio is being processed. A manager can inspect `/voice status`; any new start still requires everyone-present consent and `/voice join consent:true`. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
    }
}

pub struct VoiceRuntime {
    pub config: VoiceConfig,
    pub transition: Mutex<()>,
    current_epoch: AtomicU64,
    media_epoch: AtomicU64,
    start_generation: AtomicU64,
    pending_start_generation: AtomicU64,
    activation_gate: SyncMutex<()>,
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
            pending_start_generation: AtomicU64::new(0),
            activation_gate: SyncMutex::new(()),
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
        epoch != 0 && self.is_current(epoch) && self.media_epoch.load(Ordering::SeqCst) == epoch
    }

    /// Timing-critical receive callbacks use this synchronous compare/exchange
    /// to close the media gate, then schedule slower task/call cleanup.
    pub fn revoke_media(&self, epoch: u64) -> bool {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.media_epoch
            .compare_exchange(epoch, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
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
        generation
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
        self.start_generation.fetch_add(1, Ordering::SeqCst);
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.media_epoch.store(0, Ordering::SeqCst);
    }

    /// Clear a completed/failed start reservation without invalidating a newer
    /// attempt that replaced it.
    pub fn finish_start_attempt(&self, generation: u64) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = self.pending_start_generation.compare_exchange(
            generation,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    #[must_use]
    pub fn start_is_current(&self, generation: u64) -> bool {
        self.start_generation.load(Ordering::SeqCst) == generation
            && self.pending_start_generation.load(Ordering::SeqCst) == generation
    }

    /// Advance an exact active epoch while atomically closing media and any
    /// pending start. Sharing `activation_gate` with `activate` makes the
    /// epoch check and media transition indivisible with respect to a final
    /// activation attempt.
    fn advance_epoch_and_cancel_start(&self, epoch: u64) -> bool {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        self.media_epoch.store(0, Ordering::SeqCst);
        self.start_generation.fetch_add(1, Ordering::SeqCst);
        self.pending_start_generation.store(0, Ordering::SeqCst);
        true
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

    pub async fn activate(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
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
        if !self.advance_epoch_and_cancel_start(epoch) {
            return None;
        }
        let control = {
            let mut inner = self.inner.lock().await;
            if inner.epoch != epoch {
                return None;
            }
            inner.epoch = epoch.saturating_add(1);
            inner.phase = VoicePhase::AwaitingConsent;
            inner.status = bounded_status(status.into());
            inner.participants = participants;
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
        if inner.epoch == epoch && self.is_current(epoch) {
            inner.phase = phase;
            inner.status = bounded_status(status.into());
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
        if inner.epoch == epoch
            && inner.phase == VoicePhase::Connecting
            && !self.media_enabled(epoch)
        {
            inner.status = bounded_status(status.into());
        }
    }

    pub async fn set_presence(&self, status: impl Into<String>) {
        self.stop_to(VoicePhase::PresenceOnly, status).await;
    }

    pub async fn pause_for_consent(&self, participants: HashSet<u64>) {
        self.stop_to(
            VoicePhase::AwaitingConsent,
            "voice disconnected; renewed participant consent is required",
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
        let epoch = {
            let _activation = self
                .activation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.media_epoch.store(0, Ordering::SeqCst);
            if cancel_pending_start {
                self.start_generation.fetch_add(1, Ordering::SeqCst);
                self.pending_start_generation.store(0, Ordering::SeqCst);
            }
            self.current_epoch.fetch_add(1, Ordering::SeqCst) + 1
        };
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

    fn voice_snapshot(phase: VoicePhase) -> VoiceSnapshot {
        VoiceSnapshot {
            epoch: 12,
            phase,
            media_enabled: phase.processes_audio(),
            start_pending: false,
            status: "untrusted prose saying speech is back on".into(),
            consent_epoch: 7,
            participant_count: 4,
            dropped_input: 1,
            aborted_overruns: 2,
            barge_ins: 3,
            completed_turns: 4,
        }
    }

    #[test]
    fn text_boundary_matches_only_explicit_consent_and_control_language() {
        let snapshot = voice_snapshot(VoicePhase::PresenceOnly);
        for text in [
            "I consent to local speech recognition for this voice session.",
            "I do not consent to voice recording.",
            "You may listen now.",
            "Can you resume voice now?",
            "Please stop audio processing.",
            "The local speech components are back on.",
            "Are you listening?",
            "@Abbey, /voice status",
        ] {
            assert!(
                authoritative_text_reply(text, &snapshot).is_some(),
                "expected a match: {text}"
            );
        }

        for text in [
            "I consent to the updated code of conduct.",
            "I agree the voice sounds warmer.",
            "I agree to use a different voice.",
            "Please resume the download.",
            "We should discuss audio codecs.",
            "Start by explaining how voice synthesis works.",
            "The microphone stand is on the table.",
            "The mic is on the desk.",
            "yes",
        ] {
            assert!(
                authoritative_text_reply(text, &snapshot).is_none(),
                "unexpected match: {text}"
            );
        }
    }

    #[test]
    fn awaiting_consent_accepts_only_standalone_explicit_responses() {
        let awaiting = voice_snapshot(VoicePhase::AwaitingConsent);
        for text in [
            "I agree.",
            "I consent",
            "Abbey, we all consent!",
            "everyone consents",
            "I opt in",
            "I do not consent",
            "we don't consent",
        ] {
            assert!(
                authoritative_text_reply(text, &awaiting).is_some(),
                "expected awaiting-consent match: {text}"
            );
        }
        for text in [
            "yes",
            "sure",
            "I agree the voice sounds warmer",
            "I consent to the code of conduct",
        ] {
            assert!(
                authoritative_text_reply(text, &awaiting).is_none(),
                "unexpected awaiting-consent match: {text}"
            );
        }

        let active = voice_snapshot(VoicePhase::Listening);
        assert!(authoritative_text_reply("I agree", &active).is_none());
        assert!(authoritative_text_reply("I consent", &active).is_none());
    }

    #[test]
    fn only_explicit_negative_language_requests_active_epoch_revocation() {
        let active = voice_snapshot(VoicePhase::Listening);
        for text in [
            "I do not consent",
            "Abbey, we don't consent",
            "I do not consent to voice recording",
            "I withdraw my consent",
            "I revoke my consent",
            "I no longer consent",
            "please stop listening now",
            "stop listening please",
            "stop recording",
            "stop recording me please",
            "do not record me",
            "don't transcribe me",
            "stop transcription now",
            "stop transcribing me",
            "Abbey turn voice off",
        ] {
            assert!(
                requests_consent_withdrawal(text, &active),
                "expected withdrawal: {text}"
            );
            assert!(authoritative_text_reply(text, &active).is_some());
        }
        for text in [
            "yes",
            "I consent",
            "resume voice",
            "how do I stop voice",
            "I do not consent to the code of conduct",
            "how do I stop recording?",
            "can you stop transcribing someone else?",
            "I agree the voice sounds warmer",
        ] {
            assert!(
                !requests_consent_withdrawal(text, &active),
                "unexpected withdrawal: {text}"
            );
        }

        let inactive = voice_snapshot(VoicePhase::AwaitingConsent);
        assert!(!requests_consent_withdrawal("I do not consent", &inactive));
    }

    #[test]
    fn awaiting_consent_copy_cannot_claim_that_listening_resumed() {
        let snapshot = voice_snapshot(VoicePhase::AwaitingConsent);
        let copy = authoritative_text_reply("I consent", &snapshot).expect("fixed reply");
        assert_eq!(
            copy,
            "Voice has not resumed. Abbey is paused with its media gate closed, so no participant audio is being processed; the pause procedure also tears down any existing conversational connection. Renewed consent from everyone currently present plus a manager's `/voice resume consent:true` are required before voice can restart. Consent epoch: 7 · participants recorded: 4."
        );
        assert!(!copy.contains("untrusted prose"));
        assert!(!copy.contains("processing is enabled"));
    }

    #[test]
    fn voice_phase_copy_distinguishes_active_and_inactive_snapshots() {
        for phase in [
            VoicePhase::Disconnected,
            VoicePhase::PresenceOnly,
            VoicePhase::Connecting,
            VoicePhase::AwaitingConsent,
            VoicePhase::Failed,
        ] {
            let snapshot = voice_snapshot(phase);
            let copy = authoritative_text_reply("voice status", &snapshot).expect("fixed reply");
            assert!(
                copy.contains("no participant audio is being processed")
                    || copy.contains(
                        "Capture, recognition, reasoning, synthesis, and playback are disabled"
                    )
            );
            assert!(!copy.contains("Participant audio processing is enabled"));
        }

        for phase in [
            VoicePhase::Listening,
            VoicePhase::Thinking,
            VoicePhase::Speaking,
        ] {
            let snapshot = voice_snapshot(phase);
            let copy = authoritative_text_reply("voice status", &snapshot).expect("fixed reply");
            assert!(copy.contains("Voice is active for the current consent epoch"));
            assert!(copy.contains("Participant audio processing is enabled"));
            assert!(copy.contains(phase.label()));
            assert!(!copy.contains("untrusted prose"));
        }

        let mut closing = voice_snapshot(VoicePhase::Speaking);
        closing.media_enabled = false;
        let copy = authoritative_text_reply("voice status", &closing).expect("fixed reply");
        assert!(copy.contains("media gate is closed"));
        assert!(copy.contains("no participant audio is being processed"));
        assert!(!copy.contains("Participant audio processing is enabled"));

        let mut active_replacement = voice_snapshot(VoicePhase::Listening);
        active_replacement.start_pending = true;
        let copy = authoritative_text_reply("voice status", &active_replacement)
            .expect("fixed active replacement reply");
        assert!(copy.contains("Participant audio processing is enabled"));
        assert!(copy.contains("replacement start is pending"));
        assert!(!copy.contains("media gate is closed"));
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
        let start_generation = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::new()).await;
        assert!(runtime.snapshot().await.start_pending);
        assert!(!runtime.media_enabled(epoch));
        assert!(runtime.activate(epoch, start_generation, "ready").await);
        assert!(!runtime.snapshot().await.start_pending);
        assert!(runtime.media_enabled(epoch));
        runtime.pause_for_consent(HashSet::new()).await;
        assert!(!runtime.media_enabled(epoch));
    }

    #[tokio::test]
    async fn pending_preflight_is_visible_and_explicit_withdrawal_cancels_it() {
        let runtime = runtime();
        let generation = runtime.reserve_start();
        let pending = runtime.snapshot().await;
        assert!(pending.start_pending);
        assert!(runtime.start_is_current(generation));
        assert!(requests_consent_withdrawal("stop listening", &pending));
        let copy = authoritative_text_reply("stop listening", &pending).expect("fixed reply");
        assert!(copy.contains("voice start is pending"));
        assert!(copy.contains("media gate is closed"));

        runtime.cancel_pending_start();
        let cancelled = runtime.snapshot().await;
        assert!(!cancelled.start_pending);
        assert!(!runtime.start_is_current(generation));
        assert!(!cancelled.media_enabled);
    }

    #[test]
    fn finishing_an_old_start_cannot_clear_its_replacement() {
        let runtime = runtime();
        let old = runtime.reserve_start();
        let replacement = runtime.reserve_start();
        runtime.finish_start_attempt(old);
        assert!(runtime.start_is_current(replacement));
        runtime.finish_start_attempt(replacement);
        assert!(!runtime.start_is_current(replacement));
    }

    #[tokio::test]
    async fn provider_readiness_cannot_publish_listening_before_activation() {
        let runtime = runtime();
        let start_generation = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::new()).await;
        runtime
            .set_prepared_status(epoch, "provider ready but media closed")
            .await;
        let prepared = runtime.snapshot().await;
        assert_eq!(prepared.phase, VoicePhase::Connecting);
        assert!(!prepared.media_enabled);
        assert_eq!(prepared.status, "provider ready but media closed");

        assert!(runtime.activate(epoch, start_generation, "active").await);
        runtime
            .set_prepared_status(epoch, "late duplicate readiness")
            .await;
        let active = runtime.snapshot().await;
        assert_eq!(active.phase, VoicePhase::Listening);
        assert!(active.media_enabled);
        assert_eq!(active.status, "active");
    }

    #[tokio::test]
    async fn cancellation_and_activation_cannot_leave_media_open() {
        for _ in 0..64 {
            let runtime = Arc::new(runtime());
            let start_generation = runtime.reserve_start();
            let epoch = runtime.begin(HashSet::new()).await;
            let barrier = Arc::new(tokio::sync::Barrier::new(3));

            let activating = {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    runtime
                        .activate(epoch, start_generation, "must not survive cancellation")
                        .await
                })
            };
            let cancelling = {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    runtime.cancel_pending_start();
                })
            };

            barrier.wait().await;
            let _ = activating.await.expect("activation task");
            cancelling.await.expect("cancellation task");
            assert!(!runtime.media_enabled(epoch));
            assert!(!runtime.start_is_current(start_generation));
        }
    }

    #[tokio::test]
    async fn side_effect_gate_rejects_work_after_revocation() {
        let runtime = runtime();
        let start_generation = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::new()).await;
        assert!(runtime.activate(epoch, start_generation, "active").await);

        let mut effects = 0_u8;
        assert_eq!(
            runtime.with_media_enabled(epoch, || {
                effects += 1;
            }),
            Some(())
        );
        assert!(runtime.revoke_media(epoch));
        assert_eq!(
            runtime.with_media_enabled(epoch, || {
                effects += 1;
            }),
            None
        );
        assert_eq!(effects, 1);
    }

    #[tokio::test]
    async fn connecting_consent_pause_cannot_be_lost_to_activation() {
        for _ in 0..64 {
            let runtime = Arc::new(runtime());
            let start_generation = runtime.reserve_start();
            let epoch = runtime.begin(HashSet::from([7])).await;
            let barrier = Arc::new(tokio::sync::Barrier::new(3));

            let activating = {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    runtime.activate(epoch, start_generation, "active").await
                })
            };
            let pausing = {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    let pause = runtime
                        .begin_pause_epoch_for_consent(
                            epoch,
                            HashSet::from([7, 8]),
                            "new participant",
                        )
                        .await
                        .expect("exact connecting epoch pauses");
                    pause.finish().await;
                })
            };

            barrier.wait().await;
            let _ = activating.await.expect("activation task");
            pausing.await.expect("pause task");
            let snapshot = runtime.snapshot().await;
            assert_eq!(snapshot.phase, VoicePhase::AwaitingConsent);
            assert!(!snapshot.media_enabled);
            assert!(!snapshot.start_pending);
            assert!(!runtime.start_is_current(start_generation));
        }
    }

    #[test]
    fn status_is_flattened_and_bounded() {
        let status = bounded_status(format!("line one\n{}", "x".repeat(500)));
        assert!(!status.contains('\n'));
        assert!(status.chars().count() <= 241);
    }
}
