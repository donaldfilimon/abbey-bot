# Reply quality & speed — local-first generation that feels alive

**Date:** 2026-08-19 · **Status:** design (sub-project 1 of 4, "improve all"); Donald's standing directive 2026-08-19 ("continue and don't stop until all is completed … fully thought out") is the approval to proceed; every decision below is stated so it can be overridden.

## Goal

Make Abbey's generated replies faster to *appear*, shorter and cleaner when they land, and robust to a slow local model — without an Anthropic key (design for absence; Anthropic stays the preferred backend whenever `ANTHROPIC_API_KEY` exists).

## What was measured (2026-08-19, ollama on this Mac, Abbey's real system prompt, 3 prompts each)

See `docs/benchmarks/2026-08-19-local-models.md` (filled from the benchmark run). The chosen default is the model with the best latency among those that produced non-empty content for all three prompts without exceeding ~600 characters on the short ones.

## Decisions

1. **One generation at a time per local endpoint.** `AppState.generation: tokio::sync::Semaphore` sized from `ABBEY_BOT_LLM_CONCURRENCY` (default 1 for the local path; Anthropic default 4). Ollama wedged under concurrent requests today; serialising is the fix, and the typing keepalive keeps running while a turn waits its turn. Waiting beyond `ABBEY_BOT_LLM_QUEUE_SECS` (default 90) returns the honest failure line ("the model is busy; try again in a minute") rather than piling up.
2. **Progressive replies on the local path.** The OpenAI-compatible path requests `stream: true`; `llm.rs` gains a pure SSE line parser (`data: {...}` → `choices[0].delta.content`, `[DONE]`). The pipeline posts the reply as soon as ~60 characters or 4 seconds have arrived, then **edits** the message every 2 seconds with the accumulated text (Discord allows ~5 edits / 5 s per channel; 2 s is safe), finishing with one final edit. `Outbound` gains `edit(native_channel_id, native_message_id, text)`. Telegram edits via `editMessageText`; Slack via `chat.update`. Anthropic path stays non-streaming (it is already 1–3 s). Slash commands (`/persona ask`, `/summarize`) keep the deferred single reply — edit-in-place on an interaction followup is a different API; out of scope.
3. **The length contract is enforced, not hoped for.** Pure `ask::tidy_reply(persona, text) -> String`: strip a leading `Abbey:`/`**Abbey**` echo, strip markdown headings, collapse 3+ blank lines to 1, trim; if longer than 1,900 chars, cut at the last sentence end before 1,800 and append " …". Applied before clamp on every generated reply (pipeline, `/persona ask`, `/summarize`, welcome).
4. **Reasoning stays in the model's head.** On the OpenAI-compatible path send `"reasoning_effort": "low"` **only if** the benchmark shows the endpoint honours it without breaking content; otherwise nothing (today's probe returned empty bodies with it, so the default is *off* unless measured otherwise). `extract_text` already reports reasoning-only bodies honestly.
5. **Fallback routing.** When both `ANTHROPIC_API_KEY` and `ABBEY_BOT_LLM_ENDPOINT` are set, a failed Anthropic call falls back to the local backend once (logged). With only one configured, behaviour is unchanged. No per-guild routing (YAGNI).
6. **Model choice is config + a documented measurement**, not code: README gains the benchmark table and a recommended `ABBEY_BOT_LLM_MODEL`.

## Out of scope

Tools (sub-project 2), Anthropic streaming, interaction-followup edits, per-guild model selection, prompt A/B evaluation harness (Proposed).

## Testing

Pure: SSE parser (chunks split mid-line, `[DONE]`, malformed lines ignored), `tidy_reply` cases (echo strip, heading strip, sentence-boundary cut, short text untouched), queue semantics (semaphore + timeout → honest failure; RecordingTransport-based), fallback order. Live: a DM shows the message appear within ~4 s and grow; `ollama ps` shows one runner busy at a time under two concurrent DMs.
