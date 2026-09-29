# CLAUDE.md

`AGENTS.md` (in `zig/`) is canonical for this subdirectory and wins on any
conflict inside `zig/`; the repository root's `AGENTS.md` governs the rest. Read
`zig/AGENTS.md` first: the Zig pin, the gate (`tools/check.sh`), claims honesty, the read-only
oracles, and the machine git policy (copied there).

The one-line version: read std from `zig env` std_dir before using it, run
`tools/check.sh`, and never flip a `docs/claims.json` row without a test.
