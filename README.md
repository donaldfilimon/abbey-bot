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
| `/modcall <user> <severity> [warnings] [timeouts]` | Recommends a moderation action and says whether *you* can carry it out. |
| `/server <kind>` | Emits a role hierarchy, channel structure, and numbered setup steps. |

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

**Decision logic is separated from Discord.** `persona.rs`, `profile.rs`,
`perms.rs`, `moderation.rs`, and `server.rs` are plain Rust over plain structs
and know nothing about serenity, which is why the interesting behaviour has 48
unit tests and needs no gateway connection to run them. `commands.rs` is the only
file that translates between Discord types and those structs — including the
`SeverityChoice` and `ArchetypeChoice` mirrors that keep poise's derive out of
the escalation ladder and the blueprints.

**`/server` emits a plan and creates nothing.** Building a server is a run of
structural changes, and those stay with a human who can see what already exists.
The blueprints are property-tested rather than eyeballed, because the mistakes
here are quiet ones: every gated channel must name a role the same blueprint
creates (otherwise the steps are unfollowable), every permission string must be
one serenity actually defines (a typo yields a step nobody can perform), and
every text-channel name must already be in the form Discord will store it —
Discord lowercases text names and hyphenates whitespace, so a blueprint saying
`General Chat` describes a server you do not get. Voice channels are exempt from
that rule, which is exactly the asymmetry that trips people, so `render` applies
the normalization per channel kind.

**`/modcall` recommends and never acts.** It does not time out, kick, or ban
anyone; the decision stays with the moderator. It is ephemeral for the same
reason — a recommendation posted to the channel would be a public accusation.
Two properties of the ladder are deliberate and load-bearing: severity outranks
history (a severe incident bans on the first offence, so a clean record cannot
buy tolerance for a threat), and more history never yields a *lighter* action —
both are pinned by property tests. Every timeout is constructed through a clamp
at Discord's 28-day ceiling, so a future rung cannot produce a request the API
rejects. The command also checks whether the *invoking* moderator holds the
permission the recommendation needs, and says so when they do not.

**Persona routing refuses to guess.** The skill's rule is to switch persona when
the task type is *unambiguous*; a request that pulls toward two personas equally
therefore falls back to Abbey rather than picking a winner on list order.

## Gate

```sh
./check.sh
```

`cargo fmt --all -- --check`, then `cargo clippy --all-targets -D warnings`, then
`cargo test`.
