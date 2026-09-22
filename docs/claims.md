# Claims

GENERATED from `docs/claims.json` by `tools/claims.py`; edit the JSON, then run
`python3 tools/claims.py`. The gate (`tools/check.sh`) fails when this file is stale,
when a Current/Partial row names no test, or when a named test is not declared in `src/`.

Status meanings: **Current** = implemented and exercised by the named tests;
**Partial** = implemented and tested with a stated gap; **Proposed** = not implemented
(or implemented without a test, which counts as not implemented); **Out-of-scope** =
deliberately not part of this rewrite.

Oracle: `abbey-bot 281ee3b4fe0abb436a890d91c8a6d9701c495231`. Phase in scope for this run: 1.

Totals: Current 0, Partial 0, Proposed 63, Out-of-scope 2.

## Transport

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| RFC 6455 WebSocket client framing (src/gateway/ws.zig) | 1 | Proposed | none |  |
| Discord gateway session: Identify to Ready, heartbeat with jitter, Resume on resumable close codes, reconnect | 1 | Proposed | none |  |
| zlib-stream transport compression | - | Out-of-scope | none | Deliberately OFF per the rewrite scope; the gateway URL requests no compression. |
| TLS 1.2/1.3 client for Discord and HTTPS providers (std.crypto.tls) | 1 | Proposed | none |  |
| REST client with per-route rate-limit buckets from response headers | 1 | Proposed | none |  |
| Global + optional home-guild slash command registration (bulk overwrite) | 1 | Proposed | none |  |
| Allowed-mentions policy: generated text never pings | 1 | Proposed | none |  |

## Command surface

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Frozen slash-command catalog: serialized payload equals the oracle export (26 top-level, 68 commands) | 1 | Proposed | none |  |
| /help private task home and section reference | 1 | Proposed | none |  |
| /persona route | 1 | Proposed | none |  |
| /persona ask (generation through the configured backend) | 1 | Proposed | none |  |
| /roleplay (Aviva lane, NSFW gate) | 1 | Proposed | none |  |
| /nsfw and /admin nsfw toggles | 1 | Proposed | none |  |
| /remember, /forget, /recall, /pending list\|confirm\|dismiss | 1 | Proposed | none |  |
| /reputation | 1 | Proposed | none |  |
| /modcall moderation recommendation | 1 | Proposed | none |  |
| /whois, Abbey: profile, /perms | 1 | Proposed | none |  |
| Ask Abbey message menu | 1 | Proposed | none |  |
| /summarize | 1 | Proposed | none |  |
| /stats | 1 | Proposed | none |  |
| /admin show\|persona\|learning\|cooldown\|act\|budget\|brain\|flush\|export\|reset | 1 | Proposed | none |  |
| /admin quarantine\|contradict\|resolve (memory review) | 1 | Proposed | none |  |
| /server blueprint\|create-channel\|rename-channel\|slowmode\|delete-channel\|assign-role\|remove-role\|move-member\|purge | 1 | Proposed | none |  |
| /webhook guide | 1 | Proposed | none |  |
| /forum draft\|post\|perms | 1 | Proposed | none |  |
| /admin dashboard (classic administration dashboard) | 2 | Proposed | none | Phase 2. |
| Memory browser (Browse facts, Abbey: memory menu) | 2 | Proposed | none | Phase 2. |

## Persona and conversation

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Canonical ABI persona routing (explicit selector, keyword weights, f32 prior, tie order) | 1 | Proposed | none |  |
| Routing signals over the neutral prior (distress, confusion, terse urgency) | 1 | Proposed | none |  |
| Persona system prompts and honesty copy (byte-exact) | 1 | Proposed | none |  |
| Reply tidy (echo strip, headings, blank runs, 1,900-char sentence cut) | 1 | Proposed | none |  |
| Roleplay admission gate | 1 | Proposed | none |  |
| Conversation engine: per-scope sessions, persona switch keeps transcript, trimming, empty /roleplay stick | 1 | Proposed | none |  |
| Grounding check and hedging of unsupported specifics | 1 | Proposed | none |  |
| Unicode text semantics equal to Rust core::char (normalize, lowercase, whitespace) | 1 | Proposed | none |  |
| Adaptive learning loop (DQN stay/reply/react, rewards, budgets) | 2 | Proposed | none | Not in the phase-1 list. |
| Rolling channel summaries | 2 | Proposed | none | Not in the phase-1 list. |
| Model tools (remember_fact, lookup_reputation, recall, switch_persona, recent_messages, inspect_status, list_facts) | 2 | Proposed | none | Not in the phase-1 list. |

## Memory

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Fact validation and memory bank semantics (300-char facts, 100 facts, pending supersessions) | 1 | Proposed | none |  |
| Fact relevance selection for prompt context (recall ranking) | 1 | Proposed | none |  |
| Zig-native append-only JSONL episodic store (STM/LTM) | 1 | Proposed | none |  |
| WDBX vector memory through the abi wdbx subprocess | 1 | Proposed | none |  |
| Memory gate: episode-gated /remember, /forget, /pending confirm | 1 | Proposed | none |  |
| Episode gate client: content-free proposal and memory_candidate writes via abi wdbx episode propose | 1 | Proposed | none |  |
| Checkpoint gate against the frozen corpus | 1 | Proposed | none |  |
| Abbey contract corpus guard (scripts/check-abbey-contracts.py) | 1 | Proposed | none |  |
| WDBX v1 projection fixture parity with ../wdbx (scripts/check-wdbx-conformance.py) | 1 | Proposed | none |  |

## Generation backends

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| OpenAI-compatible local endpoint (loopback HTTP, remote HTTPS) | 1 | Proposed | none |  |
| Local-first provider order (local endpoint, then OpenAI-compatible HTTPS) | 1 | Proposed | none |  |
| Anthropic Messages API backend | - | Out-of-scope | none | The rewrite's provider order is local endpoint then OpenAI-compatible HTTP. |
| Apple Foundation Models secondary (fm serve / fm respond) | 2 | Proposed | none | Not in the phase-1 list. |
| Provider self-test and qualification manifest | 2 | Proposed | none | Not in the phase-1 list. |

## Managed service

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| serve subcommand with the .env.example environment contract | 1 | Proposed | none |  |
| Readiness and bootstrap documents in service-protocol-v1 format | 1 | Proposed | none |  |
| launchd plist under deploy/ (distinct label, never the live bot's) | 1 | Proposed | none |  |
| Refusal to run against a live token while the managed Rust service is loaded | 1 | Proposed | none |  |
| Typed operational event log with rotation | 2 | Proposed | none | Not in the phase-1 list. |
| Live test-guild acceptance: connects, answers /help and one persona reply | 1 | Proposed | none | Pending: needs a test-guild bot token from Donald; the production token is never used. |

## Voice, music and media

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Consented voice (Songbird/DAVE, STT/TTS, /voice consent\|join\|resume\|leave\|status\|diagnostics\|mode\|verify) | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |
| Music mirroring (/voice play\|pause\|resume-music\|stop-music\|volume) | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |
| macOS audio-tap sidecar | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |
| Image attachments (/see, /ocr, describe/read image menus) | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |

## Other adapters

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Telegram long-poll adapter | 2 | Proposed | none | Not in the phase-1 list. |
| Slack Socket Mode adapter | 2 | Proposed | none | Not in the phase-1 list. |
| Server blueprint CLI (--server-plan) | 2 | Proposed | none | Not in the phase-1 list. |

