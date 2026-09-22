# Claims

GENERATED from `docs/claims.json` by `tools/claims.py`; edit the JSON, then run
`python3 tools/claims.py`. The gate (`tools/check.sh`) fails when this file is stale,
when a Current/Partial row names no test, or when a named test is not declared in `src/`.

Status meanings: **Current** = implemented and exercised by the named tests;
**Partial** = implemented and tested with a stated gap; **Proposed** = not implemented
(or implemented without a test, which counts as not implemented); **Out-of-scope** =
deliberately not part of this rewrite.

Oracle: `abbey-bot 281ee3b4fe0abb436a890d91c8a6d9701c495231`. Phase in scope for this run: 1.

Totals: Current 31, Partial 6, Proposed 27, Out-of-scope 2.

## Transport

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| RFC 6455 WebSocket client framing (src/gateway/ws.zig) | 1 | Current | `RFC 6455 1.3: the sample key yields the sample accept value`<br>`RFC 6455 5.7: single-frame masked and unmasked text encode to the sample bytes`<br>`RFC 6455 5.7: 256-byte and 64 KiB binary frames use 16- and 64-bit lengths`<br>`RFC 6455 5.7: a fragmented unmasked text message reassembles to Hello`<br>`a ping between fragments is answered with a masked pong carrying the same payload`<br>`server close frames surface their code and are echoed once`<br>`protocol violations are rejected`<br>`messages over the configured maximum are refused before allocation`<br>`handshake sends the upgrade request and validates the 101 response` | Also observed live against gateway.discord.gg (docs/evidence/2026-09-22-gateway-probe.md). |
| Discord gateway session: Identify to Ready, heartbeat with jitter, Resume on resumable close codes, reconnect | 1 | Partial | `identify after Hello, heartbeat after interval times jitter, then every interval`<br>`a missed heartbeat ACK reconnects, resuming when a session exists`<br>`close codes follow the oracle transport's table`<br>`resume after reconnect sends Resume on Hello; invalid session decides the path`<br>`payloads are the documented JSON shapes`<br>`gateway URLs parse to host and port; plain ws is loopback-only`<br>`offline gateway run: identify, heartbeat, register, answer /help and a persona reply` | State machine follows serenity 0.12.5's shard.rs; the runtime (timer-thread heartbeats, zombie detection, dispatch to workers) runs end to end offline against std.http.Server's WebSocket server. Gap: a live Identify/Ready/Resume needs a test-guild bot token (pending, Donald). |
| zlib-stream transport compression | - | Out-of-scope | none | Deliberately OFF per the rewrite scope; the gateway URL requests no compression. |
| TLS 1.2/1.3 client for Discord and HTTPS providers (std.crypto.tls) | 1 | Current | `tls loopback: TLS 1.3 with an ECDSA P-256 certificate verifies and carries HTTP`<br>`tls loopback: TLS 1.2 with an RSA-2048 certificate verifies and carries HTTP`<br>`tls loopback: a certificate for another host is refused` | std.crypto.tls.Client through std.http.Client; the loopback server is openssl s_server (std has no TLS server). Live HTTPS and WSS to Discord observed 2026-09-22 with the system CA bundle (docs/evidence/2026-09-22-gateway-probe.md). |
| REST client with per-route rate-limit buckets from response headers | 1 | Current | `route keys keep major parameters and mask the rest`<br>`reset-after seconds parse to milliseconds, rounding up`<br>`remaining zero waits until the bucket resets and shared buckets share budgets`<br>`a 429 sets the retry deadline; a global 429 pauses every route`<br>`rest client authenticates, follows buckets and retries a 429 after retry_after`<br>`persistent 429s stop after the retry budget` | Exercised against a loopback std.http.Server; live Discord REST calls beyond the unauthenticated GET /gateway need a test-guild token (pending). |
| Global + optional home-guild slash command registration (bulk overwrite) | 1 | Current | `registration is a subset of the frozen surface and the rest is claimed Proposed`<br>`offline gateway run: identify, heartbeat, register, answer /help and a persona reply` | Bulk overwrite of the handled projection only; the home-guild PUT follows the same code path after the global one. |
| Allowed-mentions policy: generated text never pings | 1 | Current | `edit bodies never ping and carry the Abbey embed`<br>`offline gateway run: identify, heartbeat, register, answer /help and a persona reply` |  |

## Command surface

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Frozen slash-command catalog: serialized payload equals the oracle export (26 top-level, 68 commands) | 1 | Current | `catalog parity: the Zig registration payload is byte-identical to the oracle export`<br>`catalog parity: the compact request body parses to the same document`<br>`catalog specs match the oracle's 68 registered commands in order`<br>`every registered leaf maps to the payload with its contexts and default permission`<br>`availability golden: 72 inputs x 68 commands match the oracle`<br>`help golden: every section renders byte-identically for five permission shapes` | Gate also runs abbey-bot-zig catalog-json and cmp's it against contracts/catalog/command-payload.json. The full 26-command surface is frozen; registration of commands whose handlers are Proposed is decided in the serve row. |
| /help private task home and section reference | 1 | Partial | `handlers: /help renders the section reference with owner-bound controls`<br>`help controls: strict protocol and fixed expiry`<br>`help controls: malformed, unknown, overlong and future controls fail closed`<br>`offline gateway run: identify, heartbeat, register, answer /help and a persona reply` | Section reference and owner-bound section select are ported; the Start section's five private task workflows (Talk with Abbey, Review memory, Use an image, Voice & Music, Manage Abbey) are not, so the sentence advertising them is dropped. |
| /persona route | 1 | Current | `handlers: /persona route explains the composed route; /persona ask without a backend is the honesty copy` |  |
| /persona ask (generation through the configured backend) | 1 | Current | `handlers: /persona route explains the composed route; /persona ask without a backend is the honesty copy`<br>`handlers: /persona ask answers through the local endpoint, tidies, grounds and commits`<br>`offline gateway run: identify, heartbeat, register, answer /help and a persona reply` |  |
| /roleplay (Aviva lane, NSFW gate) | 1 | Current | `handlers: /roleplay refuses in SFW channels, sticks Aviva when empty in an enabled DM` |  |
| /nsfw and /admin nsfw toggles | 1 | Partial | `handlers: /roleplay refuses in SFW channels, sticks Aviva when empty in an enabled DM` | /nsfw (DM) is ported; /admin nsfw belongs to the unregistered /admin group. |
| /remember, /forget, /recall | 1 | Partial | `handlers: /remember, /recall and /forget keep memory self-scoped unless moderators act`<br>`memory gate: an appended candidate yields its receipt, and forget proposes a tombstone for it` | Gap: /recall has no Browse facts button (memory browser is Phase 2) and shows the neutral 0.50 standing because reputation signals come from the Phase-2 learning pipeline; /remember's replaces autocomplete is not wired. |
| /pending list\|confirm\|dismiss | 1 | Proposed | none | Memory service supports confirm/dismiss (tested there); the command with its confirm/dismiss buttons is not ported yet, so it is not registered. |
| /reputation | 1 | Proposed | none |  |
| /modcall moderation recommendation | 1 | Current | `handlers: /modcall recommends from live guild data and reports hierarchy blockers`<br>`severity outranks history completely and a first minor incident is only noted`<br>`serious incidents escalate through timeouts then ban, and more history is never lighter`<br>`hierarchy refusals and rendering match the oracle's copy`<br>`member permissions follow serenity: @everyone plus roles, all for owner or Administrator` |  |
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
| Canonical ABI persona routing (explicit selector, keyword weights, f32 prior, tie order) | 1 | Current | `routing golden: canonical weights, reasons, signals and describe match the oracle byte for byte`<br>`neutral prior and ties favor Abbey`<br>`leading names are exact overrides`<br>`stems match only token prefixes` | f32 bit patterns pinned for 212 corpus inputs; describe uses exact half-even {:.2}. |
| Routing signals over the neutral prior (distress, confusion, terse urgency) | 1 | Current | `routing golden: canonical weights, reasons, signals and describe match the oracle byte for byte`<br>`signals only decide over the neutral prior` |  |
| Persona system prompts and honesty copy (byte-exact) | 1 | Current | `prompt golden: system prompts, honesty copy, failures and roleplay messages are byte-exact` |  |
| Reply tidy (echo strip, headings, blank runs, 1,900-char sentence cut) | 1 | Current | `tidy golden: echo stripping, headings, blank runs and sentence cuts match the oracle`<br>`the prefix echo is stripped and short text is untouched` |  |
| Roleplay admission gate | 1 | Current | `behavior matrix matches the product gate`<br>`prompt golden: system prompts, honesty copy, failures and roleplay messages are byte-exact` |  |
| Conversation engine: per-scope sessions, persona switch keeps transcript, trimming, empty /roleplay stick | 1 | Current | `prepare appends context and records nothing`<br>`trimming by turn count keeps the most recent`<br>`trimming by char budget starts on a user turn`<br>`persona switch keeps the transcript`<br>`empty roleplay stick creates the session as Aviva with no turn and follow-ups read it`<br>`prepared grounding is a pre-candidate snapshot`<br>`reset forgets one scope and evict_idle drops only stale sessions`<br>`will_overflow measures prompt plus turns` | Oracle 281ee3b already fixed the empty /roleplay Abbey session (explicit persona stick); the Zig engine pins that fixed behavior, including that a neutral follow-up in the channel answers as Aviva. |
| Grounding check and hedging of unsupported specifics | 1 | Current | `grounding golden: specifics, verdicts and hedged replies match the oracle`<br>`prior user input grounds, assistant output does not`<br>`a multi-byte tail after digits is not a statistic and never crashes` | Deliberate divergence: the oracle panics (str::split_at off a char boundary) on a digit run followed by a multi-byte letter, e.g. 12é; 6 golden rows record the panic and Zig completes them instead. |
| Unicode text semantics equal to Rust core::char (normalize, lowercase, whitespace) | 1 | Current | `normalize matches the oracle's pinned cases`<br>`trim and collapse use Unicode whitespace`<br>`lowercase covers multi-scalar and final sigma`<br>`final sigma follows the oracle's Case_Ignorable and Cased sets`<br>`invalid utf-8 decodes as replacement without overrun`<br>`routing golden: canonical weights, reasons, signals and describe match the oracle byte for byte` | Tables generated from Rust 1.98.0 core::char (Unicode 17.0.0). |
| Adaptive learning loop (DQN stay/reply/react, rewards, budgets) | 2 | Proposed | none | Not in the phase-1 list. |
| Rolling channel summaries | 2 | Proposed | none | Not in the phase-1 list. |
| Model tools (remember_fact, lookup_reputation, recall, switch_persona, recent_messages, inspect_status, list_facts) | 2 | Proposed | none | Not in the phase-1 list. |

## Memory

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Fact validation and memory bank semantics (300-char facts, 100 facts, pending supersessions) | 1 | Current | `memory golden: persona context render, grounding sources and fact validation match the oracle`<br>`remember rejects duplicates and the cap; forget removes by exact text`<br>`pending supersessions are bounded and never touch facts`<br>`channel recent window is capped at 50 and counts every message` |  |
| Fact relevance selection for prompt context (recall ranking) | 1 | Current | `recall golden: selection order and omitted counts match the oracle`<br>`rare terms dominate and zero-score facts fill newest first` |  |
| Zig-native append-only JSONL episodic store (STM/LTM) | 1 | Current | `store appends durably, replays in order and never shares scopes`<br>`a torn final line is dropped and a corrupt middle line fails closed`<br>`a persistent service replays facts, settings and the projection after restart` | Decision 2. episodes.jsonl (0600, dir 0700, exclusive advisory lock, fsync per append); LTM facts/pending/receipts/settings and STM messages/summaries. No SQLite. |
| WDBX vector memory through the abi wdbx subprocess | 1 | Current | `bridge runs abi wdbx query with a scrubbed environment and keeps only the caller's scope`<br>`wdbx golden: embeddings match the oracle bit for bit and render as serde_json does`<br>`wdbx golden: segment renders, recall ranking and reconciliation match the oracle`<br>`the conformance fixture parses and re-renders byte-identically`<br>`std Wyhash reproduces the pinned Zig reference vectors`<br>`f32 text matches serde_json's ryu layout` | Facts project into the oracle's byte-compatible v1 JSONL segment; ranking runs through 'abi wdbx query' (never linked). The gate's wdbx-interop stage scores a Zig-written segment with the installed abi and requires abi's semantic scores to equal the Zig cosine to abi's printed precision (observed max diff 4.061e-7 on 2026-09-22); it prints SKIP when no abi binary is present. |
| Memory gate: episode-gated /remember, /forget, /pending confirm | 1 | Current | `memory gate: an unreachable gate stores nothing and says so; an uncovered scope is ungated`<br>`memory gate: an appended candidate yields its receipt, and forget proposes a tombstone for it`<br>`explicit replaces supersedes atomically and an absent target is refused without storing`<br>`a model proposal stores without removing anything; confirm and dismiss behave like the oracle`<br>`remember validates once, updates both representations and the projection never crosses scopes` | Propose-before-write for /remember and /forget with receipts keyed per fact, transcribed from memory_gate.rs and runtime/memory_service.rs. The model-tool write queue is Phase 2 with the tools. |
| Episode gate client: content-free proposal and memory_candidate writes via abi wdbx episode propose | 1 | Current | `episode golden: config validation, writes, edges, outcomes and principals match the oracle`<br>`episode fixtures: proposal, memory candidate and memory edge serialize exactly as the canonical types`<br>`propose runs abi with an owner-only write file, the configured flags and a scrubbed environment`<br>`a missing abi binary is unavailable, never an append` | Config, builders, serde-identical serialization and outcome classification replay oracle goldens (JSON-syntax errors keep the oracle's prefix; serde's line/column is not reproduced). Live proposal against the operator's gateway is not exercised: that gateway belongs to the running Rust bot. |
| Checkpoint gate against the frozen corpus | 1 | Partial | `checkpoints: unchanged rows are not re-proposed and changed rows supersede their receipt`<br>`checkpoints: a refused checkpoint falls back to the last admitted row or is dropped`<br>`checkpoints: the synchronous path persists only admitted rows and uncovered guilds are untouched`<br>`corpus verifier: the vendored Abbey corpus matches its pinned lock, inventory, digests and aggregate` | Gap: plan/settle/restrict are ported and tested as pure functions, but the rows they gate are DQN brain snapshots and the learning loop is Phase 2, so nothing produces checkpoints yet. The frozen-corpus side is the Zig corpus verifier. |
| Abbey contract corpus guard (scripts/check-abbey-contracts.py) | 1 | Current | `corpus verifier: the vendored Abbey corpus matches its pinned lock, inventory, digests and aggregate`<br>`corpus verifier: a modified, extra or missing artifact fails closed`<br>`layout matches serde_json and python indent=2` | Zig port of scripts/check-abbey-contracts.py (pins, inventory, per-artifact SHA-256, privacy taxonomy, aggregate digest over the Python indent=2 manifest render). The copied Python guard and its self-test also run in the gate. |
| WDBX v1 projection fixture parity with ../wdbx (scripts/check-wdbx-conformance.py) | 1 | Current | `the conformance fixture parses and re-renders byte-identically` | The gate also runs the oracle's own scripts/check-wdbx-conformance.py against ../wdbx's golden copy; that compares fixtures, and the Zig test adds parse/render identity for the same bytes. |

## Generation backends

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| OpenAI-compatible local endpoint (loopback HTTP, remote HTTPS) | 1 | Current | `endpoint validation mirrors the oracle: loopback-only primary, HTTPS-only remote fallback`<br>`extraction: stop with content answers; reasoning-only is a budget failure; others are backend failures`<br>`provider posts the persona prompt and transcript to the local endpoint and returns its answer` | Loopback-only primary exactly as the oracle validates it; request shape and extraction transcribe llm/dialect.rs and llm/protocol.rs. Non-streaming (one interaction post). |
| Local-first provider order (local endpoint, then OpenAI-compatible HTTPS) | 1 | Current | `a failed local endpoint falls through to the fallback tier with its bearer key`<br>`no configured tier yields null and a lone failing tier yields a backend failure` | Rewrite addition: ABBEY_BOT_LLM_FALLBACK_ENDPOINT/_MODEL/_KEY (HTTPS unless loopback) after the loopback primary; the oracle instead puts Anthropic first, which is Out-of-scope here. Read-only turns only, so a fallback never repeats a side effect. |
| Anthropic Messages API backend | - | Out-of-scope | none | The rewrite's provider order is local endpoint then OpenAI-compatible HTTP. |
| Apple Foundation Models secondary (fm serve / fm respond) | 2 | Proposed | none | Not in the phase-1 list. |
| Provider self-test and qualification manifest | 2 | Proposed | none | Not in the phase-1 list. |

## Managed service

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| serve subcommand with the .env.example environment contract | 1 | Partial | `token selection is source-aware and fail-closed`<br>`environment contract: blank values are unset and remote primaries are refused`<br>`serve arguments: only the sole --managed-service flag is accepted`<br>`serve refuses a blank primary token before touching the network`<br>`application owners come from the team when present, else the owner` | serve and the launchd --managed-service entry select the credential with the oracle's messages, refuse beside the live service, run the Discord preflight (GET /oauth2/applications/@me; 401 maps to the oracle's rejected diagnostic) and the gateway bot. Gap: the authenticated path has never run, because it needs Donald's test-guild token. The gate also runs the binary for missing and blank credentials (tools/check_managed_service.py). .env.example lists only keys the Zig bot reads; the gate fails on a listed key with no reader. |
| Readiness and bootstrap documents in service-protocol-v1 format | 1 | Current | `readiness bytes are the oracle's canonical v1 encoding`<br>`ready is refused until every checkpoint and state agrees`<br>`private directory is created 0700 and documents are published 0600 by rename`<br>`identity of the running test binary hashes the executable`<br>`readiness observation reaches ready only after READY and registration` | Published under ~/.local/share/abbey-bot-zig (0700 dir, 0600 files, temp plus rename), never the live bot's path. The gate parses Zig-written documents with the copied deploy/service_protocol.py read_private and checks executable_sha256 against the built binary. deploy/service-protocol-v1.json is byte-identical to the oracle's. |
| launchd plist under deploy/ (distinct label, never the live bot's) | 1 | Current | `the launchd plist runs this binary with the sole --managed-service argument under its own label` | deploy/com.donaldfilimon.abbey-bot-zig.plist is validated by the copied deploy/service_installation.validate_managed_plist in the gate, which also rejects the live label. It is not installed; installing it is Donald's call. |
| Refusal to run against a live token while the managed Rust service is loaded | 1 | Current | `serve refuses beside the live Rust service unless this is an explicit test-guild run` | Exact-label query (launchctl print gui/<uid>/com.donaldfilimon.abbey-bot), fail-closed when the query cannot run, checked before any network I/O. ABBEY_BOT_ZIG_TEST_GUILD=1 overrides it, for a separate test-guild token only. The gate runs the binary with a placeholder token while the live service is loaded and expects exit 3. |
| Typed operational event log with rotation | 2 | Proposed | none | Not in the phase-1 list. |
| Live test-guild acceptance: connects, answers /help and one persona reply | 1 | Proposed | none | Pending: needs a test-guild bot token from Donald; the production token is never used. |

## Voice, music and media

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Consented voice (Songbird/DAVE, STT/TTS, /voice consent\|join\|resume\|leave\|status\|diagnostics\|mode\|verify) | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |
| Music mirroring (/voice play\|pause\|resume-music\|stop-music\|volume) | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |
| macOS audio-tap sidecar | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |
| Image attachments (/see, /ocr, Abbey: describe image, Abbey: read image text) | later | Proposed | none | requires Opus + MLS; C-link decision pending (Donald) |

## Other adapters

| Capability | Phase | Status | Tests | Note |
|---|---|---|---|---|
| Telegram long-poll adapter | 2 | Proposed | none | Not in the phase-1 list. |
| Slack Socket Mode adapter | 2 | Proposed | none | Not in the phase-1 list. |
| Server blueprint CLI (--server-plan) | 2 | Proposed | none | Not in the phase-1 list. |

