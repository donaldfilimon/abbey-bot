//! Byte-safe OpenAI-compatible server-sent-event parsing and transport.

use std::future::Future;

use serde_json::{Value, json};

use super::{
    Backend, ChatTurn, HttpTransport, LlmError, LlmRequest, MAX_ERROR_RESPONSE_BYTES,
    MAX_RESPONSE_BYTES, MAX_TOOL_CALLS_PER_TURN, ModelTurn, build_chat_request_with_tools,
};

/// The streaming form of a chat request. Only the OpenAI-compatible path is
/// streamed; Anthropic remains a completed JSON response on this code path.
pub fn build_stream_request(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[crate::tools::ToolSpec],
) -> LlmRequest {
    let mut request = build_chat_request_with_tools(backend, system_prompt, turns, tools);
    if matches!(backend, Backend::OpenAiCompatible { .. }) {
        request.body["stream"] = json!(true);
    }
    request
}

#[derive(Debug, Default)]
struct StreamedCall {
    id: String,
    call_type: String,
    name: String,
    arguments: String,
}

fn optional_string<'a>(
    call: &'a Value,
    field: &str,
    index: usize,
) -> Result<Option<&'a str>, LlmError> {
    match call.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(LlmError::backend(format!(
            "backend streamed tool call {index} with a non-string {field}"
        ))),
    }
}

fn optional_nested_string<'a>(
    call: &'a Value,
    field: &str,
    index: usize,
) -> Result<Option<&'a str>, LlmError> {
    let Some(function) = call.get("function") else {
        return Ok(None);
    };
    match function.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(LlmError::backend(format!(
            "backend streamed tool call {index} with non-string function {field}"
        ))),
    }
}

/// Incremental parser for OpenAI-style SSE bodies.
///
/// Network chunks are retained as bytes until a complete line exists, then
/// decoded with strict UTF-8. This avoids corrupting a codepoint split across
/// chunks and rejects malformed wire data instead of silently replacing it.
#[derive(Debug, Default)]
pub struct SseAccumulator {
    buffer: Vec<u8>,
    done: bool,
    finish_reason: Option<String>,
    calls: Vec<StreamedCall>,
}

impl SseAccumulator {
    /// Consume one raw network chunk and return complete text deltas.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<String>, LlmError> {
        if self.done {
            if chunk.iter().any(|byte| !byte.is_ascii_whitespace()) {
                return Err(LlmError::backend(
                    "the backend sent data after the stream terminal marker".into(),
                ));
            }
            return Ok(Vec::new());
        }

        self.buffer.extend_from_slice(chunk);
        let mut deltas = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut deltas)?;
        }
        if self.done && self.buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Err(LlmError::backend(
                "the backend sent data after the stream terminal marker".into(),
            ));
        }
        Ok(deltas)
    }

    fn process_line(&mut self, line: &[u8], deltas: &mut Vec<String>) -> Result<(), LlmError> {
        let line = std::str::from_utf8(line)
            .map_err(|_| LlmError::backend("the backend stream contained invalid UTF-8".into()))?;
        if line.trim().is_empty() || line.starts_with(':') {
            return Ok(());
        }
        if self.done {
            return Err(LlmError::backend(
                "the backend sent an event after the stream terminal marker".into(),
            ));
        }
        let Some(payload) = line.strip_prefix("data:") else {
            // SSE fields such as `event:` and `id:` are not part of the
            // OpenAI-compatible completion contract used here.
            return Err(LlmError::backend(
                "the backend stream contained an unsupported SSE field".into(),
            ));
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            self.validate_terminal()?;
            self.done = true;
            return Ok(());
        }
        if self.finish_reason.is_some() {
            return Err(LlmError::backend(
                "the backend sent an event after its finish reason".into(),
            ));
        }

        let value: Value = serde_json::from_str(payload).map_err(|error| {
            LlmError::backend(format!("the backend stream event was not JSON: {error}"))
        })?;
        let choice = value.pointer("/choices/0").ok_or_else(|| {
            LlmError::backend("the backend stream event carried no choice".into())
        })?;
        let finish = choice.get("finish_reason").and_then(Value::as_str);
        let delta = choice.get("delta").unwrap_or(&Value::Null);

        if let Some(text) = delta.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            if finish.is_some() {
                return Err(LlmError::backend(
                    "the backend combined a terminal finish reason with more text".into(),
                ));
            }
            deltas.push(text.to_string());
        }

        if delta
            .get("tool_calls")
            .is_some_and(|calls| !calls.is_null() && !calls.is_array())
        {
            return Err(LlmError::backend(
                "the backend stream tool_calls field was not an array".into(),
            ));
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            if finish.is_some() && !calls.is_empty() {
                return Err(LlmError::backend(
                    "the backend combined a terminal finish reason with more tool data".into(),
                ));
            }
            if calls.len() > MAX_TOOL_CALLS_PER_TURN {
                return Err(LlmError::backend(format!(
                    "the backend streamed more than {MAX_TOOL_CALLS_PER_TURN} tool calls in one event"
                )));
            }
            for call in calls {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|index| usize::try_from(index).ok())
                    .ok_or_else(|| {
                        LlmError::backend(
                            "the backend streamed a tool call without a valid index".into(),
                        )
                    })?;
                if index >= MAX_TOOL_CALLS_PER_TURN {
                    return Err(LlmError::backend(format!(
                        "the backend tool-call index {index} exceeded the per-turn limit of {MAX_TOOL_CALLS_PER_TURN}"
                    )));
                }
                while self.calls.len() <= index {
                    self.calls.push(StreamedCall::default());
                }
                let slot = &mut self.calls[index];
                if let Some(id) = optional_string(call, "id", index)? {
                    slot.id.push_str(id);
                }
                if let Some(call_type) = optional_string(call, "type", index)? {
                    slot.call_type.push_str(call_type);
                }
                if call.get("function").is_some_and(|value| !value.is_object()) {
                    return Err(LlmError::backend(format!(
                        "backend streamed tool call {index} with a non-object function"
                    )));
                }
                if let Some(name) = optional_nested_string(call, "name", index)? {
                    slot.name.push_str(name);
                }
                if let Some(arguments) = optional_nested_string(call, "arguments", index)? {
                    slot.arguments.push_str(arguments);
                }
            }
        }

        if let Some(finish) = finish {
            self.finish_reason = Some(finish.to_string());
        }
        Ok(())
    }

    fn validate_terminal(&self) -> Result<(), LlmError> {
        let finish = self.finish_reason.as_deref().ok_or_else(|| {
            LlmError::backend("the backend stream ended without a finish reason".into())
        })?;
        let calls = self.parsed_tool_calls()?.len();
        match finish {
            "stop" if calls == 0 => Ok(()),
            "tool_calls" if calls > 0 => Ok(()),
            "length" => Err(LlmError::backend(
                "the backend exhausted its output limit before completing the stream".into(),
            )),
            "content_filter" => Err(LlmError::backend(
                "the backend stopped the stream because of its content filter".into(),
            )),
            "stop" => Err(LlmError::backend(
                "the backend marked a tool-bearing stream as plain completion".into(),
            )),
            "tool_calls" => Err(LlmError::backend(
                "the backend claimed tool completion without a valid tool call".into(),
            )),
            other => Err(LlmError::backend(format!(
                "the backend returned unsupported stream finish reason {other:?}"
            ))),
        }
    }

    /// Require a complete `[DONE]`-terminated stream before exposing a turn.
    pub fn finish(&self) -> Result<(), LlmError> {
        if !self.done {
            return Err(LlmError::backend(
                "the backend stream ended before the [DONE] terminal marker".into(),
            ));
        }
        if self.buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Err(LlmError::backend(
                "the backend stream ended with an incomplete event".into(),
            ));
        }
        Ok(())
    }

    /// The bounded, parsed tool calls accumulated so far.
    pub fn tool_calls(&self) -> Result<Vec<crate::tools::ToolCall>, LlmError> {
        self.finish()?;
        self.parsed_tool_calls()
    }

    fn parsed_tool_calls(&self) -> Result<Vec<crate::tools::ToolCall>, LlmError> {
        self.calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                if call.id.trim().is_empty() || call.name.trim().is_empty() {
                    return Err(LlmError::backend(format!(
                        "streamed tool call {index} had an empty id or function name"
                    )));
                }
                if call.call_type != "function" {
                    return Err(LlmError::backend(format!(
                        "streamed tool call {index} did not declare type function"
                    )));
                }
                let arguments: Value = serde_json::from_str(&call.arguments).map_err(|error| {
                    LlmError::backend(format!(
                        "streamed tool call {index} carried invalid JSON arguments: {error}"
                    ))
                })?;
                if !arguments.is_object() {
                    return Err(LlmError::backend(format!(
                        "streamed tool call {index} arguments were not a JSON object"
                    )));
                }
                Ok(crate::tools::ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments,
                })
            })
            .collect()
    }

    /// Whether the validated `[DONE]` marker has been seen.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// A transport that can deliver text deltas while resolving to a full turn.
pub trait StreamTransport {
    fn post_stream(
        &self,
        request: &LlmRequest,
        on_delta: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> impl Future<Output = Result<ModelTurn, LlmError>> + Send;
}

impl StreamTransport for HttpTransport {
    fn post_stream(
        &self,
        request: &LlmRequest,
        on_delta: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> impl Future<Output = Result<ModelTurn, LlmError>> + Send {
        let mut builder = self
            .client_for(&request.url)
            .post(&request.url)
            .json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(*name, value);
        }
        async move {
            use futures_util::StreamExt as _;

            let response = builder
                .send()
                .await
                .map_err(|error| LlmError::backend(format!("the request failed: {error}")))?;
            let status = response.status();
            if !status.is_success() {
                let body = crate::http_body::read_capped(response, MAX_ERROR_RESPONSE_BYTES)
                    .await
                    .map_err(|error| LlmError::backend(format!("HTTP {status}: {error}")))?;
                let body = String::from_utf8_lossy(&body);
                let brief: String = body.chars().take(300).collect();
                return Err(LlmError::backend(format!("HTTP {status}: {brief}")));
            }

            let mut stream = response.bytes_stream();
            let mut accumulator = SseAccumulator::default();
            let mut full = String::new();
            let mut received = 0usize;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.map_err(|error| {
                    LlmError::backend(format!("reading the stream failed: {error}"))
                })?;
                received = received.checked_add(bytes.len()).ok_or_else(|| {
                    LlmError::backend("the backend stream exceeded its response limit".into())
                })?;
                if received > MAX_RESPONSE_BYTES {
                    return Err(LlmError::backend(format!(
                        "the backend stream exceeded the {MAX_RESPONSE_BYTES}-byte response limit"
                    )));
                }
                for delta in accumulator.feed(&bytes)? {
                    full.push_str(&delta);
                    let _ = on_delta.send(delta);
                }
                if accumulator.is_done() {
                    // Stop reading immediately once the validated terminal
                    // event arrives; post-terminal bytes in this chunk were
                    // already rejected by `feed`.
                    break;
                }
            }
            accumulator.finish()?;
            let calls = accumulator.tool_calls()?;
            if full.trim().is_empty() && calls.is_empty() {
                return Err(LlmError::backend(
                    "the stream carried no answer text".into(),
                ));
            }
            Ok(ModelTurn { text: full, calls })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> Vec<u8> {
        format!("data: {json}\n").into_bytes()
    }

    #[test]
    fn split_utf8_is_buffered_until_the_line_is_complete() {
        let mut accumulator = SseAccumulator::default();
        let line = event(r#"{"choices":[{"delta":{"content":"café"},"finish_reason":null}]}"#);
        let split = line
            .windows(2)
            .position(|window| window == "é".as_bytes())
            .expect("contains multibyte codepoint")
            + 1;
        assert!(accumulator.feed(&line[..split]).unwrap().is_empty());
        assert_eq!(accumulator.feed(&line[split..]).unwrap(), ["café"]);
    }

    #[test]
    fn invalid_utf8_and_post_terminal_events_are_rejected() {
        let mut invalid = SseAccumulator::default();
        assert!(invalid.feed(b"data: \xff\n").is_err());

        let mut done = SseAccumulator::default();
        done.feed(&event(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ))
        .unwrap();
        done.feed(b"data: [DONE]\n").unwrap();
        assert!(done.feed(&event(r#"{"choices":[]}"#)).is_err());
    }

    #[test]
    fn missing_done_length_and_large_tool_indices_fail_closed() {
        let mut incomplete = SseAccumulator::default();
        incomplete
            .feed(&event(
                r#"{"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}"#,
            ))
            .unwrap();
        assert!(incomplete.finish().is_err());

        let mut length = SseAccumulator::default();
        length
            .feed(&event(
                r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
            ))
            .unwrap();
        assert!(length.feed(b"data: [DONE]\n").is_err());

        let mut huge = SseAccumulator::default();
        let json = format!(
            r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":{},"function":{{"name":"recall"}}}}]}},"finish_reason":null}}]}}"#,
            MAX_TOOL_CALLS_PER_TURN
        );
        assert!(huge.feed(&event(&json)).is_err());
        assert!(huge.calls.len() <= MAX_TOOL_CALLS_PER_TURN);
    }

    #[test]
    fn malformed_streamed_tool_calls_are_never_synthesized() {
        for fragment in [
            r#"{"id":"c1","type":"function","function":{"name":"recall","arguments":"{}"}}"#,
            r#"{"index":"0","id":"c1","type":"function","function":{"name":"recall","arguments":"{}"}}"#,
        ] {
            let mut accumulator = SseAccumulator::default();
            let payload = format!(
                r#"{{"choices":[{{"delta":{{"tool_calls":[{fragment}]}},"finish_reason":null}}]}}"#
            );
            assert!(accumulator.feed(&event(&payload)).is_err());
        }

        for fragment in [
            r#"{"index":0,"id":"","type":"function","function":{"name":"recall","arguments":"{}"}}"#,
            r#"{"index":0,"id":"c1","type":"other","function":{"name":"recall","arguments":"{}"}}"#,
            r#"{"index":0,"id":"c1","type":"function","function":{"name":"recall","arguments":"{"}}"#,
        ] {
            let mut accumulator = SseAccumulator::default();
            let payload = format!(
                r#"{{"choices":[{{"delta":{{"tool_calls":[{fragment}]}},"finish_reason":null}}]}}"#
            );
            accumulator.feed(&event(&payload)).unwrap();
            accumulator
                .feed(&event(
                    r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
                ))
                .unwrap();
            assert!(accumulator.feed(b"data: [DONE]\n").is_err());
        }
    }
}
