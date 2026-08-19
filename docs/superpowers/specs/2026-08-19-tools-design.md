# Tools — the model can call Abbey's own systems

**Date:** 2026-08-19 · **Status:** design (sub-project 2 of 4); proceeding under Donald's standing directive; decisions stated for override.

## Goal

`docs/spec/appleintelligence.md` "Tools — where slash commands and the model converge": the model decides to store a fact, look someone up, recall, or switch persona — instead of an intent classifier guessing. Both wire shapes (OpenAI function-calling, Anthropic `tool_use`), one pure tool model, a bounded loop, honest degrade when the backend cannot call tools.

## Measured 2026-08-19

gpt-oss:20b via ollama `/v1` returns `tool_calls` (non-stream) and streams each call as one delta with full `arguments`; finish_reason `tool_calls`. Anthropic shape from its public API (recording transport only — no key).

## Decisions

1. **`src/tools.rs` is pure.** `ToolSpec {name, description, parameters: Value}`, `ToolCall {id, name, arguments: Value}`, `ToolResult {call_id, name, content}`; `abbey_tools()`; `openai_tools_json`, `anthropic_tools_json`; `parse_openai_tool_calls(&message)`, `parse_anthropic_tool_use(&content)`; `dispatch(call, &mut dyn ToolHost) -> ToolResult`. `ToolHost` is the seam the runtime implements over `AppState` (memory, recall, social, engine persona); tools.rs never sees a lock or a Discord type.
2. **The tools** (all namespaced to the invoking scoped guild + user; none act on Discord): `remember_fact(fact)`, `lookup_reputation(user_id?)`, `recall(query)`, `switch_persona(persona)` (changes only this conversation's engine persona; the reply keeps the transcript — the spec's point), `recent_messages(limit≤50)` (the channel's recent lines as text — gives `/summarize`-style grounding without a second generation). No moderation, no posting, no config.
3. **Wire.** `llm::ChatTurn` grows `tool_calls: Vec<ToolCall>` (assistant) and `tool_call_id: Option<String>` (role `Tool`); `build_chat_request` emits OpenAI `tools`/`tool_calls`/`role:"tool"` and Anthropic `tools`/`tool_use`/`tool_result` blocks; `extract_turn(backend, raw) -> ModelTurn {text, calls}`; `SseAccumulator` merges streamed `delta.tool_calls` by index.
4. **Loop.** At most `MAX_TOOL_ROUNDS = 3`: generate → if calls: dispatch each, append assistant-calls + tool-result turns, generate again → final text. On the local streaming path, `stream_reply` reports `Finished::Calls(calls)` instead of text; the pipeline runs the tools and streams again (first post is deferred until a round yields text). Tool results are also recorded in the engine transcript so later turns see them.
5. **Enable/degrade.** `ABBEY_BOT_LLM_TOOLS=auto|on|off` (default auto = on). If the first tooled request fails with HTTP 4xx, retry once without tools and remember `tools_unsupported` for the process (logged once). Tools are offered on mention/DM replies and `/persona ask`; unsolicited policy replies and `/summarize` do not offer tools (YAGNI, and keeps budgeted replies single-shot).
6. **Safety.** `remember_fact` dedupes and caps like `/remember`; `lookup_reputation` never reveals another guild's numbers; every tool result is a short plain string (≤ 600 chars) so it cannot blow the context; arguments are validated (persona must parse; limit clamped).

## Out of scope

Tool use in unsolicited replies; model-initiated moderation; `summarize_channel` as a tool (use `recent_messages` + the model); Anthropic streaming.

## Testing

Pure: tool JSON shapes both ways; parse both response shapes; dispatch against a fake `ToolHost`; SSE tool-call merging (fragmented and whole); loop terminates at 3 rounds; degrade on 4xx via RecordingTransport. Live: DM "remember that I build in nightly Rust" → `remember_fact` call logged, `/recall` shows it; "what do you remember about me?" → `recall` call; "be aviva" → `switch_persona`.
