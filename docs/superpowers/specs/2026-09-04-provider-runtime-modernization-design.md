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
- `RequestClass`: `TextReadOnly`, `TextWithTools`, `VisionDescribe`, or
  `VisionOcr`. Voice, summaries, and unsolicited generation are
  `TextReadOnly`; tool-capable mentions/DMs/`/persona ask` are
  `TextWithTools`; the two vision operations remain distinct.
- `ProviderScoreProfile`: qualification baselines plus live quality,
  reliability, latency, and locality scores and the per-component comparable
  live-observation counts for one request class.
- `ScoreProducerPolicy`: the single versioned conversion from validated
  qualification/live evidence to `ProviderScoreProfile` inputs.
- `ProviderFailureKind`: the closed failure classification below.
- `RetryAfter`: `Absent`, bounded `Valid`, or `Invalid` provider delay
  metadata with the compatibility rules below.
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

Each of the four inputs is supplied by `ScoreProducerPolicy::V1` as an already
normalized, content-free qualification or live outcome. Adapters cannot supply
scores directly and the router cannot add another conversion, capability-
density preference, or provider-class prior. Monetary cost remains a hard
allowlist/budget constraint, not a score.

### Score Producer V1

The request-class partition and conversion below are production policy, not
examples. A score profile is keyed by exact `(qualification identity,
RequestClass)`, and evidence from one class never updates another.

Qualification performs exactly five end-to-end attempts for each advertised
class. A class is eligible only if at least four attempts succeed and every
mandatory validity check below is observed at least once. Its initial baseline
is then:

```text
quality_q    = passed mandatory checks / mandatory check count
reliability_q = successful attempts / 5
latency_q    = latency_score(class, nearest-rank p95 of successful attempts)
locality_q   = locality_score(validated execution locality)
```

Because all mandatory checks are required for eligibility, initial
`quality_q` is exactly `1.0`; `reliability_q` is exactly `0.8` or `1.0`.
Nearest-rank p95 sorts successful monotonic millisecond durations and selects
index `ceil(0.95 * count) - 1`. Zero successful attempts cannot qualify.

| Request class | Mandatory validity checks | Fast ms | Slow ms |
|---|---|---:|---:|
| `TextReadOnly` | schema-valid response; nonempty bounded text; no tool request | 1,000 | 30,000 |
| `TextWithTools` | schema-valid tool request; exact offered name/argument shape; accepted tool-result continuation; nonempty bounded terminal text | 1,500 | 45,000 |
| `VisionDescribe` | bounded image accepted; schema-valid nonempty description | 2,000 | 60,000 |
| `VisionOcr` | bounded image accepted; schema-valid bounded OCR result, including an explicit empty-text success | 2,000 | 60,000 |

For a finite monotonic duration `d_ms`, using the table's `fast` and `slow`:

```text
latency_score = 1.0                              when d_ms <= fast
latency_score = 0.0                              when d_ms >= slow
latency_score = (slow - d_ms) / (slow - fast)   otherwise
```

The subtraction is checked integer arithmetic and the final division is a
validated deterministic `f64` conversion. An absent, negative, overflowing, or
non-finite duration is not score evidence and makes a qualification attempt
fail; production timeout remains classified separately.

Validated execution locality is a closed manifest value, not inferred again
at route time:

| Locality | Definition | Score |
|---|---|---:|
| `SameHost` | in-process, OS framework, Unix socket, IPv4 `127.0.0.0/8`, or IPv6 `::1/128` endpoint | `1.0` |
| `PrivateNetwork` | every resolved address is in IPv4 `10/8`, `172.16/12`, `192.168/16`, or IPv6 `fc00::/7` | `0.5` |
| `PublicRemote` | every other explicitly configured remote service | `0.0` |

Redirects cannot improve locality: the least-local validated hop wins. A
hostname whose addresses span classes uses the least-local class. Ambient
endpoint discovery remains ineligible.

Live evidence has these exact mappings:

- a schema-valid successful completed attempt contributes quality `1.0`,
  reliability `1.0`, and its class-specific latency score;
- a Transient failure contributes reliability `0.0` only;
- a Blocked failure contributes reliability `0.0` before the identity is
  blocked, and contributes no quality or latency sample;
- Cancelled, InvalidRequest, and Busy contribute no component sample and do
  not increment any observation count;
- locality is immutable qualification evidence and has no live EWMA.

Quality, reliability, and latency therefore have independent EWMA values and
independent comparable counts. The `n` in the blend is the count for that
component, not a shared attempt count. All live EWMAs use `alpha = 0.2` and
process outcomes in completion order under the runtime's serialized metrics
update.

Version-2 qualification manifests add integer `score_policy` exactly `1` and
array `score_profiles`. Each object has exactly five keys:
`request_class` (the lower-snake-case enum string), `successful_attempts`
(integer `4` or `5`), `mandatory_check_mask` (integer),
`successful_duration_ms` (integer array whose length equals
`successful_attempts`, each item `0..=900000`), and `locality` (`same_host`,
`private_network`, or `public_remote`). The complete masks are
`text_read_only = 0b0111`, `text_with_tools = 0b1111`,
`vision_describe = 0b0011`, and `vision_ocr = 0b0011`; any missing or extra bit
rejects the class. Entries are unique and sorted in enum order when written.
Missing, duplicate, unknown, null, wrong-type, out-of-range, or
class-incompatible fields reject that class. Writers always use enum order;
readers accept entry order but still reject duplicates.

Existing version-1 manifests stay readable and retain their eligibility
meaning; their deterministic compatibility projection is `quality_q = 1.0`,
`reliability_q = 1.0`, `latency_q = 0.5`, and the locality score derived once
from their already validated configured execution boundary using the exact
address table above. A v1 record never gains a request class it did not already
qualify.

Shared table fixtures pin all request-class partitions, mandatory masks,
nearest-rank selection, boundary/interior latency values, locality cases,
legacy-v1 projection, manifest rejection cases, and live outcome mappings.
The qualification writer, compatibility reader, runtime metrics producer, and
router tests consume the same fixture; a second adapter-local normalization is
a test failure.

Profiles are maintained per provider and request class so, for example, tool
validity is not treated as image quality. Each live component blends its
qualification baseline `q` with its live EWMA `l` using:

```text
n = min(component_comparable_live_observations, 20)
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

`ProviderFailureKind` is a closed enum with these exact variants:

- **Success:** `Success`.
- **Transient:** `TransportUnavailable`, `Timeout`, `Http5xx`, and
  `RateLimited`.
- **Blocked until requalification:** `Authentication`, `Authorization`,
  `Configuration`, `ExecutableIdentity`, `ModelIdentity`, `SandboxIdentity`,
  `ToolSchema`, `ResponseSchema`, and `ProtocolDrift`.
- **Neutral:** `Cancelled`, `InvalidRequest`, and `Busy`.

Success records the bounded class outcome and closes only the reserved
HalfOpen probe. Transient outcomes update reliability and circuit policy.
Blocked outcomes immediately block the exact qualification identity. Neutral
outcomes change neither circuit state nor any EWMA/count.

`RetryAfter` is `Absent`, `Valid(duration)`, or `Invalid`. `Valid` is accepted
only with `Http5xx` or `RateLimited`, and only from one second through 15
minutes inclusive. A syntactically present zero, sub-second,
greater-than-15-minute, malformed, negative, non-finite, or overflowed value is
`Invalid`. A valid value attached to any other outcome is also protocol drift.
Every invalid/incompatible combination becomes `ProtocolDrift` and blocks the
exact qualification identity until requalification. The runtime never sleeps
for an unbounded provider-supplied interval.

Raw failure strings are adapter-local diagnostics only. Routing, inspection,
structured events, persistence reports, and readiness carry fixed categories.

## Circuit State Machine

All transitions take an injected monotonic `now`; pure routing code never
reads `SystemTime`, `Instant::now`, or `rand`.

The following is the complete transition table. `recent` is the Closed rolling
failure history after pruning timestamps older than 300 seconds; a timestamp
exactly 300 seconds old is retained. `level` is zero after an initial Closed
opening, one after the first failed HalfOpen probe, and two after the second;
it remains capped at two. `RA` is the validated duration.

| Current phase | Outcome | `RetryAfter::Absent` | Compatible `RetryAfter::Valid(RA)` | History/escalation effect |
|---|---|---|---|---|
| Closed | Success | remain Closed | impossible; block as `ProtocolDrift` | prune `recent`; update success metrics; do not erase retained failures |
| Closed | Transient | append `now`; remain Closed for counts 1–2, or Open until `now + 60s` at count 3 | append `now`, then Open immediately until `now + max(RA, 60s if count reached 3 else 0s)` | opening clears `recent` and sets `level = 0`; non-opening retains it |
| Closed | Blocked kind | enter Blocked | impossible; block as `ProtocolDrift` | clear `recent`; escalation is irrelevant while Blocked |
| Closed | Neutral | remain Closed | impossible; block as `ProtocolDrift` | no history, escalation, or metric change |
| Open | Success from an already in-flight request | remain Open to current deadline | impossible; block as `ProtocolDrift` | success metrics may update; never closes or shortens Open |
| Open | Transient from an already in-flight request | remain Open to current deadline | extend to `max(current open_until, now + RA)` | no Closed history change; `level` unchanged |
| Open | Blocked kind | enter Blocked | impossible; block as `ProtocolDrift` | clear Closed history |
| Open | Neutral from an already in-flight request | remain Open | impossible; block as `ProtocolDrift` | no state or metric change |
| HalfOpen, reserved probe | Success | enter Closed | impossible; block as `ProtocolDrift` | clear history, release probe, set `level = 0` |
| HalfOpen, reserved probe | Transient | Open for 5m when `level = 0`, otherwise 15m | Open for `max(normal escalation duration, RA)`, capped at 15m | clear history, release probe, increment `level` with cap two |
| HalfOpen, reserved probe | Blocked kind | enter Blocked | impossible; block as `ProtocolDrift` | clear history and release probe |
| HalfOpen, reserved probe | Neutral | remain HalfOpen | impossible; block as `ProtocolDrift` | release probe; no history, escalation, or metric change |
| Blocked | any outcome from stale in-flight work | remain Blocked | remain Blocked | no history/escalation change; only explicit requalification can exit |

`RetryAfter::Invalid` is not a separate table column because it has one result
for every phase and original outcome: classify `ProtocolDrift`, enter or remain
Blocked, clear Closed history, and release any reserved probe. A compatible
valid Retry-After therefore opens early and may extend a normal duration, but
can never shorten the 60-second/5-minute/15-minute policy or exceed 15 minutes.
Checked monotonic addition saturates only at the representable clock maximum;
the duration itself is always bounded.

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

Cross-provider fallback eligibility is exact:

| Outcome | Pre-effect cross-provider behavior |
|---|---|
| `TransportUnavailable`, `Timeout`, `Http5xx`, `RateLimited` | eligible for the one fallback after applying circuit/Retry-After policy |
| any Blocked-until-requalification kind | eligible for the one fallback after blocking the failed identity |
| `Busy` returned after selection because capacity raced | eligible for the one fallback; no circuit or metric update |
| `Cancelled` | terminal cancellation; never submit the request elsewhere |
| `InvalidRequest` | terminal caller/request error; never submit the invalid request elsewhere |
| `Success` | complete normally; no fallback |

Route-level `NoConfiguredProvider`, `CapabilityUnavailable`, `PolicyDenied`,
`AllOpen`, `BlockedPendingRequalification`, `Busy`, and `BudgetExhausted` are
terminal typed unavailable results after the router has considered every
candidate; they do not recursively start another routing pass. If the one
fallback routing pass finds no candidate, its exact `RouteUnavailableReason`
is the conversation result.

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
- the shared V1 score-producer fixtures, exact manifest fields/masks, legacy
  projection, quality/reliability event values, locality mapping, latency
  thresholds/formula, nearest-rank p95, component-local counts, and
  per-request-class separation;
- exact 40/30/25/5 arithmetic, `n/20` blending, EWMA alpha,
  configured-order and `ProviderId` ties;
- hard exclusion before scoring, including sole and pinned Open/Blocked
  candidates;
- the third-failure rolling window, exact five-minute boundary, initial open,
  half-open single reservation, five/15-minute escalation/cap, success reset,
  and Blocked requalification;
- every row of the phase/outcome/Retry-After table, including invalid/header-
  incompatible blocking, early open, extension without shortening, delayed
  in-flight outcomes, history clearing, and escalation effects;
- neutral cancellation/invalid/busy outcomes changing no circuit or metric;
- independent concurrent conversation pins, the complete pre-effect fallback
  outcome table, terminal cancellation/invalid requests, and no fallback after
  visible output, tool dispatch, or image submission.

Runtime characterization and adapter tests cover legacy precedence/labels,
unqualified exclusion, tool continuation, concurrency/queue outcomes,
Foundation Models gating, vision single-provider behavior, voice read-only
behavior, safe Inspect output, tool corpus/order, and manifest/self-test
compatibility.

Fake adapters and synthetic fixtures establish source behavior only. Exact
provider qualification, installed identity, foreground Discord behavior,
connector behavior, participant-consented voice, managed-service behavior, and
real Windows runtime remain separate acceptance layers.
