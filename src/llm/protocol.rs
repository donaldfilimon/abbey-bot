use super::dialect::{Dialect, bounded_calls, validate_terminal};
use super::{Backend, ChatTurn, LlmError, LlmRequest, ModelTurn};
use serde_json::Value;

pub fn build_request(backend: &Backend, system_prompt: &str, question: &str) -> LlmRequest {
    build_chat_request(backend, system_prompt, &[ChatTurn::user(question)])
}
pub fn build_chat_request(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
) -> LlmRequest {
    crate::llm::build_chat_request_with_tools(backend, system_prompt, turns, &[])
}
fn extract_openai_calls(msg: &Value) -> Result<Vec<Value>, LlmError> {
    match msg.get("tool_calls") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => Err(LlmError::backend(
            "the backend tool_calls field was not an array".into(),
        )),
    }
}
fn anthropic_blocks(content: &Value) -> Vec<Value> {
    content
        .as_array()
        .expect("validated")
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .cloned()
        .collect()
}
fn openai_finish(v: &Value) -> Result<&str, LlmError> {
    v.pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LlmError::backend("the backend response had no terminal finish reason".into())
        })
}
fn anthropic_stop(v: &Value) -> Result<&str, LlmError> {
    v.get("stop_reason").and_then(Value::as_str).ok_or_else(|| {
        LlmError::backend("the Anthropic response had no terminal stop reason".into())
    })
}
pub fn extract_turn(backend: &Backend, raw: &str) -> Result<ModelTurn, LlmError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| LlmError::backend(format!("the response was not JSON: {e}")))?;
    let d = Dialect::from_backend(backend);
    let turn = match d {
        Dialect::Anthropic => {
            let content = v.get("content").ok_or_else(|| {
                LlmError::backend("the Anthropic response carried no content array".into())
            })?;
            if !content.is_array() {
                return Err(LlmError::backend(
                    "the Anthropic response content was not an array".into(),
                ));
            }
            let raw_calls = anthropic_blocks(content);
            let calls = bounded_calls(&raw_calls, super::MAX_TOOL_CALLS_PER_TURN)?;
            validate_terminal(anthropic_stop(&v)?, calls.len())?;
            let text = content
                .as_array()
                .expect("validated")
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            ModelTurn { text, calls }
        }
        Dialect::OpenAi => {
            let msg = v.pointer("/choices/0/message").ok_or_else(|| {
                LlmError::backend("the backend response carried no assistant message".into())
            })?;
            let raw_calls = extract_openai_calls(msg)?;
            let calls = bounded_calls(&raw_calls, super::MAX_TOOL_CALLS_PER_TURN)?;
            validate_terminal(openai_finish(&v)?, calls.len())?;
            ModelTurn {
                text: msg
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
pub fn extract_text(backend: &Backend, raw: &str) -> Result<String, LlmError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| LlmError::backend(format!("the response was not JSON: {e}")))?;
    let d = Dialect::from_backend(backend);
    let text = match d {
        Dialect::Anthropic => {
            let content = v.get("content").ok_or_else(|| {
                LlmError::backend("the Anthropic response carried no content array".into())
            })?;
            if !content.is_array() {
                return Err(LlmError::backend(
                    "the Anthropic response content was not an array".into(),
                ));
            }
            let raw_calls = anthropic_blocks(content);
            let calls = bounded_calls(&raw_calls, super::MAX_TOOL_CALLS_PER_TURN)?;
            validate_terminal(anthropic_stop(&v)?, calls.len())?;
            if !calls.is_empty() {
                return Err(LlmError::backend(
                    "backend returned unrequested tool calls".into(),
                ));
            }
            content
                .as_array()
                .expect("validated")
                .iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|b| b.get("text").and_then(Value::as_str))
        }
        Dialect::OpenAi => {
            let msg = v.pointer("/choices/0/message").ok_or_else(|| {
                LlmError::backend("the backend response carried no assistant message".into())
            })?;
            let raw_calls = extract_openai_calls(msg)?;
            let calls = bounded_calls(&raw_calls, super::MAX_TOOL_CALLS_PER_TURN)?;
            validate_terminal(openai_finish(&v)?, calls.len())?;
            if !calls.is_empty() {
                return Err(LlmError::backend(
                    "backend returned unrequested tool calls".into(),
                ));
            }
            msg.get("content").and_then(Value::as_str)
        }
    };
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t.to_string()),
        _ => {
            let reasoned = v
                .pointer("/choices/0/message/reasoning")
                .and_then(Value::as_str)
                .map_or(0, |r| r.chars().count());
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
#[allow(dead_code)]
pub(crate) fn extract_via_dialect(d: Dialect, raw: &str) -> Result<ModelTurn, LlmError> {
    let b = match d {
        Dialect::Anthropic => Backend::Anthropic {
            api_key: String::new(),
        },
        Dialect::OpenAi => Backend::OpenAiCompatible {
            endpoint: String::new(),
            model: String::new(),
        },
    };
    extract_turn(&b, raw)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
            json!({"id":"","type":"function","function":{"name":"recall","arguments":"{}"}}),
            json!({"id":"c1","type":"other","function":{"name":"recall","arguments":"{}"}}),
            json!({"id":"c1","type":"function","function":{"name":"","arguments":"{}"}}),
            json!({"id":"c1","type":"function","function":{"name":"recall","arguments":"{"}}),
            json!({"id":"c1","type":"function","function":{"name":"recall","arguments":"[]"}}),
        ] {
            let raw=json!({"choices":[{"message":{"content":"","tool_calls":[call]},"finish_reason":"tool_calls"}]}).to_string();
            assert!(extract_turn(&local(), &raw).is_err(), "accepted {raw}");
        }
    }
    #[test]
    fn completed_anthropic_calls_require_identity_and_object_input() {
        let backend = Backend::Anthropic {
            api_key: "test".into(),
        };
        for call in [
            json!({"type":"tool_use","id":"","name":"recall","input":{}}),
            json!({"type":"tool_use","id":"c1","name":"","input":{}}),
            json!({"type":"tool_use","id":"c1","name":"recall","input":[]}),
        ] {
            let raw = json!({"content":[call],"stop_reason":"tool_use"}).to_string();
            assert!(extract_turn(&backend, &raw).is_err(), "accepted {raw}");
        }
    }
}
