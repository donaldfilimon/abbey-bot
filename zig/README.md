# abbey-bot-zig

A stdlib-only Zig 0.17-dev rewrite of the Abbey Discord bot, living in the
`zig/` subdirectory of the Rust bot's repository. The Rust bot at commit
`281ee3b` is the oracle; its surfaces are frozen in `contracts/` (provenance
in `contracts/ORACLE.md`).

What works today is exactly what `docs/claims.md` marks Current, each row with
the tests that prove it. Run the gate with `zig/tools/check.sh`.

Guidance for agents: `AGENTS.md`.
