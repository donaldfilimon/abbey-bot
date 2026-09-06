# Task 7 implementation report

Date: 2026-09-06. Scope: pure adaptive provider policy, qualification-score compatibility and synthetic tests. Independent review pending.

## Result

Replaced the unused generic router's implicit capability-density/class/latency preferences, wall-clock reads and router-global sticky pin with normalized score inputs, injected monotonic milliseconds, exact-identity circuits and conversation-owned fallback/effect state. The production Foundation Models router and every generation/voice/network entry point remain on their existing implementation pending Task 8.

The score producer implements four request classes, exact masks, five-attempt qualification with four/five successes, bounded successful durations, nearest-rank p95, class-specific linear latency, the exact address-locality table and least-local redirect/address aggregation. Qualification quality starts at 1; reliability starts at 0.8 or 1. Live quality/reliability/latency have independent comparable counts and EWMA(alpha=0.2), initialized by each component's first comparable sample. Blending uses that component's min(n,20)/20. Locality never has a live EWMA. Only the producer constructs profile baselines/updates; routing consumes the exact 0.40/0.30/0.25/0.05 weighted normalized components.

The injected-time circuit retains Closed failures exactly 300000 milliseconds old, opens on the third transient failure for 60000 milliseconds, reserves one HalfOpen probe, escalates failed probes to 300000 then 900000 milliseconds, and resets only on that probe's success. Earlier in-flight completions cannot consume another caller's probe or close/shorten Open. Neutral completions do not change histories, levels or metrics; neutral probe completions release that reservation. Authentication/authorization/configuration/identity/schema/protocol failures block the exact identity. Stale completions after blocking/requalification are ignored. Explicit requalification and exact-identity blocked-state restoration are separate methods for Task 8's runtime owner.

Retry-After validates outcome compatibility and the inclusive 1..900 second range, including direct Duration construction. Invalid/incompatible metadata becomes ProtocolDrift. Numeric range checks precede float-to-Duration rounding; deadline conversion rounds fractional milliseconds upward. Accepted values can open early or extend, never shorten the policy delay. Deadline arithmetic saturates at u64::MAX only.

All eligibility gates precede scoring, including sole/pinned Open/Blocked candidates and missing class score evidence. Exact score ties use configured position then ProviderId. Ranking does not reserve losing probes. No-provider results are typed. For mixed failure sets, the deterministic enum priority is NoConfiguredProvider < CapabilityUnavailable < PolicyDenied < AllOpen < BlockedPendingRequalification < Busy < BudgetExhausted; the greatest present reason is returned. ConversationRoute owns its selection, exclusion set, single fallback budget and three irreversible effect flags. Cancelled, InvalidRequest and Success never fallback; Busy, transient and blocked categories may consume one pre-effect pass.

## Manifest boundary

Frozen literal `provider-legacy-v1.json` and `provider-capability-only-v2.json` pin both previously supported wire formats independently of the live Rust struct. Both retain conservative 1/1/0.5/locality compatibility projection without gaining capabilities. The pure legacy projection also retains overall-pass, target, injected future-time and FM system/text-tool/image-identity prerequisites. The existing production FM verifier is unchanged.

Score-bearing v2 records require both `score_policy` (integer exactly 1) and `score_profiles` (array). Missing one member, explicit null, wrong envelope type/policy, unknown envelope fields, duplicate envelope keys, invalid identity/version/fixture/status are record/document failures. Absent both fields means compatibility; explicit empty profiles means no eligible scored class.

Inside a valid envelope, malformed profile fields reject that class only. Unknown or unidentifiable class entries grant nothing. Known class entries with missing/extra/null/wrong-type/out-of-range/class-incompatible fields are discarded; duplicate class entries reject every occurrence of that class. Parsing retains duplicate keys before any Value conversion. A duplicate request_class key poisons every known class named in the entry. Valid unrelated classes survive. Programmatic publication fails for invalid evidence instead of silently dropping it. `encode_v2` is the shared canonical writer and sorts entries in enum order; atomic `publish_v2` uses it. `ProviderCatalog::qualified_score_profile` preserves the legacy exact-identity gate while adding separate adaptive class evidence admission.

## Synthetic evidence

`provider-score-v1.json` is one shared oracle for producer, canonical qualification writer, compatibility reader and router tests. It covers qualification components, all class masks/partitions, latency boundaries/interiors, locality boundaries, legacy projection, all 17 live outcome mappings, and malformed evidence. Additional focused tests cover nonfinite/negative/oversized normalized numbers, exact weight basis vectors, actual p95 rank over 4/5/20/100 samples, independent n/20 counts, neutral atomicity, null-vs-absence, duplicate-key preservation, sorted writing, class-local rejection and catalog eligibility.

Circuit tests enumerate 4 phases x 17 outcomes x 3 Retry-After forms, plus exact rolling-window edges, opening/escalation/cap/reset, short/long metadata extensions, reserved-vs-old-inflight ownership, neutral reservation release, immutable blocked outcomes, saturating clock arithmetic and fractional-delay precision. Router tests pin class isolation, stale requalification completions, blocked restoration, sole/pinned exclusion, losing-probe nonreservation and the full fallback/effect outcome table.

Validation on the final source:

- `cargo test --locked provider::`: 114 passed, 0 failed, 1 intentional live-FM test ignored; 955 unrelated tests filtered out.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- No pure circuit/scoring/routing clock reads or sticky state remain.
- Production files in the new/refactored slice: scoring 344 lines, circuit 285, routing 321, manifest 830, manifest_scores 186. Focused tests are separate files. Existing provider.rs changes are module/export declarations only; its legacy implementation was not rewritten.

No real provider qualification, launchd, Discord, voice, network probe or deployment was performed. Synthetic and source evidence establish policy behavior only. Task 8 must serialize router mutation/completions, apply actual catalog/capacity/identity policy, restore exact-identity persisted blocks before selection, constrain legacy default selection precedence, and set conversation effects at real effect boundaries. The preexisting provider-wide allowance remains for the later runtime cutover; no new warning allowance was added.
