# CLAUDE.md

`AGENTS.md` is canonical for this repository and wins on any conflict. Read it
first: the Zig pin, the gate (`tools/check.sh`), claims honesty, the read-only
oracles, and the machine git policy (copied there).

The one-line version: read std from `zig env` std_dir before using it, run
`tools/check.sh`, and never flip a `docs/claims.json` row without a test.
