//! Explicit OpenAI Realtime backup for Discord voice.
//!
//! This path is never selected by key presence. It runs only when the join
//! that spawned it snapshotted the OpenAI backend (`ABBEY_VOICE_MODE=openai`
//! at startup, or a later `/voice mode openai`), caps WebSocket/audio memory,
//! owns cancellation, and truncates provider history to the audio actually
//! heard after barge-in.

mod protocol;

use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use futures_util::{Sink, SinkExt, StreamExt};
use songbird::input::RawAdapter;
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::offline_voice::FrameSequence;
use crate::vad::{ComposedVad, Vad, VadCtx};
use crate::voice::OpenAiVoiceConfig;
use crate::voice_session::{
    PlaybackTermination, SessionEvent, SharedPlayback, VoicePhase, VoiceRuntime,
    register_playback_termination,
};
use protocol::{
    ResponseBuffer, cancel_response, capture_output_item, next_event_id, pcm16_to_f32,
    truncate_event,
};

const MAX_WS_MESSAGE_BYTES: usize = 512 * 1024;

pub struct OpenAiSession {
    pub runtime: Arc<VoiceRuntime>,
    /// The backend the join snapshotted and announced. The actor must never
    /// re-read it from the runtime: the startup selection may be a different
    /// mode, and the consent notice already named this one.
    pub config: OpenAiVoiceConfig,
    pub call: Arc<Mutex<songbird::Call>>,
    pub epoch: u64,
    pub input: mpsc::Receiver<crate::offline_voice::VoiceFrame>,
    pub lifecycle: mpsc::UnboundedReceiver<SessionEvent>,
    pub events: mpsc::UnboundedSender<SessionEvent>,
    pub driver_disconnect: watch::Receiver<bool>,
    pub cancel: watch::Receiver<bool>,
    pub playback: SharedPlayback,
    pub ready: Option<oneshot::Sender<Result<(), String>>>,
}

pub async fn run(mut session: OpenAiSession) {
    let result = run_inner(&mut session).await;
    if let Err(error) = result {
        tracing::error!(error = %brief(&error), "OpenAI Realtime voice stopped");
        if let Some(ready) = session.ready.take() {
            let _ = ready.send(Err("OpenAI Realtime connection or setup failed".into()));
        }
        // Close this actor's exact software media epoch before any async
        // cleanup, then stop Songbird's Decode driver before publishing the
        // failed state. A stale actor cannot close a newer epoch through this
        // compare/exchange.
        let _ = session.runtime.revoke_media(session.epoch);
        disconnect_call(&session.call).await;
        session
            .runtime
            .actor_failed(
                session.epoch,
                "OpenAI Realtime stopped; audio processing is closed",
            )
            .await;
    }
    let track = { session.playback.lock().await.take() };
    if let Some(track) = track {
        let _ = track.stop();
    }
}

async fn run_inner(session: &mut OpenAiSession) -> Result<(), String> {
    let config = session.config.clone();
    let mut request = config
        .websocket_url()
        .into_client_request()
        .map_err(|error| format!("building the Realtime request failed: {error}"))?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&config.authorization())
            .map_err(|_| "OPENAI_API_KEY contains invalid header bytes".to_string())?,
    );
    let mut socket_config = WebSocketConfig::default();
    socket_config.max_message_size = Some(MAX_WS_MESSAGE_BYTES);
    socket_config.max_frame_size = Some(MAX_WS_MESSAGE_BYTES);
    socket_config.max_write_buffer_size = 2 * MAX_WS_MESSAGE_BYTES;
    let connect = tokio_tungstenite::connect_async_with_config(request, Some(socket_config), false);
    let (socket, _) = tokio::select! {
        result = connect => result.map_err(|error| format!("Realtime WebSocket connection failed: {error}"))?,
        changed = session.cancel.changed() => {
            let _ = changed;
            return Ok(());
        }
    };
    let (mut writer, mut reader) = socket.split();
    let mut event_sequence = 0_u64;
    let update = serde_json::json!({
        "type": "session.update",
        "event_id": next_event_id(session.epoch, &mut event_sequence),
        "session": {
            "type": "realtime",
            "model": config.model,
            "output_modalities": ["audio"],
            "instructions": config.instructions,
            "audio": {
                "input": {
                    "format": {"type": "audio/pcm", "rate": 24000},
                    "turn_detection": {
                        "type": "semantic_vad",
                        "interrupt_response": false
                    }
                },
                "output": {
                    "format": {"type": "audio/pcm"},
                    "voice": config.voice
                }
            }
        }
    });
    writer
        .send(Message::Text(update.to_string().into()))
        .await
        .map_err(|error| format!("sending Realtime session configuration failed: {error}"))?;

    let vad = ComposedVad::default();
    let mut responses = ResponseBuffer::default();
    let mut frame_sequence = FrameSequence::default();
    let mut playing_item_id: Option<String> = None;
    let mut playing_turn: Option<u64> = None;
    let mut playback_turn = 0_u64;

    loop {
        // Keep Tokio's fair branch ordering. The bounded audio receiver can be
        // continuously ready; biased polling would starve provider events and
        // delay cancellation, item correlation, and playback indefinitely.
        tokio::select! {
            changed = session.cancel.changed() => {
                if changed.is_err() || *session.cancel.borrow() {
                    let _ = writer.close().await;
                    return Ok(());
                }
            }
            changed = session.driver_disconnect.changed() => {
                if changed.is_err() || *session.driver_disconnect.borrow() {
                    return Err("Discord voice transport disconnected".into());
                }
            }
            lifecycle = session.lifecycle.recv() => {
                let Some(SessionEvent::PlaybackTerminated { turn, termination }) = lifecycle else {
                    continue;
                };
                if consume_matching_playback_termination(&mut playing_turn, turn) {
                    session.playback.lock().await.take();
                    playing_item_id = None;
                    if termination == PlaybackTermination::Natural
                        && session
                            .runtime
                            .with_media_enabled(session.epoch, || {
                                session.runtime.note_completed_turn();
                            })
                            .is_some()
                    {
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "direct OpenAI backup ready; buffered output; listening",
                            )
                            .await;
                    } else if termination != PlaybackTermination::Natural {
                        tracing::warn!(
                            epoch = session.epoch,
                            turn,
                            ?termination,
                            "OpenAI backup playback ended without natural completion"
                        );
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "direct OpenAI backup playback ended early; listening",
                            )
                            .await;
                    }
                }
            }
            frame = session.input.recv() => {
                let Some(frame) = frame else {
                    let _ = writer.close().await;
                    return Ok(());
                };
                if !session.runtime.media_enabled(session.epoch) {
                    continue;
                }
                enforce_input_sequence(
                    &mut frame_sequence,
                    &session.runtime,
                    frame.sequence,
                )?;
                // Energy is the pre-filter: silent frames never reach the
                // provider. This prevents local silence from clipping a turn
                // before the provider's semantic VAD can fire.
                if !vad.is_voice(&frame.samples) {
                    continue;
                }
                let bytes: Vec<u8> = frame.samples.into_iter().flat_map(i16::to_le_bytes).collect();
                let append = Message::Text(serde_json::json!({
                    "type": "input_audio_buffer.append",
                    "event_id": next_event_id(session.epoch, &mut event_sequence),
                    "audio": base64::engine::general_purpose::STANDARD.encode(bytes),
                }).to_string().into());
                if !send_media_message(
                    &mut writer,
                    &session.runtime,
                    session.epoch,
                    append,
                )
                .await?
                {
                    continue;
                }
            }
            message = reader.next() => {
                let message = message
                    .ok_or_else(|| "Realtime WebSocket closed".to_string())?
                    .map_err(|error| format!("reading the Realtime WebSocket failed: {error}"))?;
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) {
                        return Err("Realtime provider closed the session".into());
                    }
                    continue;
                };
                let event: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|error| format!("Realtime provider sent invalid JSON: {error}"))?;
                match event.get("type").and_then(serde_json::Value::as_str) {
                    Some("session.updated") => {
                        if let Some(ready) = session.ready.take() {
                            let _ = ready.send(Ok(()));
                        }
                        session.runtime
                            .set_prepared_status(
                                session.epoch,
                                "direct OpenAI backup ready; awaiting final consent activation",
                            )
                            .await;
                    }
                    Some("input_audio_buffer.speech_started") => {
                        // Semantic is the sole interruption decision: only the
                        // provider's `speech_started` event may cancel a
                        // response. No local silence counter remains.
                        let ctx = VadCtx {
                            provider_speech_started: true,
                        };
                        if vad.should_interrupt(&ctx)
                            && interrupt_response(
                                &mut writer,
                                session.epoch,
                                &mut event_sequence,
                                &mut responses,
                                &session.playback,
                                &mut playing_item_id,
                                &mut playing_turn,
                            )
                            .await?
                        {
                            session.runtime.note_barge_in();
                        }
                    }
                    Some("response.created") => {
                        if let Some(id) = event.pointer("/response/id")
                            .and_then(serde_json::Value::as_str)
                        {
                            responses.start(id)?;
                        }
                    }
                    Some("response.output_item.added") => {
                        let _ = capture_output_item(
                            &mut writer,
                            session.epoch,
                            &mut event_sequence,
                            &mut responses,
                            &event,
                        ).await?;
                    }
                    Some("response.output_audio.delta" | "response.audio.delta") => {
                        let Some(response_id) = event.get("response_id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let Some(item_id) = event.get("item_id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let Some(delta) = event.get("delta").and_then(serde_json::Value::as_str) else {
                            continue;
                        };
                        responses.append_encoded_audio(response_id, item_id, delta)?;
                    }
                    Some("response.output_audio.done" | "response.audio.done") => {}
                    Some("response.done") => {
                        let Some(response_id) = event.pointer("/response/id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let status = event.pointer("/response/status")
                            .and_then(serde_json::Value::as_str);
                        let Some((item_id, pcm)) = responses
                            .take_completed_audio(response_id, status) else { continue; };
                        if !session.runtime.media_enabled(session.epoch) {
                            continue;
                        }
                        playback_turn = playback_turn.saturating_add(1);
                        let f32_pcm = pcm16_to_f32(&pcm);
                        playing_item_id = item_id;
                        let played = play_audio(
                            &session.call,
                            &session.playback,
                            &session.events,
                            &session.runtime,
                            session.epoch,
                            f32_pcm,
                            playback_turn,
                        ).await?;
                        if !played {
                            playing_item_id = None;
                            continue;
                        }
                        playing_turn = Some(playback_turn);
                        session.runtime
                            .set_status(
                                session.epoch,
                                VoicePhase::Speaking,
                                "speaking buffered direct OpenAI backup audio",
                            )
                            .await;
                    }
                    Some("error") => {
                        let error_type = event.pointer("/error/type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown");
                        let code = event.pointer("/error/code")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown");
                        let event_id = event.pointer("/error/event_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("none");
                        if let Some(ready) = session.ready.take() {
                            let _ = ready.send(Err("OpenAI Realtime rejected session setup".into()));
                            return Err("Realtime provider rejected session setup".into());
                        }
                        tracing::warn!(error_type, code, event_id, "recoverable Realtime error event");
                    }
                    _ => {}
                }
            }
        }
    }
}

fn enforce_input_sequence(
    sequence: &mut FrameSequence,
    runtime: &VoiceRuntime,
    actual: u64,
) -> Result<(), String> {
    sequence.observe(actual).map_err(|gap| {
        runtime.note_overrun();
        format!(
            "Realtime input sequence gap (expected {}, received {}); closing rather than splicing PCM",
            gap.expected, gap.actual
        )
    })
}

/// Wait for sink capacity without holding the consent mutex, then linearize
/// the actual audio enqueue with media revocation. A withdrawal that wins this
/// boundary drops the frame before it enters the WebSocket sink.
async fn send_media_message<S>(
    writer: &mut S,
    runtime: &VoiceRuntime,
    epoch: u64,
    message: Message,
) -> Result<bool, String>
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    futures_util::future::poll_fn(|cx| Pin::new(&mut *writer).poll_ready(cx))
        .await
        .map_err(|error| format!("waiting to send live input audio failed: {error}"))?;
    let Some(started) =
        runtime.with_media_enabled(epoch, || Pin::new(&mut *writer).start_send(message))
    else {
        return Ok(false);
    };
    started.map_err(|error| format!("sending live input audio failed: {error}"))?;
    writer
        .flush()
        .await
        .map_err(|error| format!("flushing live input audio failed: {error}"))?;
    Ok(true)
}

async fn interrupt_response<S>(
    writer: &mut S,
    epoch: u64,
    event_sequence: &mut u64,
    responses: &mut ResponseBuffer,
    playback: &SharedPlayback,
    playing_item_id: &mut Option<String>,
    playing_turn: &mut Option<u64>,
) -> Result<bool, String>
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let mut interrupted = cancel_response(writer, epoch, event_sequence, responses).await?;

    if playing_turn.take().is_some() {
        interrupted = true;
    }
    let track = { playback.lock().await.take() };
    if let Some(track) = track {
        interrupted = true;
        let played_ms = tokio::time::timeout(Duration::from_millis(100), track.get_info())
            .await
            .ok()
            .and_then(Result::ok)
            .map_or(0_u64, |state| {
                u64::try_from(state.position.as_millis())
                    .unwrap_or(u64::MAX)
                    .saturating_sub(150)
            });
        let _ = track.stop();
        if let Some(item_id) = playing_item_id.take() {
            writer
                .send(truncate_event(epoch, event_sequence, &item_id, played_ms))
                .await
                .map_err(|error| format!("truncating unheard Realtime audio failed: {error}"))?;
        }
    }
    Ok(interrupted)
}

/// Consume only the still-armed matching turn. The caller separately checks
/// typed Songbird termination provenance before recording completion.
fn consume_matching_playback_termination(
    playing_turn: &mut Option<u64>,
    terminated_turn: u64,
) -> bool {
    if *playing_turn != Some(terminated_turn) {
        return false;
    }
    *playing_turn = None;
    true
}

async fn play_audio(
    call: &Arc<Mutex<songbird::Call>>,
    playback: &SharedPlayback,
    events: &mpsc::UnboundedSender<SessionEvent>,
    runtime: &VoiceRuntime,
    epoch: u64,
    pcm_f32: Vec<u8>,
    turn: u64,
) -> Result<bool, String> {
    let input: songbird::input::Input = RawAdapter::new(Cursor::new(pcm_f32), 24_000, 1).into();
    let mut call = call.lock().await;
    let Some(handle) = runtime.with_media_enabled(epoch, || call.play_input(input)) else {
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
    // The caller has already closed the software gate. Leave immediately to
    // stop Songbird's Decode driver; mute/deafen gateway round trips would
    // only delay the physical teardown.
    let _ = call.leave().await;
}

fn brief(error: &str) -> String {
    error
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RevokingReadySink {
        messages: Vec<Message>,
        runtime: Arc<VoiceRuntime>,
        epoch: u64,
        revoked: bool,
    }

    impl Sink<Message> for RevokingReadySink {
        type Error = std::convert::Infallible;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            let this = self.get_mut();
            if !this.revoked {
                assert!(this.runtime.revoke_media(this.epoch));
                this.revoked = true;
            }
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.get_mut().messages.push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn withdrawal_between_socket_readiness_and_enqueue_drops_audio() {
        let runtime = Arc::new(VoiceRuntime::new(crate::voice::VoiceConfig::selected_only(
            1,
            2,
            crate::voice::VoiceBackendConfig::Disabled,
            true,
        )));
        let generation = runtime.reserve_start();
        let epoch = runtime.begin(std::collections::HashSet::new()).await;
        assert!(runtime.activate(epoch, generation, "active").await);
        let mut sink = RevokingReadySink {
            messages: Vec::new(),
            runtime: Arc::clone(&runtime),
            epoch,
            revoked: false,
        };
        assert!(
            !send_media_message(
                &mut sink,
                &runtime,
                epoch,
                Message::Text("private audio".into())
            )
            .await
            .expect("revocation is not a transport failure")
        );
        assert!(sink.messages.is_empty());
        assert!(!runtime.media_enabled(epoch));
    }

    #[tokio::test]
    async fn input_sequence_gap_counts_overrun_and_fails_closed() {
        let runtime = VoiceRuntime::new(crate::voice::VoiceConfig::selected_only(
            1,
            2,
            crate::voice::VoiceBackendConfig::Disabled,
            true,
        ));
        let mut sequence = FrameSequence::default();
        enforce_input_sequence(&mut sequence, &runtime, 10).unwrap();
        let error = enforce_input_sequence(&mut sequence, &runtime, 12).unwrap_err();
        assert!(error.contains("expected 11, received 12"), "{error}");
        assert_eq!(runtime.snapshot().await.aborted_overruns, 1);
    }

    #[test]
    fn only_matching_playback_termination_consumes_the_armed_turn() {
        let mut playing_turn = Some(11);
        assert!(!consume_matching_playback_termination(
            &mut playing_turn,
            10
        ));
        assert_eq!(playing_turn, Some(11));
        assert!(consume_matching_playback_termination(&mut playing_turn, 11));
        assert_eq!(playing_turn, None);
        assert!(!consume_matching_playback_termination(
            &mut playing_turn,
            11
        ));

        // Barge-in/teardown invalidates the armed turn before Songbird emits
        // the End event caused by TrackHandle::stop().
        let mut interrupted_turn = Some(12);
        interrupted_turn.take();
        assert!(!consume_matching_playback_termination(
            &mut interrupted_turn,
            12
        ));
    }
}
