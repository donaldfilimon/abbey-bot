//! Explicit OpenAI Realtime backup for Discord voice.
//!
//! This path is never selected by key presence. It runs only under
//! `ABBEY_VOICE_MODE=openai`, caps WebSocket/audio memory, owns cancellation,
//! and truncates provider history to the audio actually heard after barge-in.

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use futures_util::{Sink, SinkExt, StreamExt};
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::RawAdapter;
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::offline_voice::frame_is_voice;
use crate::voice_session::{SessionEvent, SharedPlayback, VoicePhase, VoiceRuntime};

const MAX_WS_MESSAGE_BYTES: usize = 512 * 1024;
const MAX_OUTPUT_PCM_BYTES: usize = 24_000 * 2 * 45;
const SPEECH_END_SILENCE_FRAMES: usize = 15;

pub struct OpenAiSession {
    pub runtime: Arc<VoiceRuntime>,
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
        session
            .runtime
            .actor_failed(
                session.epoch,
                "OpenAI Realtime stopped; audio processing is closed",
            )
            .await;
        mute_and_deafen(&session.call).await;
    }
    let track = { session.playback.lock().await.take() };
    if let Some(track) = track {
        let _ = track.stop();
    }
}

struct ActiveResponse {
    id: String,
    item_id: Option<String>,
    pcm: Vec<u8>,
    cancelled: bool,
}

async fn run_inner(session: &mut OpenAiSession) -> Result<(), String> {
    let config = session
        .runtime
        .config
        .openai()
        .ok_or_else(|| "OpenAI voice actor started under a different mode".to_string())?
        .clone();
    let mut request = config
        .websocket_url()
        .into_client_request()
        .map_err(|error| format!("building the Realtime request failed: {error}"))?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&config.authorization())
            .map_err(|_| "OPENAI_API_KEY contains invalid header bytes".to_string())?,
    );
    let socket_config = WebSocketConfig {
        max_message_size: Some(MAX_WS_MESSAGE_BYTES),
        max_frame_size: Some(MAX_WS_MESSAGE_BYTES),
        max_write_buffer_size: 2 * MAX_WS_MESSAGE_BYTES,
        ..WebSocketConfig::default()
    };
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
        .send(Message::Text(update.to_string()))
        .await
        .map_err(|error| format!("sending Realtime session configuration failed: {error}"))?;

    let mut human_speaking = false;
    let mut silence_frames = 0_usize;
    let mut active_response: Option<ActiveResponse> = None;
    let mut playing_item_id: Option<String> = None;
    let mut playback_turn = 0_u64;

    loop {
        tokio::select! {
            biased;
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
                let Some(SessionEvent::PlaybackEnded(turn)) = lifecycle else { continue; };
                if turn == playback_turn {
                    session.playback.lock().await.take();
                    playing_item_id = None;
                    session.runtime
                        .set_status(
                            session.epoch,
                            VoicePhase::Listening,
                            "direct OpenAI backup ready; buffered output; listening",
                        )
                        .await;
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
                let voiced = frame_is_voice(&frame.samples);
                if voiced && !human_speaking {
                    human_speaking = true;
                    silence_frames = 0;
                    if interrupt_response(
                        &mut writer,
                        session.epoch,
                        &mut event_sequence,
                        &mut active_response,
                        &session.playback,
                        &mut playing_item_id,
                    ).await? {
                        session.runtime.note_barge_in();
                    }
                } else if voiced {
                    silence_frames = 0;
                } else if human_speaking {
                    silence_frames = silence_frames.saturating_add(1);
                    if silence_frames >= SPEECH_END_SILENCE_FRAMES {
                        human_speaking = false;
                        silence_frames = 0;
                    }
                }
                let bytes: Vec<u8> = frame.samples.into_iter().flat_map(i16::to_le_bytes).collect();
                writer.send(Message::Text(serde_json::json!({
                    "type": "input_audio_buffer.append",
                    "event_id": next_event_id(session.epoch, &mut event_sequence),
                    "audio": base64::engine::general_purpose::STANDARD.encode(bytes),
                }).to_string())).await
                    .map_err(|error| format!("sending live input audio failed: {error}"))?;
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
                            .set_status(
                                session.epoch,
                                VoicePhase::Listening,
                                "direct OpenAI backup ready; buffered output; listening",
                            )
                            .await;
                    }
                    Some("input_audio_buffer.speech_started") => {
                        human_speaking = true;
                        silence_frames = 0;
                        if interrupt_response(
                            &mut writer,
                            session.epoch,
                            &mut event_sequence,
                            &mut active_response,
                            &session.playback,
                            &mut playing_item_id,
                        ).await? {
                            session.runtime.note_barge_in();
                        }
                    }
                    Some("response.created") => {
                        if let Some(id) = event.pointer("/response/id")
                            .and_then(serde_json::Value::as_str)
                        {
                            active_response = Some(ActiveResponse {
                                id: id.to_string(),
                                item_id: None,
                                pcm: Vec::new(),
                                cancelled: false,
                            });
                        }
                    }
                    Some("response.output_audio.delta" | "response.audio.delta") => {
                        let Some(response_id) = event.get("response_id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let Some(item_id) = event.get("item_id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let Some(delta) = event.get("delta").and_then(serde_json::Value::as_str) else {
                            continue;
                        };
                        let decoded = base64::engine::general_purpose::STANDARD
                            .decode(delta)
                            .map_err(|error| format!("Realtime audio delta was not valid base64: {error}"))?;
                        append_audio_delta(
                            &mut active_response,
                            response_id,
                            item_id,
                            &decoded,
                        )?;
                    }
                    Some("response.output_audio.done" | "response.audio.done") => {}
                    Some("response.done") => {
                        let Some(response_id) = event.pointer("/response/id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let status = event.pointer("/response/status")
                            .and_then(serde_json::Value::as_str);
                        let Some((item_id, pcm)) = take_completed_audio(
                            &mut active_response,
                            response_id,
                            status,
                        ) else { continue; };
                        if !session.runtime.media_enabled(session.epoch) {
                            continue;
                        }
                        playback_turn = playback_turn.saturating_add(1);
                        let f32_pcm = pcm16_to_f32(&pcm);
                        playing_item_id = item_id;
                        play_audio(
                            &session.call,
                            &session.playback,
                            &session.events,
                            f32_pcm,
                            playback_turn,
                        ).await?;
                        session.runtime.note_completed_turn();
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

fn append_audio_delta(
    active_response: &mut Option<ActiveResponse>,
    response_id: &str,
    item_id: &str,
    decoded: &[u8],
) -> Result<bool, String> {
    let Some(active) = active_response.as_mut() else {
        return Ok(false);
    };
    if active.cancelled || active.id != response_id {
        return Ok(false);
    }
    if active.item_id.as_deref().is_some_and(|id| id != item_id) {
        return Ok(false);
    }
    if !decoded.len().is_multiple_of(2)
        || active.pcm.len().saturating_add(decoded.len()) > MAX_OUTPUT_PCM_BYTES
    {
        return Err("Realtime audio exceeded the bounded PCM duration".into());
    }
    active.item_id.get_or_insert_with(|| item_id.to_string());
    active.pcm.extend_from_slice(decoded);
    Ok(true)
}

fn take_completed_audio(
    active_response: &mut Option<ActiveResponse>,
    response_id: &str,
    status: Option<&str>,
) -> Option<(Option<String>, Vec<u8>)> {
    if active_response.as_ref()?.id != response_id {
        return None;
    }
    let active = active_response.take()?;
    if active.cancelled || status != Some("completed") || active.pcm.is_empty() {
        return None;
    }
    Some((active.item_id, active.pcm))
}

async fn interrupt_response<S>(
    writer: &mut S,
    epoch: u64,
    event_sequence: &mut u64,
    active_response: &mut Option<ActiveResponse>,
    playback: &SharedPlayback,
    playing_item_id: &mut Option<String>,
) -> Result<bool, String>
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let mut interrupted = false;
    let response_to_cancel = active_response.as_mut().and_then(|active| {
        if active.cancelled {
            None
        } else {
            active.cancelled = true;
            active.pcm.clear();
            interrupted = true;
            Some((active.id.clone(), active.item_id.clone()))
        }
    });
    if let Some((response_id, buffered_item_id)) = response_to_cancel {
        writer
            .send(Message::Text(
                serde_json::json!({
                    "type": "response.cancel",
                    "event_id": next_event_id(epoch, event_sequence),
                    "response_id": response_id,
                })
                .to_string(),
            ))
            .await
            .map_err(|error| format!("cancelling the Realtime response failed: {error}"))?;
        // Output is intentionally whole-response buffered in this backup path.
        // If speech arrives before playback starts, none of the already-created
        // assistant item was heard; truncate it to zero so provider history
        // cannot retain an answer Discord participants never received.
        if let Some(item_id) = buffered_item_id {
            writer
                .send(Message::Text(
                    serde_json::json!({
                        "type": "conversation.item.truncate",
                        "event_id": next_event_id(epoch, event_sequence),
                        "item_id": item_id,
                        "content_index": 0,
                        "audio_end_ms": 0,
                    })
                    .to_string(),
                ))
                .await
                .map_err(|error| format!("truncating buffered Realtime audio failed: {error}"))?;
        }
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
                .send(Message::Text(
                    serde_json::json!({
                        "type": "conversation.item.truncate",
                        "event_id": next_event_id(epoch, event_sequence),
                        "item_id": item_id,
                        "content_index": 0,
                        "audio_end_ms": played_ms,
                    })
                    .to_string(),
                ))
                .await
                .map_err(|error| format!("truncating unheard Realtime audio failed: {error}"))?;
        }
    }
    Ok(interrupted)
}

fn next_event_id(epoch: u64, sequence: &mut u64) -> String {
    *sequence = sequence.saturating_add(1);
    format!("abbey-{epoch}-{sequence}")
}

fn pcm16_to_f32(pcm: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(pcm.len() * 2);
    for pair in pcm.chunks_exact(2) {
        let sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
        output.extend_from_slice(&sample.to_le_bytes());
    }
    output
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
    pcm_f32: Vec<u8>,
    turn: u64,
) -> Result<(), String> {
    let input: songbird::input::Input = RawAdapter::new(Cursor::new(pcm_f32), 24_000, 1).into();
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

    #[derive(Default)]
    struct RecordingSink(Vec<Message>);

    impl Sink<Message> for RecordingSink {
        type Error = std::convert::Infallible;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(
            mut self: std::pin::Pin<&mut Self>,
            item: Message,
        ) -> Result<(), Self::Error> {
            self.0.push(item);
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn pcm16_conversion_does_not_duplicate_samples_or_channels() {
        let bytes = [
            i16::MIN.to_le_bytes(),
            0_i16.to_le_bytes(),
            i16::MAX.to_le_bytes(),
        ]
        .concat();
        let output = pcm16_to_f32(&bytes);
        assert_eq!(output.len(), 3 * size_of::<f32>());
        let samples: Vec<f32> = output
            .chunks_exact(4)
            .map(|chunk| f32::from_ne_bytes(chunk.try_into().unwrap()))
            .collect();
        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!((samples[2] - (32767.0 / 32768.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn output_duration_cap_is_explicit() {
        assert_eq!(MAX_OUTPUT_PCM_BYTES, 2_160_000);
    }

    #[test]
    fn cancelled_and_stale_response_audio_can_never_be_played() {
        let mut active = Some(ActiveResponse {
            id: "r1".into(),
            item_id: None,
            pcm: Vec::new(),
            cancelled: true,
        });
        assert!(!append_audio_delta(&mut active, "r1", "i1", &[0, 0]).unwrap());
        assert!(take_completed_audio(&mut active, "r1", Some("completed")).is_none());

        let mut active = Some(ActiveResponse {
            id: "new".into(),
            item_id: None,
            pcm: Vec::new(),
            cancelled: false,
        });
        assert!(!append_audio_delta(&mut active, "old", "i0", &[0, 0]).unwrap());
        assert!(append_audio_delta(&mut active, "new", "i1", &[1, 0]).unwrap());
        assert!(take_completed_audio(&mut active, "old", Some("completed")).is_none());
        assert!(take_completed_audio(&mut active, "new", Some("cancelled")).is_none());
    }

    #[tokio::test]
    async fn interrupting_buffered_audio_cancels_then_truncates_to_zero() {
        let mut sink = RecordingSink::default();
        let mut sequence = 0;
        let mut active = Some(ActiveResponse {
            id: "r1".into(),
            item_id: Some("i1".into()),
            pcm: vec![1, 0, 2, 0],
            cancelled: false,
        });
        let playback: SharedPlayback = Arc::new(Mutex::new(None));
        let mut playing_item = None;

        assert!(
            interrupt_response(
                &mut sink,
                7,
                &mut sequence,
                &mut active,
                &playback,
                &mut playing_item,
            )
            .await
            .unwrap()
        );
        let events: Vec<serde_json::Value> = sink
            .0
            .into_iter()
            .map(|message| match message {
                Message::Text(text) => serde_json::from_str(&text).unwrap(),
                other => panic!("unexpected event: {other:?}"),
            })
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "response.cancel");
        assert_eq!(events[0]["response_id"], "r1");
        assert_eq!(events[1]["type"], "conversation.item.truncate");
        assert_eq!(events[1]["item_id"], "i1");
        assert_eq!(events[1]["audio_end_ms"], 0);
    }
}
