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

## Current executable qualification profile

The command-workflow release preserves the configured legacy generation and
vision routes and the existing `--provider-self-test primary|fm|all --json`
interface. The primary report can establish synthetic text, streaming,
structured-output, tools, image-description and OCR results for the exact
Abbey binary and configured route. It binds the endpoint and model selection,
but does not itself attest immutable model bytes. The checked-in publisher
emits version-1 qualification reports; a primary probe does not qualify an
unconfigured FM route.

For local-only release probes, explicitly select numeric loopback endpoints
and omit cloud credentials from the probe environment. An empty generic
`ABBEY_PROVIDER_CLOUD_ALLOW` does not override legacy route authorization.
Opening help remains observational and never runs these probes.

The current launchd transaction stages and rolls back the binary and plist.
It preserves the owner environment and does not atomically replace models or
provider manifests. Consequently, passing that transaction and the primary
synthetic probes does not satisfy the broader immutable-model, sandbox,
version-2 publication or complete identity-transaction requirements in stages
2 and 5 below. Record those requirements as unverified when their specified
evidence is unavailable; do not synthesize a manifest or expand provider
adapters to label this workflow release complete.

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

## 3a — guided command workflows

Exercise the reviewed command-workflow release as an ordinary member and as a
manager in both sandbox guilds. Verify the exact running artifact and actual
registration scope first. Guild-only registration does not establish that the
same commands are available in DMs. Record each role/workflow result separately;
a manager completing a task does not establish ordinary-member authorization.

| Help task | Required observation |
|---|---|
| Conversation | The button opens the question modal promptly. Blank input receives actionable guidance. A synthetic valid question produces a private provider answer or a specific provider failure, without publishing a shared-channel transcript. |
| Memory | The button opens the existing private fact browser. Navigation reads the current authorized subject snapshot; another user's control, another guild, or a lost permission cannot expose facts. |
| Images | The view names the exact attachment slash commands and image message-menu actions, distinguishes description from OCR availability, and explains any policy/provider blocker. Exercise both operations with synthetic images. |
| Voice & Music | The view reports the current state and supported commands without starting capture or playback. Unconfigured voice still permits status. Active status includes wake-name and stop guidance; audible behavior belongs to stage 4. |
| Administration | An authorized manager opens the existing dashboard. Ordinary members cannot gain administration through a button or slash command. Requested settings and effective blockers appear together. Restore every setting changed during the test. |

Reopen `/help` after an expired control and confirm that its recovery action
works. Recheck a control after permission revocation and in a different guild or
channel. Verify that button and slash entry points enforce the same authority.
An accepted operation must end in a result, a truthful partial result, an
explicit cancellation, or an actionable failure. A response-delivery failure
after a successful mutation does not authorize repeating that mutation; verify
the resulting state before any deliberate retry. Source fault-injection tests
cover failures that cannot be induced safely in these live sandbox runs; retain
their evidence separately from observed Discord behavior.

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

## Managed lifecycle and status observation

This section describes the managed source contract, not a record of live
acceptance. The earlier modernization WDBX-required strict gate passed on tested source
`ff5d594`: 1,183 Rust tests passed with zero failures and four intentional live
ignores, alongside 26 offline installer tests, all-target locked Clippy and
the locked Rust release build and offline Swift release build. Deployment/Python, privacy, 81-artifact contracts,
required WDBX, TLS, module-size and Swift checks passed. The RustSec policy
retains four accepted vulnerabilities and three unmaintained warnings; this
is not a clean audit. Those counts do not validate the later command-workflow
redesign. Stages 0–6 require their own evidence, recorded by tested revision in
canonical `.superpowers/completion-20260906/delivery.json`.

The launchd installer validates private environment syntax before stopping the
prior service. It installs direct `--managed-service` arguments and accepts a
replacement only when its installed binary hash, current launchd PID and fresh
run identity match a valid readiness document. Discord and the scheduler must
be ready for five continuous seconds of observations within one 30-second budget
that includes bootstrap. Restoring a prior binary requires a fresh transaction
and the same readiness checks. Failed child cleanup or unconfirmed rollback
retains the install lock and recovery material; do not interpret a retained lock
as permission to bypass those checks. Uninstall removes only the managed plist
and same-identity readiness/bootstrap records, preserving binary, data,
environment, recovery material and logs.

`python3 -I deploy/service-status.py` performs one read-only current observation.
Exit 0 means ready, 1 means unavailable/not ready, and 2 means usage or unsupported
platform. It neither writes nor probes providers, and displays no PID, nonce,
hash or raw log content. A successful observation does not establish the
installer's five-second stability interval or any provider/voice acceptance.

The managed readiness owner refreshes starting, ready and draining state at
least every ten seconds; observed Discord reconnects publish non-ready state
without waiting for that interval. Ready requires the committed canonical
privacy rewrite, a running scheduler, Discord Ready, completed registration and
applied presence. The root-retained refresh owner also runs during slow credential, state and
framework initialization. Its paused-time regression passed in the integrated suite.

On shutdown the root closes work and media admission, then budgets voice
teardown, Discord shard shutdown, task cancellation/reaping and final persistence
as four stages of at most five seconds each within one 20-second deadline.
Dropping a waiter is not proof that a child, blocking write or actor stopped.
Unfinished work stays owned through the terminal runtime boundary; incomplete
cleanup does not produce a clean-exit claim. Final persistence runs at most once
after quiescence, and canonical/projection outcomes remain distinct.

Managed operational JSONL uses a private directory, owner-only files, bounded
lines and an 8 MiB rotation limit with at most five archives. It contains closed
operational categories rather than raw diagnostics, content or identities.
Queue acceptance is not a durability receipt. The implemented shutdown producer
uses `ShutdownFinalizing`/`Started` before final cleanup; it must not describe
that row as completed shutdown. The root's final shutdown report can attest
completion only after observed joins and component outcomes. A log row cannot
prove its own writer's future retirement. These changes passed independent review
and the fresh strict source gate on
`ff5d594`. Historical legacy logs are not rewritten.

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
