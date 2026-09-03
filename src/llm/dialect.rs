use super::{Backend, ChatTurn, LlmError, LlmRequest, Role};
use crate::tools::{ToolCall, ToolSpec};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Anthropic,
    OpenAi,
}

impl Dialect {
    pub fn from_backend(b: &Backend) -> Self {
        match b {
            Backend::Anthropic { .. } => Self::Anthropic,
            Backend::OpenAiCompatible { .. } => Self::OpenAi,
        }
    }
    pub fn url(self, b: &Backend) -> String {
        match (self, b) {
            (Self::Anthropic, _) => super::ANTHROPIC_URL.to_string(),
            (Self::OpenAi, Backend::OpenAiCompatible { endpoint, .. }) => {
                format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'))
            }
            _ => unreachable!(),
        }
    }
    pub fn headers(self, b: &Backend) -> Vec<(&'static str, String)> {
        match (self, b) {
            (Self::Anthropic, Backend::Anthropic { api_key }) => vec![
                ("x-api-key", api_key.clone()),
                ("anthropic-version", super::ANTHROPIC_VERSION.to_string()),
            ],
            (Self::OpenAi, _) => Vec::new(),
            _ => unreachable!(),
        }
    }
    // trait-like build: dialect builds body Value
    pub fn build(
        self,
        system_prompt: &str,
        turns: &[ChatTurn],
        tools: &[ToolSpec],
        backend: &Backend,
    ) -> Value {
        let mut messages: Vec<Value> = Vec::with_capacity(turns.len() + 1);
        if self == Self::OpenAi {
            messages.push(serde_json::json!({"role":"system","content":system_prompt}));
        }
        for turn in turns {
            match self {
                Self::Anthropic => match turn.role {
                    Role::User => {
                        messages.push(serde_json::json!({"role":"user","content":turn.text}))
                    }
                    Role::Assistant if turn.tool_calls.is_empty() => {
                        messages.push(serde_json::json!({"role":"assistant","content":turn.text}))
                    }
                    Role::Assistant => {
                        let mut blocks = Vec::new();
                        if !turn.text.is_empty() {
                            blocks.push(serde_json::json!({"type":"text","text":turn.text}));
                        }
                        for call in &turn.tool_calls {
                            blocks.push(serde_json::json!({"type":"tool_use","id":call.id,"name":call.name,"input":call.arguments}));
                        }
                        messages.push(serde_json::json!({"role":"assistant","content":blocks}));
                    }
                    Role::Tool => {
                        let block = serde_json::json!({"type":"tool_result","tool_use_id":turn.tool_call_id.clone().unwrap_or_default(),"content":turn.text});
                        if let Some(last) = messages.last_mut()
                            && last["role"] == "user"
                            && last["content"].is_array()
                        {
                            last["content"].as_array_mut().expect("array").push(block);
                        } else {
                            messages.push(serde_json::json!({"role":"user","content":[block]}));
                        }
                    }
                },
                Self::OpenAi => {
                    let mut m = serde_json::json!({"role":turn.role.as_str(),"content":turn.text});
                    if !turn.tool_calls.is_empty() {
                        m["tool_calls"] = Value::Array(turn.tool_calls.iter().map(|c| serde_json::json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.to_string()}})).collect());
                    }
                    if let Some(id) = &turn.tool_call_id {
                        m["tool_call_id"] = serde_json::json!(id);
                    }
                    messages.push(m);
                }
            }
        }
        let mut body = match self {
            Self::Anthropic => {
                serde_json::json!({"model":super::ANTHROPIC_MODEL,"max_tokens":super::MAX_TOKENS,"thinking":{"type":"disabled"},"system":system_prompt,"messages":messages})
            }
            Self::OpenAi => {
                let model = match backend {
                    Backend::OpenAiCompatible { model, .. } => model.as_str(),
                    _ => unreachable!(),
                };
                serde_json::json!({"model":model,"max_tokens":super::LOCAL_MAX_TOKENS,"messages":messages})
            }
        };
        if !tools.is_empty() {
            body["tools"] = match self {
                Self::Anthropic => crate::tools::anthropic_tools_json(tools),
                Self::OpenAi => crate::tools::openai_tools_json(tools),
            };
        }
        body
    }
    #[allow(dead_code)]
    pub fn extract(self, raw: &str) -> Result<super::ModelTurn, LlmError> {
        super::protocol::extract_via_dialect(self, raw)
    }
}

pub fn bounded_calls(calls: &[Value], limit: usize) -> Result<Vec<ToolCall>, LlmError> {
    if calls.len() > limit {
        return Err(LlmError::backend(format!(
            "the backend returned {} tool calls; the per-turn limit is {limit}",
            calls.len()
        )));
    }
    if calls.is_empty() {
        return Ok(Vec::new());
    }
    let is_anthropic = calls
        .first()
        .and_then(|v| v.get("type").and_then(Value::as_str))
        == Some("tool_use")
        || calls.iter().any(|v| v.get("input").is_some());
    if is_anthropic {
        calls
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let id = required(b.get("id"), "id", i)?;
                let name = required(b.get("name"), "name", i)?;
                let input = b.get("input").filter(|v| v.is_object()).ok_or_else(|| {
                    LlmError::backend(format!("Anthropic tool call {i} input was not an object"))
                })?;
                Ok(ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: input.clone(),
                })
            })
            .collect()
    } else {
        calls.iter().enumerate().map(|(i,c)| {
            let o=c.as_object().ok_or_else(|| LlmError::backend(format!("backend tool call {i} was not an object")))?;
            if o.get("type").and_then(Value::as_str)!=Some("function") { return Err(LlmError::backend(format!("backend tool call {i} did not declare type function"))); }
            let id=required(c.get("id"),"id",i)?;
            let f=o.get("function").and_then(Value::as_object).ok_or_else(|| LlmError::backend(format!("backend tool call {i} carried no function object")))?;
            let name=required(f.get("name"),"function name",i)?;
            let args=match f.get("arguments") {
                Some(Value::String(s))=>serde_json::from_str::<Value>(s).map_err(|e| LlmError::backend(format!("backend tool call {i} carried invalid JSON arguments: {e}")))?,
                Some(Value::Object(m))=>Value::Object(m.clone()),
                _=>return Err(LlmError::backend(format!("backend tool call {i} arguments were not a JSON object or encoded object"))),
            };
            if !args.is_object() { return Err(LlmError::backend(format!("backend tool call {i} arguments were not a JSON object"))); }
            Ok(ToolCall{id:id.to_string(),name:name.to_string(),arguments:args})
        }).collect()
    }
}
fn required<'a>(v: Option<&'a Value>, field: &str, i: usize) -> Result<&'a str, LlmError> {
    v.and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            LlmError::backend(format!("backend tool call {i} carried no nonempty {field}"))
        })
}
pub fn validate_terminal(finish: &str, calls: usize) -> Result<(), LlmError> {
    match finish {
        "stop" | "end_turn" | "stop_sequence" if calls == 0 => Ok(()),
        "tool_calls" | "tool_use" if calls > 0 => Ok(()),
        "length" | "max_tokens" => Err(LlmError::backend(
            "the backend exhausted its output limit before completing the response".into(),
        )),
        "content_filter" => Err(LlmError::backend(
            "the backend stopped without a complete response because of its content filter".into(),
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
        "stop" | "end_turn" | "stop_sequence" => Err(LlmError::backend(
            "the backend marked a tool-bearing response as plain completion".into(),
        )),
        "tool_calls" | "tool_use" => Err(LlmError::backend(
            "the backend claimed tool completion without a valid tool call".into(),
        )),
        other => Err(LlmError::backend(format!(
            "the backend returned unsupported finish reason {other:?}"
        ))),
    }
}
pub fn build_chat_request_with_tools(
    backend: &Backend,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[ToolSpec],
) -> LlmRequest {
    let d = Dialect::from_backend(backend);
    LlmRequest {
        url: d.url(backend),
        headers: d.headers(backend),
        body: d.build(system_prompt, turns, tools, backend),
    }
}
