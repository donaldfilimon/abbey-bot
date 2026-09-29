# abbey-bot-zig

A stdlib-only Zig 0.17-dev rewrite of the Abbey Discord bot. The Rust bot in
`~/dev/active/abbey-bot` is the oracle; its surfaces are frozen in
`contracts/` (provenance in `contracts/ORACLE.md`).

What works today is exactly what `docs/claims.md` marks Current, each row with
the tests that prove it. Run the gate with `tools/check.sh`.

Guidance for agents: `AGENTS.md`.
