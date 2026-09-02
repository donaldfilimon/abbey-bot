# Live acceptance protocol

This is an operator protocol, not evidence that it ran. Begin only after the
final provider-routing commit equals `origin/main` and Ubuntu, macOS, and
Windows CI are green for that exact SHA. A local gate, a pushed commit, hosted
CI, provider qualification, an installed artifact, foreground Discord,
consented voice, and a managed service are separate acceptance layers. Never
promote one into proof of another.

Use two operator-supplied sandbox guilds and consenting test users. Retain only
the neutral labels **Guild A** and **Guild B**; actual Discord identifiers are
transient execution inputs, not acceptance evidence. Use synthetic content and
restore both guilds to their initial settings after every run.

## Evidence and privacy boundary

Create each evidence directory mode 0700 and every evidence file mode 0600.
Use an owner-only `umask 077` before creating them. Retain only:

- commit, binary, immutable model, and manifest hashes;
- normalized provider IDs and fixed result categories;
- timestamps and PIDs verified during the run;
- aggregate counters, Guild A/Guild B role labels, and human pass/fail
  attestations.

Never retain credentials or environment values, Discord IDs, participant or
user identities, prompts, messages, replies, provider errors or provider-
controlled text, raw logs, executable/model paths, image contents, audio,
transcripts, packet captures, or generated response bodies. Inspect live state
without copying those values into the record. Record an unobserved bounded
condition as **NOT OBSERVED**; do not create unbounded traffic to force it.

## 0 — exact source and hosted state

1. Record the clean canonical `main` SHA and prove it equals `origin/main`.
2. Record the isolated strict-gate and locked-release result for that SHA.
3. Record the Ubuntu, macOS, and Windows job results whose `headSha` is exactly
   that SHA. A successful Windows CI job is source-contract evidence, not a
   real Windows runtime acceptance.
4. Do not proceed if the checkout, remote SHA, release source, or hosted job SHA
   differs.

## 1 — safe transition and exact foreground artifact

Do not trust a saved PID. Immediately before transition, re-resolve the old
manual process by PID, owner, parent PID, start time, working directory,
executable path, executable mode, and SHA-256 hash. Verify listener ownership
without reading its log. Do not inspect `launch.sh`, `run_bot.sh`, or `bot.log`,
and do not replace the executable while that process is running.

Only after every identity field matches the just-frozen observation, send
SIGINT to that exact PID. Verify graceful exit and removal of its listeners. If
the identity changes, the process does not exit cleanly, or a listener remains,
do not send a broad signal and do not escalate to TERM or KILL automatically;
preserve state and request operator intervention.

Build the exact pushed SHA with `cargo build --release --locked` in a fresh
external `CARGO_TARGET_DIR`. Record its binary hash. All later foreground and
managed checks must use that same hash. Rotate any Discord credential that
previously appeared in inherited GUI process state before loading it; never
quote or record the old or replacement value.

## 2 — provider qualification before Discord

Run this stage without Discord credentials or the production data directory.

1. Provision fresh Abbey-private model copies. Bind MLX to its fixed revision
   and import an Ollama model only by immutable digest. Foundation Models is the
   sole OS-managed exception.
2. With `ABBEY_PROVIDER_CLOUD_ALLOW` empty, prove that no cloud provider is
   eligible and that no cloud call occurs.
3. Prove agent CLIs without the approved external sandbox and attestation are
   detected but never spawned. An allowlist alone must not promote them.
4. Run synthetic, content-free qualification for the provider selected for the
   live run. Require exact binary/model/OS/tool-schema/sandbox identity,
   structured output, size limits, environment clearing, cancellation, and
   descendant cleanup to pass.
5. Publish the v2 manifest only by atomic replacement into a mode-0700
   directory with a mode-0600 regular file. Confirm it contains only normalized
   identities/hashes, capability categories, and qualification results.
6. If an immutable model digest, model artifact, sandbox attestation, explicit
   credential, or required operator allowlist is absent, fail closed and mark
   only provider qualification pending. Do not weaken the design.

An explicitly allowed cloud provider may be qualified only with an operator-
supplied allowlist entry, an explicit provider credential, and synthetic
content. Ambient credentials never authorize routing.

## 3 — foreground two-guild text, tools, policy, and vision

Launch the exact pushed release hash directly in the foreground. Verify the
Discord credential preflight reports only the selected source variable and
that the effective provider is the exact qualified provider from stage 2.

Capture the initial sandbox settings transiently. Begin with Guild A learning
and acting enabled under a small bounded budget and cooldown; leave Guild B at
the default-off policy. Then exercise:

1. A generated DM and follow-up turn, with no retained prompt or reply text.
2. All seven tools in stable order through the selected qualified provider:
   `remember_fact`, `lookup_reputation`, `recall`, `switch_persona`,
   `recent_messages`, `inspect_status`, and `list_facts`.
3. `inspect_status` for runtime, guild, provider, voice, and all. Require only
   effective routable capabilities and safe configuration-versus-qualified
   provenance. Reject endpoint, path, model, OS-build, hash, key, manifest,
   credential, or raw-error leakage.
4. `list_facts` as the bounded canonical subject snapshot, including pending
   replacements. Verify independent omitted-fact and omitted-pending counts and
   that no pending replacement is clipped into a partial value.
5. Exact user/guild isolation for memory, pending replacements, reputation,
   recent context, provider status, and the coarse voice state. A DM or the
   other guild must observe voice `off`.
6. `/see`, `/ocr`, `/webhook`, and `/forget`, plus bounded multi-round tool
   handling. Retain only result categories and permitted aggregate metadata.
7. Policy decisions, cooldown, reward settlement, and a deliberately bounded
   `OverBudget` state in Guild A. Guild B must remain silent by default and
   receive no Guild A state.

If the bounded `OverBudget` condition is not observed, record **NOT OBSERVED**
and continue without increasing traffic beyond the approved limit.

Swap the roles: restore Guild A to default-off, enable Guild B under the same
small bounds, and repeat the isolation-sensitive tool, memory, policy,
provider, budget, and voice-Inspect checks. Restore both guilds to their exact
initial settings when complete and remove all temporary facts.

## 4 — consented foreground voice

Do not begin merely because Abbey has voice presence. Publish the documented
local-processing/no-raw-retention notice and obtain fresh explicit consent from
every person currently present. Silence, ambiguous reactions, historical
consent, and one person's assertion for another do not count. Retain only the
human pass/fail attestation, never participant identities.

Have the owner or an administrator run `/voice verify start`, then have an
authorized in-channel manager invoke `/voice join consent:true`. Verify:

1. Voice Inspect moves through only the approved coarse states: `off`,
   `presence`, `awaiting-consent`, `active`, and `paused`; another guild and a
   DM remain `off`.
2. Capture opens only after the public notice, fresh unanimous consent, and
   final participant/permission checks.
3. A human witness confirms an audible wake/reply and the completed-turn
   milestone.
4. Barge-in audibly truncates playback and records its aggregate milestone.
5. A membership change immediately closes capture, playback, and STT and moves
   Inspect to `paused`; no frame from a new or unattested participant enters
   processing.
6. A new notice, fresh unanimous consent, and `/voice resume consent:true`
   create a new consent epoch before processing resumes.
7. A written stop is authoritative. `/voice leave` removes voice presence,
   media, UDP activity, and STT, with no later speech request.
8. `/voice verify report` returns the complete `observed: 8/8` lifecycle and a
   human confirms the audible result and current unanimous consent.

The report is an ephemeral content-free counter set, not proof of human
identity or consent by itself. Do not copy identities, consent epochs, audio,
messages, transcripts, replies, or raw output into evidence. If unanimous
consent or the human audible confirmation is unavailable, leave immediately,
mark voice externally pending, and stop this layer.

## 5 — atomic installation

Install the identical accepted binary hash, provider manifest, and private
model identities in one atomic transaction with a recorded rollback hash.
Verify the managed PID, executable hash, environment source category, model
identity hashes, manifest hash, and listener ownership. Validate environment
and filesystem paths in place but do not retain their raw values. If any
identity differs or startup is unstable, roll back the complete transaction;
do not mix old and new components.

Installation proves artifact identity only. It does not inherit the foreground
Discord or voice result.

## 6 — complete managed-service acceptance

Repeat the full protocol through the managed service, not an abbreviated smoke:

1. Repeat the Guild A enabled/Guild B default-off run, all seven tools,
   provider provenance, memory/pending snapshots, `/see`, `/ocr`, `/webhook`,
   `/forget`, bounded tool loops, policy, cooldown, reward, and bounded budget.
2. Swap Guild A and Guild B and repeat the isolation-sensitive subset.
3. Repeat the complete consent notice, fresh unanimous consent, audible
   wake/reply, barge-in, membership pause, renewed consent, written stop,
   final leave, and 8/8 voice lifecycle.
4. Restore both guilds to their initial settings, remove temporary facts and
   credentials, set `ABBEY_QUIET=1`, and verify no test jobs, provider
   descendants, voice presence, media, or temporary listeners remain.

Only this successful repeat is managed-deployment acceptance for the installed
hash. A later binary, manifest, model, OS, sandbox, or configuration change
requires the affected layers to run again.

## 7 — independent connector and Windows status

Telegram and Slack share source paths but need their own explicitly authorized
native round trips before being called live-qualified. If credentials are not
operator-supplied, record each as pending without reading ambient stores.

Windows CI proves compile/test/Job Object contracts only. Record Windows live
provider and managed-runtime acceptance as pending unless this complete
protocol runs on a real Windows host.
