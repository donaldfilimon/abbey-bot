//! Local STT → canonical Abbey cognition → local TTS session actor.
//!
//! Audio callbacks only enqueue fixed frames. This task owns turn segmentation,
//! cancellation, WDBX-scoped context, persona routing, the existing tool loop,
//! and one playback track per response. Raw audio and provider transcripts are
//! never persisted. While an operator verification run is armed, completed
//! conversational transcripts and responses are not committed either.

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use songbird::input::RawAdapter;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::generation;
use crate::memory::PersonaContext;
use crate::offline_voice::{DecodedAudio, MlxAudioClient, SegmentEvent, Segmenter, Utterance};
use crate::persona;
use crate::pipeline;
use crate::runtime::{self, AppState};
use crate::voice_session::{
    PlaybackTermination, SessionEvent, SharedPlayback, VoicePhase, VoiceRuntime,
    authoritative_text_reply, register_playback_termination, requests_consent_withdrawal,
};

const VOICE_SYSTEM_SUFFIX: &str = "You are speaking aloud in a consented Discord voice session. Respond in one to three short, natural sentences unless the user explicitly asks for detail. Avoid Markdown, raw URLs, emoji, tables, headings, and unspoken formatting. Pronounce code, symbols, and acronyms clearly. Voice turns are read-only: never claim an external action or durable memory change succeeded.";
const CONTINUATION_WINDOW: Duration = Duration::from_secs(45);

pub struct LocalSession {
    pub runtime: Arc<VoiceRuntime>,
    pub state: Arc<AppState>,
    pub call: Arc<Mutex<songbird::Call>>,
    pub client: MlxAudioClient,
    pub epoch: u64,
    pub input: mpsc::Receiver<crate::offline_voice::VoiceFrame>,
    pub lifecycle: mpsc::UnboundedReceiver<SessionEvent>,
    pub events: mpsc::UnboundedSender<SessionEvent>,
    pub driver_disconnect: watch::Receiver<bool>,
    pub cancel: watch::Receiver<bool>,
    pub playback: SharedPlayback,
    pub backend: crate::llm::Backend,
}

#[derive(Default)]
struct WakeState {
    speaker: Option<u64>,
    until: Option<Instant>,
}

enum TurnOutcome {
    Ready {
        turn: u64,
        scope: String,
        transcript: String,
        spoken_answer: String,
        persist: bool,
        audio: DecodedAudio,
    },
    Ignored {
        turn: u64,
    },
    WithdrawConsent {
        turn: u64,
    },
    Failed {
        turn: u64,
        stage: &'static str,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OperationalVoiceTurn {
    Reply(String),
    WithdrawConsent,
}

fn operational_voice_turn(
    transcript: &str,
    snapshot: &crate::voice_session::VoiceSnapshot,
) -> Option<OperationalVoiceTurn> {
    if requests_consent_withdrawal(transcript, snapshot) {
        return Some(OperationalVoiceTurn::WithdrawConsent);
    }
    authoritative_text_reply(transcript, snapshot).map(OperationalVoiceTurn::Reply)
}

fn pre_wake_withdrawal(
    transcript: &str,
    snapshot: &crate::voice_session::VoiceSnapshot,
    safely_attributed: bool,
    currently_attested: bool,
) -> bool {
    safely_attributed && currently_attested && requests_consent_withdrawal(transcript, snapshot)
}

struct PendingCommit {
    turn: u64,
    scope: String,
    transcript: String,
    spoken_answer: String,
}

fn should_commit_turn(persist_requested: bool, verification_active: bool) -> bool {
    persist_requested && !verification_active
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackObservation {
    NaturalCompletion,
    Errored,
    OrdinaryStop,
    ConfirmedBargeInCancellation,
    Unclassified,
    Stale,
}

#[derive(Default)]
struct PlaybackLifecycle {
    pending_barge_stop: Option<u64>,
}

impl PlaybackLifecycle {
    fn note_barge_stop_requested(&mut self, turn: u64) {
        self.pending_barge_stop = Some(turn);
    }

    fn observe(
        &mut self,
        current_turn: u64,
        turn: u64,
        termination: PlaybackTermination,
    ) -> PlaybackObservation {
        let pending_barge_stop = self.pending_barge_stop == Some(turn);
        if pending_barge_stop {
            self.pending_barge_stop = None;
            if termination == PlaybackTermination::Stopped {
                return PlaybackObservation::ConfirmedBargeInCancellation;
            }
        }
        if turn != current_turn {
            return PlaybackObservation::Stale;
        }
        match termination {
            PlaybackTermination::Natural => PlaybackObservation::NaturalCompletion,
            PlaybackTermination::Stopped => PlaybackObservation::OrdinaryStop,
            PlaybackTermination::Errored => PlaybackObservation::Errored,
            PlaybackTermination::Unclassified => PlaybackObservation::Unclassified,
        }
    }
}

pub async fn run(mut session: LocalSession) {
    let client = session.client.clone();
    let wake = Arc::new(Mutex::new(WakeState::default()));
    let mut segmenter = Segmenter::new();
    let mut turns = JoinSet::new();
    let mut turn_generation = 0_u64;
    let mut pending_commit: Option<PendingCommit> = None;
    let mut playback_lifecycle = PlaybackLifecycle::default();

    session
        .runtime
        .set_prepared_status(
            session.epoch,
            "local inference ready; awaiting final consent activation",
        )
        .await;

    loop {
        tokio::select! {
            biased;
            changed = session.cancel.changed() => {
                if changed.is_err() || *session.cancel.borrow() {
                    break;
                }
            }
            changed = session.driver_disconnect.changed() => {
                if changed.is_err() || *session.driver_disconnect.borrow() {
                    session.runtime
                        .actor_failed(
                            session.epoch,
                            "Discord voice transport disconnected; audio stopped",
                        )
                        .await;
                    disconnect_call(&session.call).await;
                    break;
                }
            }
            lifecycle = session.lifecycle.recv() => {
                let Some(SessionEvent::PlaybackTerminated { turn, termination }) = lifecycle else {
                    continue;
                };
                match playback_lifecycle.observe(turn_generation, turn, termination) {
                    PlaybackObservation::NaturalCompletion => {
                        session.playback.lock().await.take();
                        let pending = pending_commit.take().filter(|pending| pending.turn == turn);
                        let has_pending_commit = pending.is_some();
                        let committed = session
                            .runtime
                            .with_media_enabled(session.epoch, || {
                                if let Some(pending) = pending {
                                    AppState::lock(&session.state.engine).commit(
                                        &pending.scope,
                                        &pending.transcript,
                                        &pending.spoken_answer,
                                        runtime::now(),
                                    );
                                }
                                // Only Songbird's natural `PlayMode::End` is
                                // completion evidence. Verification may still
                                // suppress the conversational commit.
                                session.runtime.note_completed_turn();
                                has_pending_commit
                            })
                            .unwrap_or(false);
                        tracing::info!(
                            epoch = session.epoch,
                            turn,
                            committed,
                            "local Abbey playback completed"
                        );
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "local inference ready; listening for Abbey",
                            )
                            .await;
                    }
                    PlaybackObservation::ConfirmedBargeInCancellation => {
                        session.runtime.note_verification_barge_in_cancellation();
                        tracing::info!(
                            epoch = session.epoch,
                            turn,
                            "local Abbey playback stop confirmed after barge-in"
                        );
                    }
                    PlaybackObservation::Errored
                    | PlaybackObservation::OrdinaryStop
                    | PlaybackObservation::Unclassified => {
                        session.playback.lock().await.take();
                        if pending_commit
                            .as_ref()
                            .is_some_and(|pending| pending.turn == turn)
                        {
                            pending_commit = None;
                        }
                        tracing::warn!(
                            epoch = session.epoch,
                            turn,
                            ?termination,
                            "local Abbey playback ended without natural completion"
                        );
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "speech playback ended early; listening remains active",
                            )
                            .await;
                    }
                    PlaybackObservation::Stale => {}
                }
            }
            frame = session.input.recv() => {
                let Some(frame) = frame else { break; };
                if !session.runtime.media_enabled(session.epoch) {
                    continue;
                }
                for event in segmenter.push(frame) {
                    match event {
                        SegmentEvent::SpeechStarted { .. } => {
                            let playback_turn = turn_generation;
                            turn_generation = turn_generation.saturating_add(1);
                            pending_commit = None;
                            let playback_stop_requested = stop_playback(&session.playback).await;
                            if playback_stop_requested {
                                playback_lifecycle.note_barge_stop_requested(playback_turn);
                            }
                            let interrupted = !turns.is_empty() || playback_stop_requested;
                            if interrupted {
                                session.runtime.note_barge_in();
                            }
                            turns.abort_all();
                            while turns.try_join_next().is_some() {}
                            session
                                .runtime
                                .set_status(
                                    session.epoch,
                                    VoicePhase::Listening,
                                    "speech detected; listening",
                                )
                                .await;
                        }
                        SegmentEvent::AbortedOverrun => {
                            turn_generation = turn_generation.saturating_add(1);
                            pending_commit = None;
                            turns.abort_all();
                            session.runtime.note_overrun();
                            session
                                .runtime
                                .set_status(
                                    session.epoch,
                                    VoicePhase::Listening,
                                    "input overrun aborted one utterance safely",
                                )
                                .await;
                        }
                        SegmentEvent::Completed(utterance) => {
                            turn_generation = turn_generation.saturating_add(1);
                            pending_commit = None;
                            let turn = turn_generation;
                            turns.abort_all();
                            session
                                .runtime
                                .set_status(
                                    session.epoch,
                                    VoicePhase::Thinking,
                                    "transcribing locally",
                                )
                                .await;
                            let work = TurnWork {
                                turn,
                                runtime: Arc::clone(&session.runtime),
                                client: client.clone(),
                                state: Arc::clone(&session.state),
                                backend: session.backend.clone(),
                                guild_id: session.runtime.config.guild_id,
                                channel_id: session.runtime.config.channel_id,
                                consent_epoch: session.epoch,
                                wake_word_required: session.runtime.config.wake_word_required,
                                wake: Arc::clone(&wake),
                                utterance,
                            };
                            let _ = session
                                .runtime
                                .with_media_enabled(session.epoch, || {
                                    turns.spawn(process_turn(work));
                                });
                        }
                    }
                }
            }
            result = turns.join_next(), if !turns.is_empty() => {
                let Some(result) = result else { continue; };
                let outcome = match result {
                    Ok(outcome) => outcome,
                    Err(error) if error.is_cancelled() => continue,
                    Err(error) => {
                        tracing::error!(error = %error, "local voice turn task panicked");
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "one local voice turn failed safely",
                            )
                            .await;
                        continue;
                    }
                };
                match outcome {
                    TurnOutcome::Ignored { turn } if turn == turn_generation => {
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "listening; wake name required",
                            )
                            .await;
                    }
                    TurnOutcome::WithdrawConsent { turn } if turn == turn_generation => {
                        let _ = session.runtime.revoke_media(session.epoch);
                        session.runtime
                            .actor_awaiting_consent(
                                session.epoch,
                                "audio stopped because a participant withdrew consent by voice",
                            )
                            .await;
                        // The phase is now AwaitingConsent, so Abbey's own
                        // disconnect event cannot be misclassified as an
                        // external transport fault. Stop Decode immediately.
                        disconnect_call(&session.call).await;
                        let _ = stop_playback(&session.playback).await;
                        break;
                    }
                    TurnOutcome::Ready {
                        turn,
                        scope,
                        transcript,
                        spoken_answer,
                        persist,
                        audio,
                    }
                        if turn == turn_generation && session.runtime.media_enabled(session.epoch) =>
                    {
                        match play_audio(
                            &session.call,
                            &session.playback,
                            &session.events,
                            &session.runtime,
                            session.epoch,
                            audio,
                            turn,
                        ).await {
                            Ok(true) => {
                                pending_commit = should_commit_turn(
                                    persist,
                                    session.runtime.verification_active(),
                                )
                                .then_some(PendingCommit {
                                    turn,
                                    scope,
                                    transcript,
                                    spoken_answer,
                                });
                                tracing::info!(
                                    epoch = session.epoch,
                                    turn,
                                    committed_on_completion = persist,
                                    "local Abbey playback started"
                                );
                                session.runtime
                                    .set_status(
                                        session.epoch,
                                        VoicePhase::Speaking,
                                        "speaking locally generated Abbey audio",
                                    )
                                    .await;
                            }
                            Ok(false) => {
                                // Consent or transport state changed after the
                                // turn was prepared; playback did not begin.
                            }
                            Err(error) => {
                                tracing::error!(error = %brief(&error), "local voice playback failed");
                                session.runtime
                                    .set_status(
                                        session.epoch,
                                        VoicePhase::Listening,
                                        "speech playback failed; listening remains active",
                                    )
                                    .await;
                            }
                        }
                    }
                    TurnOutcome::Failed { turn, stage, error } if turn == turn_generation => {
                        if crate::offline_voice::sidecar_is_unavailable(&error) {
                            tracing::warn!(
                                stage,
                                error = %brief(&error),
                                "local speech sidecar became unavailable; failing closed"
                            );
                            session
                                .runtime
                                .actor_failed(
                                    session.epoch,
                                    format!(
                                        "local speech sidecar became unavailable during {stage}; audio stopped"
                                    ),
                                )
                                .await;
                            disconnect_call(&session.call).await;
                            let _ = stop_playback(&session.playback).await;
                            break;
                        }
                        tracing::warn!(stage, error = %brief(&error), "local voice turn failed");
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                format!("{stage} failed locally; listening for the next turn"),
                            )
                            .await;
                    }
                    _ => {}
                }
            }
        }
    }

    turns.abort_all();
    while turns.join_next().await.is_some() {}
    let _ = stop_playback(&session.playback).await;
}

struct TurnWork {
    turn: u64,
    runtime: Arc<VoiceRuntime>,
    client: MlxAudioClient,
    state: Arc<AppState>,
    backend: crate::llm::Backend,
    guild_id: u64,
    channel_id: u64,
    consent_epoch: u64,
    wake_word_required: bool,
    wake: Arc<Mutex<WakeState>>,
    utterance: Utterance,
}

async fn process_turn(work: TurnWork) -> TurnOutcome {
    let transcript = match work.client.transcribe(&work.utterance.pcm).await {
        Ok(transcript) => transcript,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "speech recognition",
                error,
            };
        }
    };
    work.runtime
        .note_verification_stt_completion(work.consent_epoch);
    let safely_attributed = work.utterance.speaker_id.is_some() && !work.utterance.overlap;
    let snapshot = work.runtime.snapshot().await;
    let withdrawal_authorized = if safely_attributed {
        if let Some(speaker_id) = work.utterance.speaker_id {
            work.runtime
                .epoch_attests(work.consent_epoch, speaker_id)
                .await
        } else {
            false
        }
    } else {
        false
    };
    // Consent withdrawal is asymmetric with activation: a safely attributed,
    // currently attested participant may stop voice without a wake name, but
    // positive prose can never start or resume it.
    if pre_wake_withdrawal(
        &transcript,
        &snapshot,
        safely_attributed,
        withdrawal_authorized,
    ) {
        return TurnOutcome::WithdrawConsent { turn: work.turn };
    }
    if !is_addressed(
        &transcript,
        work.utterance.speaker_id,
        safely_attributed,
        work.wake_word_required,
        &work.wake,
    )
    .await
    {
        return TurnOutcome::Ignored { turn: work.turn };
    }

    let snapshot = work.runtime.snapshot().await;
    let persona = persona::route(&transcript, None).persona;
    let scope = voice_scope(
        work.guild_id,
        work.channel_id,
        work.consent_epoch,
        work.utterance.speaker_id,
        work.turn,
        safely_attributed,
    );
    if let Some(operational) = operational_voice_turn(&transcript, &snapshot) {
        let OperationalVoiceTurn::Reply(answer) = operational else {
            // Unattributed/overlapping speech may receive no operational or
            // generative answer, and it may never revoke another person's
            // session. Authorized withdrawal already returned above.
            return TurnOutcome::Ignored { turn: work.turn };
        };
        let spoken_answer = crate::offline_voice::spoken_text(&answer);
        let audio = match work.client.synthesize(&spoken_answer).await {
            Ok(audio) => audio,
            Err(error) => {
                return TurnOutcome::Failed {
                    turn: work.turn,
                    stage: "speech synthesis",
                    error,
                };
            }
        };
        return TurnOutcome::Ready {
            turn: work.turn,
            scope,
            transcript,
            spoken_answer,
            persist: false,
            audio,
        };
    }
    let scoped_guild = format!("discord:{}", work.guild_id);
    let scoped_user = work.utterance.speaker_id.map_or_else(
        || "discord:voice:unattributed".into(),
        |id| format!("discord:{id}"),
    );
    let context = if safely_attributed {
        let reputation = work.state.reputation_snapshot(&scoped_guild, &scoped_user);
        pipeline::assemble_context(
            &work.state,
            &scoped_guild,
            &scoped_user,
            &scope,
            &transcript,
            reputation,
        )
    } else {
        PersonaContext::empty()
    };
    let generation = match work.state.acquire_generation().await {
        Err(error) => Err(error),
        Ok(_slot) => {
            generation::generate_without_delivery(
                &work.state,
                &work.backend,
                persona,
                &generation::Ask {
                    scope: &scope,
                    context: &context,
                    user_input: &transcript,
                    now: runtime::now(),
                },
                Some(VOICE_SYSTEM_SUFFIX),
            )
            .await
        }
    };
    let (answer, _) = match generation {
        Ok(answer) => answer,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "Abbey reasoning",
                error: error.to_string(),
            };
        }
    };
    let spoken_answer = crate::offline_voice::spoken_text(&answer);
    let audio = match work.client.synthesize(&spoken_answer).await {
        Ok(audio) => audio,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "speech synthesis",
                error,
            };
        }
    };
    TurnOutcome::Ready {
        turn: work.turn,
        scope,
        transcript,
        spoken_answer,
        persist: safely_attributed,
        audio,
    }
}

async fn is_addressed(
    transcript: &str,
    speaker: Option<u64>,
    safely_attributed: bool,
    required: bool,
    wake: &Mutex<WakeState>,
) -> bool {
    if !required {
        return true;
    }
    let now = Instant::now();
    let named = contains_wake_name(transcript);
    let mut wake = wake.lock().await;
    let continuation = speaker.is_some()
        && speaker == wake.speaker
        && wake.until.is_some_and(|until| until >= now);
    if named || continuation {
        if !safely_attributed {
            return named;
        }
        wake.speaker = speaker;
        wake.until = Some(now + CONTINUATION_WINDOW);
        true
    } else {
        false
    }
}

fn contains_wake_name(text: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphabetic())
        .any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "abbey" | "abby" | "aviva" | "abi"
            )
        })
}

fn voice_scope(
    guild_id: u64,
    channel_id: u64,
    consent_epoch: u64,
    speaker_id: Option<u64>,
    turn: u64,
    safely_attributed: bool,
) -> String {
    if safely_attributed {
        format!(
            "discord:voice:{guild_id}:{channel_id}:consent:{consent_epoch}:speaker:{}",
            speaker_id.expect("safe attribution requires a speaker")
        )
    } else {
        // Unattributed/overlapping speech gets an isolated, one-shot prompt
        // scope and is never committed after playback.
        format!("discord:voice:{guild_id}:{channel_id}:ephemeral:{consent_epoch}:{turn}")
    }
}

async fn stop_playback(playback: &SharedPlayback) -> bool {
    let track = { playback.lock().await.take() };
    let Some(track) = track else {
        return false;
    };
    playback_stop_requested(track.stop())
}

fn playback_stop_requested(result: songbird::tracks::TrackResult<()>) -> bool {
    result.is_ok()
}

async fn play_audio(
    call: &Arc<Mutex<songbird::Call>>,
    playback: &SharedPlayback,
    events: &mpsc::UnboundedSender<SessionEvent>,
    runtime: &VoiceRuntime,
    epoch: u64,
    audio: DecodedAudio,
    turn: u64,
) -> Result<bool, String> {
    let input: songbird::input::Input = RawAdapter::new(
        Cursor::new(audio.pcm_f32),
        audio.sample_rate,
        u32::from(audio.channels),
    )
    .into();
    let mut call = call.lock().await;
    let Some(handle) = runtime.with_media_enabled(epoch, || call.play_only_input(input)) else {
        return Ok(false);
    };
    drop(call);
    if let Err(error) = register_playback_termination(&handle, events, turn) {
        let _ = handle.stop();
        return Err(error);
    }
    *playback.lock().await = Some(handle);
    Ok(true)
}

async fn disconnect_call(call: &Arc<Mutex<songbird::Call>>) {
    let mut call = call.lock().await;
    // A closed software gate prevents forwarding, but a retained Decode
    // driver would still receive/decrypt packets. Stop it physically on actor
    // failure before any cosmetic mute/deafen gateway round trips;
    // command-driven teardown also removes the manager entry.
    let _ = call.leave().await;
}

fn brief(error: &str) -> String {
    let flattened = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut value: String = flattened.chars().take(300).collect();
    if flattened.chars().count() > 300 {
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(phase: VoicePhase) -> crate::voice_session::VoiceSnapshot {
        crate::voice_session::VoiceSnapshot {
            epoch: 9,
            phase,
            media_enabled: phase.processes_audio(),
            start_pending: false,
            status: "internal status must not become spoken copy".into(),
            consent_epoch: 3,
            participant_count: 2,
            dropped_input: 0,
            aborted_overruns: 0,
            barge_ins: 0,
            completed_turns: 0,
        }
    }

    #[test]
    fn wake_names_are_token_bounded_and_case_insensitive() {
        assert!(contains_wake_name("Abbey, can you help?"));
        assert!(contains_wake_name("Abby, can you help?"));
        assert!(contains_wake_name("AVIVA be direct"));
        assert!(contains_wake_name("abi: orchestrate"));
        assert!(!contains_wake_name("an abbeylike building"));
    }

    #[tokio::test]
    async fn continuation_is_scoped_to_the_same_speaker() {
        let wake = Mutex::new(WakeState::default());
        assert!(is_addressed("Abbey hello", Some(1), true, true, &wake).await);
        assert!(is_addressed("and one more thing", Some(1), true, true, &wake).await);
        assert!(!is_addressed("private aside", Some(2), true, true, &wake).await);
        assert!(!is_addressed("unknown voice", None, false, true, &wake).await);
        assert!(is_addressed("Abbey explicit", Some(2), false, true, &wake).await);
        assert!(!is_addressed("unsafe continuation", Some(2), false, true, &wake).await);
    }

    #[test]
    fn safely_attributed_withdrawal_is_classified_before_wake_gating() {
        let active = snapshot(VoicePhase::Thinking);
        assert!(pre_wake_withdrawal("stop listening", &active, true, true));
        assert!(pre_wake_withdrawal(
            "I withdraw my consent",
            &active,
            true,
            true
        ));
        assert!(pre_wake_withdrawal(
            "stop listening please",
            &active,
            true,
            true
        ));
        assert!(pre_wake_withdrawal("I do not consent", &active, true, true));
        assert!(!pre_wake_withdrawal("stop listening", &active, false, true));
        assert!(!pre_wake_withdrawal("stop listening", &active, true, false));
        assert!(!pre_wake_withdrawal("I consent", &active, true, true));
    }

    #[test]
    fn operational_voice_questions_use_runtime_copy_not_model_prose() {
        let active = snapshot(VoicePhase::Listening);
        let Some(OperationalVoiceTurn::Reply(reply)) =
            operational_voice_turn("Abbey, is voice active?", &active)
        else {
            panic!("expected fixed operational reply");
        };
        assert!(reply.contains("Voice is active for the current consent epoch"));
        assert!(!reply.contains("internal status"));
        assert!(operational_voice_turn("Abbey, tell me a joke", &active).is_none());
    }

    #[test]
    fn transcript_scopes_isolate_consent_speakers_and_unattributed_turns() {
        let first = voice_scope(1, 2, 3, Some(4), 5, true);
        let same = voice_scope(1, 2, 3, Some(4), 99, true);
        let other_speaker = voice_scope(1, 2, 3, Some(6), 5, true);
        let later_consent = voice_scope(1, 2, 7, Some(4), 5, true);
        let unknown_a = voice_scope(1, 2, 3, None, 8, false);
        let unknown_b = voice_scope(1, 2, 3, None, 9, false);
        assert_eq!(first, same);
        assert_ne!(first, other_speaker);
        assert_ne!(first, later_consent);
        assert_ne!(unknown_a, unknown_b);
    }

    #[test]
    fn armed_verification_disables_conversation_commits() {
        assert!(should_commit_turn(true, false));
        assert!(!should_commit_turn(true, true));
        assert!(!should_commit_turn(false, false));
        assert!(!should_commit_turn(false, true));
    }

    #[test]
    fn stop_command_result_only_arms_later_confirmation() {
        assert!(playback_stop_requested(Ok(())));
        assert!(!playback_stop_requested(Err(
            songbird::tracks::ControlError::Finished
        )));
    }

    #[test]
    fn playback_lifecycle_distinguishes_every_terminal_outcome() {
        let mut lifecycle = PlaybackLifecycle::default();
        assert_eq!(
            lifecycle.observe(7, 7, PlaybackTermination::Natural),
            PlaybackObservation::NaturalCompletion
        );
        assert_eq!(
            lifecycle.observe(8, 8, PlaybackTermination::Errored),
            PlaybackObservation::Errored
        );
        assert_eq!(
            lifecycle.observe(9, 9, PlaybackTermination::Stopped),
            PlaybackObservation::OrdinaryStop
        );

        lifecycle.note_barge_stop_requested(9);
        assert_eq!(
            lifecycle.observe(10, 9, PlaybackTermination::Natural),
            PlaybackObservation::Stale,
            "a natural-end race must not be reported as barge cancellation"
        );

        lifecycle.note_barge_stop_requested(10);
        assert_eq!(
            lifecycle.observe(11, 10, PlaybackTermination::Errored),
            PlaybackObservation::Stale,
            "a playback error must not be reported as barge cancellation"
        );

        lifecycle.note_barge_stop_requested(11);
        assert_eq!(
            lifecycle.observe(12, 11, PlaybackTermination::Stopped),
            PlaybackObservation::ConfirmedBargeInCancellation
        );
    }
}
