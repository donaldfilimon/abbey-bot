# Service, Persistence, and Observability Design

**Date:** 2026-09-04
**Status:** Approved and binding
**Scope:** Truthful persistence results, supervised background services,
bounded shutdown, privacy-safe operational events, managed readiness, bounded
JSONL logs, and offline launchd transaction acceptance
**Implementation plan:**
`docs/superpowers/plans/2026-09-04-abbey-bot-full-modernization.md`

## Objective

Make process and deployment claims correspond to typed evidence. Persistence
must report what actually reached each durable authority. Every long-lived task
must have an owner and cancellation path. A managed process must publish a
private identity-bound readiness record only after the application is truly
ready. Operational retention must be bounded and incapable of storing Discord
content or private identifiers. The launchd installer must prove this contract
against fakes before any separately authorized live transaction.

This design does not operate or reconfigure the current service. It specifies
the source and offline acceptance work required before a future live managed-
service acceptance.

## Binding Global Constraints

- Preserve Rust edition 2024 and exact Rust 1.98.0 support.
- Preserve existing slash commands, `Abbey: profile`, `Ask Abbey`, and Discord's `PRIMARY_ENTRY_POINT` launch command.
- Preserve the exact seven-tool production order and byte-compatible five-tool Abbey corpus.
- Preserve WDBX v1, guild settings, provider-manifest compatibility, and voice-consent persistence formats.
- Preserve non-privileged intents by default; add no new privileged intent.
- Preserve generated-mention suppression, self-or-moderator memory boundaries, four unsolicited-speech gates, and consent-gated voice behavior.
- Every ordinary command acknowledges before network access; `/voice leave` alone may close the media gate before its concurrent acknowledgement.
- Every rendered model/core answer passes through `clamp_message`.
- Never add a `GuildChannel` command parameter.
- Components remain classic Action Rows; do not use Components V2 on the pinned Serenity/Poise pair.
- Source, CI, provider qualification, installation, Discord acceptance, connector acceptance, voice acceptance, and managed-service acceptance remain separate evidence layers.
- Do not touch the live launchd service, real logs, owner env, providers, Discord configuration, or voice session without a fresh explicit authorization at that acceptance layer.
- Preserve existing unrelated untracked files, stashes, and the linked Cursor worktree.
- Do not hide dead code or warnings with module-wide allowances.

## Truthful Persistence Result

`AppState::persist_all() -> PersistReport` becomes the only process-level
persistence result. Callers do not infer success from a configured data path,
an attempted write, or an emitted log line.

The pure reporting model is:

- `PersistErrorCategory`: a closed, content-free failure category.
- `PersistComponentOutcome`: `NotConfigured`, `Committed`,
  `SkippedCanonicalFailure`, or `Failed(PersistErrorCategory)`.
- `PersistOverall`: `MemoryOnly`, `Complete`, `Partial`, or `Failed`.
- `PersistReport`: overall outcome plus separate `canonical_state` and
  `wdbx_projection` component outcomes.
- `PersistenceSink`: an injectable filesystem/output boundary used to exercise
  type, temporary-write, sync, rename, and projection failures without touching
  a real data directory.

`PersistErrorCategory` initially contains `UnsafeFileType`, `CreateDirectory`,
`SnapshotEncode`, `CreateTemporary`, `WriteTemporary`, `SyncTemporary`,
`PublishRename`, `SyncDirectory`, and `ProjectionEncode`. It never contains a
path, OS error text, serialized state, fact, guild/user/channel identifier, or
other arbitrary string. Human foreground diagnostics may render the internal
source error separately, but the report, readiness, command result, and
structured event use only the category.

### Truth Table

| Data directory | Canonical result | WDBX result | Overall |
|---|---|---|---|
| absent | `NotConfigured` | `NotConfigured` | `MemoryOnly` |
| configured | `Committed` | `Committed` | `Complete` |
| configured | `Committed` | `Failed(category)` | `Partial` |
| configured | `Failed(category)` | `SkippedCanonicalFailure` | `Failed` |

There is no row in which WDBX is attempted after canonical failure. Canonical
JSON remains the authority and WDBX v1 remains its rebuildable projection. The
independent `voice-consent.json` transaction and pending marker retain their
existing format and are not folded into `persist_all`.

Before returning any row, `persist_all` still takes one consistent in-memory
snapshot after persisting brain state, flushing reputation, exporting pending
rewards, and evicting idle conversations under the existing lock order. An
unset data directory therefore returns `MemoryOnly` after the in-memory flush;
it is not labeled a disk success.

### Atomic File Contract

For both canonical state and WDBX projection, the sink:

1. rejects symlinks and unexpected types for the data directory, destination,
   and temporary target;
2. creates a unique owner-only temporary regular file in the destination
   directory without following links or overwriting an existing path;
3. encodes and writes the complete new value;
4. flushes and synchronizes the temporary file;
5. publishes it with a same-directory atomic rename;
6. synchronizes the directory where the platform supports the required
   durability primitive.

Any pre-publication failure removes only the owned temporary file and preserves
the previous destination byte-for-byte. Rename and sync failure paths are
injected in tests. A canonical failure prevents the WDBX attempt. A WDBX
failure leaves the newly committed canonical authority in place and reports
`Partial`; startup may rebuild the projection from it. No fallback truncating
write is allowed.

`/admin flush`, the scheduled persistence actor, startup privacy rewrite, and
shutdown consume the same report. The command renders all component outcomes
without paths. Scheduled calls emit one categorized event. Shutdown includes
the final report in `ShutdownReport` and never emits `Complete` unless both
durable components committed.

## Service Supervisor

The process adds a direct `tokio-util` dependency and uses
`tokio_util::sync::CancellationToken`. Detached tasks are replaced with one
`ServiceSupervisor` that owns every long-lived handle and its child token.

The lifecycle types are:

- `TaskName`: `Scheduler`, `Telegram`, or `Slack`; additional long-lived owned
  tasks require a new closed variant and tests rather than a free-form label.
- `TaskExit`: `Cancelled`, `Returned`, or `Panicked` with a fixed categorized
  reason and no panic payload.
- `ShutdownReason`: `Signal`, `DiscordClientReturned`,
  `DiscordClientFailed`, `SupervisedTaskFailed(TaskName)`, or
  `OperationalInvariantFailed`.
- `ShutdownReport`: reason, per-stage fixed outcomes, aborted/reaped task names,
  total monotonic duration, and the one final `PersistReport`.
- `SchedulerIntervals`: injectable learn, reward-settle, reputation-flush,
  persistence, and summary intervals. Production retains 30 seconds, 30
  seconds, 60 seconds, five minutes, and ten minutes respectively.

Every supervised task is spawned through the supervisor and observed through a
completion channel. Unexpected return or panic from any named task is fatal to
managed readiness and triggers root shutdown. Panic payloads and raw task
errors are discarded after conversion to fixed categories.

Telegram and Slack configuration is parsed explicitly before spawn with secret
wrappers that have redacted `Debug` and no value-bearing display. Missing
connector configuration means `disabled`, not task failure. Once configured,
ordinary remote disconnects, rate limits, and outages are handled inside the
connector as bounded degraded retries. Network I/O and every retry/backoff wait
select on cancellation and have explicit timeouts. The connector task itself
returns only after cancellation or an unrecoverable local invariant; an
unexpected return remains supervisor-fatal.

## Single Scheduler and Serialized Persistence

The five detached heartbeat loops become one named Scheduler actor. Each Tokio
interval uses `MissedTickBehavior::Skip`, consumes the immediate first tick
without running work, and accepts injected paused-time intervals in tests.
Long summary/provider work cannot create a burst of missed executions.

All periodic and command-triggered persistence requests pass through one
serialized persistence worker owned by the Scheduler/supervisor boundary.
Requests may coalesce while one write is active, but each waiter receives the
report for the write satisfying it. Persistence never overlaps itself, and the
final shutdown snapshot begins only after periodic requests are stopped and
quiesced.

Learning, settlement, reputation flush, and summary behavior retain their
existing pure authorities and intervals. The actor changes ownership and
cancellation, not policy.

## Root Shutdown and Budgets

The duplicate spawned signal path and post-client-return cleanup path are
replaced by one root `tokio::select!` over:

- Ctrl-C/SIGTERM;
- Discord client completion;
- a supervisor fatal-exit notification.

The first event atomically fixes one `ShutdownReason` and enters draining.
Simultaneous later triggers are observed only as categorized secondary facts;
they cannot run a second shutdown or persistence path.

Shutdown has a 20-second overall monotonic deadline. In order, each stage gets
at most five seconds and never more than the overall time remaining:

1. close voice media/cancel voice work and leave;
2. shut down Discord shards;
3. cancel, join, then abort and reap supervised tasks;
4. perform exactly one final serialized persistence attempt.

Before stage four, no accepted command, connector event, scheduler tick, voice
turn, reward settlement, or summary may mutate the snapshot authorities. If a
stage exceeds its budget, shutdown records `TimedOut`, performs its safe abort
or reap action, and continues within the overall deadline. Aborted task handles
are awaited so no owned task survives process cleanup. Final persistence is
attempted once even when an earlier stage fails, subject to the remaining
overall budget; it is never repeated by `Drop`, the Discord return path, or a
second signal.

## Run Identity and Startup Order

`RunIdentity` contains the process PID, a cryptographically random per-process
nonce, and the SHA-256 of the executable currently running. The nonce comes
from the operating system cryptographic RNG. The executable is resolved and
hashed directly. Environment-supplied PIDs, nonces, hashes, executable paths,
or identity files are never trusted.

Managed observability initializes and validates its private directories before
Abbey reads `DISCORD_TOKEN`, connector tokens, provider credentials, or
production data. The startup order is:

1. parse the non-secret startup mode and explicit token-free self-test exits;
2. construct `RunIdentity` and initialize managed logging/readiness safety;
3. read and sanitize state, including legacy interaction rows;
4. perform the required canonical privacy rewrite in managed mode;
5. read/authenticate credentials and construct application/provider state;
6. start the supervised scheduler/connectors;
7. connect Discord, receive Ready, preserve/register commands, and apply
   presence;
8. publish managed `ready` only after every required checkpoint succeeded.

Token-free provider/voice self-test modes preserve their current behavior and
do not open production state or publish managed ready.

## Privacy-Safe Interaction History

The durable interaction row is replaced by a schema containing only:

- stable catalog command key/name;
- success boolean;
- optional fixed error category;
- total interaction latency in milliseconds;
- wall-clock timestamp in milliseconds.

It contains no user, guild, channel, message, interaction, or role ID and no raw
error. Legacy rows still deserialize. During load, their command/success/
bounded timing values are retained, raw error is categorized or reduced to an
unknown fixed category, and every legacy private ID/error field is discarded.
Managed startup writes the sanitized canonical state before becoming ready.
Failure to commit that rewrite prevents managed ready; a successfully committed
canonical rewrite with a WDBX projection failure is truthfully `Partial`, not a
reason to claim the projection succeeded.

Owned interval durations use a monotonic clock with millisecond precision.
Total Discord interaction latency uses Discord's interaction creation
timestamp in milliseconds, not seconds rounded then multiplied by 1,000; a
clock-skewed negative delta saturates to zero. Wall-clock timestamps are only
for event occurrence and readiness freshness, never timeout ordering.

## Structured Operational Events

The managed event model is closed and content-free:

- `EventComponent`: `Process`, `State`, `Discord`, `Scheduler`, `Persistence`,
  `Provider`, `Voice`, `Telegram`, `Slack`, or `Shutdown`.
- `EventCode`: `Starting`, `StateLoaded`, `PrivacyRewrite`, `TaskStarted`,
  `TaskExit`, `DiscordReady`, `CommandsRegistered`, `PresenceApplied`,
  `ConnectorState`, `PersistenceAttempt`, `ProviderAttempt`, `VoiceState`,
  `ReadinessPublished`, `ShutdownStarted`, or `ShutdownCompleted`.
- `EventOutcome`: `Started`, `Succeeded`, `Ready`, `Degraded`, `Failed`,
  `Cancelled`, `TimedOut`, `Skipped`, `Draining`, or `Stopped`.
- `OperationalErrorCategory`: `Configuration`, `Authentication`,
  `Authorization`, `Unavailable`, `Timeout`, `Protocol`, `Capacity`,
  `Persistence`, `UnsafeFileType`, `UnexpectedReturn`, `Panic`, or `Internal`.
- `OperationalEvent`: schema version, occurrence timestamp milliseconds,
  component, code, outcome, optional fixed error category, optional monotonic
  duration milliseconds, optional bounded aggregate count, and optional
  normalized provider ID or closed `TaskName`.

There is no arbitrary `message`, `error`, `details`, `fields`, or payload map.
Serde rejects unknown output variants. Static and dynamic privacy gates reject
Discord/platform IDs, raw errors, filesystem paths, URLs, endpoints, model
names, environment values, prompts, transcripts, tool arguments/results,
provider response bodies, message payloads, media, audio, and image bytes.
Process PID, run nonce, and executable hash occur only in the private readiness
identity where required; operational events do not duplicate them.

Foreground mode preserves the existing human-readable tracing/stderr
experience. Managed mode, selected by a fixed non-secret launchd startup
argument, uses the structured writer. It does not reinterpret arbitrary user
environment as run identity.

## Managed Readiness Contract

The fixed managed readiness path is:

```text
$HOME/.local/share/abbey-bot/readiness.json
```

It is not configurable through the owner environment. The parent directory is
an owner-controlled mode-0700 regular directory. The readiness document is an
owner-only mode-0600 regular file published by unique same-directory temporary
file, file sync, atomic rename, and directory sync. Symlinks and unexpected
types fail closed.

The version-1 document contains only:

- schema version;
- PID, run nonce, and running executable SHA-256;
- phase: `starting`, `ready`, or `draining`;
- publication timestamp in Unix milliseconds;
- Discord state: `connecting`, `ready`, or `stopped`;
- scheduler state: `starting`, `running`, or `stopped`;
- Telegram and Slack coarse states: `disabled`, `starting`, `connected`,
  `degraded`, or `stopped`;
- last persistence overall category: `not_attempted`, `memory_only`,
  `complete`, `partial`, or `failed`.

It contains no Discord IDs, provider identity/configuration, model, endpoint,
path, secret, prompt, transcript, error string, message, media, counter, or
participant data.

The process may publish `starting` after identity and path validation, but
publishes `ready` only after the canonical privacy rewrite committed, the
scheduler is running, Discord emitted Ready, command registration including the
Entry Point preservation completed, and presence was applied. Connector
outages may be `degraded` without preventing ready because they are optional
bounded retry services. Unexpected supervised task exit removes eligibility
for ready and triggers draining.

On shutdown the process publishes `draining` before cancellation. It removes a
readiness file only after re-reading it and proving both PID and nonce equal the
current `RunIdentity`; it never removes a successor's file. A crash may leave a
stale file, which the checker rejects by process/identity/freshness checks.

## Bounded Managed JSONL

Managed operational events are written internally to:

```text
$HOME/Library/Logs/abbey-bot/abbey-bot.events.jsonl
```

The existing `abbey-bot.log` is never read, truncated, rotated, moved,
overwritten, or treated as current evidence. The launchd plist stops directing
new stdout/stderr to that legacy file and sends managed standard output/error
to `/dev/null`; the internal JSONL writer is the managed log authority and is
initialized before credential access. Foreground stderr remains human-readable
because foreground execution does not use the launchd plist.

The log directory must be a non-symlink owner directory at mode 0700. Active
and archive files are non-symlink owner regular files at mode 0600. One
process-local synchronized writer serializes all lines and rotation. Every
encoded event plus newline is at most 16 KiB. Because the event schema has no
unbounded strings, oversize is an invariant failure; the original event is not
partially written, and only a fixed categorized failure may be emitted.

Before each write, if `current_length + next_line_length` would exceed 8 MiB,
rotation occurs before the line is appended. Archives are exactly `.1` through
`.5`, newest first; an existing `.5` regular file is removed, `.4` through
`.1` are renamed upward, and the active file becomes `.1`. Any symlink or
unexpected type aborts the operation. The new active file is created mode 0600.
There are no more than five archives and one active file.

Initialization/type/permission failure prevents managed ready. A writer
invariant or I/O failure after ready emits the fixed
`OperationalInvariantFailed` shutdown trigger and enters bounded shutdown; the
bot does not silently keep claiming managed readiness without its required
observability channel.

## Offline launchd Transaction Verification

`deploy/check-service-readiness.py` validates one candidate or rollback process.
It accepts only explicit expected transaction start time, installed binary
SHA-256, launchd-reported PID, and the fixed readiness path. Within a 30-second
total readiness budget it requires a private regular document whose:

- publication is fresh for the current transaction;
- PID equals launchd's current PID and that PID remains present;
- nonce is nonempty, valid, and differs from any pre-transaction readiness
  identity;
- executable SHA equals the installed binary SHA;
- phase is `ready`;
- scheduler is `running`;
- Discord is `ready`.

After the first match, it requires five additional continuous seconds with the
same PID, nonce, SHA, ready phase, scheduler, and Discord state. Connector state,
last persistence category, and publication timestamp may update without
invalidating identity. Stable PID alone, a stale file, a matching hash under a
different PID/nonce, or a transient ready file never proves success.

The installer uses the checker after a fresh install/update and after automatic
rollback. A rollback is successful only when the restored binary's matching
readiness contract also passes; restoring bytes or seeing a stable PID is not
enough.

`deploy/test-install-launchd.py` runs the complete shell transaction under a
temporary HOME with fake `cargo`, `launchctl`, `plutil`, and `sleep`. The
harness uses no real launchd domain, process, service, owner environment, data,
logs, or readiness file. It records every fake invocation and asserts every
mutated path is a descendant of the temporary HOME or the test's explicit
repository/build fixture.

The offline matrix covers:

- fresh install and update success;
- missing, stale, malformed, symlinked, wrong-owner/mode, wrong-PID,
  wrong-nonce, wrong-SHA, non-ready, scheduler-stopped, and Discord-not-ready
  documents;
- PID or nonce changes during the five-second stability window;
- bootstrap failure, candidate readiness failure, rollback bootstrap failure,
  rollback readiness failure, and retained recovery state;
- install-lock contention/ownership and unsafe file types for every published
  target;
- invalid owner environment and binary/hash mismatch without printing values;
- HUP/INT/TERM cleanup at transaction phases;
- install and uninstall modes;
- secret-canary exclusion from stdout/stderr, fake logs, readiness, and JSONL.

Uninstall preserves today's data-retention contract: stop the exact service,
remove only its plist and same-identity readiness file, and leave installed
binary, data, owner environment, rollback material, structured logs, and legacy
log in place. Unexpected types fail closed rather than being recursively
removed.

The POSIX offline behavior suite runs from `check.sh`. Windows runs syntax,
schema, and privacy checks for the Python helpers and explicitly reports
launchd execution skipped; that is not Windows managed-service acceptance.

## Verification

Focused source tests cover:

- every `PersistReport` truth-table row, canonical-before-WDBX ordering, old
  file survival under temporary/write/sync/rename failures, safe type checks,
  command/scheduler/shutdown rendering, and exactly one final report;
- paused scheduler cadence, `Skip` behavior, serialized/coalesced persistence,
  cancellation during connector I/O/backoff, degraded retry, unexpected return
  and panic, simultaneous root triggers, all shutdown budgets, abort/reap, and
  post-final-snapshot quiescence;
- legacy interaction deserialization and canonical privacy rewrite, accurate
  sub-second total latency, monotonic interval timing, secret/private canary
  exclusion, and initialization before credential access;
- readiness ownership, mode, type, atomicity, freshness, identity, phase
  gating, successor-safe removal, and stale-crash behavior;
- JSONL limits, pre-write rotation, archive retention, concurrent writers,
  permissions, unsafe types, and invariant failure;
- the complete fake-launchd matrix above with descendant-path assertions.

These gates prove source and offline transaction behavior only. They do not
prove a real installation, launchd state, logs, Discord connection, connector
round trip, provider qualification, or consented voice session. Each requires
fresh authorization and evidence at its own acceptance layer.
