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

impl ToolResult {
    /// Runtime read results that may add lexical evidence to a final reply.
    /// Mutation acknowledgements, persona switches, and unknown tools are not
    /// evidence merely because execution returned a string.
    pub fn grounding_source(&self) -> Option<&str> {
        matches!(
            self.name.as_str(),
            "lookup_reputation" | "recall" | "recent_messages" | "inspect_status" | "list_facts"
        )
        .then_some(self.content.as_str())
    }
}

/// Named in-process tool groups. Not a plugin host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPack {
    /// Original five: remember_fact, lookup_reputation, recall,
    /// switch_persona, recent_messages.
    Core,
    /// inspect_status, list_facts. Default on with Core when tools are enabled.
    Inspect,
}

/// Closed aspect enum for `inspect_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectAspect {
    Runtime,
    Guild,
    Voice,
    Provider,
    All,
}

impl InspectAspect {
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("runtime") {
            "runtime" => Some(Self::Runtime),
            "guild" => Some(Self::Guild),
            "voice" => Some(Self::Voice),
            "provider" => Some(Self::Provider),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// Longest tool result handed back to the model.
pub const MAX_RESULT_CHARS: usize = 600;
/// Most recent messages `recent_messages` will return.
pub const MAX_RECENT: usize = 50;
/// Rounds of call → result → call before the loop gives up and answers.
pub const MAX_TOOL_ROUNDS: usize = 3;

/// The original five-tool compatibility vocabulary. Order and wire details are
/// intentionally stable; production uses [`production_tools`] instead.
pub fn abbey_tools() -> Vec<ToolSpec> {
    tools_for(&[SkillPack::Core])
}

/// The complete production vocabulary whenever the global tools policy is on.
pub fn production_tools() -> Vec<ToolSpec> {
    let mut tools = abbey_tools();
    tools.extend(inspect_tools());
    tools
}

/// Assemble packs in stable order: Core, then Inspect. Never sort names.
pub fn tools_for(packs: &[SkillPack]) -> Vec<ToolSpec> {
    let mut out = Vec::new();
    if packs.contains(&SkillPack::Core) {
        out.extend(core_tools());
    }
    if packs.contains(&SkillPack::Inspect) {
        out.extend(inspect_tools());
    }
    out
}

fn core_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "remember_fact",
            description: "Store one durable fact about the person you are talking to, in third person, for future conversations in this server.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "fact": { "type": "string", "description": "A single concise fact", "maxLength": crate::memory::MAX_FACT_CHARS },
                    "supersedes": { "type": "string", "description": "An existing fact this probably replaces. This only PROPOSES the replacement — the old fact is kept until the person confirms it themselves. Never assume it was removed.", "maxLength": crate::memory::MAX_FACT_CHARS }
                },
                "required": ["fact"]
            }),
        },
        ToolSpec {
            name: "lookup_reputation",
            description: "Look up a member's standing in this server (0 = poor, 1 = excellent). Omit user_id for the person you are talking to.",
            parameters: json!({
                "type": "object",
                "properties": { "user_id": { "type": "string", "description": "Native user id on the current network (Discord, Telegram, or Slack)" } }
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
            description: "Hand this conversation to a persona better suited to it: abbey (warm, sharp friend), aviva (direct, technical), or abi (orchestrating, de-escalating). The transcript is kept.",
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

fn inspect_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "inspect_status",
            description: "Read privacy-bounded status for this process and conversation. aspect=runtime (default): coarse runtime flags. aspect=guild: recorded settings for this server without creating defaults. aspect=voice: off, presence, awaiting-consent, active, or paused for this server only. aspect=provider: effective routable capability categories and configured-versus-qualified provenance. aspect=all: all four bounded views. Never returns endpoints, paths, model names, hashes, credentials, participant data, counters, timestamps, audio, or transcripts.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "aspect": { "type": "string", "enum": ["runtime", "guild", "voice", "provider", "all"] }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "list_facts",
            description: "Read a bounded canonical subject view of stored facts about the person you are talking to and replacements waiting for that person to confirm with /pending. Omitted facts and pending replacements are counted independently. This is not semantic search; use recall to search by meaning.",
            parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
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
#[cfg(test)]
fn parse_openai_tool_calls(message: &Value) -> Vec<ToolCall> {
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
#[cfg(test)]
fn parse_anthropic_tool_use(content: &Value) -> Vec<ToolCall> {
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
    /// `supersedes` is a PROPOSAL, never an instruction. The host stores the
    /// new fact and queues the proposal; the named old fact survives until a
    /// human explicitly confirms. A model must never be able to delete a
    /// remembered fact.
    fn remember_fact(&mut self, fact: &str, supersedes: Option<&str>) -> String;
    fn lookup_reputation(&mut self, user_id: Option<&str>) -> String;
    fn recall(&mut self, query: &str) -> String;
    fn switch_persona(&mut self, persona: Persona) -> String;
    fn recent_messages(&mut self, limit: usize) -> String;
    fn inspect_status(&mut self, aspect: InspectAspect) -> String;
    fn list_facts(&mut self) -> String;
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
            Some(fact) => host.remember_fact(&fact, arg_str("supersedes").as_deref()),
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
        "inspect_status" => match inspect_aspect_argument(&call.arguments) {
            Ok(aspect) => host.inspect_status(aspect),
            Err(message) => message.into(),
        },
        "list_facts" => match empty_inspect_arguments(&call.arguments) {
            Ok(()) => host.list_facts(),
            Err(message) => message.into(),
        },
        other => format!("Unknown tool `{other}`."),
    };
    ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        content: truncate(&content),
    }
}

fn inspect_aspect_argument(arguments: &Value) -> Result<InspectAspect, &'static str> {
    let Value::Object(arguments) = arguments else {
        return Err("inspect_status needs an arguments object.");
    };
    if arguments.keys().any(|key| key != "aspect") {
        return Err("inspect_status received an unsupported argument.");
    }
    let raw = match arguments.get("aspect") {
        None => None,
        Some(Value::String(raw)) => Some(raw.as_str()),
        Some(_) => return Err("inspect_status needs `aspect` to be a string."),
    };
    InspectAspect::parse(raw)
        .ok_or("inspect_status needs `aspect` = runtime | guild | voice | provider | all.")
}

fn empty_inspect_arguments(arguments: &Value) -> Result<(), &'static str> {
    match arguments {
        Value::Object(arguments) if arguments.is_empty() => Ok(()),
        Value::Object(_) => Err("list_facts does not accept arguments."),
        _ => Err("list_facts needs an empty arguments object."),
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
        fn remember_fact(&mut self, fact: &str, supersedes: Option<&str>) -> String {
            match supersedes {
                Some(old) => self.log.push(format!("remember:{fact};supersedes:{old}")),
                None => self.log.push(format!("remember:{fact}")),
            }
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
        fn inspect_status(&mut self, aspect: InspectAspect) -> String {
            self.log.push(format!("inspect:{aspect:?}"));
            format!("status:{aspect:?}")
        }
        fn list_facts(&mut self) -> String {
            self.log.push("list_facts".into());
            "Nothing on record.".into()
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
        assert_eq!(
            oa[0]["function"]["parameters"]["properties"]["fact"]["maxLength"],
            crate::memory::MAX_FACT_CHARS
        );
        assert_eq!(an[3]["name"], "switch_persona");
        assert_eq!(
            an[3]["input_schema"]["properties"]["persona"]["enum"][1],
            "aviva"
        );
        assert_eq!(
            oa[1]["function"]["parameters"]["properties"]["user_id"]["description"],
            "Native user id on the current network (Discord, Telegram, or Slack)"
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

    #[test]
    fn only_read_tools_expose_grounding_sources() {
        let result = |name: &str| ToolResult {
            call_id: "c".into(),
            name: name.into(),
            content: "source".into(),
        };
        for name in [
            "lookup_reputation",
            "recall",
            "recent_messages",
            "inspect_status",
            "list_facts",
        ] {
            assert_eq!(result(name).grounding_source(), Some("source"), "{name}");
        }
        for name in ["remember_fact", "switch_persona", "unknown"] {
            assert_eq!(result(name).grounding_source(), None, "{name}");
        }
    }

    #[test]
    fn inspect_pack_appends_after_core_without_reordering() {
        let tools = production_tools();
        let names: Vec<_> = tools.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "remember_fact",
                "lookup_reputation",
                "recall",
                "switch_persona",
                "recent_messages",
                "inspect_status",
                "list_facts",
            ]
        );
        assert_eq!(abbey_tools().len(), 5);
    }

    #[test]
    fn inspect_dispatch_defaults_and_rejects_unknown_aspect() {
        let mut host = FakeHost::default();
        let mk = |name: &str, args: Value| ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments: args,
        };
        assert_eq!(
            dispatch(&mk("inspect_status", json!({})), &mut host).content,
            "status:Runtime"
        );
        assert!(
            dispatch(&mk("inspect_status", json!({"aspect": "nope"})), &mut host)
                .content
                .contains("runtime | guild")
        );
        assert_eq!(
            dispatch(&mk("list_facts", json!({})), &mut host).content,
            "Nothing on record."
        );
        assert_eq!(host.log, ["inspect:Runtime", "list_facts"]);
    }

    #[test]
    fn inspect_dispatch_rejects_malformed_arguments_without_calling_the_host() {
        let mut host = FakeHost::default();
        let mk = |name: &str, arguments: Value| ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments,
        };

        for call in [
            mk("inspect_status", json!({"aspect": 7})),
            mk("inspect_status", json!({"aspect": " runtime "})),
            mk("inspect_status", json!({"unexpected": true})),
            mk("inspect_status", json!(null)),
            mk("list_facts", json!({"user_id": "someone-else"})),
            mk("list_facts", json!([])),
        ] {
            let result = dispatch(&call, &mut host);
            assert!(
                result.content.contains("needs")
                    || result.content.contains("unsupported")
                    || result.content.contains("does not accept"),
                "{result:?}"
            );
        }
        assert!(
            host.log.is_empty(),
            "malformed calls must not reach the host"
        );
    }

    #[test]
    fn production_wire_shapes_expose_exactly_the_same_seven_tools() {
        let tools = production_tools();
        let expected = [
            "remember_fact",
            "lookup_reputation",
            "recall",
            "switch_persona",
            "recent_messages",
            "inspect_status",
            "list_facts",
        ];
        let openai = openai_tools_json(&tools);
        let anthropic = anthropic_tools_json(&tools);
        let openai_names: Vec<_> = openai
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        let anthropic_names: Vec<_> = anthropic
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();

        assert_eq!(openai_names, expected);
        assert_eq!(anthropic_names, expected);
        assert_eq!(
            abbey_tools().len(),
            5,
            "compatibility vocabulary remains five"
        );
    }
}
