# Initiative native admission addendum — 2026-10-03

Current implementation contract under the approved completion program. It
clarifies the source adapter and due-time rule of the2026-10-01 Initiative
design; existing conversation candidates and required Work reminders retain
their contracts. Source execution does not authorize a live usefulness pilot.

One real human invokes private `/engage follow_up` for themselves in an exact
Guild or bot-DM origin. Inputs select an existing native task ID/revision,
eligible human message ID and expiry seconds. Derive project, Work scope,
member, destination, source revision and time from current canonical authority
and native facts. A command/interaction ID, task ID or URL cannot invent a
SourceRef. Require an already recorded completed eligible exchange, its exact
bot response, and the current canonical observation. Task revision0 is valid;
Decision references, changed/completed/cancelled tasks and other origins refuse.
No WorkTask source rewrite or fake task is allowed.

Use the existing checked source timestamp plus86,400 seconds as `due_at`.
Request expiry is checked UTC `now + seconds`, with60–604,800 inclusive seconds
and `due_at < expires_at`. A request whose expiry leaves no send window refuses
truthfully. Task due/reminder fields do not choose optional contact timing.
The command records a candidate only; the retained scheduler owns delivery.

The acknowledged catalog guard precedes native reads. Check the actual human,
application, qualified ephemeral leaf, Guild/BotDm envelope and observed initial
acknowledgment. Retain one admitted request owner through bounded fresh source,
exchange and Work audience proofs and the existing canonical Work commit. Carry
original owner admission time through erasure checks. Recheck exact source,
response, task/project/status/revision, membership, audience, destination,
current policy and expiry after awaits. Return saved only after observed
canonical publication; failure copy directs the human to existing receipts.

Use existing approved Engagement MemberPolicy only: positive explicit daily
limit, timezone and exact scoped origin/private choice, existing stops, snooze,
quiet hours, shared calendar capacity and guild guard. There is no second
preference, budget, scheduler or delivery ledger. Preserve existing source dedupe
and add scope/project/task/revision dedupe independent of human source/member
and resolved destination. A conversation candidate occupying the same source
is not upgraded. Preserve both replay commitments after erasure and restart.

Candidate migration adds paired serde-default `work_ref` and `expires_at`; old
rows have bothNone. Linked rows require FollowUp, an exact Task, coherent source
and member, and checked source-derived due before expiry. Reject half-linked or
malformed records. An optional closed terminal reason records a known cause;
current eligibility and a historical receipt state are distinct facts.

Delivery extends the existing Engagement owner with a default-refuse Work proof
for linked rows. Prove current exact scope and full audience before reservation,
then recheck under the canonical commit. Persist the existing reservation and
single charge before send. Repeat fresh Work proof after reservation publication
before extracting task text for generation. Hydrate only the exact task revision
and real exchange as bounded quoted data, with SourceOnly, empty personal memory,
read-only generation and no tools. Ordinary tasks request no Activity; actual
Activity invitations retain their accepted-version proof.

After generation repeat source/exchange and Work proofs, compare their frozen
resolved destination, then run the existing Engagement destination proof last.
Validate current canonical task, audience, expiry, source, policy and reservation
immediately before send. Preserve Sent versus definite rejection versus
ReviewRequired; uncertain/crashed attempts never replay or refund coverage.
Discord does not atomically lock GET facts through send; no external lease or
undo guarantee is claimed.

Private inspection uses owned receipts in the exact invoking origin. Fresh Work
access precedes task details; revoked/unknown rows render fixed content-free
copy. Explain policy with the shared calculations without charging inspection:
Allowed, Disabled, Quiet, OptedOut, StaleTask, Expired, AccessDenied, Budget,
Cooldown, AlreadyAttempted, ActivityUnavailable. Keep recorded terminal causes
separate from current computed blockers. Stops and erasure cancel callbacks;
retained minimized safety commitments prevent rearming the same revision.

The existing Engagement help section fits the additional leaf (maximum reviewed
1,533 characters). No Work help section expansion is required. Independent
external domain/delivery reviews pinned these invariants before source edits.
Actual usefulness, enrollment of at most three willing recipients, observed
replies/stops/failures and installed/live acceptance remain separate proof gaps.

Native input amendment before release: `source_message` is a decimal String
option, parsed as an exact positive u64 after acknowledgement and before REST.
Discord INTEGER options are limited to53 bits, so they cannot accept ordinary
message snowflakes. Reject empty, non-ASCII-decimal, zero, over20-character and
u64-overflow values without reading source facts. Internal task/revision and
bounded expiry remain numeric; existing command contracts are unchanged.
The real registration regression observed Integer4 versus required String3.
Reference: [Discord application command option types](https://github.com/discord/discord-api-docs/blob/main/developers/interactions/application-commands.mdx).

Retained receipt denominator amendment before implementation: inspection counts
only the at-most-five exact owned-origin rows whose fresh native Work and current
canonical checks pass. Terminal total equals useful + stopped + failed + unanswered.
Useful requires the recipient's existing explicit Useful feedback. Stopped means
Cancelled with a recorded OptedOut cause. Failed/non-useful covers other terminal
refusals, cancellation, uncertainty and explicit Dismissed feedback; it does not
claim a proven transport failure. Unanswered is rendered as no explicit feedback,
which never proves no human reply. Pending/Reserved remain active outside the
terminal denominator; erased, inaccessible and stale rows remain omitted. New
explicit stops record OptedOut only for unfinished linked rows actually cancelled;
old unknown causes remain unknown. These source receipts do not qualify the
willing-recipient pilot. No persistence field or public API changes are required.
