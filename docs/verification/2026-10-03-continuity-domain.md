# Continuity tested-domain evidence — 2026-10-03

**Current tested domain; Partial production integration.** ContinuityTask1 adds
bounded, exact-scope confirmed cards and a transient immutable proposal registry.
Opaque once-consumed grants repeat current native Work authority, expiry, source
joins/revisions and CAS. Text is at most1600UTF-8bytes, refs8, cards/proposals256;
seven-day cards and five-minute proposals use checked arithmetic. Rejected
confirmation leaves live cards unchanged. Domain loading rejects malformed or
unowned metadata, duplicate scopes and excess cards.

Tests-first initial missing API behavior failed18tests; the real transition then
produced15guard failures/3passes. Reviewer-added guards produced4failures/2passes;
one preceding test-authoring compile mismatch is excluded from behavioralRED.
Final focused smoke24passed/0failed/0ignored, including immutable text, stale CAS,
held-grant expiry/actor/scope/native authority revocation, restart nonce separation,
UTF-8/native sources, exact8-ref and distinct256-card boundaries. Independent Abbey
Reviewer found no unresolved blocking domain finding or actionable deferred minor.

Strict `CARGO_BUILD_JOBS=2 ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` session2967
exited0 with2,114Rust passed/0failed/8existing exclusions,339Python cases plus16
publication scenarios,Swift12+16synthetic tests,fmt/Liquid,Clippy,required WDBX,
locked release and both rebuilt offline startup suites. Before/after804-input
manifests were identical:SHA256
`f1a6e2ce96642ff1555592127805fce6315d0be05a51ef8328344c0c47e7a6a7`,
HEAD`e742375ff9b3d20b97df09c7fad10c1225ae957c`; staged index unchanged.
Required external WDBX fixture771bytes/SHA256
`a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90` was stable.
Accepted RustSec debt is5vulnerabilities+3unmaintained informational advisories;
audit is not clean. Nonfatal macOS unwind warning remains in the full log.

The snapshot is source-qualified only. This receipt and checklist updates follow
that frozen gate, so their different whole-tree fingerprint is not qualified by
that command. Actual result, full log, manifests, review/rulings and exclusion
inventory are retained outside shared source under
[/Users/donaldfilimon/.codex/verification/abbey-bot-continuity-20261003](/Users/donaldfilimon/.codex/verification/abbey-bot-continuity-20261003/qualification.md).

The new domain remains cfg(test). No private native-human/model dispatch proof,
fresh REST prompt/delivery authorization, canonical card publication, covered
episode admission, clear/member erasure or restart feature is claimed. Those
remain mandatoryTasks2/3. Domain serde tests do not prove canonical restoration.
Installed/provider/Discord/Activity/platform/human acceptance and all downstream
program source obligations remain open. No production dependency, existing public
contract, Git/index or service state was changed. No deployment or live action ran.
