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

## Oracle defect found by the goldens

`grounding::classify` in the oracle calls `rest.split_at(rest.len() - 1)` on a
token that starts with an ASCII digit. When the token's final scalar is
multi-byte (for example `12é`), that index is not a char boundary and Rust
panics. Any model reply or grounding source containing such a token panics the
grounding check. `golden/grounding.json` records those rows as `"panic": true`;
the Zig port treats such a tail as "not a suffixed statistic" and pins it by
test. This is a divergence by necessity, not a behavior change anyone relied on.

## abi CLI quirk measured 2026-09-22

The installed `abi` at `~/.local/libexec/abbey-bot/abi` (the one the live Rust
bot's episode gateway uses; run read-only here with HOME and TMPDIR set to a
private temp directory) writes the JSON object of `abi wdbx query ... --json`
to **stderr** when stdout is not a terminal. `src/memory/wdbx_bridge.zig`
accepts the object from either stream. This is an abi defect to fix in the abi
repository (read-only for this rewrite), not a Zig behavior.

## deploy/ copies (2026-09-22)

Copied from the oracle's `deploy/` at 281ee3b:

- `service-protocol-v1.json`: byte-identical, and the gate compares it by content through the Python suites.
- `service_protocol.py`, `service_installation.py`, `service_readiness.py`, `service_status.py`, `check-service-readiness.py`, `service-status.py`, and `test-service-{protocol,installation,readiness,status}.py`.

The only edits are mechanical identity substitutions, made with `perl -pi -e 's/abbey-bot(?![-\w])/abbey-bot-zig/g; s#tests/fixtures/service-protocol#contracts/fixtures/service-protocol#g'`. After them:

- the component directory is `~/.local/share/abbey-bot-zig`
- the binary is `~/.local/libexec/abbey-bot-zig/abbey-bot-zig`
- the launchd label is `com.donaldfilimon.abbey-bot-zig`
- the fixtures are read from `contracts/fixtures/service-protocol`

No logic changed. `diff` against the oracle shows only those lines. The rewrite never writes the live bot's `~/.local/share/abbey-bot`, and its plist can never carry the live label: `tools/check_managed_service.py` asserts that the validator rejects the live label.

`deploy/com.donaldfilimon.abbey-bot-zig.plist` is the oracle's plist with the same substitution. `EnvironmentVariables.RUST_LOG` is dropped because it has no Zig consumer.

Not copied:

- The installers (`install-*.sh`), because installing is Donald's call.
- `service_transaction.py` and `service_environment.py`, because they belong to the installer.
- The audio, MLX and provider-qualification scripts, because they cover Proposed rows.

`scripts/check-privacy.py` is not copied. It parses Rust logging macros (`tracing::`, `println!` and the like), so on Zig source it would match nothing and pass vacuously. In the rewrite the equivalent property is structural: `serve` prints only fixed diagnostics that name a variable and never its value. The credential tests pin those diagnostics. A Zig-aware privacy scanner is future work, and no claim rests on it.
