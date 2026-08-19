# Local model benchmark — 2026-08-19

ollama 127.0.0.1:11434 on this Mac (Apple silicon), Abbey's real system prompt
(`ask::system_prompt(Abbey)` + "User standing: 0.50"), `max_tokens` 4096, three
prompts, one request at a time, the bot idle. Columns: wall seconds · reply
chars · hidden `reasoning` chars · completion tokens. First row of each model
includes cold load.

| model | hey abbey | poise/GuildChannel question | frustrated message | verdict |
|---|---|---|---|---|
| **gpt-oss:20b** | 9.8 s · 25 · 136 · 46 | 25.1 s · 643 · 3069 · 765 | 7.1 s · 754 · 124 · 209 | **default** — fastest, light reasoning, native tool calling (needed for sub-project 2); replies a little long (tidy_reply trims) and the greeting is generic |
| gemma4:e4b | 36.9 s (load) · 15 · 503 · 125 | 13.0 s · 316 · 1590 · 412 | 20.5 s · 358 · 1511 · 420 | runner-up — best register and length, ~2× slower than gpt-oss |
| gemma4:12b | 32.5 s · 101 · 1540 · 455 | 93.5 s · 581 · 3855 · 960 | 74.6 s · 491 · 3010 · 843 | too slow: 1.5–3.9k chars of reasoning per turn |
| qwen3.5:latest | 86.8 s · 113 · 6765 · 1663 | 114.6 s · 388 · 11076 · 2316 | 46.6 s · 485 · 3528 · 956 | reasoning runaway |
| ornith:9b | 88.5 s · 55 · 6861 · 1783 | 82.9 s · 352 · 7423 · 1746 | 181.6 s · 527 · 14177 · 3760 | reasoning runaway |
| gemma4:26b | — | — | — | wedged the runner earlier today (HTTP 000); not re-measured |

Not measured: cloud-tagged ollama models (`abbey:cloud`, `gpt-oss:120b-cloud`,
`kimi-k3:cloud`, `glm-5.2:cloud`) — they need an ollama cloud account; the
`abbey:cloud` name returned "model not found".

Recommendation: `ABBEY_BOT_LLM_MODEL=gpt-oss:20b`. With streaming (post after
60 chars / 4 s, edit every 2 s) the user sees text within ~5 s on every model
above; the table is about when the *full* answer lands.
