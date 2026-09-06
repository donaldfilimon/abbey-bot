# Server plan regression fixes

## Requirements

- Post-apply verification must fail when a fresh diff contains blockers, even if it contains no changes, and must print the blocker details with recovery guidance.
- Reveal must distinguish leaving a topic unchanged, clearing it, and setting it. Clearing must reach the Discord writer, converge after apply, and a parent-only move must preserve an already-correct topic.
- Limit changes to `src/server/` and this report.

## Changes

- Added `TopicEdit::{Unchanged, Clear, Set}` to preserve topic intent from `diff` through `Change`, resolved `Op`, the fake guild, and `DiscordWriter`. Serenity receives an empty topic string for `Clear`, which is Discord's channel-edit representation for removing a topic.
- Reveal now emits `Clear` when the plan omits a topic and the live channel has one, `Set` when a desired topic differs, and `Unchanged` when only the parent needs editing.
- Added `verify_applied`, the decision path used by `run`, requiring both no blockers and no remaining changes. Failure renders the full fresh report and tells the operator to resolve blockers or changes and rerun the dry run.
- Added a regression test that creates a blocker-bearing, zero-change report through `diff`, then exercises the CLI verification decision.
- Added an apply regression covering topic clear, parent-only topic preservation, and a repeated diff with no remaining changes.

## Validation

```text
$ CARGO_TARGET_DIR=/Users/donaldfilimon/dev/active/abbey-bot/target cargo test --locked server::
running 63 tests
test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured; 941 filtered out

$ cargo fmt --all -- --check
exit 0

$ CARGO_TARGET_DIR=/Users/donaldfilimon/dev/active/abbey-bot/target cargo check --locked
Finished `dev` profile [unoptimized + debuginfo]
```

## Self-review

- The verification success condition now matches `Report::is_clear()` and convergence; newly appearing permission, hierarchy, ambiguity, feature, or managed-role blockers cannot be accepted as success.
- The failure output includes preflight, changes, blockers, warnings, and manual steps from the fresh snapshot, so the operator sees the actionable cause rather than only a count.
- Topic state is explicit only for edits. Channel creation retains its natural `Option<String>` because omission there does not compete with an unchanged state.
- Clearing uses the same `EditChannel` request as parent movement and does not affect names, kinds, overwrites, or unrelated channel fields.
- No neighboring direct failures were found in the owned paths.

## Commit

`fix: fail closed when server plan verification drifts`

## Round 1 review follow-up

- Extracted the production `TopicEdit` to Serenity `EditChannel` translation into `edit_channel_builder`; `DiscordWriter` now calls that boundary directly.
- Added a network-free serialization regression over the real Serenity builder. It pins `Clear` to `"topic": ""`, `Set` to the exact supplied text, and `Unchanged` to an omitted `topic` field while all three retain the requested `parent_id`.
- Focused validation: `cargo test --locked server::discord` passed all 7 tests; `cargo fmt --all -- --check` exited 0. The already-reported full gate at `f5537eb` passed 1,002 tests with 2 ignored and completed the release build; it was intentionally not repeated for this local adapter-test follow-up.
