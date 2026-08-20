//! Pure request construction and completed-response validation.
//!
//! Both provider dialects converge here before a [`ModelTurn`] can reach tool
//! dispatch. That makes completion semantics and the per-turn tool bound one
//! shared invariant instead of caller folklore.

use serde_json::{Value, json};

use super::{
    ANTHROPIC_MODEL, ANTHROPIC_URL, ANTHROPIC_VERSION, Backend, ChatTurn, LOCAL_MAX_TOKENS,
    LlmError, LlmRequest, MAX_TOKENS, MAX_TOOL_CALLS_PER_TURN, ModelTurn, Role,
};

/// Build the request a backend call would send for a single question.
pub fn build_request(backend: &Backend, system_prompt: &str, question: &str) -> LlmRequest {
    build_chat_request(backend, system_prompt, &[ChatTurn::user(question)])
}

/// Build a multi-turn request without exposing any tools.
pub fn build_chat_request(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
) -> LlmRequest {
    build_chat_request_with_tools(backend, system_prompt, turns, &[])
}

/// Build a multi-turn request with the supplied tool vocabulary.
pub fn build_chat_request_with_tools(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[crate::tools::ToolSpec],
) -> LlmRequest {
    match backend {
        Backend::Anthropic { api_key } => {
            let mut messages: Vec<Value> = Vec::with_capacity(turns.len());
            for turn in turns {
                match turn.role {
                    Role::User => messages.push(json!({"role": "user", "content": turn.text})),
                    Role::Assistant if turn.tool_calls.is_empty() => {
                        messages.push(json!({"role": "assistant", "content": turn.text}));
                    }
                    Role::Assistant => {
                        let mut blocks = Vec::new();
                        if !turn.text.is_empty() {
                            blocks.push(json!({"type": "text", "text": turn.text}));
                        }
                        for call in &turn.tool_calls {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": call.id,
                                "name": call.name,
                                "input": call.arguments,
                            }));
                        }
                        messages.push(json!({"role": "assistant", "content": blocks}));
                    }
                    Role::Tool => {
                        let block = json!({
                            "type": "tool_result",
                            "tool_use_id": turn.tool_call_id.clone().unwrap_or_default(),
                            "content": turn.text,
                        });
                        if let Some(last) = messages.last_mut()
                            && last["role"] == "user"
                            && last["content"].is_array()
                        {
                            last["content"].as_array_mut().expect("array").push(block);
                        } else {
                            messages.push(json!({"role": "user", "content": [block]}));
                        }
                    }
                }
            }
            let mut body = json!({
                "model": ANTHROPIC_MODEL,
                "max_tokens": MAX_TOKENS,
                // Sonnet 5 otherwise enables adaptive thinking by default.
                // This Discord path intentionally reserves its small output
                // budget for the terse visible answer and tool calls.
                "thinking": {"type": "disabled"},
                "system": system_prompt,
                "messages": messages,
            });
            if !tools.is_empty() {
                body["tools"] = crate::tools::anthropic_tools_json(tools);
            }
            LlmRequest {
                url: ANTHROPIC_URL.to_string(),
                headers: vec![
                    ("x-api-key", api_key.clone()),
                    ("anthropic-version", ANTHROPIC_VERSION.to_string()),
                ],
                body,
            }
        }
        Backend::OpenAiCompatible { endpoint, model } => {
            let mut messages = Vec::with_capacity(turns.len() + 1);
            messages.push(json!({"role": "system", "content": system_prompt}));
            for turn in turns {
                let mut message = json!({"role": turn.role.as_str(), "content": turn.text});
                if !turn.tool_calls.is_empty() {
                    message["tool_calls"] = Value::Array(
                        turn.tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": {
                                        "name": call.name,
                                        "arguments": call.arguments.to_string(),
                                    }
                                })
                            })
                            .collect(),
                    );
                }
                if let Some(id) = &turn.tool_call_id {
                    message["tool_call_id"] = json!(id);
                }
                messages.push(message);
            }
            let mut body = json!({
                "model": model,
                "max_tokens": LOCAL_MAX_TOKENS,
                "messages": messages,
            });
            if !tools.is_empty() {
                body["tools"] = crate::tools::openai_tools_json(tools);
            }
            LlmRequest {
                url: format!("{}/v1/chat/completions", endpoint.trim_end_matches('/')),
                headers: Vec::new(),
                body,
            }
        }
    }
}

fn bounded_openai_calls(message: &Value) -> Result<Vec<crate::tools::ToolCall>, LlmError> {
    let calls = match message.get("tool_calls") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(calls)) => calls,
        Some(_) => {
            return Err(LlmError::backend(
                "the backend tool_calls field was not an array".into(),
            ));
        }
    };
    if calls.len() > MAX_TOOL_CALLS_PER_TURN {
        return Err(LlmError::backend(format!(
            "the backend returned {} tool calls; the per-turn limit is {MAX_TOOL_CALLS_PER_TURN}",
            calls.len()
        )));
    }
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let object = call.as_object().ok_or_else(|| {
                LlmError::backend(format!("backend tool call {index} was not an object"))
            })?;
            if object.get("type").and_then(Value::as_str) != Some("function") {
                return Err(LlmError::backend(format!(
                    "backend tool call {index} did not declare type function"
                )));
            }
            let id = required_nonempty_string(call.get("id"), "id", index)?;
            let function = object
                .get("function")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    LlmError::backend(format!(
                        "backend tool call {index} carried no function object"
                    ))
                })?;
            let name = required_nonempty_string(function.get("name"), "function name", index)?;
            let arguments = match function.get("arguments") {
                Some(Value::String(raw)) => serde_json::from_str::<Value>(raw).map_err(|error| {
                    LlmError::backend(format!(
                        "backend tool call {index} carried invalid JSON arguments: {error}"
                    ))
                })?,
                Some(Value::Object(arguments)) => Value::Object(arguments.clone()),
                _ => {
                    return Err(LlmError::backend(format!(
                        "backend tool call {index} arguments were not a JSON object or encoded object"
                    )));
                }
            };
            if !arguments.is_object() {
                return Err(LlmError::backend(format!(
                    "backend tool call {index} arguments were not a JSON object"
                )));
            }
            Ok(crate::tools::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments,
            })
        })
        .collect()
}

fn bounded_anthropic_calls(content: &Value) -> Result<Vec<crate::tools::ToolCall>, LlmError> {
    let tool_blocks: Vec<&Value> = content
        .as_array()
        .expect("caller validated content array")
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .collect();
    if tool_blocks.len() > MAX_TOOL_CALLS_PER_TURN {
        return Err(LlmError::backend(format!(
            "the backend returned {} tool calls; the per-turn limit is {MAX_TOOL_CALLS_PER_TURN}",
            tool_blocks.len()
        )));
    }
    tool_blocks
        .into_iter()
        .enumerate()
        .map(|(index, block)| {
            let id = required_nonempty_string(block.get("id"), "id", index)?;
            let name = required_nonempty_string(block.get("name"), "name", index)?;
            let input = block
                .get("input")
                .filter(|value| value.is_object())
                .ok_or_else(|| {
                    LlmError::backend(format!(
                        "Anthropic tool call {index} input was not an object"
                    ))
                })?;
            Ok(crate::tools::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: input.clone(),
            })
        })
        .collect()
}

fn required_nonempty_string<'a>(
    value: Option<&'a Value>,
    field: &str,
    index: usize,
) -> Result<&'a str, LlmError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            LlmError::backend(format!(
                "backend tool call {index} carried no nonempty {field}"
            ))
        })
}

fn validate_openai_finish(value: &Value, calls: usize) -> Result<(), LlmError> {
    let finish = value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LlmError::backend("the backend response had no terminal finish reason".into())
        })?;
    match finish {
        "stop" if calls == 0 => Ok(()),
        "tool_calls" if calls > 0 => Ok(()),
        "length" => Err(LlmError::backend(
            "the backend exhausted its output limit before completing the response".into(),
        )),
        "content_filter" => Err(LlmError::backend(
            "the backend stopped without a complete response because of its content filter".into(),
        )),
        "stop" => Err(LlmError::backend(
            "the backend marked a tool-bearing response as plain completion".into(),
        )),
        "tool_calls" => Err(LlmError::backend(
            "the backend claimed tool completion without a valid tool call".into(),
        )),
        other => Err(LlmError::backend(format!(
            "the backend returned unsupported finish reason {other:?}"
        ))),
    }
}

fn validate_anthropic_stop(value: &Value, calls: usize) -> Result<(), LlmError> {
    let stop = value
        .get("stop_reason")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LlmError::backend("the Anthropic response had no terminal stop reason".into())
        })?;
    match stop {
        "end_turn" | "stop_sequence" if calls == 0 => Ok(()),
        "tool_use" if calls > 0 => Ok(()),
        "max_tokens" => Err(LlmError::backend(
            "Anthropic exhausted max_tokens before completing the response".into(),
        )),
        "model_context_window_exceeded" => Err(LlmError::backend(
            "Anthropic exhausted the model context window before completing the response".into(),
        )),
        "refusal" => Err(LlmError::backend(
            "Anthropic refused the request without a complete answer".into(),
        )),
        "pause_turn" => Err(LlmError::backend(
            "Anthropic paused the turn instead of completing it".into(),
        )),
        "end_turn" | "stop_sequence" => Err(LlmError::backend(
            "Anthropic marked a tool-bearing response as plain completion".into(),
        )),
        "tool_use" => Err(LlmError::backend(
            "Anthropic claimed tool completion without a valid tool call".into(),
        )),
        other => Err(LlmError::backend(format!(
            "Anthropic returned unsupported stop reason {other:?}"
        ))),
    }
}

/// Parse a completed response into text and/or bounded tool calls.
pub fn extract_turn(backend: &Backend, raw: &str) -> Result<ModelTurn, LlmError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| LlmError::backend(format!("the response was not JSON: {error}")))?;
    let turn = match backend {
        Backend::Anthropic { .. } => {
            let content = value.get("content").ok_or_else(|| {
                LlmError::backend("the Anthropic response carried no content array".into())
            })?;
            if !content.is_array() {
                return Err(LlmError::backend(
                    "the Anthropic response content was not an array".into(),
                ));
            }
            let calls = bounded_anthropic_calls(content)?;
            validate_anthropic_stop(&value, calls.len())?;
            let text = content
                .as_array()
                .expect("validated array")
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            ModelTurn { text, calls }
        }
        Backend::OpenAiCompatible { .. } => {
            let message = value.pointer("/choices/0/message").ok_or_else(|| {
                LlmError::backend("the backend response carried no assistant message".into())
            })?;
            let calls = bounded_openai_calls(message)?;
            validate_openai_finish(&value, calls.len())?;
            ModelTurn {
                text: message
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                calls,
            }
        }
    };
    if turn.text.trim().is_empty() && turn.calls.is_empty() {
        return extract_text(backend, raw).map(|text| ModelTurn {
            text,
            calls: Vec::new(),
        });
    }
    Ok(turn)
}

/// Extract visible text for the plain, non-tool ask path.
pub fn extract_text(backend: &Backend, raw: &str) -> Result<String, LlmError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| LlmError::backend(format!("the response was not JSON: {error}")))?;
    let text = match backend {
        Backend::Anthropic { .. } => {
            let content = value.get("content").ok_or_else(|| {
                LlmError::backend("the Anthropic response carried no content array".into())
            })?;
            if !content.is_array() {
                return Err(LlmError::backend(
                    "the Anthropic response content was not an array".into(),
                ));
            }
            let calls = bounded_anthropic_calls(content)?;
            validate_anthropic_stop(&value, calls.len())?;
            if !calls.is_empty() {
                return Err(LlmError::backend(
                    "backend returned unrequested tool calls".into(),
                ));
            }
            content
                .as_array()
                .expect("validated array")
                .iter()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|block| block.get("text").and_then(Value::as_str))
        }
        Backend::OpenAiCompatible { .. } => {
            let message = value.pointer("/choices/0/message").ok_or_else(|| {
                LlmError::backend("the backend response carried no assistant message".into())
            })?;
            let calls = bounded_openai_calls(message)?;
            validate_openai_finish(&value, calls.len())?;
            if !calls.is_empty() {
                return Err(LlmError::backend(
                    "backend returned unrequested tool calls".into(),
                ));
            }
            message.get("content").and_then(Value::as_str)
        }
    };
    match text {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ => {
            let reasoned = value
                .pointer("/choices/0/message/reasoning")
                .and_then(Value::as_str)
                .map_or(0, |reasoning| reasoning.chars().count());
            if reasoned > 0 {
                return Err(LlmError::response_budget(format!(
                    "the model spent its whole budget reasoning ({reasoned} chars) and produced no answer"
                )));
            }
            Err(LlmError::backend(
                "the response carried no answer text".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Backend {
        Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8080".into(),
            model: "test".into(),
        }
    }

    #[test]
    fn plain_extraction_requires_a_complete_call_free_turn() {
        let truncated =
            r#"{"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}"#;
        assert!(extract_text(&local(), truncated).is_err());

        let unsolicited = r#"{"choices":[{"message":{"content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"remember_fact","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#;
        assert_eq!(
            extract_text(&local(), unsolicited).unwrap_err().detail(),
            "backend returned unrequested tool calls"
        );
    }

    #[test]
    fn completed_openai_calls_require_exact_identity_type_and_object_arguments() {
        for call in [
            json!({"id":"", "type":"function", "function":{"name":"recall", "arguments":"{}"}}),
            json!({"id":"c1", "type":"other", "function":{"name":"recall", "arguments":"{}"}}),
            json!({"id":"c1", "type":"function", "function":{"name":"", "arguments":"{}"}}),
            json!({"id":"c1", "type":"function", "function":{"name":"recall", "arguments":"{"}}),
            json!({"id":"c1", "type":"function", "function":{"name":"recall", "arguments":"[]"}}),
        ] {
            let raw = json!({
                "choices": [{
                    "message": {"content": "", "tool_calls": [call]},
                    "finish_reason": "tool_calls"
                }]
            })
            .to_string();
            assert!(extract_turn(&local(), &raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn completed_anthropic_calls_require_identity_and_object_input() {
        let backend = Backend::Anthropic {
            api_key: "test".into(),
        };
        for call in [
            json!({"type":"tool_use", "id":"", "name":"recall", "input":{}}),
            json!({"type":"tool_use", "id":"c1", "name":"", "input":{}}),
            json!({"type":"tool_use", "id":"c1", "name":"recall", "input":[]}),
        ] {
            let raw = json!({"content": [call], "stop_reason": "tool_use"}).to_string();
            assert!(extract_turn(&backend, &raw).is_err(), "accepted {raw}");
        }
    }
}
