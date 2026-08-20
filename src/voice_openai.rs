//! Explicit OpenAI Realtime backup for Discord voice.
//!
//! This path is never selected by key presence. It runs only under
//! `ABBEY_VOICE_MODE=openai`, caps WebSocket/audio memory, owns cancellation,
//! and truncates provider history to the audio actually heard after barge-in.

use std::io::Cursor;
use std::pin::Pin;
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
    let mut playing_turn: Option<u64> = None;
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
                if consume_natural_playback_end(&mut playing_turn, turn) {
                    session.playback.lock().await.take();
                    playing_item_id = None;
                    if session
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
                        &mut playing_turn,
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
                let append = Message::Text(serde_json::json!({
                    "type": "input_audio_buffer.append",
                    "event_id": next_event_id(session.epoch, &mut event_sequence),
                    "audio": base64::engine::general_purpose::STANDARD.encode(bytes),
                }).to_string());
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
                        human_speaking = true;
                        silence_frames = 0;
                        if interrupt_response(
                            &mut writer,
                            session.epoch,
                            &mut event_sequence,
                            &mut active_response,
                            &session.playback,
                            &mut playing_item_id,
                            &mut playing_turn,
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
                    Some("response.output_item.added") => {
                        let _ = capture_assistant_audio_item(&mut active_response, &event);
                    }
                    Some("response.output_audio.delta" | "response.audio.delta") => {
                        let Some(response_id) = event.get("response_id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let Some(item_id) = event.get("item_id")
                            .and_then(serde_json::Value::as_str) else { continue; };
                        let Some(delta) = event.get("delta").and_then(serde_json::Value::as_str) else {
                            continue;
                        };
                        append_encoded_audio_delta(
                            &mut active_response,
                            response_id,
                            item_id,
                            delta,
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

/// Correlate the assistant message item while its content is still empty.
/// This session requests audio as its only output modality, and Realtime emits
/// `response.output_item.added` before it adds the audio content part/deltas.
/// Recording the item here lets a pre-delta barge-in truncate unheard audio.
fn capture_assistant_audio_item(
    active_response: &mut Option<ActiveResponse>,
    event: &serde_json::Value,
) -> bool {
    if event.get("type").and_then(serde_json::Value::as_str) != Some("response.output_item.added")
        || event
            .get("output_index")
            .and_then(serde_json::Value::as_u64)
            .is_none()
    {
        return false;
    }
    let Some(response_id) = event.get("response_id").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let Some(item) = event.get("item") else {
        return false;
    };
    let Some(active) = active_response.as_mut() else {
        return false;
    };
    if active.cancelled || active.id != response_id {
        return false;
    }
    if item.get("type").and_then(serde_json::Value::as_str) != Some("message")
        || item.get("role").and_then(serde_json::Value::as_str) != Some("assistant")
    {
        return false;
    }
    let Some(item_id) = item
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return false;
    };
    if active.item_id.as_deref().is_some_and(|id| id != item_id) {
        return false;
    }
    active.item_id.get_or_insert_with(|| item_id.to_string());
    true
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
    // WebSocket events are ordered: the matching assistant item must already
    // have been announced. Never let a stale or unrelated delta choose which
    // conversation item will later be truncated.
    if active.item_id.as_deref() != Some(item_id) {
        return Ok(false);
    }
    if !decoded.len().is_multiple_of(2)
        || active.pcm.len().saturating_add(decoded.len()) > MAX_OUTPUT_PCM_BYTES
    {
        return Err("Realtime audio exceeded the bounded PCM duration".into());
    }
    active.pcm.extend_from_slice(decoded);
    Ok(true)
}

fn append_encoded_audio_delta(
    active_response: &mut Option<ActiveResponse>,
    response_id: &str,
    item_id: &str,
    encoded: &str,
) -> Result<bool, String> {
    let matches_active = active_response.as_ref().is_some_and(|active| {
        !active.cancelled && active.id == response_id && active.item_id.as_deref() == Some(item_id)
    });
    if !matches_active {
        return Ok(false);
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Realtime audio delta was not valid base64: {error}"))?;
    append_audio_delta(active_response, response_id, item_id, &decoded)
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
    playing_turn: &mut Option<u64>,
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

/// A Songbird `End` event also fires after `TrackHandle::stop()`. Only the
/// still-armed matching turn represents audio that reached natural playback
/// completion and may advance `/voice status`.
fn consume_natural_playback_end(playing_turn: &mut Option<u64>, ended_turn: u64) -> bool {
    if *playing_turn != Some(ended_turn) {
        return false;
    }
    *playing_turn = None;
    true
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
    runtime: &VoiceRuntime,
    epoch: u64,
    pcm_f32: Vec<u8>,
    turn: u64,
) -> Result<bool, String> {
    let input: songbird::input::Input = RawAdapter::new(Cursor::new(pcm_f32), 24_000, 1).into();
    let mut call = call.lock().await;
    let Some(handle) = runtime.with_media_enabled(epoch, || call.play_only_input(input)) else {
        return Ok(false);
    };
    drop(call);
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

    fn output_item_added(response_id: &str, item: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "response.output_item.added",
            "response_id": response_id,
            "output_index": 0,
            "item": item,
        })
    }

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
        let runtime = Arc::new(VoiceRuntime::new(crate::voice::VoiceConfig {
            guild_id: 1,
            channel_id: 2,
            backend: crate::voice::VoiceBackendConfig::Disabled,
            wake_word_required: true,
        }));
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
        assert!(capture_assistant_audio_item(
            &mut active,
            &output_item_added(
                "new",
                serde_json::json!({
                    "id": "i1",
                    "type": "message",
                    "role": "assistant",
                    "content": []
                })
            ),
        ));
        assert!(!append_audio_delta(&mut active, "new", "other", &[1, 0]).unwrap());
        assert!(append_audio_delta(&mut active, "new", "i1", &[1, 0]).unwrap());
        assert!(take_completed_audio(&mut active, "old", Some("completed")).is_none());
        assert!(take_completed_audio(&mut active, "new", Some("cancelled")).is_none());
    }

    #[test]
    fn malformed_stale_delta_is_ignored_before_base64_decode() {
        let mut active = Some(ActiveResponse {
            id: "current".into(),
            item_id: Some("item-current".into()),
            pcm: Vec::new(),
            cancelled: false,
        });
        assert!(
            !append_encoded_audio_delta(&mut active, "stale", "item-stale", "not base64!")
                .expect("stale malformed payload is irrelevant")
        );
        assert!(
            append_encoded_audio_delta(&mut active, "current", "item-current", "not base64!")
                .is_err()
        );
    }

    #[test]
    fn output_item_correlation_rejects_stale_non_assistant_and_conflicting_items() {
        let assistant = |response_id: &str, id: &str| {
            output_item_added(
                response_id,
                serde_json::json!({
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "content": []
                }),
            )
        };
        let mut active = Some(ActiveResponse {
            id: "r1".into(),
            item_id: None,
            pcm: Vec::new(),
            cancelled: false,
        });
        assert!(!capture_assistant_audio_item(
            &mut active,
            &assistant("stale", "old")
        ));
        assert!(!capture_assistant_audio_item(
            &mut active,
            &output_item_added(
                "r1",
                serde_json::json!({"id": "u1", "type": "message", "role": "user"})
            )
        ));
        assert!(!capture_assistant_audio_item(
            &mut active,
            &output_item_added(
                "r1",
                serde_json::json!({"id": "f1", "type": "function_call"})
            )
        ));
        assert!(!capture_assistant_audio_item(
            &mut active,
            &serde_json::json!({
                "type": "response.output_item.added",
                "response_id": "r1",
                "item": {"id": "missing-index", "type": "message", "role": "assistant"}
            })
        ));
        assert!(capture_assistant_audio_item(
            &mut active,
            &assistant("r1", "i1")
        ));
        assert!(capture_assistant_audio_item(
            &mut active,
            &assistant("r1", "i1")
        ));
        assert!(!capture_assistant_audio_item(
            &mut active,
            &assistant("r1", "i2")
        ));
        assert_eq!(active.unwrap().item_id.as_deref(), Some("i1"));
    }

    #[tokio::test]
    async fn interrupt_after_item_added_but_before_first_delta_cancels_and_truncates_to_zero() {
        let mut sink = RecordingSink::default();
        let mut sequence = 0;
        let mut active = Some(ActiveResponse {
            id: "r1".into(),
            item_id: None,
            pcm: Vec::new(),
            cancelled: false,
        });
        assert!(capture_assistant_audio_item(
            &mut active,
            &output_item_added(
                "r1",
                serde_json::json!({
                    "id": "i1",
                    "type": "message",
                    "role": "assistant",
                    "content": []
                })
            ),
        ));
        let playback: SharedPlayback = Arc::new(Mutex::new(None));
        let mut playing_item = None;
        let mut playing_turn = None;

        assert!(
            interrupt_response(
                &mut sink,
                7,
                &mut sequence,
                &mut active,
                &playback,
                &mut playing_item,
                &mut playing_turn,
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
        assert_eq!(playing_turn, None);
    }

    #[test]
    fn only_natural_matching_playback_end_counts_as_complete() {
        let mut playing_turn = Some(11);
        assert!(!consume_natural_playback_end(&mut playing_turn, 10));
        assert_eq!(playing_turn, Some(11));
        assert!(consume_natural_playback_end(&mut playing_turn, 11));
        assert_eq!(playing_turn, None);
        assert!(!consume_natural_playback_end(&mut playing_turn, 11));

        // Barge-in/teardown invalidates the armed turn before Songbird emits
        // the End event caused by TrackHandle::stop().
        let mut interrupted_turn = Some(12);
        interrupted_turn.take();
        assert!(!consume_natural_playback_end(&mut interrupted_turn, 12));
    }
}
