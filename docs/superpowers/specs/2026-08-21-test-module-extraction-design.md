# Test module extraction — and why this crate stays a single binary

**Date:** 2026-08-21
**Status:** Implemented
**Scope:** `src/vision.rs`, `src/brain/reward.rs`, `src/llm.rs`, `src/memory.rs`, `src/platform.rs`, `src/routing_signals.rs`, `src/wdbx.rs`, `src/generation/foundation_models.rs`

## What changed

Eight modules carried 300–639 lines of inline `#[cfg(test)] mod tests { … }`. Each block moved to a sibling `tests.rs`, declared from the parent exactly the way `voice_session.rs` already did it:

```rust
#[cfg(test)]
mod tests;
```

No test was added, removed, renamed, or rewritten. No production code changed. The suite is 612 tests before and after, and that count is the acceptance criterion — an inline test module in a file that stops being reachable disappears silently rather than failing, so "the gate is green" alone would not have proven this correct.

`src/pipeline.rs` and `src/voice_session.rs` already used this pattern; this change brings the other large modules in line with it rather than inventing anything.

Also removed: an empty, untracked, unreferenced `logs/` directory.

## Why the tests were moved verbatim, not dedented

The obvious implementation — strip four spaces from every line of the block — is wrong here, and the first attempt at it was reverted.

`wdbx.rs` (and `vision.rs`, `llm.rs`, `platform.rs`) contain multi-line string literals inside their test modules. Dedenting rewrites the *contents* of those strings, not just the code's indentation. In `wdbx/tests.rs` the affected literal is a WDBX v1 fixture whose exact bytes are the thing under test:

```
{\"type\":\"vector\",\"id\":1,\"values\":[0.5,-0.25,0.125]}\n\
```

The blocks were therefore moved byte-for-byte and re-indented by `cargo fmt`, which parses Rust and leaves string literals alone. Anyone repeating this work on another module must do the same.

## Rejected: extracting a library target

The alternative considered first was adding `src/lib.rs` with a thin `main.rs` binary on top, which would make `tests/` usable for integration tests. **It was rejected**, and the reasoning is recorded here so it does not get re-proposed:

1. **There is no customer.** The two test files this crate had already extracted — `pipeline/tests.rs` and `voice_session/tests.rs` — both open with `use super::*`, and `pipeline/tests.rs` additionally uses `super::testing::FakeOut`, a `#[cfg(test)]`-gated helper *inside* the module. An integration test under `tests/` could never reach either. A lib target would not let these move out; it would add a target nothing consumes.

2. **It would disable a lint this codebase uses deliberately.** `CLAUDE.md` records that dead-code lints under `-D warnings` have twice forced a `pub` item to either become load-bearing or be honestly marked `#[cfg(test)]`, and that both outcomes were better code. Under a `[lib]` target, `pub` items become public API and stop firing that lint.

3. **The test story already works.** 612 tests run with no network, no key, and no environment variables. "Integration tests are structurally impossible" is only a defect if something wants to be one, and nothing does.

The file-size symptom that motivated the idea is real, but its cause is inline tests, not module boundaries — which is what this change addresses directly.

## Also rejected: deduplicating `CLAUDE.md` and `AGENTS.md`

The two files are byte-identical except their H1, which reads as an obvious defect. It is not. `CLAUDE.md` states the rule explicitly:

> `AGENTS.md` is a verbatim mirror of this file for non-Claude agents — only the header line differs. Apply any edit to both, or they drift.

Replacing either with a pointer or a symlink breaks a documented decision. Leave them mirrored, and keep applying edits to both.

## Verification

`./check.sh` — fmt, deploy/lock validation, `scripts/check-privacy.py`, plist lint, `clippy --all-targets --locked -D warnings`, `cargo test --locked`, and the locked release build. **612 passed, 0 failed, 2 ignored**, matching the pre-change count exactly.
