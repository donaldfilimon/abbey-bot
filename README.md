# Abbey Bot

Abbey's Discord operational layer, in Rust — [serenity](https://github.com/serenity-rs/serenity)
0.12 with [poise](https://github.com/serenity-rs/poise) 0.6 for slash commands.

This is the Rust implementation. Prior Abbey bots existed in Bun/TypeScript, Zig,
and Swift; none of those are on this machine. The one adjacent thing that is —
`~/dev/archive/swift-discord`, a home-grown Swift Discord library — is unrelated
to this crate and shares no code with it.

## Commands

| Command | What it does |
|---|---|
| `/persona <request> [as]` | Shows which persona takes a request and why. `as` forces one. |
| `/whois <user>` | Profile read: identity, standing, roles, join date. |
| `/perms <channel> <user>` | Walks a channel's permission overwrites in Discord's evaluation order. |

## Running

```sh
export DISCORD_TOKEN=...        # from the Developer Portal; never commit it
export ABBEY_GUILD_ID=...       # optional, but see below
cargo run
```

`ABBEY_GUILD_ID` registers commands to a single guild, which takes effect
immediately. Leave it unset and commands register globally, where propagation can
take up to an hour — fine once the command set is stable, miserable while
iterating. See `.env.example`.

## Design notes

**Intents are `non_privileged()`.** Nothing here reads message content, presence,
or the member list off the gateway, so the bot deploys without requesting
privileged intents in the Dev Portal. `/whois` and `/perms` fetch member data over
REST instead.

The visible consequence: **`/whois` does not report online/idle/DND status.** That
needs `GUILD_PRESENCES`. Rather than print a status it cannot actually observe,
the summary says presence is unavailable. If you want it, enable the intent in the
portal *and* add it to the intent list in `main.rs` — both, or the gateway
silently sends nothing.

**Every command defers before touching the network.** Discord invalidates an
interaction token 3 seconds after issuing it, and one cold REST round-trip can
spend that alone. The deferral is unconditional; a command that defers only
sometimes is one that races eventually.

**Decision logic is separated from Discord.** `persona.rs`, `profile.rs`, and
`perms.rs` are plain Rust over plain structs and know nothing about serenity,
which is why the interesting behaviour has 25 unit tests and needs no gateway
connection to run them. `commands.rs` is the only file that translates between
Discord types and those structs.

**Persona routing refuses to guess.** The skill's rule is to switch persona when
the task type is *unambiguous*; a request that pulls toward two personas equally
therefore falls back to Abbey rather than picking a winner on list order.

## Gate

```sh
./check.sh
```

`cargo fmt --all -- --check`, then `cargo clippy --all-targets -D warnings`, then
`cargo test`.
