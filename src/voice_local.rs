//! Local STT → canonical Abbey cognition → local TTS session actor.
//!
//! Audio callbacks only enqueue fixed frames. This task owns turn segmentation,
//! cancellation, WDBX-scoped context, persona routing, the existing tool loop,
//! and one playback track per response. Raw audio and provider transcripts are
//! never persisted.

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
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
use crate::voice_session::{SessionEvent, SharedPlayback, VoicePhase, VoiceRuntime};

const VOICE_SYSTEM_SUFFIX: &str = "You are speaking aloud in a consented Discord voice session. Respond in one to three short, natural sentences unless the user explicitly asks for detail. Avoid Markdown, raw URLs, emoji, tables, headings, and unspoken formatting. Pronounce code, symbols, and acronyms clearly. Voice turns are read-only: never claim an external action or durable memory change succeeded.";
const CONTINUATION_WINDOW: Duration = Duration::from_secs(45);

pub struct LocalSession {
    pub runtime: Arc<VoiceRuntime>,
    pub state: Arc<AppState>,
    pub call: Arc<Mutex<songbird::Call>>,
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
    Failed {
        turn: u64,
        stage: &'static str,
        error: String,
    },
}

struct PendingCommit {
    turn: u64,
    scope: String,
    transcript: String,
    spoken_answer: String,
}

pub async fn run(mut session: LocalSession) {
    let local_config = session
        .runtime
        .config
        .local()
        .expect("local actor starts only for local mode")
        .clone();
    let client = match MlxAudioClient::new(local_config) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(error = %brief(&error), "local voice client construction failed");
            session
                .runtime
                .set_status(
                    session.epoch,
                    VoicePhase::Failed,
                    "local speech client could not start",
                )
                .await;
            return;
        }
    };
    let wake = Arc::new(Mutex::new(WakeState::default()));
    let mut segmenter = Segmenter::new();
    let mut turns = JoinSet::new();
    let mut turn_generation = 0_u64;
    let mut pending_commit: Option<PendingCommit> = None;

    session
        .runtime
        .set_status(
            session.epoch,
            VoicePhase::Listening,
            "local inference ready; listening for Abbey",
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
                    mute_and_deafen(&session.call).await;
                    break;
                }
            }
            lifecycle = session.lifecycle.recv() => {
                let Some(SessionEvent::PlaybackEnded(turn)) = lifecycle else { continue; };
                if turn == turn_generation {
                    session.playback.lock().await.take();
                    if let Some(pending) = pending_commit.take()
                        && pending.turn == turn
                        && session.runtime.media_enabled(session.epoch)
                    {
                        AppState::lock(&session.state.engine).commit(
                            &pending.scope,
                            &pending.transcript,
                            &pending.spoken_answer,
                            runtime::now(),
                        );
                        session.runtime.note_completed_turn();
                    }
                    session.runtime
                        .set_status(
                            session.epoch,
                            VoicePhase::Listening,
                            "local inference ready; listening for Abbey",
                        )
                        .await;
                }
            }
            frame = session.input.recv() => {
                let Some(frame) = frame else { break; };
                for event in segmenter.push(frame) {
                    match event {
                        SegmentEvent::SpeechStarted { .. } => {
                            turn_generation = turn_generation.saturating_add(1);
                            pending_commit = None;
                            let interrupted =
                                !turns.is_empty() || stop_playback(&session.playback).await;
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
                            turns.spawn(process_turn(work));
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
                            audio,
                            turn,
                        ).await {
                            Ok(()) => {
                                pending_commit = persist.then_some(PendingCommit {
                                    turn,
                                    scope,
                                    transcript,
                                    spoken_answer,
                                });
                                session.runtime
                                    .set_status(
                                        session.epoch,
                                        VoicePhase::Speaking,
                                        "speaking locally generated Abbey audio",
                                    )
                                    .await;
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
    let safely_attributed = work.utterance.speaker_id.is_some() && !work.utterance.overlap;
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

    let persona = persona::route(&transcript, None).persona;
    let scope = voice_scope(
        work.guild_id,
        work.channel_id,
        work.consent_epoch,
        work.utterance.speaker_id,
        work.turn,
        safely_attributed,
    );
    let scoped_guild = format!("discord:{}", work.guild_id);
    let scoped_user = work.utterance.speaker_id.map_or_else(
        || "discord:voice:unattributed".into(),
        |id| format!("discord:{id}"),
    );
    let context = if safely_attributed {
        pipeline::assemble_context(
            &work.state,
            &scoped_guild,
            &scoped_user,
            &scope,
            &transcript,
        )
    } else {
        PersonaContext::empty()
    };
    let mut host = crate::runtime::ToolScope {
        state: &work.state,
        scoped_guild,
        scoped_user,
        scoped_channel: scope.clone(),
        persona,
    };
    let generation = match work.state.acquire_generation().await {
        Err(error) => Err(crate::llm::LlmError(error)),
        Ok(_slot) => {
            generation::generate_with_backend::<generation::NoDelivery>(
                &work.state,
                &work.backend,
                &mut host,
                &generation::Ask {
                    scope: &scope,
                    context: &context,
                    user_input: &transcript,
                    // A canceled task cannot roll back a tool that already
                    // mutated memory. Voice therefore receives WDBX/persona
                    // context but never side-effecting model tools.
                    offer_tools: false,
                    now: runtime::now(),
                },
                None,
                Some(VOICE_SYSTEM_SUFFIX),
            )
            .await
        }
    };
    let (answer, _, _) = match generation {
        Ok(answer) => answer,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "Abbey reasoning",
                error: error.0,
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
                "abbey" | "aviva" | "abi"
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
    let _ = track.stop();
    true
}

struct PlaybackEnd {
    tx: mpsc::UnboundedSender<SessionEvent>,
    turn: u64,
}

#[serenity::async_trait]
impl EventHandler for PlaybackEnd {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let _ = self.tx.send(SessionEvent::PlaybackEnded(self.turn));
        None
    }
}

async fn play_audio(
    call: &Arc<Mutex<songbird::Call>>,
    playback: &SharedPlayback,
    events: &mpsc::UnboundedSender<SessionEvent>,
    audio: DecodedAudio,
    turn: u64,
) -> Result<(), String> {
    let input: songbird::input::Input = RawAdapter::new(
        Cursor::new(audio.pcm_f32),
        audio.sample_rate,
        u32::from(audio.channels),
    )
    .into();
    let handle = call.lock().await.play_only_input(input);
    if let Err(error) = handle.add_event(
        Event::Track(TrackEvent::End),
        PlaybackEnd {
            tx: events.clone(),
            turn,
        },
    ) {
        let _ = handle.stop();
        return Err(format!(
            "registering the playback completion event failed: {error}"
        ));
    }
    *playback.lock().await = Some(handle);
    Ok(())
}

async fn mute_and_deafen(call: &Arc<Mutex<songbird::Call>>) {
    let mut call = call.lock().await;
    let _ = call.deafen(true).await;
    let _ = call.mute(true).await;
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

    #[test]
    fn wake_names_are_token_bounded_and_case_insensitive() {
        assert!(contains_wake_name("Abbey, can you help?"));
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
}
