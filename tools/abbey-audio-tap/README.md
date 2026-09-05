# Abbey audio tap

A macOS 14+ Swift 6 sidecar for the proposed `/voice play` audio source. It uses
Apple ScreenCaptureKit and Network.framework, without third-party dependencies.
This package implements capture and loopback transport only. The Rust client,
music commands, player control and Discord mixer are separate work.

The source build and offline tests do not establish TCC permission, live capture,
audible Discord playback or freedom from feedback on a particular Mac. Those
acceptance steps remain unperformed by this implementation task.

## Capture boundary

The stream is the mix of eligible native applications visible to ScreenCaptureKit
when the consumer connects. Music and Spotify are eligible. A positive application
filter excludes Discord Stable/PTB/Canary/Development and known alternative
clients, their identified helpers, the sidecar, Abbey processes, common browsers,
and terminal hosts. Bundle IDs, application names and executable ancestry are
checked case-insensitively; missing identity or launch time excludes an app.
`excludesCurrentProcessAudio` is enabled as an additional self-capture guard.

Browsers are omitted because ScreenCaptureKit filters audio at application level:
it cannot exclude a Discord tab while including another tab in the same browser.
Terminal hosts are omitted because macOS may attribute a command-line child's
audio to its responsible application. The maintained identity rules cover common
native Discord clients and browser/terminal families, not arbitrary renamed or
custom embedded web clients whose identity does not disclose their purpose.

A newly launched process is never automatically admitted into the running filter.
Before each audio callback, the selected PID, launch time, bundle, name and
executable must still match the native application identity. A changed or exited
selected application stops capture; reconnect to build a fresh filter, including
newly launched eligible apps. This deliberately avoids an exclusion-list polling
window in which a newly started Discord process would initially be captured.

This is an eligible application mix, not a promise to capture every OS sound.
Unidentified processes, sources absent from ScreenCaptureKit's application list,
protected audio and system components the framework does not expose can be
omitted. Notifications and other calls emitted by eligible applications can still
be included. The OS controls audio attribution and application filtering; offline
policy tests cannot prove the live attribution or feedback behavior.

Only the `.audio` output is registered. There is no `.screen`, microphone,
screenshot or recording output. The stream requests 48,000 Hz and two channels;
the adapter validates those properties and converts planar/interleaved float32
or signed int16 buffers into interleaved signed 16-bit little-endian PCM.
Unsupported formats fail instead of being mislabeled or guessed. Video delivery
is not registered; the SCStream's required visual configuration is minimized.

## HTTP protocol, version 1

`abbey-audio-tap serve` binds only IPv4 `127.0.0.1:8182`. There are no production
host, port or environment overrides. The test harness injects an ephemeral port
and fake source, so tests never bind the production port or initialize capture.

Requests must be body-free HTTP/1.1 `GET`s with `Host: 127.0.0.1:8182` exactly.
Duplicate headers, bodies, upgrade requests, `Origin`, and any `Sec-Fetch-*`
header are rejected. No CORS headers are emitted. This prevents ordinary browser
pages and DNS rebinding from triggering capture; it is not authentication against
other native processes on the same Mac.

| Endpoint | Response |
| --- | --- |
| `GET /health` | HTTP 200 JSON containing the fixed service identity, protocol, audio format and current session state. Never constructs a capture source or calls a TCC API. |
| `GET /stream` | HTTP 200 with chunked raw PCM after the first validated frame. One consumer at a time; a competing consumer or an unfinished capture stop gets HTTP 409. Unavailable capture gets HTTP 503 before PCM headers. |

An idle health response is:

```json
{"service":"abbey-audio-tap","protocol_version":1,"status":"idle","ready":false,"audio":{"sample_rate":48000,"channels":2,"format":"s16le"},"stream_path":"/stream","error":null}
```

`status` is `idle`, `starting`, `capturing` or `failed`. `ready` is true only
while a session is delivering validated audio, and is normally false when the
service is idle. Health proves service identity and state; it does not attest
permission or capture quality. Failure reasons are fixed, content-free codes.
No app names, process IDs, audio or transcripts appear in health.

Successful stream headers include:

```http
Content-Type: application/octet-stream
Transfer-Encoding: chunked
X-Audio-Format: s16le
X-Audio-Sample-Rate: 48000
X-Audio-Channels: 2
Cache-Control: no-store
Connection: close
```

Decode HTTP chunk framing before consuming PCM. The body contains no WAV header,
JSON metadata or silence padding. Each chunk contains a whole number of four-byte
stereo frames. Its size may vary. A capture failure aborts the connection without
the HTTP terminal zero chunk; consumers must treat EOF/incomplete chunking as
termination and stop playback immediately. Audio already handed to the OS TCP
stack cannot be recalled, so clients must also bound their own buffering.

## Lifecycle and bounds

Capture starts only for `/stream`, after a non-prompting permission preflight.
Disconnect stops capture and discards all queued PCM. A new consumer receives
only its own newly started capture. Generation tokens discard late callbacks;
the old source remains retained until stop completes. If native capture cannot
confirm it has stopped, new streams remain blocked until service restart.

The sidecar accepts at most 16 connections and 4,096 bytes of request headers,
with a five-second header timeout. Capture must supply its first frame within
five seconds. A two-second absence of frames terminates a started capture,
including when ScreenCaptureKit stops producing callbacks during silence.
The sidecar never generates silence to disguise an unavailable source.

Each PCM callback is limited to 100 ms. The application queue, including its
single in-flight write, is capped at 48,000 bytes (250 ms). A write or queued
frame older than 250 ms terminates the consumer; buffers are discarded instead
of catching up later. The watchdog runs every 100 ms. Source presentation times
are checked against `SCStream.synchronizationClock`: old, duplicate, future or
invalid timestamps fail closed before transport. Apple owns its internal capture
and TCP buffers; these limits describe this sidecar's application buffering.

All mutable runtime state and capture callbacks use one serial dispatch queue.
PCM callbacks perform bounded synchronous conversion and enqueue no async tasks.
Audio is never persisted or logged, and no providers or Discord APIs are called.

## Build and offline verification

From the repository root, use an external scratch directory:

```sh
tap_scratch=$(mktemp -d "${TMPDIR:-/tmp}/abbey-audio-tap.XXXXXX")
env -u TOOLCHAINS swift test --package-path tools/abbey-audio-tap --scratch-path "$tap_scratch"
env -u TOOLCHAINS swift build --package-path tools/abbey-audio-tap --scratch-path "$tap_scratch" -c release
```

The tests exercise policy, identities, timestamp freshness, PCM conversion,
malformed HTTP, bounded queues and lifecycle deadlines. Real HTTP tests bind an
ephemeral loopback port and inject synthetic sources, checking exact PCM bytes,
capture-free health, permission failure responses, single-consumer refusal,
disconnect, source death, queue overflow and the startup watchdog. CoreMedia
tests construct in-memory sample buffers to validate native buffer conversion;
they create no ScreenCaptureKit stream.

`--help` and `--version` do not start a listener or touch capture permissions.
The launchd installer builds and copies this executable only; there are no
resource files to install alongside it.

## Operator permission setup, separate from installation

On the installed executable, the explicit `request-permission` command calls
Apple's screen-capture permission request without starting capture or a listener:

```sh
"$HOME/.local/libexec/abbey-bot/audio-tap/abbey-audio-tap" request-permission
```

Review macOS **Privacy & Security → Screen & System Audio Recording** and the
responsible executable or terminal macOS names, then restart the sidecar after
granting access. TCC behavior depends on OS version and executable identity;
rebuilding, replacing, signing or launching from a different host can require
renewed approval. Installation, health checks and HTTP requests never request
permission. The setup command and live capture must be run deliberately by the
operator; neither is run by the test suite or repository gate.

## Apple API references

- [ScreenCaptureKit content filters](https://developer.apple.com/documentation/screencapturekit/sccontentfilter)
- [Capturing screen content in macOS](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos)
- [Take ScreenCaptureKit to the next level](https://developer.apple.com/videos/play/wwdc2022/10155/) explains that audio filtering applies to whole applications.
- [SCStream synchronization clock](https://developer.apple.com/documentation/screencapturekit/scstream/synchronizationclock)

The native adapter was also checked against the installed Xcode SDK's
`SCStream.h` and `SCShareableContent.h`. Source compilation does not substitute
for the outstanding live capture and audible feedback acceptance.
