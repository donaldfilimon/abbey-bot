# Abbey Live Voice Design

**Date:** 2026-08-20
**Status:** offline-first implementation and private full-chain audition
validated; earlier safe presence, participant pause, and manager leave observed;
exact candidate deployment and refreshed consented acceptance pending

## Outcome

Abbey can hold one explicitly consented conversation in one configured Discord
voice channel. Local Apple-silicon speech is the default: MLX-Audio performs
Whisper STT and Kokoro TTS, while Abbey's existing loopback text backend runs
canonical persona/cognition. OpenAI Realtime is an explicit degraded backup
only; provider transcription is not an authoritative spoken-control channel, so
`/voice leave` or a written `stop listening` in the configured voice chat is
the deterministic stop path. The feature is safe to deploy with persistent
presence because autojoin never creates a conversational actor or enables
decoding, regardless of backend configuration. No destination means no voice
surface, and a provider key never selects cloud audio by itself.

## Boundaries

- Discord media uses Serenity 0.12 + Songbird 0.6 with `receive`. Songbird owns
  voice signaling, Opus, jitter buffering, transport encryption, and Discord's
  mandatory DAVE/MLS E2EE handshake.
- The Discord shell is `commands_voice.rs`; `voice.rs` owns provider-neutral
  configuration, `offline_voice.rs` owns the loopback client and deterministic
  framing/segmentation, `voice_session.rs` owns epochs and cancellation, and
  `voice_local.rs` / `voice_openai.rs` own their respective actors.
- Local speech endpoints must be credential-free loopback HTTP. Local voice
  also refuses a remote text-generation backend, and both HTTP clients bypass
  process/system proxies for loopback requests. Cloud Realtime requires
  `ABBEY_VOICE_MODE=openai`, a key, and WSS (loopback WS is test-only); it is a
  direct, whole-response-buffered degraded backup without local ABI routing or
  WDBX context. It must not claim that spoken requests changed Discord voice
  state; participants use `/voice leave` or written withdrawal in the
  configured voice chat when the backup is active.
- Conversational Songbird calls start in mono 24 kHz `Decode` mode because
  Songbird 0.6 cannot promote a running `Pass` receiver to `Decode`. Mapping,
  transport-liveness, and `VoiceTick` handlers are installed before the join so
  one-shot core events cannot be missed, but a separate atomic media epoch
  makes `VoiceTick` return before inspecting or forwarding samples and keeps
  playback closed until disclosure and final activation.
- The 20 ms callback performs mapping/attestation checks, bounded energy work,
  and `try_send` only. Unknown speakers atomically revoke media before a frame
  is forwarded. The pause transition advances the exact epoch, closes media,
  detaches the actor, and physically leaves the conversational `Decode` call;
  provider/playback cleanup completes behind that closed gate.
- Utterances, provider messages, and output are duration/size bounded. Raw
  audio is never written. Local voice turns are read-only: model tools and
  durable memory changes are disabled; a safely attributed completed playback
  may commit only its conversational transcript.

## Consent and control

`/voice join consent:true`, `/voice resume consent:true`, and `/voice status`
require Manage Server. Join/resume additionally require the caller to be in the
configured voice channel. The boolean is an attestation that everyone currently
present was notified and agreed; Abbey snapshots non-bot participants, joins
muted/self-deafened, posts a public backend/retention disclosure, verifies the
snapshot again, installs the actor, verifies again, and only then opens the
software media epoch. A new, unknown, or unattested participant revokes the
epoch before their frame is forwarded, stops queued work and playback,
disconnects the conversational `Decode` call, and requires renewed consent via
resume. Cache loss, a bot move, transport failure, public-notice failure, model
failure, and replacement races all fail closed. `/voice leave` is available to
a manager or someone in the channel and synchronously invalidates work before
disconnecting.

`/voice verify start|report` is a separate local-mode acceptance surface limited
to the server owner or an administrator. One armed run spans successful
activation, participant-change pause, a fresh consent-epoch resume, and final
leave. It retains only aggregate counts and epochs in process memory. It never
stores participant ids, audio, transcripts, responses, or message content, and
it disables the ordinary completed-turn conversation commit while armed. Its
redacted report observes code/runtime milestones but leaves unanimous consent
and audible human reception as explicit manual witness facts.

A conversation is permitted only while Abbey has View Channel, Send Messages,
Connect, and Speak and is not server-muted, server-deafened, or suppressed.
Permissions are fetched again after slow model preflight and on both sides of
the activation transition. Channel overwrite, role update/deletion, and
current-bot member-role events re-evaluate the effective permissions and close
the media epoch before teardown if the call could become receive-only.

`ABBEY_VOICE_AUTOJOIN=1` is a separate operations presence path. It always
constructs Songbird in `DecodeMode::Pass`, mutes and self-deafens before/after
joining, installs no receive/playback actor, and is valid even when local or
cloud conversation is configured. This keeps safe presence restart-resilient
without treating deployment as consent.

The code does not ingest Discord Go Live video. Realtime supports audio plus
discrete image inputs, not video; Songbird's supported receive API is voice and
RT(C)P. Stream understanding therefore remains a separate future slice that
must define an explicit consented screenshot source and retention policy.

## Configuration

Required to opt in to the Discord connection:

- `ABBEY_VOICE_GUILD_ID`
- `ABBEY_VOICE_CHANNEL_ID`

`ABBEY_VOICE_MODE` is `local` by default, `disabled` for presence-only, or
`openai` for the explicit backup. Local defaults are loopback port 8181,
`mlx-community/whisper-large-v3-turbo-asr-fp16`,
`mlx-community/Kokoro-82M-bf16`, voice `af_heart`, language `en`, and a required
Abbey/Aviva/Abi wake name. `ABBEY_VOICE_LOCAL_*` overrides those values.
`ABBEY_VOICE_AUTOJOIN` controls only safe presence. OpenAI mode alone consumes
`OPENAI_API_KEY`, `ABBEY_VOICE_REALTIME_ENDPOINT`, `_MODEL`,
`ABBEY_VOICE_NAME`, and `ABBEY_VOICE_INSTRUCTIONS`.
The implementation appends non-overridable control guidance to provider
instructions: Realtime speech does not itself change Discord voice state.

Canonical local reasoning uses `ABBEY_BOT_LLM_MODEL`; vision uses the same
OpenAI-compatible seam unless independently overridden. The current
cross-platform model name and deployment intent is `gemma4:12b`. This latest
operator selection supersedes both the interim `gemma4:e4b` choice and the
2026-08-19 benchmark recommendation of `gpt-oss:20b`. Dated gpt-oss, e4b, and
12b measurements and the existing private audition remain historical evidence,
not a claim about the pending deployment.

The endpoint contract stays portable across macOS, Linux, and Windows. Linux
and Windows may use a verified Ollama, llama.cpp, or equivalent
OpenAI-compatible runtime. macOS may use MLX acceleration only after the exact
reasoning, tool-calling, and vision interfaces pass Abbey's validation. Apple
`fm serve` is an optional macOS OpenAI-compatible adapter, not the
cross-platform default and not an installed-service claim. Nothing in this
design claims unverified MLX Gemma multimodal or tool support.

## Verification contract

The offline gate covers fail-closed mode/endpoint policy, key redaction,
participant/media epochs, stale-session cancellation, bounded PCM/segmentation,
wake routing, spoken-text shaping, raw PCM codec registration, local client
validation, OpenAI backup protocol handling, formatting, strict Clippy, tests,
and a locked release build. Beyond the sidecar smoke, the owner-only private
audition ran Kokoro synthesis → Whisper recognition → canonical read-only Abbey
cognition → Kokoro synthesis → Whisper recognition and promoted the final
24 kHz mono PCM through Songbird's playable input path. Its final transcript
had 100% word recall. This proves the complete local speech/cognition/playback
format chain without Discord, a microphone, production state, or cloud
credentials; it is not a deployment, transport, or subjective voice-quality
claim.

Earlier deployed builds independently proved persistent and crash-recovered
muted/self-deafened `DecodeMode::Pass` presence in Engineering, plus a removed
one-shot output greeting. Durable evidence also records `/voice status`, an
automatic participant-change pause, and a successful manager `/voice leave`;
the later process had no voice UDP socket. A manager resume was recorded in an
older session, but the exact participant set was not captured, so it is not
used as all-participant acceptance evidence.

Completion of the current candidate still requires deploying the exact gated
build, observing safe-presence rejoin without media activity, collecting a
fresh everyone-present consent epoch, and observing a refreshed manager resume,
an audible wake-name/reply heard by a human, and barge-in. The owner/admin-only
in-memory verifier makes decoded receive, local STT, synthesized playback end,
actual playback cancellation, consent-epoch change, participant pause/resume,
and final leave independently reportable without retaining content. Until
those steps, the voice goal remains in progress. Stream vision remains a
separate consented slice.

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
