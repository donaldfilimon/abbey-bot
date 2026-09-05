# Command catalog and macOS audio-tap implementation evidence

This record covers modernization Task 4 and the separately requested
Discord-excluded macOS audio-tap sidecar. It is source evidence only.

## Scope and decisions

- Baseline: clean canonical Rust checkout at `3c13589`, after fast-forwarding
  the documentation-only upstream change. Existing linked worktrees remain
  owned by their other sessions.
- The audio-tap request selects approach A in the voice-play design. This
  delivers a Swift sidecar and deployment artifacts; player control, Rust
  streaming, mixer ducking, and `/voice play` registration are separate work.
- Future Task 5/6 catalog entries must remain explicitly planned until their
  handlers exist. Help and active registration must never advertise a handler
  that cannot run.
- Existing detailed voice status must remain private and manager-gated until
  Task 6 supplies the member-safe projection. Catalog discoverability must not
  expose its current operational detail to ordinary members.
- Poise's built-in runtime permission checks precede its command hook. The
  catalog adapter must retain Discord registration defaults while evaluating
  current access only after acknowledgement.
- Discord browser tabs share their browser's application audio source. Browser
  audio is therefore excluded from the sidecar, alongside Discord and
  unidentified sources. Eligible native apps such as Spotify and Music remain
  the intended sources.
- No live Discord, launchd, provider, owner environment, audio capture, Screen
  Recording permission, or real log operation is part of this task.

## Verification

At baseline, `cargo test --locked registration` passed all three Entry Point
registration fixtures. Rust and Cargo resolve to exact version 1.98.0;
`cargo-audit` is 0.22.2, and the canonical sibling WDBX checkout exists.
The macOS SDK toolchain reports Apple Swift 6.4.

The implementation provides 41 registered command leaves and five explicitly
planned entries. The pure catalog owns named access and condition rules;
recursive registration, runtime binding and generated README parity are tested.
Help is private, permission-aware, mention-suppressed and clamped. Component
sessions keep their original 15-minute deadline and reject malformed, foreign,
expired and unknown controls before permission lookup. Existing Entry Point
registration is preserved.

Focused Rust checks passed for catalog policy and README parity (7), help adapter
and component boundaries (12), the pure component protocol (2), registered Poise
dispatch (12), generation fallback (10), voice adapters (22), Entry Point
registration (3), memory/brain adapters (6), and common command adapters (10).
These are 84 focused test executions, not additional tests beyond the full suite.

The 12 dispatch fixtures construct real Poise contexts and invoke registered
checks over a loopback HTTP fixture and inert local WebSocket handshake. They
cover current permission denial/allowance, missing capabilities, guild-only DM
denial, caller presence, application ownership, defer ordering, failed
acknowledgements, failed permission lookup, and self-service with zero permission
REST. All five registered autocomplete callbacks are exercised with cross-user,
cross-guild and cross-DM canaries; autocomplete sends neither an ordinary defer
nor a permission lookup.

The Swift package passed 27 named tests (50 parameter-expanded cases): 15 core
tests and 12 runtime tests. Runtime coverage includes six real HTTP tests with
synthetic sources, four in-memory CoreMedia conversion tests, and two tests that
hold capture-stop completion to prove source retention and reconnect blocking.
The release executable built; its capture-free `--help` and `--version` passed.
The launchd installer passed 25 tests using a temporary home and fake platform,
build, launchctl, listener and curl tools. No real installer was executed.

The final combined gate, `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`, exited 0
after all source changes. It passed formatting, deployment shell/Python/plist
checks, privacy, Pages, contract, TLS, RustSec-debt and WDBX checks, the offline
Swift test/release gate, Clippy with warnings denied, **904 Rust tests with zero
failures and two ignored**, and the locked Rust release build. The ignored tests
require a live generation backend or the on-device Foundation Model and were
not enabled for this source-only task.

Strict cross-repository WDBX fixture parity passed with SHA-256
`a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90`.
The RustSec check confirmed the existing accepted debt still matches: four
vulnerabilities and three informational unmaintained-package warnings remain.
This is not a clean dependency audit. The final local gate log is retained in
the ignored `.superpowers/catalog-audio-review-20260904/check-staged.log`.
The Pages inventory includes both new Markdown files; its 22 tests and scanner
were rerun after staging them, and the combined gate was repeated with that
complete tracked-file inventory.

## Review and corrections

Independent source and deployment reviews have no open findings. Review fixes
preserve autocomplete's response protocol, retain qualified FM text fallback
after tool rejection, authorize `/modcall` before target lookup, and prevent an
owner curl configuration from extending the installer's health-only probe.
The registered-check coverage gap was closed with the 12 real-context fixtures.
Final reinspection also confirmed that the self-service permission shortcut
cannot bypass cross-member or mandatory management access rules.

The audio review covers the positive application inclusion filter, callback
identity revalidation, PCM and timestamp validation, bounded transport queues,
serial state ownership, capture-stop retention and fail-closed reconnects.
Deployment review covers staged publication, rollback, recovery lock retention,
owner-only files and capture-free service identity verification.

## Acceptance boundaries

Local validation does not establish hosted CI, installed artifact identity,
ScreenCaptureKit capture behavior, macOS permission, audible Discord playback or
feedback exclusion on a particular host. Those layers were not exercised.
The sidecar omits common browsers and terminal hosts because their audio may
contain Discord, and cannot identify arbitrary renamed/custom embedded clients.
Eligible native applications can still emit private notifications or calls.
Application buffering bounds do not control Apple's capture or TCP buffers.

No live Discord or launchd state, owner environment, real logs, capture permission,
providers or consent records were changed. Existing linked worktrees and unrelated
files remain outside this implementation. Tasks 5 and 6 remain open; their future
context menus, member-safe status, dashboard and diagnostics are not registered
by this change. The Rust streaming client, player commands and Discord mixer are
also still required before `/voice play` is usable.
