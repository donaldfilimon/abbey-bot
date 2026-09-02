//! Bounded, deterministic state for the OpenAI Realtime wire protocol.
//!
//! The actor owns sockets and Discord playback; this module owns correlation,
//! cancellation, sequence continuity, and the exact client events those state
//! transitions require. Keeping the protocol state together prevents a late
//! provider event from silently reopening or splicing a cancelled response.

use base64::Engine as _;
use futures_util::{Sink, SinkExt};
use tokio_tungstenite::tungstenite::Message;

const MAX_OUTPUT_PCM_BYTES: usize = 24_000 * 2 * 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseState {
    Active,
    Cancelled,
    /// Barge-in won before the provider announced the assistant item. The
    /// item must be truncated to zero as soon as its correlated event arrives.
    AwaitingItemForZeroTruncation,
}

struct ActiveResponse {
    id: String,
    item_id: Option<String>,
    pcm: Vec<u8>,
    state: ResponseState,
}

impl ActiveResponse {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            item_id: None,
            pcm: Vec::new(),
            state: ResponseState::Active,
        }
    }
}

/// The single provider response Abbey may buffer at a time. A new response is
/// rejected while an older cancelled response still awaits its item id; losing
/// that correlation would retain unheard assistant audio in provider history.
#[derive(Default)]
pub(super) struct ResponseBuffer {
    active: Option<ActiveResponse>,
}

impl ResponseBuffer {
    pub(super) fn start(&mut self, response_id: &str) -> Result<(), String> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.state == ResponseState::AwaitingItemForZeroTruncation)
        {
            return Err(
                "Realtime started another response before a pending zero-truncation item arrived"
                    .into(),
            );
        }
        self.active = Some(ActiveResponse::new(response_id));
        Ok(())
    }

    pub(super) fn append_encoded_audio(
        &mut self,
        response_id: &str,
        item_id: &str,
        encoded: &str,
    ) -> Result<bool, String> {
        let matches_active = self.active.as_ref().is_some_and(|active| {
            active.state == ResponseState::Active
                && active.id == response_id
                && active.item_id.as_deref() == Some(item_id)
        });
        if !matches_active {
            return Ok(false);
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("Realtime audio delta was not valid base64: {error}"))?;
        self.append_audio(response_id, item_id, &decoded)
    }

    fn append_audio(
        &mut self,
        response_id: &str,
        item_id: &str,
        decoded: &[u8],
    ) -> Result<bool, String> {
        let Some(active) = self.active.as_mut() else {
            return Ok(false);
        };
        if active.state != ResponseState::Active
            || active.id != response_id
            || active.item_id.as_deref() != Some(item_id)
        {
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

    pub(super) fn take_completed_audio(
        &mut self,
        response_id: &str,
        status: Option<&str>,
    ) -> Option<(Option<String>, Vec<u8>)> {
        let active = self.active.as_ref()?;
        if active.id != response_id || active.state != ResponseState::Active {
            // In particular, retain AwaitingItemForZeroTruncation across a
            // cancellation response.done so the later item can still be fixed.
            return None;
        }
        let active = self.active.take()?;
        if status != Some("completed") || active.pcm.is_empty() {
            return None;
        }
        Some((active.item_id, active.pcm))
    }

    fn cancel(&mut self) -> Option<Cancellation> {
        let active = self.active.as_mut()?;
        if active.state != ResponseState::Active {
            return None;
        }
        active.pcm.clear();
        active.state = if active.item_id.is_some() {
            ResponseState::Cancelled
        } else {
            ResponseState::AwaitingItemForZeroTruncation
        };
        Some(Cancellation {
            response_id: active.id.clone(),
            zero_truncate_item_id: active.item_id.clone(),
        })
    }

    fn capture_item(&mut self, event: &serde_json::Value) -> ItemCapture {
        if event.get("type").and_then(serde_json::Value::as_str)
            != Some("response.output_item.added")
            || event
                .get("output_index")
                .and_then(serde_json::Value::as_u64)
                .is_none()
        {
            return ItemCapture::Ignored;
        }
        let Some(response_id) = event.get("response_id").and_then(serde_json::Value::as_str) else {
            return ItemCapture::Ignored;
        };
        let Some(item) = event.get("item") else {
            return ItemCapture::Ignored;
        };
        let Some(active) = self.active.as_mut() else {
            return ItemCapture::Ignored;
        };
        if active.id != response_id || active.state == ResponseState::Cancelled {
            return ItemCapture::Ignored;
        }
        if item.get("type").and_then(serde_json::Value::as_str) != Some("message")
            || item.get("role").and_then(serde_json::Value::as_str) != Some("assistant")
        {
            return ItemCapture::Ignored;
        }
        let Some(item_id) = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return ItemCapture::Ignored;
        };
        if active.item_id.as_deref().is_some_and(|id| id != item_id) {
            return ItemCapture::Ignored;
        }
        active.item_id.get_or_insert_with(|| item_id.to_string());
        if active.state == ResponseState::AwaitingItemForZeroTruncation {
            active.state = ResponseState::Cancelled;
            ItemCapture::TruncateZero(item_id.to_string())
        } else {
            ItemCapture::Captured
        }
    }
}

struct Cancellation {
    response_id: String,
    zero_truncate_item_id: Option<String>,
}

enum ItemCapture {
    Ignored,
    Captured,
    TruncateZero(String),
}

/// Cancel the active response and immediately repair provider history when its
/// item id is already known. Otherwise `ResponseBuffer` retains a pending
/// zero-truncation until [`capture_output_item`] receives that id.
pub(super) async fn cancel_response<S>(
    writer: &mut S,
    epoch: u64,
    event_sequence: &mut u64,
    responses: &mut ResponseBuffer,
) -> Result<bool, String>
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let Some(cancel) = responses.cancel() else {
        return Ok(false);
    };
    writer
        .send(response_cancel_event(
            epoch,
            event_sequence,
            &cancel.response_id,
        ))
        .await
        .map_err(|error| format!("cancelling the Realtime response failed: {error}"))?;
    if let Some(item_id) = cancel.zero_truncate_item_id {
        writer
            .send(truncate_event(epoch, event_sequence, &item_id, 0))
            .await
            .map_err(|error| format!("truncating buffered Realtime audio failed: {error}"))?;
    }
    Ok(true)
}

/// Capture the correlated assistant item. If barge-in happened before this
/// event, emit the deferred zero-truncation in the same transition.
pub(super) async fn capture_output_item<S>(
    writer: &mut S,
    epoch: u64,
    event_sequence: &mut u64,
    responses: &mut ResponseBuffer,
    event: &serde_json::Value,
) -> Result<bool, String>
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    match responses.capture_item(event) {
        ItemCapture::Ignored => Ok(false),
        ItemCapture::Captured => Ok(true),
        ItemCapture::TruncateZero(item_id) => {
            writer
                .send(truncate_event(epoch, event_sequence, &item_id, 0))
                .await
                .map_err(|error| {
                    format!("truncating late buffered Realtime audio failed: {error}")
                })?;
            Ok(true)
        }
    }
}

fn response_cancel_event(epoch: u64, sequence: &mut u64, response_id: &str) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "response.cancel",
            "event_id": next_event_id(epoch, sequence),
            "response_id": response_id,
        })
        .to_string()
        .into(),
    )
}

pub(super) fn truncate_event(
    epoch: u64,
    sequence: &mut u64,
    item_id: &str,
    audio_end_ms: u64,
) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "conversation.item.truncate",
            "event_id": next_event_id(epoch, sequence),
            "item_id": item_id,
            "content_index": 0,
            "audio_end_ms": audio_end_ms,
        })
        .to_string()
        .into(),
    )
}

pub(super) fn next_event_id(epoch: u64, sequence: &mut u64) -> String {
    *sequence = sequence.saturating_add(1);
    format!("abbey-{epoch}-{sequence}")
}

pub(super) fn pcm16_to_f32(pcm: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(pcm.len() * 2);
    // Match `chunks_exact(2)`: a trailing partial PCM16 sample is ignored.
    let (pairs, _remainder) = pcm.as_chunks::<2>();
    for pair in pairs {
        let sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
        output.extend_from_slice(&sample.to_le_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::*;

    fn output_item_added(response_id: &str, item: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "response.output_item.added",
            "response_id": response_id,
            "output_index": 0,
            "item": item,
        })
    }

    fn assistant_item(response_id: &str, item_id: &str) -> serde_json::Value {
        output_item_added(
            response_id,
            serde_json::json!({
                "id": item_id,
                "type": "message",
                "role": "assistant",
                "content": []
            }),
        )
    }

    #[derive(Default)]
    struct RecordingSink(Vec<Message>);

    impl Sink<Message> for RecordingSink {
        type Error = std::convert::Infallible;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.0.push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    fn events(sink: RecordingSink) -> Vec<serde_json::Value> {
        sink.0
            .into_iter()
            .map(|message| match message {
                Message::Text(text) => serde_json::from_str(&text).unwrap(),
                other => panic!("unexpected event: {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn barge_in_before_item_defers_then_emits_exact_zero_truncation() {
        let mut sink = RecordingSink::default();
        let mut sequence = 0;
        let mut responses = ResponseBuffer::default();
        responses.start("r1").unwrap();

        assert!(
            cancel_response(&mut sink, 7, &mut sequence, &mut responses)
                .await
                .unwrap()
        );
        assert_eq!(sink.0.len(), 1, "only response.cancel is possible yet");

        assert!(
            capture_output_item(
                &mut sink,
                7,
                &mut sequence,
                &mut responses,
                &assistant_item("r1", "i1"),
            )
            .await
            .unwrap()
        );
        let events = events(sink);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "response.cancel");
        assert_eq!(events[0]["response_id"], "r1");
        assert_eq!(events[1]["type"], "conversation.item.truncate");
        assert_eq!(events[1]["item_id"], "i1");
        assert_eq!(events[1]["content_index"], 0);
        assert_eq!(events[1]["audio_end_ms"], 0);
    }

    #[tokio::test]
    async fn item_known_before_barge_in_is_truncated_immediately() {
        let mut sink = RecordingSink::default();
        let mut sequence = 0;
        let mut responses = ResponseBuffer::default();
        responses.start("r1").unwrap();
        assert!(
            capture_output_item(
                &mut sink,
                7,
                &mut sequence,
                &mut responses,
                &assistant_item("r1", "i1"),
            )
            .await
            .unwrap()
        );
        assert!(sink.0.is_empty());
        assert!(
            cancel_response(&mut sink, 7, &mut sequence, &mut responses)
                .await
                .unwrap()
        );
        let events = events(sink);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "response.cancel");
        assert_eq!(events[1]["type"], "conversation.item.truncate");
        assert_eq!(events[1]["audio_end_ms"], 0);
    }

    #[test]
    fn cancelled_and_stale_response_audio_can_never_be_played() {
        let mut responses = ResponseBuffer::default();
        responses.start("r1").unwrap();
        responses.capture_item(&assistant_item("r1", "i1"));
        responses.cancel();
        assert!(!responses.append_audio("r1", "i1", &[0, 0]).unwrap());
        assert!(
            responses
                .take_completed_audio("r1", Some("completed"))
                .is_none()
        );

        responses.start("new").unwrap();
        assert!(!responses.append_audio("old", "i0", &[0, 0]).unwrap());
        assert!(matches!(
            responses.capture_item(&assistant_item("new", "i1")),
            ItemCapture::Captured
        ));
        assert!(!responses.append_audio("new", "other", &[1, 0]).unwrap());
        assert!(responses.append_audio("new", "i1", &[1, 0]).unwrap());
        assert!(
            responses
                .take_completed_audio("old", Some("completed"))
                .is_none()
        );
        assert!(
            responses
                .take_completed_audio("new", Some("cancelled"))
                .is_none()
        );
    }

    #[test]
    fn malformed_stale_delta_is_ignored_before_base64_decode() {
        let mut responses = ResponseBuffer::default();
        responses.start("current").unwrap();
        responses.capture_item(&assistant_item("current", "item-current"));
        assert!(
            !responses
                .append_encoded_audio("stale", "item-stale", "not base64!")
                .expect("stale malformed payload is irrelevant")
        );
        assert!(
            responses
                .append_encoded_audio("current", "item-current", "not base64!")
                .is_err()
        );
    }

    #[test]
    fn output_item_correlation_rejects_stale_non_assistant_and_conflicting_items() {
        let mut responses = ResponseBuffer::default();
        responses.start("r1").unwrap();
        assert!(matches!(
            responses.capture_item(&assistant_item("stale", "old")),
            ItemCapture::Ignored
        ));
        assert!(matches!(
            responses.capture_item(&output_item_added(
                "r1",
                serde_json::json!({"id": "u1", "type": "message", "role": "user"})
            )),
            ItemCapture::Ignored
        ));
        assert!(matches!(
            responses.capture_item(&output_item_added(
                "r1",
                serde_json::json!({"id": "f1", "type": "function_call"})
            )),
            ItemCapture::Ignored
        ));
        assert!(matches!(
            responses.capture_item(&serde_json::json!({
                "type": "response.output_item.added",
                "response_id": "r1",
                "item": {"id": "missing-index", "type": "message", "role": "assistant"}
            })),
            ItemCapture::Ignored
        ));
        assert!(matches!(
            responses.capture_item(&assistant_item("r1", "i1")),
            ItemCapture::Captured
        ));
        assert!(matches!(
            responses.capture_item(&assistant_item("r1", "i1")),
            ItemCapture::Captured
        ));
        assert!(matches!(
            responses.capture_item(&assistant_item("r1", "i2")),
            ItemCapture::Ignored
        ));
    }

    #[test]
    fn pending_zero_truncation_blocks_replacement_instead_of_losing_correlation() {
        let mut responses = ResponseBuffer::default();
        responses.start("r1").unwrap();
        responses.cancel();
        let error = responses.start("r2").unwrap_err();
        assert!(error.contains("pending zero-truncation"), "{error}");
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
        let (samples, remainder) = output.as_chunks::<4>();
        assert!(remainder.is_empty());
        let samples: Vec<f32> = samples
            .iter()
            .map(|chunk| f32::from_ne_bytes(*chunk))
            .collect();
        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!((samples[2] - (32767.0 / 32768.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn pcm16_conversion_ignores_a_trailing_partial_sample() {
        let mut bytes = 123_i16.to_le_bytes().to_vec();
        bytes.push(0xff);
        assert_eq!(pcm16_to_f32(&bytes).len(), size_of::<f32>());
    }

    #[test]
    fn output_duration_cap_is_explicit() {
        assert_eq!(MAX_OUTPUT_PCM_BYTES, 2_160_000);
    }
}
