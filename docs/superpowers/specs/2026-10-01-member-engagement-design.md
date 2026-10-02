# Abbey Member Engagement Release Design

Status: Proposed implementation; user-approved written specification, 2026-10-01.
Donald approved the expanded design, then supplied this specification with the
instruction to complete it. This file preserves that approved scope. Defaults
below resolve implementation details; they do not attest deployed behavior.

## Outcome and release sequence

Abbey initiates useful conversations, follows up on unresolved exchanges, invites
members into functioning Activities and voice sessions, and helps communities
connect around questions and projects. Deliver three independently testable
increments: (1) member controls and durable delivery, (2) conversation and
invitations, (3) community facilitation. Each passes tests and review before the
next begins. Voice reliability, text responsiveness, and Activity readiness
remain separate release acceptance requirements.

## Approved member policy

- Add `/engage` controls for preferences, contact limits, timezone/quiet hours,
  destination, status, snooze, stop, subscriptions, and feedback.
- Prior direct interaction or explicit subscription establishes eligibility,
  never permission to send. Personalized contact remains disabled until the
  member explicitly saves a positive limit and IANA timezone.
- Use the existing scheduler's daily range, 1–4. Allow an optional member-chosen
  weekly limit, 1–28 and no greater than seven times the daily limit. Omission
  adds no weekly restriction. Never invent a configured positive limit.
- Quiet hours default to 22:00–08:00 in the saved timezone. Equal endpoints
  disable quiet hours. Preserve the existing DST policy: first fall-back
  occurrence; advance spring-forward gaps to the first valid local minute.
- All personalized engagement kinds share one member budget across Discord
  servers and DMs. Server-originated deliveries additionally consume the
  existing server unsolicited budget and cooldown. Forced replies remain
  outside this proactive budget; Work reminders retain their existing policy.
- Server conversations default to the exact original channel/thread. DM
  conversations stay in the same individual's bot DM. A member must explicitly
  choose private delivery for server follow-ups; recheck access to the source
  server/channel before generating or delivering that private message.
  A private-delivery choice applies only to the invoking origin scope, never
  silently to every other server. Weekly subscriptions likewise retain their
  chosen origin and destination.
- Global stop disables all personalized contact. Scoped stops disable a server
  or conversation. Resuming requires an explicit preference update; inbound
  messages never undo a stop. Snooze postpones existing candidates only.
- Members see their own settings and delivery state. Managers see aggregate
  server outcomes, never another member's private conversation or preferences.

## Durable delivery contract

Reuse canonical `abbey-state.json`, Work's owned commit path, and the retained
60-second scheduler. Add an engagement domain inside WorkStore rather than fake
tasks, a second persistence file, a second gateway, or an independent scheduler.
Policy computation remains pure; Discord REST belongs to the gateway adapter.

Persist settings, eligible source identities, candidates, reservations and
outcomes atomically. Reserve member capacity before sending. Obtain the same
server cooldown/hourly gate used by unsolicited pipeline replies. A failed
reservation or send may conservatively consume its attempted capacity; it must
never refund an uncertain send or exceed either limit. No locks cross network
awaits. The same source/recipient/kind has one durable identity, independent of
restart, destination change, and scheduler duplication.

Delivery states are Pending, Reserved, Sent, Cancelled, Rejected and
ReviewRequired. Save a Discord message ID for Sent. Any timeout, disconnect,
cancellation after send admission, or startup recovery of Reserved becomes
ReviewRequired without retry. Rejected access is terminal for that candidate;
never reroute to a different channel or DM. Recheck configuration, candidate
revision, source existence, recipient identity and access after reservation and
immediately before send. Discord allowed_mentions has an empty parse list.

Keep new records bounded: at most 10,000 candidates/receipts and 1,000 pending
candidates. Reject additional creation visibly when full; do not prune dedupe
identities while a source could be proposed again. No raw voice audio or voice
transcript persistence. No message text, prompts, secrets or DM bodies in logs.

## Conversation and invitation behavior

A bounded classifier returns only resolved, unresolved or unclear plus source
message IDs. Evaluate at most eight existing recent turns from one origin, only
after a direct interaction with Abbey. Treat text as source data, never policy.
Reject unknown fields, invented IDs, cross-scope sources, empty evidence, or a
provider failure. Only validated unresolved output proposes a candidate; unclear
schedules nothing. Classification has no tool authority and cannot send.

Schedule one follow-up at source time + 86,400 seconds. Any later inbound message
from the member in that origin, resolution, dismissal, opt-out, loss of source
access, or deletion cancels it. Bot messages do not cancel or establish member
eligibility. A settings update never rearms a consumed source. Generate a short
Abbey-authored contextual question from freshly authorized source at delivery;
generation failure rejects the candidate without claiming delivery.

Weekly check-ins are separately subscribed and use the member's chosen weekday
and local hour. Skip missed occurrences rather than replaying a backlog. Check-ins
must have authorized useful context; empty context produces no generic nag.
Optional learned preferences may reduce contact or suggest timing, but never
increase explicit limits, enable subscriptions, move destinations, or remove stops.

Activity invitations require a current operator-maintained readiness record for
a public HTTPS shared host and successful Discord iframe/two-client acceptance
of the deployed version. Presence and local previews do not qualify. Invalid or
outdated readiness blocks invitations visibly. Voice invitations explain personal
saved agreement and manager join/resume; they never create agreement or open
capture. A consumed unanswered invitation is not repeated until the member
explicitly requests another or starts a new relevant exchange.

## Community facilitation

Public assistance requires existing guild act/learning eligibility and a separate
manager-configured switch and channel allowlist for each behavior. All new switches
default off. Settings do not change permissions, intents or installed guilds.

- Conversation starters: contextual, unaddressed public posts; at most one per
  configured channel per 24 hours; no post without useful current context.
- Unanswered-question assistance: one candidate after 24 hours for a referenced
  substantive unanswered human question; cancel when a human reply or resolution
  appears. Prefer a helpful response or focused clarification over an engagement
  announcement. Preserve forum/thread identity and access.
- Welcomes: use actual available member-join events, once per join event, only in
  a configured channel. Never infer a join from a member-list scan. Generic public
  welcomes use server policy; addressed/personal contact also needs member controls.
  Missing required events/intents show unavailable; do not enable privileged intents.
- Project check-ins: at most weekly, grounded in existing authorized Work projects;
  respect the project audience and member contact policy, and do not create new work.
- Introductions: both configured members explicitly opt into an exact proposed
  introduction. Each privately previews the exact information about them and its
  destination. Both approve the same revision before publication; any revision,
  withdrawal, access loss or stop invalidates approval. Do not reveal identities
  or private facts to the other member before approval. Never infer matchmaking
  solely from private conversations or publish across guilds without a common
  approved destination and current access.
  Reserve both members' contact capacity in one canonical transaction before
  publication; one member's available capacity cannot bypass the other's limit.

## Personality and isolation

Use existing bounded style addenda: preserve evidence thresholds, expiry,
suppression and revert. Quoted examples/code are not feedback. Keep explicit
feedback, delivery outcomes and inferred usefulness distinct. Expose evidence and
active changes without member identities or feedback text. No self-rewriting
code, foundation-model training, or autonomous tool writes are included.

Conversation content stays isolated by server/channel or individual DM. Only
global member contact preferences and counters cross these contexts. A budget
setting is never authority to retrieve or disclose content from another scope.

## Verification and rollout

Regression coverage: restart recovery; concurrent ticks; duplicate source events;
cancellation immediately before send; quiet/DST boundaries; global and guild
budget exhaustion; opt-out; inaccessible channels and threads; blocked DMs;
malformed/adversarial classifier output; uncertain sends; all public feature gates;
and introduction approval/revision races. Fake transports verify policy; they do
not prove live Discord delivery or hearing.

Live acceptance: configured member receives one contextual follow-up in the chosen
destination; reply/resolution/snooze/stop prevent matching deliveries; concurrent
features honor the shared member budget; guild/DM content remains isolated;
Activity invitations launch the verified shared experience; a consenting person
hears and answers Abbey in Discord voice; text generation and delivery timings
are measured separately. Run the authoritative gate, read/review the complete
diff, install transactionally, verify installed/release hashes and service status.

Preserve other edits, credentials and server settings. No commits, pushes, new
production dependencies, permission expansion, participant consent fabrication,
or mass mentions. Portal actions and hosting publication requiring those excluded
operations remain explicit operator steps. Completion requires current evidence
for every item; neither a green source gate nor healthy presence substitutes.
