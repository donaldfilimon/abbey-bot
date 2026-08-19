//! Tools the model may call — Abbey's own systems exposed as functions
//! (`docs/spec/appleintelligence.md`, `docs/superpowers/specs/2026-08-19-tools-design.md`).
//!
//! Pure: this module defines the tool vocabulary, renders it in both wire
//! shapes (OpenAI function-calling, Anthropic `tool_use`), parses both
//! response shapes, and dispatches a call against a [`ToolHost`] — the seam
//! the runtime implements over `AppState`. Nothing here touches a lock, the
//! network, or Discord. Every tool result is a short plain string so a call
//! can never blow the context budget.

use serde_json::{Value, json};

use crate::persona::Persona;

/// A function the model may call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the arguments object.
    pub parameters: Value,
}

/// One call the model asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// What a call produced, keyed back to the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub content: String,
}

/// Longest tool result handed back to the model.
pub const MAX_RESULT_CHARS: usize = 600;
/// Most recent messages `recent_messages` will return.
pub const MAX_RECENT: usize = 50;
/// Rounds of call → result → call before the loop gives up and answers.
pub const MAX_TOOL_ROUNDS: usize = 3;

/// Abbey's tool vocabulary. Order is the order the model sees.
pub fn abbey_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "remember_fact",
            description: "Store one durable fact about the person you are talking to, in third person, for future conversations in this server.",
            parameters: json!({
                "type": "object",
                "properties": { "fact": { "type": "string", "description": "A single concise fact" } },
                "required": ["fact"]
            }),
        },
        ToolSpec {
            name: "lookup_reputation",
            description: "Look up a member's standing in this server (0 = poor, 1 = excellent). Omit user_id for the person you are talking to.",
            parameters: json!({
                "type": "object",
                "properties": { "user_id": { "type": "string", "description": "Discord user id" } }
            }),
        },
        ToolSpec {
            name: "recall",
            description: "Search what you remember about the person you are talking to, by meaning.",
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        },
        ToolSpec {
            name: "switch_persona",
            description: "Hand this conversation to a persona better suited to it: abbey (warm polymath), aviva (direct, technical), or abi (orchestrating, de-escalating). The transcript is kept.",
            parameters: json!({
                "type": "object",
                "properties": { "persona": { "type": "string", "enum": ["abbey", "aviva", "abi"] } },
                "required": ["persona"]
            }),
        },
        ToolSpec {
            name: "recent_messages",
            description: "Read the most recent messages in this channel (oldest first), to ground a summary or an answer.",
            parameters: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 50 } }
            }),
        },
    ]
}

/// OpenAI function-calling shape: `tools: [{type: function, function: {...}}]`.
pub fn openai_tools_json(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect(),
    )
}

/// Anthropic shape: `tools: [{name, description, input_schema}]`.
pub fn anthropic_tools_json(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters }))
            .collect(),
    )
}

/// Calls in an OpenAI-style assistant `message` (`tool_calls[]`, with
/// `function.arguments` as a JSON *string*). Unparseable argument strings
/// become `{}` rather than dropping the call, so the model still gets a
/// result telling it what went wrong.
pub fn parse_openai_tool_calls(message: &Value) -> Vec<ToolCall> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c.pointer("/function/name")?.as_str()?.to_string();
            let id = c
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("call_0")
                .to_string();
            let arguments = match c.pointer("/function/arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
                Some(v @ Value::Object(_)) => v.clone(),
                _ => json!({}),
            };
            Some(ToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

/// Calls in an Anthropic `content` array (`type: tool_use` blocks).
pub fn parse_anthropic_tool_use(content: &Value) -> Vec<ToolCall> {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|b| {
            Some(ToolCall {
                id: b.get("id")?.as_str()?.to_string(),
                name: b.get("name")?.as_str()?.to_string(),
                arguments: b.get("input").cloned().unwrap_or(json!({})),
            })
        })
        .collect()
}

/// What the runtime must provide for dispatch. Every method returns the
/// plain text the model will read; the host owns scoping (guild/user) and
/// any locking.
pub trait ToolHost {
    fn remember_fact(&mut self, fact: &str) -> String;
    fn lookup_reputation(&mut self, user_id: Option<&str>) -> String;
    fn recall(&mut self, query: &str) -> String;
    fn switch_persona(&mut self, persona: Persona) -> String;
    fn recent_messages(&mut self, limit: usize) -> String;
}

/// Run one call against the host. Unknown tools and bad arguments produce a
/// result that says so — never an error that aborts the turn.
pub fn dispatch(call: &ToolCall, host: &mut dyn ToolHost) -> ToolResult {
    let arg_str = |key: &str| {
        call.arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let content = match call.name.as_str() {
        "remember_fact" => match arg_str("fact") {
            Some(fact) => host.remember_fact(&fact),
            None => "remember_fact needs a non-empty `fact`.".to_string(),
        },
        "lookup_reputation" => host.lookup_reputation(arg_str("user_id").as_deref()),
        "recall" => match arg_str("query") {
            Some(q) => host.recall(&q),
            None => "recall needs a non-empty `query`.".to_string(),
        },
        "switch_persona" => match arg_str("persona")
            .as_deref()
            .and_then(crate::guild::parse_persona)
        {
            Some(p) => host.switch_persona(p),
            None => "switch_persona needs `persona` = abbey | aviva | abi.".to_string(),
        },
        "recent_messages" => {
            let limit = call
                .arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(20, |n| usize::try_from(n).unwrap_or(MAX_RECENT))
                .clamp(1, MAX_RECENT);
            host.recent_messages(limit)
        }
        other => format!("Unknown tool `{other}`."),
    };
    ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        content: truncate(&content),
    }
}

/// Cap a result at [`MAX_RESULT_CHARS`] codepoints.
pub fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_RESULT_CHARS - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeHost {
        log: Vec<String>,
    }

    impl ToolHost for FakeHost {
        fn remember_fact(&mut self, fact: &str) -> String {
            self.log.push(format!("remember:{fact}"));
            format!("Stored: {fact}")
        }
        fn lookup_reputation(&mut self, user_id: Option<&str>) -> String {
            self.log.push(format!("rep:{}", user_id.unwrap_or("self")));
            "Reputation 0.50 (0 = poor, 1 = excellent).".into()
        }
        fn recall(&mut self, query: &str) -> String {
            self.log.push(format!("recall:{query}"));
            "• builds in nightly Rust".into()
        }
        fn switch_persona(&mut self, persona: Persona) -> String {
            self.log.push(format!("switch:{persona}"));
            format!("Switched to {persona}.")
        }
        fn recent_messages(&mut self, limit: usize) -> String {
            self.log.push(format!("recent:{limit}"));
            "a: hi\nb: yo".into()
        }
    }

    #[test]
    fn both_wire_shapes_carry_every_tool() {
        let tools = abbey_tools();
        let oa = openai_tools_json(&tools);
        let an = anthropic_tools_json(&tools);
        assert_eq!(oa.as_array().unwrap().len(), 5);
        assert_eq!(oa[0]["type"], "function");
        assert_eq!(oa[0]["function"]["name"], "remember_fact");
        assert_eq!(an[3]["name"], "switch_persona");
        assert_eq!(
            an[3]["input_schema"]["properties"]["persona"]["enum"][1],
            "aviva"
        );
    }

    #[test]
    fn openai_calls_parse_with_string_arguments_and_survive_bad_json() {
        let message = json!({
            "role": "assistant", "content": "",
            "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "remember_fact", "arguments": "{\"fact\":\"builds in nightly Rust\"}"}},
                {"id": "call_2", "type": "function", "function": {"name": "recall", "arguments": "{not json"}}
            ]
        });
        let calls = parse_openai_tool_calls(&message);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["fact"], "builds in nightly Rust");
        assert_eq!(calls[1].arguments, json!({}));
        assert!(parse_openai_tool_calls(&json!({"role": "assistant", "content": "hi"})).is_empty());
    }

    #[test]
    fn anthropic_tool_use_blocks_parse() {
        let content = json!([
            {"type": "text", "text": "Let me check."},
            {"type": "tool_use", "id": "toolu_1", "name": "lookup_reputation", "input": {"user_id": "42"}}
        ]);
        let calls = parse_anthropic_tool_use(&content);
        assert_eq!(
            calls,
            vec![ToolCall {
                id: "toolu_1".into(),
                name: "lookup_reputation".into(),
                arguments: json!({"user_id": "42"})
            }]
        );
    }

    #[test]
    fn dispatch_routes_validates_and_never_aborts() {
        let mut host = FakeHost::default();
        let mk = |name: &str, args: Value| ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments: args,
        };
        assert_eq!(
            dispatch(
                &mk("remember_fact", json!({"fact": " builds in nightly Rust "})),
                &mut host
            )
            .content,
            "Stored: builds in nightly Rust"
        );
        assert!(
            dispatch(&mk("remember_fact", json!({})), &mut host)
                .content
                .contains("needs")
        );
        assert!(
            dispatch(&mk("lookup_reputation", json!({})), &mut host)
                .content
                .starts_with("Reputation")
        );
        assert!(
            dispatch(
                &mk("switch_persona", json!({"persona": "AVIVA"})),
                &mut host
            )
            .content
            .contains("Aviva")
        );
        assert!(
            dispatch(&mk("switch_persona", json!({"persona": "bob"})), &mut host)
                .content
                .contains("abbey | aviva | abi")
        );
        dispatch(&mk("recent_messages", json!({"limit": 999})), &mut host);
        dispatch(&mk("recent_messages", json!({})), &mut host);
        assert!(
            dispatch(&mk("nope", json!({})), &mut host)
                .content
                .contains("Unknown tool")
        );
        assert_eq!(
            host.log,
            [
                "remember:builds in nightly Rust",
                "rep:self",
                "switch:Aviva",
                "recent:50",
                "recent:20"
            ]
        );
    }

    #[test]
    fn results_are_capped() {
        let long = "x".repeat(2_000);
        let t = truncate(&long);
        assert_eq!(t.chars().count(), MAX_RESULT_CHARS);
        assert!(t.ends_with('…'));
    }
}
