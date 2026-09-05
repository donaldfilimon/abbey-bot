# Voice play: abbey-bot mirrors local music into Discord voice

Status: **Approach A approved for sidecar and Rust control-path implementation by Donald (2026-09-04).** This request extends the previous sidecar-only slice to the Rust client, player controls, mixing and command registration. Installation, permission setup, actual capture and live Discord/launchd acceptance remain outside authorization.

## Context

Abbey is already present in MLAI Community `Office Hours` (`ABBEY_VOICE_GUILD_ID=1275617641620443146`, `ABBEY_VOICE_CHANNEL_ID=1495755277859815595`), verified live via `GET /guilds/{id}/voice-states/@me` (HTTP 200, unmuted, undeafened). The only missing piece is an audio *source*: every `songbird::input::Input` in the crate is a `RawAdapter` over synthesized PCM (`voice_local.rs:738`, `voice_openai.rs:470`, `commands.rs:796`, `voice_self_test.rs:136`); `symphonia` is compiled with the `pcm` feature only, by design. The registered voice surface is `/voice join|resume|leave|status` and `/voice verify start|report`.

Donald's instruction "use local apps like Spotify and Music for now" removes the decoder problem entirely: raw PCM through `RawAdapter` is already the native path. The blocker moves from *decoding* to *capture*.

## Settled requirements

1. **Scope: control + mirror.** Abbey drives Spotify/Music via AppleScript (`osascript`, tokio `process` feature already enabled) *and* rebroadcasts the resulting audio. She is not a queue-owning music bot.
2. **Capture scope: whole system mix.** Chosen over per-app capture with the leak risk stated: notifications, other calls, and anything audible reach a public channel. Recorded as Donald's decision.
3. **Coexistence: both, with ducking.** Abbey keeps listening and answering; the music track drops in volume while she speaks. Two simultaneous songbird tracks; `TrackHandle::set_volume` for ducking. Consent epoch stays open for the listening half.
4. **Gate: `MANAGE_GUILD`**, identical to `/voice join consent:true`.

## Derived requirements (not optional)

- **Discord must be excluded from the capture.** Whole-system capture while Abbey is in the same call re-broadcasts every participant back to them on a delay: a feedback loop. This rules out any capture method that cannot filter by application.
- **Consent semantics.** `voice_session/control.rs` models consent around Discord *input* (listening / recording / transcribing me; phases Listening, Thinking, Speaking). Music playback is a separate output path, so it must not create or extend anyone's Discord listening agreement. Capturing the host application mix can still include other applications' private audio; excluding Discord does not make all host audio non-personal. The existing listening epoch stays open only under its unchanged consent rules.
- **Fail closed**, matching the existing sidecar posture (`offline_voice::sidecar_is_unavailable`): if the capture source dies, music stops and the bot reports it; it never transmits silence or stale buffers.
- **No `unsafe` in the Rust crate** unless approach B is chosen explicitly; the crate has zero `unsafe` blocks today.
- **Pure core, thin shell** (AGENTS.md): no serenity/poise imports in decision code.

## Approaches

**A. Audio-tap sidecar (recommended).** A small Swift binary uses ScreenCaptureKit to capture system audio with Discord excluded by application, and serves 48 kHz stereo PCM over loopback on its own port. Own launchd job, mirroring `com.donaldfilimon.abbey-mlx-audio`. Rust side: an `AudioTapClient` mirroring `MlxAudioClient`, feeding `RawAdapter`. No driver install, no admin password, no `unsafe`, reuses the established sidecar + fail-closed pattern. Cost: a Swift build step in a Rust repo, a Screen Recording (TCC) prompt once, a third launchd job.

**B. CoreAudio process tap in-crate.** macOS 14.4+ per-process taps (`AudioHardwareCreateProcessTap` / `CATapDescription`) can exclude Discord by PID. One binary, no Swift. Cost: first `unsafe` FFI in a crate with none, CoreAudio bindings, platform code inside the pure core, harder to test without a gateway.

**C. BlackHole + Multi-Output Device. Rejected.** macOS has no native per-application output routing, so Discord's own output lands in the loopback and the feedback loop is unavoidable without paid Loopback. Also requires an admin-password driver install.

## Selected approach

**A, the Swift sidecar.** The full architecture below describes the eventual `/voice play` feature. The sidecar and Rust control path are selected for this implementation; the Rust crate remains free of audio-tap FFI and `unsafe`.

## Architecture (approach A)

```
Spotify/Music ──system audio──▶ abbey-audio-tap (Swift, SCStream, exclude Discord)
                                     │  PCM s16le 48k stereo, chunked HTTP or WS on 127.0.0.1:8182
                                     ▼
                              AudioTapClient (Rust, mirrors MlxAudioClient)
                                     │  RawAdapter → songbird Track "music"
                                     ▼
                     VoiceSession mixer: music track + TTS track, ducking on phase Speaking
                                     ▼
                              Discord voice (Office Hours)
```

Components:
- `tools/abbey-audio-tap/` (Swift package): `SCStream` audio-only capture, `SCContentFilter` excluding `com.hnc.Discord`, HTTP server streaming PCM frames, `/health`.
- `launchd`: `com.donaldfilimon.abbey-audio-tap.plist`, `KeepAlive`, loopback only.
- `src/audio_tap.rs`: `AudioTapClient` (connect, frames stream, health, typed errors), `sidecar_is_unavailable` parity.
- `src/commands_voice/play.rs`: `/voice play <query>`, `/voice pause`, `/voice resume`, `/voice stop-music`, `/voice volume <0-100>`. Gate `MANAGE_GUILD`, `guild_only`, ephemeral acks.
- `src/player_control.rs` (pure): builds the `osascript` for Spotify/Music (`play track`, `pause`, `search`); no I/O in decision code.
- `voice_session`: a second `TrackHandle`; `set_volume(0.25)` on `VoicePhase::Speaking`, restore on Listening. New `SessionEvent::MusicTerminated { reason }` reusing `PlaybackTermination`.
- Env: `ABBEY_AUDIO_TAP_ENDPOINT` (default `http://127.0.0.1:8182`), loopback-only enforced by `llm::url_is_loopback`.

Data flow on `/voice play`: gate check → `osascript` tell player to play → connect tap → start music track → status reply. On `/voice stop-music` or tap failure: stop track, `SessionEvent::MusicTerminated`, reply.

Error handling: tap unreachable → refuse with message, no track; tap dies mid-play → track stops within one frame interval, ephemeral notice; `osascript` non-zero → surface stderr (bounded), do not start capture; consent withdrawal (`stop listening`) does **not** stop music (output is outside consent), but `/voice leave` tears down both.

Testing:
- Pure: `player_control` script generation (golden), ducking state machine (phase → volume), gate matrix.
- Integration (no Discord): `AudioTapClient` against a fake sidecar serving a known PCM ramp; assert frames reach a `RawAdapter` unchanged.
- Sidecar: Swift test that the content filter excludes Discord's bundle id; `/health` contract test.
- Acceptance (human-witnessed, separate claim): music audible in Office Hours, ducks when Abbey speaks, no echo of participants.

## Sequencing constraint

The original draft recorded concurrent dirty voice files and PR #75. That is historical coordination context, not current checkout state. The sidecar implementation began from clean canonical source at `3c13589`; existing linked worktrees remain untouched. Sidecar work does not require changing the bot's voice lifecycle. Deploying the eventual bot integration requires restarting `com.donaldfilimon.abbey-bot`, which remains a separate operator action.

## Out of scope

Queue/library ownership, per-app capture (declined), non-macOS capture, Go Live video, any change to the consent grammar.


## Implementation clarifications

- The existing `/voice resume consent:true` retains its required consent argument.
  Music uses `/voice resume-music` to avoid overloading a listening authorization.
- The sidecar has already landed with positive application inclusion filtering;
  native Discord variants, browsers, terminal hosts and unidentified applications
  are excluded, including processes launched after the capture snapshot.
- Songbird RawAdapter takes f32 PCM. The Rust client losslessly converts the
  sidecar s16le samples to f32 and checks the exact adapter bytes in integration tests.
- Spotify supports a native track URI or current selection; Music searches its
  library. Spotify text search is refused because the native scripting dictionary
  exposes no search command. No browser automation or cloud search is substituted.
- Consent withdrawal physically destroys Decode first. An independent music token
  may then restore a self-deafened Pass call after an exact-epoch teardown marker.
  Buffers are discarded across this transition; brief music interruption is possible.
- Missing-input detection uses a 250 ms timeout, while known failures stop immediately
  on observation. The client bounds queued audio to 100 ms. A literal one-frame
  guarantee for an unobservable network stall or OS buffers cannot be promised.
- Failure reporting uses an ephemeral follow-up while its interaction token remains
  valid and retains a content-free private music status afterward.
