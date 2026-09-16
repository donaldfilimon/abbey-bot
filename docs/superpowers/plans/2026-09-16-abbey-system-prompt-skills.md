# Abbey system prompt + skills Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve Discord Abbey/Aviva/Abi system prompts in `src/ask.rs` for local-only inference honesty and sharper voice, without thawing frozen `persona.rs` routing; align Grok Bot skills that maintain that contract.

**Architecture:** Persona *routing* stays frozen in `persona.rs`. Prompt *copy* lives in `ask.rs` (`contract_description`, `contract_character`, `system_prompt`, `degraded_reply`) and is pinned by unit tests. Skills document when/how to change that copy.

**Tech Stack:** Rust abbey-bot, Discord framing, loopback LLM (Ollama/mlx), Grok Bot `update_state` skills

## Global Constraints

- Keep `persona.rs` frozen (golden contracts / wyhash)
- Preserve `You are {persona}. ` prompt prefix
- Preserve Abbey `contract_character` U+2019 apostrophe (no ASCII `'`)
- Prefer loopback `ABBEY_BOT_LLM_ENDPOINT`; do not advertise remote Realtime
- No secrets in commits; brand: IWL = Abbey/ABI; Quesar private
- Update verbatim tests when changing degraded_reply

---

## File map

| File | Responsibility |
|------|----------------|
| `src/ask.rs` | System prompt assembly + degraded honesty copy + tests |
| Grok skill `abbey-system-prompt` | When/how to edit Discord persona prompts |
| Related skills | Cross-link local-only + Discord Abbey automation |

## Tasks

### Task 1: Tighten Abbey/Aviva/Abi contract copy

- [ ] Update `contract_description` for Abbey: warm/direct, no invented metrics, no Quesar-as-bot, local Discord companion
- [ ] Light Aviva/Abi clarifications only if needed (keep ABI invisible-unless-architecture)
- [ ] Keep Abbey character line starting with `I\u{2019}ll` and “lead with the answer” / “when I’m not sure”

### Task 2: Discord framing + degraded reply (local-first)

- [ ] Extend `system_prompt` framing: fail-closed tools; never invent numbers/metrics; only conversation + provided facts
- [ ] Rewrite `degraded_reply` to prioritize `ABBEY_BOT_LLM_ENDPOINT` (loopback OpenAI-compatible); Anthropic secondary or omitted if local-only policy
- [ ] Update `degraded_reply_is_the_verbatim_honesty_copy` and any pipeline pins

### Task 3: Verify

- [ ] `cargo test --locked ask::` (and engine prompt starts_with tests)
- [ ] Commit + PR

### Task 4: Skills

- [ ] Write Grok skill for Abbey system prompt maintenance
- [ ] Cross-link from Discord Abbey automation / local-only backends
