# Abbey Live Voice Design

**Date:** 2026-08-20  
**Status:** self-deafened Discord join validated live; provider speech blocked on configuration

## Outcome

Abbey can join one explicitly configured Discord voice channel and hold a
full-duplex audio conversation through OpenAI Realtime. The
feature is safe to deploy while disabled: no destination variables means no
voice session, and an OpenAI key used elsewhere does not opt voice in.

## Boundaries

- Discord media uses Serenity 0.12 + Songbird 0.6 with `receive`. Songbird owns
  voice signaling, Opus, jitter buffering, transport encryption, and Discord's
  mandatory DAVE/MLS E2EE handshake.
- The Discord shell stays in `commands.rs`/`main.rs`. `voice.rs` contains only
  provider-neutral configuration policy and deterministic PCM transforms.
- The Realtime server connection uses a standard backend API key over WSS.
  Remote plaintext WS, credentials embedded in URLs, partial destination
  configuration, zero snowflakes, and unsafe model/voice names fail at startup.
- Discord's 48 kHz stereo PCM is mixed and decimated to Realtime 24 kHz mono
  PCM16. Realtime PCM16 is converted to 48 kHz stereo f32 for Songbird.
- The 20 ms Discord callback only mixes and `try_send`s. Both directions use
  bounded 50-chunk queues; overflow drops and increments an observable counter
  instead of blocking the audio deadline.

## Consent and control

`/voice join`, `/voice leave`, and `/voice status` require Manage Server. Join
works only in `ABBEY_VOICE_GUILD_ID` and only for the voice channel named by
`ABBEY_VOICE_CHANNEL_ID`; Stage channels are rejected. The operations-only
`ABBEY_VOICE_AUTOJOIN=1` path is permitted only when no provider key exists and
immediately self-deafens, so it cannot receive or transmit call audio. Normal
full-duplex mode still requires `/voice join`. Its response states that channel
audio is sent to the configured provider and names `/voice leave` as the stop
control.

The code does not ingest Discord Go Live video. Realtime supports audio plus
discrete image inputs, not video; Songbird's supported receive API is voice and
RT(C)P. Stream understanding therefore remains a separate future slice that
must define an explicit consented screenshot source and retention policy.

## Configuration

Required to opt in to the Discord connection:

- `ABBEY_VOICE_GUILD_ID`
- `ABBEY_VOICE_CHANNEL_ID`

`OPENAI_API_KEY` is required for full-duplex Realtime speech. Without it,
Abbey can connect only in self-deafened, audio-free mode.

Optional: `ABBEY_VOICE_AUTOJOIN` (no-key/self-deafened only),
`ABBEY_VOICE_REALTIME_ENDPOINT` (default OpenAI WSS),
`ABBEY_VOICE_REALTIME_MODEL` (current default `gpt-realtime-2.1`),
`ABBEY_VOICE_NAME` (default `marin`), and `ABBEY_VOICE_INSTRUCTIONS`.

## Verification contract

The offline gate covers fail-closed configuration, key redaction, endpoint
policy, PCM channel/rate transforms, formatting, Clippy, all unit tests, and a
locked release build. Live completion additionally requires a funded provider
key, command registration in the target guild, bot Connect/Speak permission,
in-channel consent, an observed DAVE join, an observed spoken turn in each
direction, provider-side VAD interruption behavior (including how much already
buffered Discord playback remains audible), `/voice status`, and `/voice leave`.

## Dependency audit

The locked release gate passes, but `cargo audit` is not green. Four findings
are the repository's pre-existing Serenity/rustls-webpki 0.102 findings. The
Songbird DAVE lock adds six libcrux findings through
`davey 0.1.4 -> openmls_rust_crypto 0.5.1 -> hpke-rs 0.6.1`:

- the AES-GCM and ChaCha findings occur only in optional packages present in
  `Cargo.lock`; `cargo tree --target all -i` finds no enabled path to them;
- `libcrux-sha3 0.0.8` and `libcrux-secrets 0.0.5` are in the enabled tree, but
  hpke-rs calls SHAKE only for X-Wing/ML-KEM key derivation while DAVE v1 pins
  `MLS_128_DHKEMP256_AES128GCM_SHA256_P256`, so the advisory-listed SHAKE and
  swap/select functions are not on Abbey's DAVE path;
- direct lock updates to sha3 0.0.10 and secrets 0.0.6 are rejected by the
  upstream 0.x version constraints. DAVEy's current main still pins OpenMLS
  0.8.1 and openmls_rust_crypto 0.5.1.

This is documented upstream debt, not a claim that a red crypto audit is
"fixed." Do not replace the provider stack with a hand-edited crypto fork.
Re-run the audit when DAVEy/Songbird publishes an OpenMLS/HPKE update.
