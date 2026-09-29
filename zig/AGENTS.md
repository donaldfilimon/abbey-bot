# AGENTS.md

Canonical agent guidance for `abbey-bot-zig`, which lives in the `zig/`
subdirectory of the abbey-bot repository (`~/dev/active/abbey-bot/zig`).
`CLAUDE.md` is a pointer to this file; when they disagree, this file wins
inside `zig/`. The repository root's `AGENTS.md` governs everything outside
`zig/`.

## What this is

A ground-up, stdlib-only Zig rewrite of the Rust Discord bot at the root of
this same repository. The **oracle** is that bot pinned at commit `281ee3b`
(see `contracts/ORACLE.md`), not the Rust working tree beside `zig/`. Phase 1 is text: gateway, REST, command catalog,
persona routing, grounding and memory gates, WDBX memory through the `abi`
subprocess, moderation, help, managed-service readiness. Voice, music and
image attachments are Proposed (see `docs/claims.md`) and are never stubbed.

Decisions already made (2026-09-21, Donald), not to reopen:

1. `abi` is consumed only as a subprocess (`abi complete`, `abi wdbx`), never linked.
2. STM/LTM live in a Zig-native append-only JSONL store; vectors go to WDBX via `abi wdbx`. No SQLite.
3. Voice is Proposed, never stubbed.

## Toolchain and gate

- Zig master, pinned in `build.zig.zon` as `.minimum_zig_version =
  "0.17.0-dev.2320+1e770dbef"` (same as `~/dev/active/cell-lang`). The gate
  refuses any other `zig version`.
- **Never write std calls from memory.** Read the signature under
  `zig env` -> `std_dir` first and cite the file in a comment the first time an
  API is used. Anything unverifiable gets `// NOTE(SDK): unverified against
  0.17-dev.2251` plus a failing test.
- Stdlib only: no `build.zig.zon` dependencies, no `@cImport`, no C sources.
- **`tools/check.sh` is the gate** and its exit code is the verdict. It runs:
  toolchain pin, `zig fmt --check`, `zig build`, `zig build test --summary all`
  (the test runner fails any test that leaks from `std.testing.allocator`),
  catalog parity against the oracle export, the vendored contract and WDBX
  fixture gates, oracle provenance (the vendored contracts against
  `281ee3b`), generated-table sync, claims sync, and a 1000-line size guard.
  Run it from anywhere: `zig/tools/check.sh` changes into `zig/` itself.
- Capture exit codes by redirect (`zig build > log 2>&1; echo EXIT:$?`), never
  through a pipe. A failed build leaves the previous binary in `zig-out/`.

## Claims honesty

`docs/claims.md` is generated from `docs/claims.json` by `tools/claims.py`.
A row is **Current** only when it names tests declared in `src/`; the gate
enforces that. A capability with no test is Proposed, whatever the code looks
like. Never flip a row without the test that proves it.

## Oracles are read-only

The Rust bot's oracle is commit `281ee3b` in this repository's history. Read it
with `git show 281ee3b:<path>`, or `git archive 281ee3b` from the repository
root into a scratch directory and build there. Never compare against, or build
in, the live Rust tree at the repository root: it moves on and is not a
contract. Zig work stays under `zig/`; a change outside `zig/` (for example a
root gate exclusion) follows the root `AGENTS.md`, in its own commit. Never
run `git checkout` in this checkout (root `AGENTS.md`); use `git show`.

`~/dev/active/{abbey,abi,wdbx,cell-lang}` belong to other sessions. Never edit,
stage, checkout, stash, or clean build output in them. The WDBX fixture stage
reads `../../wdbx` (the sibling of abbey-bot) or `ABBEY_WDBX_REPO`.

The oracle's managed service (`com.donaldfilimon.abbey-bot`) is **live**. Never
stop or unload it, never read or source its `.env` or `~/.config/abbey-bot/env`,
never use its token. This bot's paths and launchd label are distinct
(`abbey-bot-zig`), and `serve` refuses to run with a real token while the
oracle's service is loaded.

## Layout

- `src/main.zig`: entry and argument dispatch only (at most 200 lines).
- `src/root.zig`: library root; `refAllDecls` pulls in every module's tests.
- One directory per subsystem under `src/` (`text/`, `persona/`, ...).
- `contracts/`: oracle material and goldens. Byte-identical where noted in
  `contracts/ORACLE.md`; regenerating a golden is a decision, not a fix.

## Machine git policy

This block restates `~/.claude/CLAUDE.md` "Git discipline" (added 2026-08-27).
The rewrite prompt asked for the block to be copied from
`~/dev/active/cell-lang`; as of 2026-09-21 cell-lang's `AGENTS.md` carries no
separately named block, so the text below is taken from the charter it defers to.

- Work in this canonical checkout on `main`. Never create a clone, copy or
  scratch checkout of this project.
- Branches and worktrees are the exception: only when the task genuinely needs
  isolation, the repository mandates a review flow, or Donald asks. A worktree
  or topic branch is not finished until merged into `main`, removed, and its
  branch deleted, before pushing and before calling the task done.
- Never leave unique work only in a worktree, and never under `/tmp`.
- Before removing any worktree, check for activity by others: a fresh commit
  or a branch dated today means another session owns it.
- Since 2026-09-28 this tree is part of the abbey-bot repository (folded from
  the local-only `~/dev/active/abbey-bot-zig`, created 2026-09-21 with
  `git init -b main`, with `git subtree add`; its history is kept). It shares
  abbey-bot's `main` and its GitHub remote; pushing is the operator's call.
  The pre-fold repository is bundled at
  `~/at-risk-bundles/2026-09-28-consolidation/abbey-bot-zig.bundle`.
- CI for this tree is `.github/workflows/zig.yml` at the repository root
  (GitHub reads workflows only there).

No em dashes in source comments, docs, or commit messages written for this repo.
