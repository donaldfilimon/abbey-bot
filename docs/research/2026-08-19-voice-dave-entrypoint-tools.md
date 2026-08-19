# Research — Discord voice/DAVE, Entry Point command, local tool calling (2026-08-19)

Deep-research run (5 search angles → 15 sources → 3-vote adversarial verification per claim; claims needing ≥2/3 refutes to die). Only claims that survived are listed, with confidence.

## 1. Discord voice in 2026 — settled

- **DAVE (MLS-based E2EE) is mandatory for every non-Stage voice/video participant since early March 2026 (dev docs: March 1; support pages/field reports: March 2); bots have no exemption.** HIGH, 3–0 ×6. Sources: https://docs.discord.com/developers/topics/voice-connections · https://support.discord.com/hc/en-us/articles/38749827197591 · https://discord.com/blog/every-voice-and-video-call-on-discord-is-now-end-to-end-encrypted
- **Transport-only voice no longer connects**: Identify with `max_dave_protocol_version` 0/omitted → close code 4017 "E2EE/DAVE protocol required". HIGH, 3–0 ×3. Sources: opcodes doc; songbird issue #293 (2026-03-07); Pycord #3135.
- **Mechanics**: `max_dave_protocol_version` in voice Opcode 0 Identify; gateway picks `dave_protocol_version` in Opcode 4; versions 0 (transport-only) and 1 (DAVE); opcodes 21–31 carry the MLS exchange. HIGH. Sources: voice docs; https://daveprotocol.com/
- **Transport encryption to the SFU still applies**: `aead_xchacha20_poly1305_rtpsize` required, `aead_aes256_gcm_rtpsize` preferred; XSalsa20 and non-rtpsize modes discontinued 2024-11-18. HIGH, 3–0.
- **Rust path exists**: songbird **v0.6.0 "Hoopoe"** (2026-04-05, PR #291, closes #293) adds DAVE via the pure-Rust **`davey`** crate (Snazzah; 0.1.4 as of 2026-06-22; optional dep on by default through the `driver` feature), bumps serenity-voice-model to 0.3, MSRV 1.83, opus2. songbird 0.5 bots fail at `join()` with "E2EE/DAVE protocol required". HIGH, 3–0 ×3 (verified on crates.io + Cargo.toml).

**What this means for abbey-bot:** voice stays out of scope (no `voice.md` was ever supplied), but the spec's open decisions #4/#6/#7/#8 (`docs/spec/SKILL.md`) are now answerable — DAVE is required, XChaCha20 is the mandatory transport mode, and a future voice sub-project would build on serenity 0.12 + songbird 0.6 + davey, not on hand-rolled RTP/AES-GCM as `voice.md` planned.

## 2. Entry Point bulk-overwrite rule — no surviving third-party claims

No claim passed verification. **Our own live evidence stands** (2026-08-19): a bulk `PUT /applications/{id}/commands` omitting the app's `PRIMARY_ENTRY_POINT` command is rejected with "You cannot remove this app's Entry Point command in a bulk update operation"; re-sending it alongside ours (with its `handler`, `integration_types`, `contexts`) succeeds. Deleting it would disable the app's Activity — we preserve it (`main.rs::register_globally_keeping_entry_point`).

## 3. ollama `/v1` tool calling and Anthropic streaming — no surviving third-party claims

No claim passed verification. **Our own measurements stand**: gpt-oss:20b via ollama `/v1` returns `tool_calls` non-streamed and streams each call whole in one `delta.tool_calls[]` entry with full `arguments`; `think`/`reasoning_effort` on `/v1` returned empty bodies in today's probe (not honoured usably); the accumulator merges by index so OpenAI's fragmented-arguments shape is covered too. Anthropic `tool_use`/`tool_result` shapes are implemented from the public API reference and pinned by recording-transport tests only (no key).
