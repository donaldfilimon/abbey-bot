# Oracle provenance

Everything under `contracts/` was copied, read-only, from the Rust bot at
`~/dev/active/abbey-bot`, commit **`281ee3b4fe0abb436a890d91c8a6d9701c495231`**
(`281ee3b`, "Merge content/review-20260919: touch_session reuse, roleplay
persona persistence, ledger restoration"), on 2026-09-21.

The oracle checkout was never edited, checked out, stashed, or built. Reads
went through `git -C ~/dev/active/abbey-bot archive 281ee3b` into a private
scratch extraction; that extraction (not the oracle) was built with Rust
1.98.0 into a scratch `CARGO_TARGET_DIR`.

| Path here | Source at 281ee3b | How |
|---|---|---|
| `abbey/` (corpus + lock) | `contracts/abbey/` | `cp -R`; `diff -r` reported no difference |
| `fixtures/` | `tests/fixtures/` | `cp -R`; `diff -r` reported no difference |
| `catalog/command-payload.json` | output of the oracle's own ignored test `command_registration_tests::export_command_registration_payload` (`poise::builtins::create_application_commands`) | run in the scratch extraction with `ABBEY_COMMAND_PAYLOAD_OUTPUT`; mode changed to 0644, bytes unchanged |
| `golden/*.json` | output of `golden/zig_golden_dump.rs.txt`, a test module added to the **scratch extraction only** | see below |
| `../scripts/check-abbey-contracts.py`, `test-check-abbey-contracts.py` | `scripts/` | copied unchanged |
| `../scripts/check-wdbx-conformance.py`, `test-check-wdbx-conformance.py` | `scripts/` | one edit each: fixture path `tests/fixtures/` became `contracts/fixtures/` |
| `../.gitattributes` | `.gitattributes` | rewritten for this layout |

## Goldens

`golden/zig_golden_dump.rs.txt` was compiled into the scratch extraction as
`src/zig_golden_dump.rs` (one `#[cfg(test)] mod zig_golden_dump;` line added
to its `main.rs`) and run with `cargo +1.98.0 test --locked --bin abbey-bot
zig_golden_dump -- --ignored`. It calls the oracle's own functions, so every
golden string, f32 bit pattern and Unicode table is the oracle's output, not a
retyped transcription. `golden/inputs.json` is the routing input corpus:
every string literal in the oracle's persona, routing-signal, text and ask
tests (`golden/extract_inputs.py`) plus a fixed list of edge cases.

`golden/unicode.json` is Rust 1.98.0 `core::char` property data (Unicode
17.0.0) over every scalar value; `tools/gen_unicode_tables.py` renders it into
`src/text/unicode_tables.zig`, and the gate fails if the two drift.

## Empty-prompt `/roleplay`

The rewrite prompt describes an open bug where an empty-prompt `/roleplay`
created the session as Abbey. At 281ee3b the oracle's empty arm calls
`Engine::set_session_persona(scope, persona, now)` with the gate's persona
(Aviva) on scope `discord:<channel id>`, and `touch_session` inserts with that
persona, so the session is created as **Aviva with no transcript turn**. That
merge fixed it ("explicit empty-roleplay persona stick"). The Zig engine pins
the fixed behavior; see `docs/claims.md`, row "Conversation engine".
