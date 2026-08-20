# Live acceptance protocol

This is an operator protocol, not evidence that it ran. Record a date, commit,
installed binary hash, model/CLI identity, PID/listener ownership, and each
observed result in a separate acceptance record. The 2026-08-19 observations in
`tasks/goals.md` remain historical and do not qualify a replaced binary,
provider, model, or OS build. Never record credentials, prompts, transcripts,
provider response bodies, image contents, or raw audio.

Use an isolated Discord test guild/user scope. Keep `ABBEY_QUIET=1` and guild
`act off` except during the explicit policy exercise. Non-model commands have a
5-second observation window; provider turns use the configured timeout. Record
anything not seen by its deadline as **NOT observed**, with only a fixed failure
category and the last safe operational log line.

## 0 — source, provider, and installed identity

Treat each row as a separate acceptance layer:

1. Record the clean `main` commit and successful Ubuntu, macOS, and Windows CI
   runs. This is source evidence only.
2. Run the installed release binary with
   `--provider-self-test primary --json`, then `fm`, then `all`. Use synthetic
   fixtures and no Discord token or production data directory. Require exit 0
   for every capability required by the selected runtime configuration and
   retain the redacted JSON. Optional FM image failures remain explicit `fail`
   evidence and must stay unavailable at runtime; they do not invalidate a
   text-only FM secondary. This is provider qualification only.
3. For FM `system`, publish the resulting manifest atomically to an owner-only
   regular file with `deploy/publish-provider-qualification.py`, then verify its
   Abbey binary hash, `/usr/bin/fm` hash, OS build, mode, and fixture version
   match the running service. The publisher is POSIX-only; Windows performs
   syntax/static validation and records the publisher runtime test as skipped,
   not as publication evidence. Do not probe PCC.
4. Record the installed Abbey/MLX-Audio/MLX-VLM artifact hashes, launchd PIDs,
   exact MLX-VLM snapshot, effective KV limit, and processes owning
   `127.0.0.1:8181` and `127.0.0.1:8282`. Restart offline and require the same
   installed identities and loopback-only listeners.
5. Verify the deployed bot reaches the Discord gateway with persistent state
   checksums unchanged. Confirm autojoin, if configured, is muted,
   self-deafened `DecodeMode::Pass` with no receive/playback actor and no
   conversational voice UDP activity. This still is not consent to capture.
   On Linux/Windows, verify local voice configuration is rejected and use
   `disabled` or explicitly configured OpenAI Realtime; no destination remains
   the valid default-off configuration.

If staging, self-test, hash, listener, persistence, or gateway verification
fails, stop and restore the previous complete environment/artifact set. Do not
continue from a partial cutover.

## A — DM and all five tools

1. DM `hi abbey, what can you do here?` and require a generated reply-to from
   the configured MLX-VLM primary.
2. DM `and what did I just ask you?` and require turn-one context.
3. Exercise `remember_fact`, `lookup_reputation`, `recall`, `switch_persona`,
   and `recent_messages` through model requests. Verify exact subject/guild
   scoping, authorization, and tool-result continuation.
4. Confirm the remembered test fact appears in both canonical recall and its
   semantic projection, then remove it and confirm it is absent from both.
5. React to Abbey's reply and observe only the typed reward outcome/counters.
6. Confirm a second DM user cannot retrieve the first user's fact or context.

## B — guild commands and persistence

1. `/stats` records counters and selected provider labels without credentials.
2. `/admin show` confirms persona, learning, vision, cooldown, act, and budget.
3. Exercise self-scoped `/remember`, `/recall`, and `/forget`; then verify a
   cross-member attempt is rejected without the required invocation-time
   permission and succeeds only for an authorized manager.
4. `/persona ask` as Abbey and Aviva must retain the requested persona header
   and report the provider that actually answered.
5. `/summarize` before captured channel context gives the honest empty result;
   after context exists it produces and persists a summary.
6. Flush, restart, and confirm the intended state survives with unchanged
   caller/guild scopes. Remove every temporary acceptance fact afterward.

## C — text, reward, and bounded policy behavior

1. Mention Abbey and require typing plus a reply reference; verify the primary
   MLX-VLM provider label and one pending reward.
2. React, wait through the settlement window, and confirm one settled
   experience and the expected counters.
3. Only in the sandbox guild, enable message content, remove quiet mode, set
   `act on`, and set a small budget. Verify policy decisions, cooldown, reward,
   and `OverBudget` behavior; every other guild remains `act off`.
4. Restore `ABBEY_QUIET=1` and `act off` before continuing.

## D — vision/OCR and local rejection

1. With `ABBEY_VISION_PROVIDER=remote`, run `/see` and `/ocr` using real JPEG,
   PNG, WebP, and GIF attachments. Record only pass/fail and format/size
   metadata, never image bytes or generated data URLs.
2. Require local rejection before provider invocation for truncated or
   malformed images, over-10-MB input, allocation/decompression bombs, HEIC,
   AVIF, JXL, SVG, PDF, and HTML.
3. If FM CLI vision is separately selected, first require the exact manifest
   to report both semantic shape and OCR probes passing. Selection is exclusive:
   one image must never be retried through a second provider.

## E — Telegram and Slack boundary

The ordinary gate proves source-level parity through the shared pipeline; it
does not prove a live connector. If current connector credentials and an
explicit operator test are unavailable, record Telegram and Slack as **NOT live
tested**. When they are available, run native text/persona/tool/memory/vision
round trips independently and retain their network-scoped identity evidence.

## F — human-gated voice

Do not begin this section merely because the bot is joined. Identify every
human currently present, post the local-processing/no-raw-retention notice, and
wait for each person to explicitly agree. Silence, ambiguous reactions,
historical consent, and a manager's assertion for someone else do not count.

After unanimous current consent, an authorized in-channel manager invokes
`/voice join consent:true` or `/voice resume consent:true`. Verify:

1. Public activation disclosure, a fresh media epoch, and decode opening only
   after the final participant/permission checks.
2. An audible wake-name request and human-confirmed response, with the
   completed-turn counter incrementing.
3. Barge-in immediately truncates playback and increments its counter.
4. A participant change synchronously closes the epoch, disconnects the
   conversational call, and stops STT/TTS before slow cleanup; no new or
   unattested participant frame enters STT.
5. A new notice, unanimous renewed consent, and a new manager resume create a
   distinct epoch.
6. Written `stop listening` is authoritative even if provider prose claims
   otherwise; `/voice leave` removes presence and voice UDP activity, and no
   later MLX speech request occurs.

If unanimous consent cannot be obtained, leave Abbey muted and self-deafened in
`DecodeMode::Pass`, record voice as **externally pending**, and stop. Offline
tests, a generated WAV, or an earlier consented session are not substitutes.
