# Voice play source implementation evidence

## Scope and ownership

Donald's current request approves approach A and extends the prior sidecar-only
slice to Rust streaming, native player control, music mixing and private commands.
No service installation, launchd modification, permission request, real capture,
native-player execution, Discord connection or provider call was performed.

Initial inspection found clean canonical `main` at `e39e13c`. A fresh fetch
matched upstream. The separate modernization worktree held dirty voice/command
files and was left untouched; the existing Cursor worktree was also left intact.
The already-landed Swift sidecar was reused. During implementation an external
process committed and pushed an early subset of this task's files as `3fafe54`.
That commit was preserved; it is a partial snapshot, not completed-feature evidence.

## Implementation

- `AudioTapClient` accepts numeric loopback HTTP only, disables proxies and
  redirects, validates health identity and stream headers, and fails on EOF,
  malformed PCM, stalled input, old frames or overflow. It converts s16le to
  exact f32 samples for Songbird RawAdapter. Queues hold at most 100 ms and
  expire at 100 ms; no-input timeout is 250 ms. No silence is manufactured.
- Pure player scripts take untrusted text through argv. Spotify accepts native
  track URIs/current selection; Music searches library track names. The installed
  scripting dictionaries were read without launching either app. Spotify has no
  native search command, so unsupported text searches are refused.
- Private `/voice play`, `pause`, `resume-music`, `stop-music` and `volume`
  require Manage Server, the configured guild and channel presence. Catalog,
  registration and generated README remain aligned. `/voice status` includes
  music state. Existing `/voice resume consent:true` is unchanged.
- Music owns a generation token, buffer and separate Songbird track. Phase
  publication ducks to one quarter of user volume while speaking. TTS replaces
  only its own handle and checks the media epoch during installation.
- Listening withdrawal never grants or renews consent and does not cancel the
  music token. The exact Decode call must be destroyed before an exact-epoch
  teardown marker permits a fresh self-deafened Pass output connection. Old PCM
  is discarded, so this can briefly interrupt music. Busy capture teardown may
  be awaited for at most two seconds; actual capture failures are not retried.
- Leave/shutdown/permission loss cancel music. Pending tap requests observe
  cancellation every 20 ms. Known failures terminate playback when observed;
  Apple/TCP/Discord buffering prevents promising a literal end-to-end one-frame
  failure deadline. Private follow-up errors are attempted while the original
  interaction token is valid; private status remains available afterward.
- Swift and Rust tests share frozen wire fixtures. The HTTP fixture is explicitly
  binary in Git so CRLF and the final header terminator survive checkout.

## Validation

The strict repository gate is `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh`.
The final source run passed 921 Rust tests with zero failures and two existing
live-backend tests ignored. Swift passed 28 named tests (16 core, 12 runtime),
including the shared wire fixture, and its release executable built. Installer
checks used fake tools and a temporary home (25 audio-tap installer tests).

Coverage added here includes endpoint and redirect rejection, exact PCM ramp
conversion through RawAdapter across fragmented HTTP, malformed/truncated input,
no-input timeout, stale/overflow buffers, reconnect isolation, pending-capture
cancellation, script argument safety, music access rules, pure ducking, separate
handle ownership, stale installation/completion, and unchanged listening consent.
The standalone Songbird handle test establishes ownership; it does not assert
an audible mix or query playback timing without a Discord connection.

The gate also checks formatting, deployment syntax/plists, privacy, Pages,
contracts, Linux TLS dependencies, RustSec debt, strict sibling WDBX parity,
warnings-denied Clippy and the locked Rust release build. Four accepted RustSec
vulnerabilities and three unmaintained-package notices remain; the audit is not
clean. WDBX parity hash:
`a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90`.

Logs are retained locally under the ignored `.superpowers/voice-play-20260904/`.
The strict gate exited 0 after its locked release build completed. After staging
the evidence document, its path was added to the explicit Pages inventory; the
Pages scanner and all 22 inventory/scanner tests passed again.

## Acceptance boundary

This is source/offline evidence. Native AppleScript execution, installed identity,
TCC permission, actual capture filtering, music audible in Discord, heard ducking,
participant echo exclusion and cross-platform hosted CI remain unverified. No
live launchd services, owner environment, consent receipts or real audio sources
were changed. These acceptance layers require separate operator action.
