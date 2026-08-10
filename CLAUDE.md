# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`README.md` is written for a human running the bot — commands, env vars, and the
per-feature design notes. Read it first. This file holds what the README leaves
out: the shape of the codebase, the rules that keep it that shape, and the traps
this project has already hit.

## Commands

```bash
./check.sh          # the gate: fmt --check, clippy --all-targets -D warnings, test
cargo test <name>   # single test, substring-matched against the full path
cargo run           # needs DISCORD_TOKEN; see README
```

`cargo test moderation::` runs one module's tests (10 of 48). There is no `-p`
and no `--workspace` — this is a single binary crate. Note that means test
targets are inside the `bin`, so `--lib` matches nothing.

## Architecture: a pure core with a thin Discord shell

Five modules hold every decision the bot makes, and **none of them import
serenity or poise**:

| Module | Decides |
|---|---|
| `persona.rs` | Which of Abbey/Aviva/Abi answers a request, and why |
| `profile.rs` | How a member's profile reads as one paragraph |
| `perms.rs` | How a channel's overwrites resolve, in Discord's evaluation order |
| `moderation.rs` | Which action an incident warrants, given severity and history |
| `server.rs` | Role hierarchy, channel structure, and setup steps per archetype |

`commands.rs` is the *only* file that touches Discord types, and its job is
translation: fetch over REST, build the plain struct, hand it to a pure function,
post the string back. `main.rs` is just env parsing and framework wiring.

**This split is the reason 48 unit tests exist with no gateway connection**, and
it is load-bearing rather than stylistic. Putting decision logic into a command
body makes it untestable in this repository, because there is no test harness for
a Discord interaction here and no token to build one with. If you find yourself
writing an `if` inside a `#[poise::command]` function that isn't about fetching
data, it belongs in a pure module.

### Adding a command

1. Put the logic in a pure module with tests.
2. If it needs a dropdown, add a `…Choice` enum **in `commands.rs`** with
   `#[derive(poise::ChoiceParameter)]` plus a `From` impl to the pure type. See
   `SeverityChoice` and `ArchetypeChoice`. The mirror exists so the pure module
   never gains a poise dependency — that dependency is what would make it
   un-unit-testable.
3. Write the thin command; **defer first** (see below).
4. Register it in the `commands: vec![…]` list in `main.rs`. Forgetting this
   compiles fine and silently ships nothing.

## Rules that are not preferences

**Defer before touching the network, unconditionally.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. Every command here calls `ctx.defer()` or
`ctx.defer_ephemeral()` first — including `/persona`, which makes no network call
at all. A command that defers only when it looks slow is a command that races
eventually.

**Intents stay `non_privileged()`.** Nothing reads message content, presence, or
the member list off the gateway; `/whois`, `/perms`, and `/modcall` fetch over
REST instead. This is why the bot deploys without requesting privileged intents,
and why `profile::summarize` *states* that presence is unavailable rather than
printing a status it cannot observe. Adding a privileged intent means enabling it
in the Dev Portal **and** adding it here — both, or the gateway silently sends
nothing.

**Commands that recommend do not act.** `/modcall` never times out, kicks, or
bans; `/server` creates no roles or channels. Both are ephemeral. Keep it that
way unless the user explicitly asks for an acting command — a moderation
recommendation posted publicly is an accusation, and structural changes need
someone who can see the existing server.

**Fetch over REST, not from the cache.** The cache is only as complete as the
intents held, so a cache read produces a silently thinner answer instead of an
error.

## Traps this repository has already hit

**`Permissions` does not `Debug` into flag names.** It prints `Permissions(3072)`
— a raw bitfield. An early version derived permission names by scraping that and
would have rendered numbers into chat. Use `get_permission_names()`, which
returns the client-facing strings (`"View Channel"`, `"Ban Members"`). Two tests
pin the strings this codebase hardcodes against that vocabulary, because a typo
there fails silently — `/modcall` would tell every moderator they cannot act.

**Dead-code lints: this is a binary crate, so `pub` exempts nothing.** Clippy
runs with `-D warnings`, and a `pub` constant used only by tests is an error.
This has come up twice, and the resolution both times was one of two honest
moves, never `#[allow]`:

- *Make it load-bearing* if it deserves to be. `MAX_TIMEOUT_MINUTES` became a
  clamp every `Action::Timeout` is built through; `normalize_text_name` now runs
  inside `server::render`. Both are better code than before the lint fired.
- *Mark it `#[cfg(test)]`* if it is genuinely a specification constant the
  property tests enforce — `Archetype::ALL`, `NEVER_FOR_EVERYONE`,
  `Action::severity_rank`.

**Discord rewrites text channel names and leaves voice names alone.** Text and
forum names are lowercased with whitespace hyphenated, so a blueprint saying
`General Chat` describes a server you do not get; `Squad 1` is a perfectly legal
voice channel. `server::render` normalizes per channel kind, and a test asserts
the voice exemption is actually exercised so the asymmetry cannot rot into an
untested claim.

**A passing property test is not evidence that output reads well.** The `/server`
blueprints passed every invariant while rendering `🗂help` with the glyph jammed
against the name. Print the rendered string and read it before shipping anything
a user sees.

## What has never been verified

**No part of this bot has ever connected to Discord.** No gateway handshake, no
command registration, no interaction answered. The gate proves the decision logic
and the startup path; the binary is confirmed to start and fail fast on a missing
`DISCORD_TOKEN` or a non-numeric `ABBEY_GUILD_ID`. That is the whole of the
evidence — do not describe the bot as working, tested against Discord, or
deployed. A live run needs a token, and creating one is Dev Portal
authentication that belongs to the user.

## Related, and easy to confuse

`~/dev/archive/swift-discord` is a home-grown **Swift** Discord library with its
own gateway and REST targets. It shares no code with this crate and is not a
dependency, a port source, or a reference implementation. The `discord-abbey`
skill's reference files describe a Swift/Vapor/DiscordBM bot at
`~/Desktop/AbbeyBot`, which does not exist on this machine; the skill's
non-code guidance (capability map, output format rules, persona registers) is
what this project actually implements, and those reference files are historical.
