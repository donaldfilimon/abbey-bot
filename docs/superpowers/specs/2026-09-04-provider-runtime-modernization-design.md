# Provider Runtime Modernization Design

**Date:** 2026-09-04
**Status:** Approved and binding
**Scope:** Pure provider scoring and circuit policy followed by one production
provider runtime with conversation-local ownership and effect-aware fallback
**Implementation plan:**
`docs/superpowers/plans/2026-09-04-abbey-bot-full-modernization.md`

## Objective

Replace the current partially integrated provider catalog/router and the
separate primary/Foundation Models execution paths with one runtime authority.
Selection must be explicit, deterministic, content-free, testable under an
injected clock, and unable to replay a conversation after Abbey has created an
externally visible or mutating effect.

The modernization proceeds in two stages. First, correct and fully test the
pure router without wiring it into production. Second, make `ProviderRuntime`
own provider execution and migrate generation, tools, vision, voice, and safe
inspection through it while temporarily retaining crate-private compatibility
accessors. This document supersedes conflicting scoring, global-stickiness, and
implicit-normalization details in the 2026-09-02 provider-routing design; its
configuration, isolation, and manifest privacy boundaries remain compatible.

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

## Preserved Provider and Application Contracts

- `ChatTurn`, `ModelTurn`, `ToolSpec`, and existing generation entry points
  remain the application-facing vocabulary during migration.
- Production exposes exactly these seven tools, in this order:
  `remember_fact`, `lookup_reputation`, `recall`, `switch_persona`,
  `recent_messages`, `inspect_status`, `list_facts`.
- The original five-Core-tool `abbey_tools()` corpus and its wire fixtures
  remain byte-compatible. Inspect remains additive, and
  `ABBEY_BOT_LLM_TOOLS=off` suppresses the complete vocabulary.
- Abbey remains independent of ABI and keeps its compatibility
  transcriptions.
- The existing `--provider-self-test primary|fm|all --json` grammar, exit-code
  meanings, version-1 report fields, and content-free failure categories remain
  accepted.
- Existing provider qualification manifests remain readable under their
  current ownership, mode, type, size, identity, and privacy checks. A new
  writer may migrate a schema only with an atomic publication path and a
  compatibility reader; no task may reinterpret an old record as broader
  capability evidence.
- Runtime selection remains configuration-driven. Ambient credentials,
  detected binaries, listening ports, or installed applications never make a
  provider eligible by themselves.

## Domain Model

The corrected pure layer owns:

- `NormalizedScore`: a finite number in the inclusive range `[0, 1]`.
- `ProviderScoreProfile`: qualification baselines plus live quality,
  reliability, latency, and locality scores and the count of comparable live
  observations for one request class.
- `ProviderFailureKind`: the closed failure classification below.
- `CircuitPhase`: `Closed`, `Open`, `HalfOpen`, or `Blocked`.
- `CircuitSnapshot`: phase, content-free reason, opening escalation level,
  open-until time, and whether the one half-open probe is reserved.
- `RouteDecision`: selected provider ID, final normalized component values,
  weighted score, and deterministic tie position.
- `RouteUnavailableReason`: `NoConfiguredProvider`, `CapabilityUnavailable`,
  `PolicyDenied`, `AllOpen`, `BlockedPendingRequalification`, `Busy`, or
  `BudgetExhausted`.

`ProviderRuntime` owns the provider catalog, eligible adapters, pure router,
qualification state, capacity controls, and privacy-safe inspection view.
`ProviderConversation` owns one conversation's selected provider, fallback
budget, and effect state. The router itself owns no conversation pin.

All provider IDs are stable normalized identities. Descriptor and inspection
views contain only normalized IDs, provider class, effective capability
categories, eligibility, and safe provenance. They contain no secrets,
endpoints, paths, models, prompts, outputs, raw provider errors, or session
data.

## Hard Eligibility Before Scoring

Every candidate is filtered before any score is computed. A route is eligible
only when all of these hold:

1. it was explicitly configured or explicitly enabled through the preserved
   provider configuration contract;
2. its exact required qualification/manifest identity is current;
3. it supports the request's capability and effect policy;
4. cloud, agent-CLI, sandbox, network, and operator allow/deny policy permits
   it;
5. its monetary budget and concurrency capacity permit the turn;
6. its circuit is neither Open nor Blocked, except for the single successfully
   reserved HalfOpen probe.

Open and Blocked providers are excluded, not assigned score zero. This rule
applies when the provider is the only candidate and when it was previously
pinned by a `ProviderConversation`. The router returns a typed unavailable
reason rather than silently selecting it.

No real Discord traffic performs exploration. Synthetic qualification and
normal eligible request selection are the only inputs.

## Explicit Normalized Scoring

`NormalizedScore::new` rejects negative values, values greater than one, NaN,
and positive or negative infinity. Larger always means better. The router does
not derive a score from raw milliseconds, capability count, provider class,
price, endpoint shape, or any other operational value.

For one eligible candidate, the exact score is:

```text
score = 0.40 * quality
      + 0.30 * reliability
      + 0.25 * latency
      + 0.05 * locality
```

Each of the four inputs is supplied as an already normalized, content-free
qualification or live outcome. Producers define and test their own conversion
from bounded operational evidence; the router does not add a hidden
normalization or capability-density preference. Monetary cost remains a hard
allowlist/budget constraint, not a score.

Profiles are maintained per provider and comparable request class so, for
example, tool validity is not treated as image quality. Live outcome EWMAs use
`alpha = 0.2`. Each component blends its qualification baseline `q` with its
live EWMA `l` using:

```text
n = min(comparable_live_observations, 20)
component = q * (1 - n/20) + l * (n/20)
```

With zero comparable observations the qualification score is used exactly. At
20 or more, the live EWMA is used exactly. An outcome excluded by the failure
policy does not increment `n` or change an EWMA.

Higher weighted score wins. Exact ties use the operator's configured provider
order; a provider absent from that list sorts after every listed provider.
Remaining ties use stable lexical `ProviderId`. Floating comparison is over the
same validated deterministic calculation on all candidates; tests include
boundary values and exact ties.

## Failure Classification

`ProviderFailureKind` has these policy groups:

- **Success:** records latency and the bounded success/quality outcomes for the
  request class, updates applicable EWMAs, and closes a HalfOpen circuit.
- **Transient:** transport unavailable, timeout, bounded 5xx/overload, or a
  validated provider retry response. It updates failure/reliability evidence
  and the circuit window.
- **Blocked until requalification:** authentication/authorization,
  configuration, executable or model identity, sandbox identity, tool schema,
  response schema, and protocol drift. It immediately enters `Blocked` and is
  not reopened by elapsed time.
- **Neutral:** caller cancellation, invalid request, and local busy/capacity
  rejection. It changes neither circuit state nor latency/quality/reliability
  EWMA and does not increment the comparable observation count.

A `Retry-After` duration is valid only from one second through 15 minutes,
inclusive. A valid value opens the provider until that deadline without
exceeding the 15-minute cap. Zero, sub-second, greater-than-15-minute,
malformed, negative, non-finite, or overflowed values are rejected as protocol
drift and block the exact qualification identity until requalification. The
runtime never sleeps for an unbounded provider-supplied interval.

Raw failure strings are adapter-local diagnostics only. Routing, inspection,
structured events, persistence reports, and readiness carry fixed categories.

## Circuit State Machine

All transitions take an injected monotonic `now`; pure routing code never
reads `SystemTime`, `Instant::now`, or `rand`.

### Closed

Transient failure timestamps are retained in a rolling five-minute window,
including a timestamp exactly 300 seconds old. The third transient failure in
that window transitions to `Open` for 60 seconds and clears the collected
window. Success while Closed updates metrics and prunes expired failure
timestamps but does not erase still-in-window transient failures; the threshold
is three failures in time, not three consecutive attempts. Neutral outcomes do
nothing.

### Open

An Open provider is ineligible. Before `open_until` it cannot be selected or
reserved. At or after `open_until`, it transitions to `HalfOpen` on the first
reservation attempt.

### HalfOpen

Exactly one caller may atomically reserve the probe. Concurrent callers see the
provider as temporarily unavailable. A probe success closes the circuit,
clears failure history, and resets escalation. A transient probe failure opens
for five minutes after the initial 60-second opening; the next and every later
consecutive failed probe opens for 15 minutes. Fifteen minutes is the cap. A
Blocked failure enters Blocked immediately. Cancellation or invalid request
releases the reservation without treating the provider as healthy or failed;
the circuit remains HalfOpen for a later probe. A local busy result also
releases it.

### Blocked

Blocked candidates remain ineligible until the exact provider identity is
requalified or the invalid configuration/policy identity is replaced. A timer,
process restart, configured-order position, lack of alternatives, or prior
conversation pin cannot clear Blocked.

Circuit state is runtime operational state, not provider-manifest evidence.
Any persistence of it must remain content-free and may never turn a stale
qualification identity into an eligible provider.

## Conversation-Local Selection and Pinning

Each generation/tool request creates or receives a `ProviderConversation`.
The first turn asks the router for the best eligible provider and stores the
selected `ProviderId` in that conversation. Subsequent tool-continuation turns
use the same eligible provider. Concurrent conversations have independent pins;
ending or failing one conversation cannot alter another's selection.

There is no router-global sticky state and no public `pin`/`unpin` pair whose
correctness depends on callers serializing unrelated conversations.

The conversation tracks three irreversible effects:

- `visible_output_posted`: the first initial response, stream edit, follow-up,
  or other user-visible provider output was accepted for delivery;
- `tool_dispatched`: a model-requested host tool was handed to the tool host,
  regardless of its eventual result;
- `image_submitted`: image bytes were submitted to a provider.

The flags are set at the effect boundary, before waiting for the effect result,
so cancellation cannot create a replay window.

## Effect-Aware Fallback

At most one provider fallback is permitted in one conversation, and only when
all three effect flags are false. The failed candidate is excluded, the router
selects the next eligible candidate using the same request requirements, and
the conversation pin moves to that provider for all remaining turns.

No fallback is allowed after the first visible stream edit or other visible
post, after any tool dispatch, or after any image submission. No fallback is
allowed merely because a pinned or sole provider became Open or Blocked; if an
effect already occurred, the conversation fails honestly. This prevents
duplicate text, repeated memory writes, duplicated external effects, and image
disclosure to a second provider.

Provider-level retries that are provably pre-effect and idempotent remain an
adapter concern, are bounded, and do not create more than the conversation's
one cross-provider fallback.

## Production Runtime Boundary

`ProviderRuntime` is constructed once and placed in `AppState`. It owns:

- provider descriptors and exact eligible adapters;
- router metric/circuit state and injected production clock adapter;
- qualification/manifest decisions;
- generation capacity and queue policy;
- adapter supervision and typed outcomes;
- the safe inspection snapshot.

Initially active production routes are limited to the currently supported
Anthropic adapter, explicitly configured OpenAI-compatible adapter, and
qualified Foundation Models routes. The broader provider catalog may describe
detected future adapters, but unsupported MLX-specialized, Ollama-native,
cloud, or agent-CLI adapters remain ineligible until their adapter,
qualification, isolation, and acceptance work is separately approved and
implemented. Detection is never activation.

Generation with tools uses a `ProviderConversation`. Unsolicited replies,
summaries, and voice remain explicitly read-only/tool-incapable. Voice may use
the runtime's eligible text generation seam but cannot construct or dispatch
the tool vocabulary. Vision remains single-provider: it selects the explicitly
configured eligible vision route and never cross-provider-falls back after
image submission.

The existing Foundation-Models-only router is renamed during migration so it
cannot be confused with the application router, then removed after all
adapters use `ProviderRuntime`. Crate-private legacy backend/fallback accessors
may exist only during the cutover and must delegate to the runtime. Provider-
wide dead-code/import allowances are removed rather than retained around
unused parallel implementations.

## Legacy Compatibility

Before production wiring, characterization tests freeze these behaviors:

- a nonblank `ANTHROPIC_API_KEY` is the legacy primary ahead of a configured
  `ABBEY_BOT_LLM_ENDPOINT`; a blank value remains unset as documented;
- an explicitly configured OpenAI-compatible endpoint remains the next primary
  and keeps the base-URL rule that Abbey appends `/v1/chat/completions`;
- with neither primary, Abbey returns the existing honest no-backend result;
- explicitly enabled and qualified Foundation Models remains a secondary under
  its existing mode/fallback/capability policy and is never ambiently selected;
- existing safe backend labels, local concurrency/queue defaults, tool-off
  behavior, vision provider selection, and platform rejection remain
  equivalent at the public boundary;
- version-1 self-test reports and existing provider manifests retain their
  current read/validation behavior;
- the seven-tool runtime order, five-tool corpus bytes, and at-most-three tool
  rounds do not change.

Legacy precedence determines the default configured order when the operator
has not supplied a newer explicit provider order. Migration must not cause a
different provider to answer an otherwise identical legacy configuration.
Compatibility does not preserve unsafe global stickiness, implicit score
normalization, fallback after effects, or selection of an Open/Blocked
provider; those are defects this design intentionally removes.

## Privacy and Inspection

Provider inspection reports only normalized provider ID, effective capability
categories, eligible/temporarily-unavailable/blocked state, fixed reason, and
`configuration` versus `qualified-manifest` provenance. It does not report
model names, endpoint URLs, executable paths, credentials, raw failures,
prompts, responses, tool arguments/results, images, Discord IDs, or conversation
pins.

Metrics retain only normalized bounded scores, counts, fixed outcomes, and
monotonic durations. Quality producers may use bounded schema/tool completion
signals but never store the prompt or output content used to derive them.

## Verification

The pure-router suite uses injected time and covers:

- every `NormalizedScore` boundary and rejection of NaN/infinity;
- exact 40/30/25/5 arithmetic, `n/20` blending, EWMA alpha, per-request-class
  separation, configured-order and `ProviderId` ties;
- hard exclusion before scoring, including sole and pinned Open/Blocked
  candidates;
- the third-failure rolling window, exact five-minute boundary, initial open,
  half-open single reservation, five/15-minute escalation/cap, success reset,
  and Blocked requalification;
- valid and invalid Retry-After values;
- neutral cancellation/invalid/busy outcomes changing no circuit or metric;
- independent concurrent conversation pins, one pre-effect fallback, and no
  fallback after visible output, tool dispatch, or image submission.

Runtime characterization and adapter tests cover legacy precedence/labels,
unqualified exclusion, tool continuation, concurrency/queue outcomes,
Foundation Models gating, vision single-provider behavior, voice read-only
behavior, safe Inspect output, tool corpus/order, and manifest/self-test
compatibility.

Fake adapters and synthetic fixtures establish source behavior only. Exact
provider qualification, installed identity, foreground Discord behavior,
connector behavior, participant-consented voice, managed-service behavior, and
real Windows runtime remain separate acceptance layers.
