# Initiative source qualification receipt — 2026-10-03

Current qualification of the exact frozen snapshot below. This receipt was saved after the successful gate and therefore is not part of that fingerprint; the later final repository gate must cover subsequent documentation/source inputs.

## Result and exact snapshot

Result: `Current: source-qualified; no unresolved blocking review findings` — source qualification only.

| Evidence | Actual value |
|---|---|
| Checkout | /Users/donaldfilimon/dev/active/abbey-bot |
| HEAD | e742375ff9b3d20b97df09c7fad10c1225ae957c |
| Branch | main |
| Preserved index SHA256 | 83c7456df6d4a61d3fd6e822b7a44f166e55cafc9093775c0a5f433efb79e28f |
| Dirty-source input count | `851` |
| Dirty-source fingerprint | `59e31a27fb64ce36da3ad7fbef8dfac7ae52d573b193a66c541b48cc6af55700` |
| Gate command | ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh |
| ABBEY_WDBX_REPO override | `unset; required sibling fixture used` |
| Authoritative terminal exit | `0` |
| Full authoritative log | initiative-gate3.log |
| Input manifests | initiative-gate3-before.json; `initiative-gate3-after.json` |
| Complete before/after equality | `true; complete JSON inputs identical` |
| HEAD/index before/after equality | `true; unchanged` |
| Retained input archive and archive member verification | `initiative-gate3-inputs.tar.gz; all 851 members verified; archive SHA256 19b228379fdd4cc92cc6ffc7df849400a2d02dcf7703a76e0dbc8d0389013bdb` |

Observed before Gate 3: 851 inputs, fingerprint
59e31a27fb64ce36da3ad7fbef8dfac7ae52d573b193a66c541b48cc6af55700.
Abbey Reviewer's independent Clippy-repair manifest compares equal to the
controller's entire Gate 3-before manifest, including every input row and HEAD/index.
The controller subsequently observed terminal exit0 and complete unchanged after-manifest; the retained result JSON records actual counts.

The current shared diff contains 136 tracked and 76 relevant untracked paths,
212 unique paths. Root preserved every existing edit and the staged index; no
checkout/reset/staging/commit/worktree/dependency change, push, PR, deployment,
service restart, live provider call or Discord mutation was authorized or used.

## Authoritative suite evidence

Only actual Gate 3 completed stages/counts may fill this table. A stage without a
reported test/scenario count should be described as such; do not invent a count
from the number of source test functions. Cargo-only checks do not replace the
authoritative gate.

| Gate stage | Actual result/count |
|---|---|
| Locked formatter check | `passed` |
| Python deployment/privacy/instruction/contracts/security/plist suites | `339 Python unittest cases plus 16 provider publication scenarios; all passed` |
| Docs Liquid delimiter validation | `passed` |
| Rust production/test module caps and mirrored instructions | `passed; mirrors agree and module caps hold` |
| Strict external WDBX conformance | `required parity passed; fixture SHA256 a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90` |
| Offline macOS audio tap synthetic tests/build | `12 system-audio tests +16 audio-tap tests; offline synthetic PCM; release build passed` |
| cargo clippy --all-targets --locked -- -D warnings | `passed; warnings denied` |
| Rust tests | `2277` passed; `0` failed; `8` ignored |
| Rust ignored test names/acceptance boundaries | `8 existing exclusions: operator command export; live episode acceptance; episode preflight; three subprocess helper entry points; live backend DM; live FM schema. Full names/reasons in initiative-gate3-result.json; helpers are exercised through parent tests.` |
| Locked release build | `passed; locked release plus token-free regression/self-test stages completed; gate ended == ok ==` |
| Explicit skips or checks without numeric counts | `no explicit skips; instruction, privacy, contract, plist, shell, offline self-test checks passed with their reported assertions. RustSec retains 5 accepted vulnerabilities and 3 unmaintained informational warnings; audit is not clean.` |

Activity has no reviewed diff; `npm --prefix activity test` is inapplicable to
this qualification. Source validation of Activity URL assets/constants in the
gate does not complete operator-only Developer Portal mapping or iframe acceptance.
The accepted RustSec locked debt remains the documented policy; do not describe
it as an audit with no vulnerabilities. Record any actual observed linker warning
separately from warnings-denied Clippy.

## Review, confirmed repairs and focused evidence

Independent final report: `initiative-abbey-final-review.md`, with complete path
inventory in `initiative-final-review-coverage.json`. Coverage combines the
original six sequential subsystem reports and cross/final review, complete
Continuity integration, earlier Initiative admission/runtime reviews, all 67
Task23-to-Initiative deltas and all 13 later captured-source changes. The scanner
fixture rename is a separately reviewed one-path correction after the first gate.
Every changed production module above 800 lines is explicitly reviewed; all remain
below 1,000. No unresolved blocking P1/P2 remains in the reviewed source. Root is
the sole writer; Abbey Reviewer made no source/Git/Cargo changes.

Confirmed Initiative repairs preserved the exact native human command and
acknowledgement envelope; decimal String snowflakes; original absolute expiry;
owner DM Origin/Private consent; frozen task/project/revision/status/full audience;
bound source and response; retained Work/erasure admission and observed publication;
current shared guild/member policy, limits and cooldown; current post-writer task
authority before plaintext; final recipient authorization; single durable charge;
truthful definite/uncertain/no-replay delivery and erased task/source commitments.
Native unsupported adapters refuse. Unsolicited generation remains SourceOnly,
read-only, without tools or unrelated personal memory.

Final source review confirmed and closed global catalog permission identity/full
roles, shared member/guild and plain moderation target facts, exact selected
permissions/webhook channel identity, exact roleplay guild NSFW channel, unproved
DM roleplay admission, final Engagement recipient/bot/parent/thread/DM facts, and
valid REST embedded-thread guild omission. The compatible internal Unproved pure
gate revision was saved and reviewed before implementation. Valid public command,
output visibility, serialization and dependency contracts remain unchanged.

| Controller focused evidence read by reviewer | Actual result |
|---|---|
| initiative-receipt-green/test.log (follow_up) | 70 passed, 0 failed |
| initiative-receipt-domain-green2/test.log | 10 passed, 0 failed |
| initiative-final-recipient-red/test.log | 1 passed, 3 failed; RecipientOwner, ParentId, DmRecipientBot first triggers |
| initiative-final-recipient-green/test.log | 4 passed, 0 failed |
| native-guild-member-red/test.log | 1 passed, 1 failed at GuildIdentity; owner variant was hierarchy-blocked |
| native-guild-member-help-green/test.log | 91 passed, 0 failed |
| native-channel-batch-red/test.log | 64 passed, 3 failed at wrong-guild variants |
| native-channel-help-green/test.log | 97 passed, 0 failed |
| native-roleplay-dm-red/test.log | 1 passed, 1 failed at WrongRecipient activating Aviva |
| native-roleplay-green/test.log | 11 passed, 0 failed |
| native-dm-help-green/test.log | 99 passed, 0 failed |
| native-thread-rest-red/test.log | 0 passed, 1 failed at valid omitted REST guild fields |
| initiative-engagement-qualified-focus/test.log | 198 passed, 0 failed, 0 ignored |
| native-roleplay-dm-rendered.log | 1 passed, 0 failed; actual refusal printed and read |

Other original domain/native/delivery RED/GREEN and preserved completion evidence
are indexed by the earlier Initiative reports and controller progress, rather than
being rewritten or reclassified as current whole-source gate evidence.

## Receipt denominator and usefulness limits

Task 3 inspection uses at most 5 already-authorized owned exact-origin linked rows.
Native current Work proof precedes details/counters; canonical candidate, current
task and exact full audience are rechecked under the same short lock after awaits.
Current time is measured after proof. Inspection does not create unknown guild
configuration, send, generate, charge or alter preference/candidate state.

The shown terminal denominator partitions into explicit Useful, known stopped/
OptedOut, failed/non-useful and no explicit feedback; active rows remain separate.
Useful requires explicit human Useful feedback. Dismissed, refused and uncertain
outcomes are conservative labels. Newly unfinished linked stops record known
OptedOut; historical terminal/unknown causes are preserved. Resume changes only
current policy. Missing feedback does not prove no reply; erased/inaccessible
receipts are omitted, never reconstructed from hashes or consumed charges.

The actual private status copy in `initiative-receipt-rendered.log` was printed
and read. It labels counts as the shown readable subset and states that erased or
inaccessible outcomes are omitted. Task 3 optional pilot/population usefulness,
human willingness and feedback quality remain unverified; this source result does
not claim the human pilot complete.

## Historical gate correction

Gate 1 completed with controller-observed terminal exit 1 and retained full log
`initiative-gate1.log`; its before/after manifests were stable at 851 inputs,
fingerprint 399d797629074c6b7700e565af129cb3d6a4fa00c2c4303cfc00655f6d448f8c.
It failed the privacy scanner at the synthetic fixture `.write_all(&body)` before
later authoritative stages. The final correction renames that synthetic local to
synthetic_reply, retaining identical serialized bytes/header/loopback behavior and
unchanged scanner policy. The archived exact comparison is
`initiative-scanner-repair-review.diff`; only recipient.rs differs in complete
input comparison. Gate 1 remains a stable failed historical attempt and cannot
qualify the changed Gate 3 snapshot.

Gate 2 then completed with controller-observed terminal exit 101 and retained
`initiative-gate2.log`. Its complete before/after manifests were stable at 851
inputs, fingerprint 38262c6ff75d2bb363bb9d9db7d11ed2783e55fe1f7146d537914228e383c495.
After completing privacy/offline audio stages, it failed warnings-denied Clippy at
`src/work/follow_up/tests.rs:85` for field_reassign_with_default. Rust tests and
locked Rust release were not reached. Root repaired only the fixture initializer
to `WorkStore { sequence: TASK, ..Default::default() }`, retaining the same values,
insertion behavior/assertions and lint policy without suppression. Independent
review confirmed exactly one changed input and preserved HEAD/index; the focused
Clippy log completed without diagnostics. Gate 3 subsequently qualified this changed frozen snapshot.

## Proof gaps and evidence discipline

No installed binary identity, deployment, managed-service readiness, live provider
or Discord behavior, human voice/usefulness/corpus acceptance, Linux/Windows
runtime qualification or optional paid training was exercised. Offline macOS
audio validation uses synthetic PCM. Live/paid/hardware acceptance tests stay
explicitly ignored or out of scope unless the actual gate log says otherwise.

A source-qualified result requires all three: complete stable input/HEAD/index
attribution, authoritative strict gate exit 0, and no unresolved blocking review
finding. If any input changes during/after the gate, reconcile and review the
change and rerun for a stable snapshot. Append the resulting evidence correction
to tasks/goals.md and update tasks/todo.md without rewriting older receipts.
If those documentation writes enter the qualification input manifest, capture and
verify their final state before claiming that new complete snapshot qualified.

Evidence directory: /Users/donaldfilimon/.codex/verification/abbey-bot-continuity-20261003. Full log, manifests, result JSON, archive and independent review remain outside shared source. These later receipt/checklist/ledger writes do not retroactively change the captured fingerprint.
