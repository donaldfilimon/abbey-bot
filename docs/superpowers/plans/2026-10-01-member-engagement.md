# Abbey Member Engagement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver member-controlled proactive engagement, contextual follow-ups,
verified invitations and community facilitation with durable receipts and scope isolation.

**Architecture:** Put pure engagement records and policy in a new domain stored
inside the existing WorkStore. Reuse Work's canonical owned commit path, retained
scheduler and Discord authorization patterns; do not represent conversations as
fake Work tasks. Classification proposes candidates; deterministic policy and
fresh transport facts control every delivery.

**Tech Stack:** Existing Rust 1.98/edition 2024, Serenity/Poise, Tokio, serde,
chrono/chrono-tz, existing generation/provider infrastructure; Node Court Activity.

**Spec:** `docs/superpowers/specs/2026-10-01-member-engagement-design.md`.

## Global Constraints

- Daily limit is explicitly chosen, 1–4; optional weekly limit is 1–28 and no
  greater than daily_limit * 7. Missing positive limit or IANA timezone disables
  personalized contact. Quiet hours default 22–08. No inherited timezone.
- One follow-up at source time + 86,400 seconds. Public starters/question
  assistance have a 24-hour floor; project check-ins and subscribed check-ins
  are weekly. Missed schedules skip, never burst.
- Shared personalized member budget across Discord guilds/DMs; guild sends also
  use the existing unsolicited cooldown/hourly gate. No mass mentions.
- New guild feature switches default off. Existing settings, intents, permissions,
  voice consent and installed guilds are preserved.
- At most 10,000 engagement records and 1,000 pending candidates. At capacity,
  fail visibly rather than silently forgetting dedupe or opting someone in.
- Canonical state is abbey-state.json; retain the Work commit/shutdown discipline.
  No new gateway, scheduler, persistence file, production dependency or raw voice
  retention. No locks across I/O. Logs contain counters/phases, not bodies.
- No commits, pushes, history changes or production permission expansion.
  Preserve concurrent edits. Do not create a worktree unless repo/user-authorized.
- One task at a time with implementer/reviewer gates. Build ownership must be
  coordinated with Restore Abbey Discord bot; never kill another session's Cargo
  process or overwrite its source. Revalidate live state before starting.

## Review Focus

1. Stop/reply arrives after durable reservation: final validation cancels without send.
2. Discord accepts a send but response is lost: ReviewRequired; never automatic retry.
3. Same member is due in two guilds and a DM: one shared limit, no content crossover.
4. Private/archived thread changes access: prove thread membership, never parent-only access or fallback.
5. Classifier sees quoted instructions/invented IDs: no candidate or policy mutation.

Each condition is pinned in its owning task below. All new Rust tests are inline
or under src modules; this crate has no --lib/--workspace/-p test target.

## Task 1: Pure records, disabled defaults and durable storage

**Files:** Create `src/engagement.rs`, `src/engagement/policy.rs`,
`src/engagement/loading.rs`, `src/engagement/tests.rs`; modify `src/main.rs`,
`src/work.rs`, `src/work/loading.rs`, `src/persist.rs` tests.

**Interfaces:**
- `EngagementScope::{Guild { guild:u64, channel:u64 }, Dm { member:u64, channel:u64 }}`.
- `DestinationPreference::{Origin, Private}`; `MemberPolicy` contains revision,
  optional daily/weekly limits, optional timezone, quiet_start/end, global_stop,
  stopped_scopes, snoozed_until and optional WeeklySubscription { weekday:u8,
  hour:u8, scope:EngagementScope, destination:DestinationPreference }; weekdays
  use Monday=0..Sunday=6. Store explicit destinations in
  `BTreeMap<EngagementScope,DestinationPreference>`; missing scope means Origin.
  Defaults have no limit/timezone, no subscription and 22/08 quiet boundaries.
- `SourceRef { scope, message:u64, author:u64, revision:u64, at:u64 }`.
- `EngagementKind::{FollowUp, WeeklyCheckIn, ActivityInvite, VoiceInvite,
  ConversationStarter, UnansweredQuestion, Welcome, ProjectCheckIn, Introduction}`.
- Declare `CommunityFeature::{Starters,Questions,Welcomes,Projects,Introductions}`
  and `GuildFeaturePolicy { revision:u64, enabled:BTreeSet<CommunityFeature>,
  channels:BTreeMap<CommunityFeature,BTreeSet<u64>> }` in this task, default empty.
  Task 7 implements their planning behavior, not their storage declarations.
- Declare `IntroductionState::{Pending,Ready,Consumed,Cancelled}` and
  `Introduction { id:u64, revision:u64, scope:EngagementScope, members:[u64;2],
  approved_self_descriptions:[Option<String>;2], approvals:[Option<u64>;2],
  destination:u64, state:IntroductionState }` here. Task 8 implements controls;
  no introduction is deliverable before its controls are implemented.
- `CandidateState::{Pending, Reserved, Sent, Cancelled, Rejected, ReviewRequired}`.
- `Candidate { id:u64, kind, source:Option<SourceRef>, member:Option<u64>, scope,
  due_at:u64, revision:u64, state, dedupe_key:String, policy_revision:u64,
  destination:DestinationPreference, message_id:Option<u64>,
  introduction_id:Option<u64> }`. No stored transcript.
- `EngagementStore` owns sequence, member policies/eligibility, guild feature
  policies, candidates, introduction records and attempted-capacity ledger;
  derive default and validated serde loading. `WorkStore.engagement` defaults
  empty on legacy load and follows Work's validation/migration path.
- Capacity ledger rows are `ContactCharge { candidate_id:u64, member:u64,
  local_day:String, local_week:String, at:u64 }`; one unique candidate/member pair.
  Preserve charges and dedupe through stop/resume and timezone changes. Count
  previous UTC attempt timestamps in the member's currently configured local
  day/week as well as stored original buckets, using the higher count so timezone
  changes cannot reset capacity. Introduction members create two rows atomically.
- Reuse `WorkError` for Invalid/Denied/Full/Stale/Persistence outcomes.
- `MemberPolicy::validate(&self)->Result<(),WorkError>`;
  `MemberPolicy::personalized_enabled(&self)->bool`;
  `EngagementStore::validate(&self)->Result<(),WorkError>`.

- [x] Write tests for disabled defaults, daily 0/5, weekly over 28 or daily*7,
  invalid timezone/weekday/hour, quiet boundaries, zero IDs, inconsistent scope,
  duplicate dedupe identity and the record caps. Assertions include:
  `assert!(!MemberPolicy::default().personalized_enabled());` and
  `assert_eq!(invalid.validate(), Err(WorkError::Invalid));`.
- [x] Run `cargo test --locked engagement::tests`; require nonzero test count
  and the intended new tests fail before implementation.
- [x] Implement the types/validation/loading. Keep legacy Work receipts unchanged;
  do not populate eligibility from existing memory automatically.
- [x] Test loading legacy canonical state without engagement and round-tripping
  populated engagement through Stores. Assert all other fields are unchanged.
- [x] Run focused tests plus `cargo fmt --all -- --check`; review this task's diff.
  No commit. Reviewer confirms defaults cannot authorize contact.

## Task 2: Explicit member and guild command controls

**Files:** Create `src/commands_engage.rs`, `src/commands_engage/controls.rs`;
modify `src/main.rs`, `src/command_catalog.rs` and catalog/help tests;
create `src/runtime/engagement_commit.rs`.

**Interfaces:**
- `AppState::commit_engagement<R:Send+'static>(&self, change:impl
  FnOnce(&mut EngagementStore)->Result<R,WorkError>+Send+'static)
  ->Result<R,WorkError>` wraps `commit_work(|s| change(&mut s.engagement))`.
- Register `/engage preferences`, `/engage configure daily_limit timezone
  [weekly_limit quiet_start quiet_end destination]`, `/engage status`,
  `/engage snooze until`, `/engage stop [scope]`, `/engage resume [scope]`,
  `/engage weekly enabled [weekday hour]`, `/engage dismiss candidate`,
  `/engage feedback delivery useful|dismissed`.
- Scope values are global (default), current_server, current_conversation;
  current_server is unavailable in a DM. Only the invoking member changes their
  own policy. A scoped resume does not clear a global stop; status explains blockers.
  Configure's destination choice affects only the current origin scope; weekly
  subscription creation captures that scope and its destination explicitly.
- Register manager-only `/engage community feature enabled [channel]` and
  `/engage community-status`. Feature is one of starters/questions/welcomes/
  projects/introductions. Enabling requires the invoking guild and explicit
  channel; disable can clear the saved switch. No settings enable `/admin act`.
- Follow existing acknowledged command guard: defer before I/O and fetch fresh
  manager authority. Render private settings; cap every response to Discord limit.

- [x] Add failing catalog/authorization tests: ordinary member cannot alter
  another member or guild policy; DM cannot set current_server; resume does not
  bypass global_stop; missing configure fields do not enable delivery.
- [x] Run `cargo test --locked engage` and catalog focused tests; verify real failures.
- [x] Implement commands and commit wrapper with fresh authorization. All success
  responses require canonical commit; persistence failure reports no saved change.
- [x] Test cancellation during persistence using existing Work commit fixtures;
  retained owner finishes publication, caller cancellation cannot discard it.
- [x] Print/read representative owner/member/DM responses in tests, including
  blocked/default state, then run focused gates and get review. No commit.

## Task 3: Pure candidate scheduling, shared limits and cancellation

**Files:** Create `src/engagement/schedule.rs`, `src/engagement/lifecycle.rs`,
`src/engagement/schedule/tests.rs`; modify `src/engagement.rs` exports.

**Interfaces:**
- `EngagementStore::propose(&mut self, proposal:CandidateProposal, now:u64)
  ->Result<Option<u64>,WorkError>` validates source/scope/eligibility, capacity and
  durable dedupe. `CandidateProposal { kind,source,member,scope,due_at }`.
- `EngagementStore::due(&self, now:u64)->Vec<u64>` returns stable due_at/id order.
- `EngagementStore::reserve(&mut self,id:u64,revision:u64,now:u64)
  ->Result<EngagementReservation,WorkError>` atomically checks policy/revision,
  quiet hours/stops/snooze/global member capacity, marks Reserved and charges
  capacity. `EngagementReservation { candidate_id, revision, policy_revision,
  scope, member, destination }`.
- `validate_reserved(&self,reservation:&EngagementReservation,now:u64)
  ->Result<(),WorkError>`; `settle(&mut self,id:u64,outcome:DeliveryOutcome)
  ->Result<(),WorkError>` with Sent { message_id }, Rejected, Cancelled,
  ReviewRequired. No refund of attempted member capacity.
- `cancel_source(&mut self,scope:&EngagementScope,message:u64)->usize` and
  `cancel_member_origin(&mut self,member:u64,scope:&EngagementScope,after:u64)->usize`.
- `recover_reserved(&mut self)->usize` converts unfinished reservations to
  ReviewRequired at load; dedupe remains consumed. Clone existing pure DST
  calculation into a shared calendar helper used by both domains without changing
  Work behavior. Member quota uses saved timezone local day and Monday local week.

- [x] Add failing tests: one source cannot rearm; global daily/weekly caps cover
  two guilds plus one DM; quiet hours wrap; DST gap/overlap; missed weeks skip;
  policy revision/stop/source cancellation invalidates Reserved immediately;
  duplicate/concurrent ticks reserve once; restart Reserved becomes ReviewRequired.
- [x] Run `cargo test --locked engagement::schedule`; require nonzero failures.
- [x] Implement the pure lifecycle and deterministic calendar reuse; no clock,
  network or random reads. Saturating time math cannot schedule overflow.
- [x] Assert Pending->Reserved->Sent records message ID; Reserved->ReviewRequired
  never returns due; cancelled/consumed dedupe survives settings/destination changes.
- [x] Run schedule and existing Work schedule suites; review failure races. No commit.

## Task 4: Retained Discord delivery and shared guild gate

**Files:** Create `src/runtime/engagement_delivery.rs`,
`src/runtime/engagement_delivery/tests.rs`, `src/gateway/engagement_delivery.rs`;
modify `src/runtime.rs`, `src/runtime/scheduler.rs`, `src/service.rs`,
`src/gateway/mod.rs`, `src/pipeline.rs` gate visibility and gateway startup wiring.

**Interfaces:**
- `EngagementTransport::authorize(&self,reservation:&EngagementReservation)
  ->impl Future<Output=Result<AuthorizedDestination,WorkError>>+Send`;
  `source_exists(&self,source:&SourceRef)->impl Future<Output=Result<bool,WorkError>>+Send`;
  `send(&self,channel:u64,body:&str)->impl Future<Output=Result<u64,WorkError>>+Send`.
- `AuthorizedDestination { channel:u64, member:Option<u64>, scope:EngagementScope }`.
- `AppState::deliver_engagement<T:EngagementTransport>(self:Arc<Self>,transport:&T,
  cancel:CancellationToken,now:impl Fn()->u64)->Result<(),WorkError>`.
- Add retained `OperationKind::EngagementDelivery` on existing Tick::Work, with
  a single-flight engagement mutex. Settlement uses an owned Work commit and
  remains retained through service draining. Do not create a second timer.
- Extract the existing pipeline unsolicited cooldown/hourly acquisition into one
  runtime callable seam, preserving quiet/act/learning guard order and lock order.
  Engagement guild sends acquire that same gate before durable member reservation;
  conservative capacity loss if later persistence fails is allowed, excess is not.
- Fresh REST must prove guild membership, original message/channel, bot send
  permission, and private recipient identity. For threads, prove parent access,
  thread state and private membership; handle archived/locked inability explicitly.
  Never join/unarchive/change permissions or fall back to a parent/DM.

- [x] Add fake-transport tests for stop/reply between reservation and send, blocked
  DM, deleted source, private-thread loss, channel mismatch, budget races with a
  pipeline reply, duplicate ticks, timeout after remote acceptance, and shutdown.
- [x] Run `cargo test --locked engagement_delivery`; require intended failures.
- [x] Implement authorize -> shared guild gate -> durable reserve -> generate ->
  authorize/source/local revalidation -> send -> durable settlement. All external
  waits have the existing 30-second transport bound; generation uses provider bounds.
  A failure before send is Rejected/Cancelled; uncertain send is ReviewRequired.
- [x] Verify no store lock crosses I/O; no fallback destination; allowed_mentions
  empty; uncertain settlement fails closed and startup recovery is review-only.
- [x] Run pipeline rate-limit/Work delivery/shutdown tests as well as new suite.
  Reviewer confirms service ownership and shared budget remain coherent. No commit.

## Task 5: Conversation classification, source events and weekly check-ins

**Files:** Create `src/engagement/classifier.rs`, `src/runtime/engagement_candidates.rs`;
modify `src/gateway/discord.rs` event translation, `src/pipeline.rs` completion hook,
and new classifier/candidate inline tests.

**Interfaces:**
- `ConversationAssessment { outcome:ConversationOutcome, source_messages:Vec<u64> }`;
  `ConversationOutcome::{Resolved,Unresolved,Unclear}` uses snake_case JSON.
- `parse_assessment(raw:&str,scope:&EngagementScope,available:&[SourceRef])
  ->Result<ConversationAssessment,WorkError>` rejects unknown fields, empty or
  >8 refs, invented/cross-scope IDs, extra tool instructions and invalid JSON.
- `AppState::assess_engagement(self:Arc<Self>,scope:EngagementScope,member:u64,
  sources:Vec<SourceRef>)->Result<(),WorkError>` operates only after direct
  human interaction with Abbey, within one authorized scope and at most eight turns.
- Hook accepted direct exchanges to eligibility (identity only); bot messages do
  not establish it. Inbound human reply, source delete, resolution/dismiss command
  invalidates candidates through owned commits. No arbitrary message-content opt-in.
- Weekly subscriptions plan one occurrence per member-chosen local weekday/hour,
  grounded in authorized context. Empty/unclear context is skipped, not generic spam.

- [x] Add tests for resolved/unclear/provider-down, prompt injection in source,
  quoted requests, forged IDs, cross-guild content, wrong DM principal, eight-turn
  cap, no configured policy, no useful weekly context, and cancellation races.
- [x] Run classifier/candidate suites and observe intended failures.
- [x] Implement read-only classifier through canonical provider admission with
  no ToolScope mutation authority. Check result against actual source references;
  only valid Unresolved creates FollowUp at source time +86,400 seconds.
- [x] Generate final body from freshly fetched permitted source; do not store a new
  transcript copy. Respect existing style addenda and explicit member reductions.
- [x] Run regression suites, review prompt/rendered examples and the evidence path.
  Reject arbitrary classifier confidence numbers as permission. No commit.

## Task 6: Verified Activity and voice invitations

**Files:** Create `src/engagement/readiness.rs`, `src/commands_engage/invitations.rs`;
modify engagement candidate generation; update Activity/voice acceptance docs.

**Interfaces:**
- `ActivityReadiness { https_origin:String, deployed_digest:String,
  iframe_receipt:String, shared_receipt:String, verified_at:u64 }` is operator
  configuration checked by pure `validate_activity_readiness(&ActivityReadiness)
  ->Result<(),WorkError>`. HTTPS origin has no credentials/query; digest is 64 hex;
  receipts must identify the deployed version and real acceptance records.
- Do not expose public commands that let a model or member declare readiness.
  A digest mismatch disables Activity invitations. Public HTTP health validates
  reachability but never manufactures iframe/two-user proof.
- `/engage invite kind` lets an eligible configured member request an invitation
  candidate; kind is Activity or Voice, current context determines origin.
  Proactive invitations use the same validated candidate and receipt path.
- Voice invitation copy points to `/voice consent`, `/voice status` and manager
  `/voice join`/`resume`; it never calls consent or voice activation internally.

- [x] Test local-only origin, missing/stale-version receipts, malformed URL/digest,
  unavailable host, invitation dedupe, missing member policy and voice consent isolation.
- [x] Implement readiness gate/invitation copy and run new tests plus voice guards.
- [ ] Qualify deployed Activity via public HTTPS and actual Discord iframe with two
  participants: same room/case/votes, disconnection/recovery, no solo-vote upload.
  Respect operator Portal/publication constraints; record blockers, do not label ready.
- [ ] With willing participants, verify consented audible reply, barge-in,
  participant-change pause/resume, withdrawal and leave. Synthetic self-test is
  separate. Do not fabricate consent or duplicate outreach to obtain a witness.
- [x] Reviewer confirms invitation honesty and exact receipts. No commit.

## Task 7: Server-controlled public facilitation

**Files:** Create `src/engagement/community.rs`, `src/runtime/engagement_community.rs`;
modify Discord event/candidate hooks and command community status.

**Interfaces:** Consumes the CommunityFeature and GuildFeaturePolicy storage
declarations from Task 1; implements their behavior here.
- `GuildFeaturePolicy { revision:u64, enabled:BTreeSet<CommunityFeature>,
  channels:BTreeMap<CommunityFeature,BTreeSet<u64>> }`, default empty.
- `CommunityFeature::{Starters,Questions,Welcomes,Projects,Introductions}`.
- `community_candidates(store:&EngagementStore,facts:&CommunityFacts,now:u64)
  ->Vec<CandidateProposal>` consumes verified scoped facts and injected time.
- Channel starters: one contextual unaddressed post/24h; question assistance:
  one source-based candidate after24h cancelled by a human reply/resolution;
  welcome: actual member-join event dedupe; project: weekly authorized Work source.
- All public kinds need corresponding switch+allowlist and existing server gates.
  Personalized kinds additionally need member policy/global capacity. No automatic
  guild enumeration, intent enablement, channel creation or permission changes.

- [x] Test each switch independently/default-off, allowlist mismatch, act/quiet/
  learning blockers, duplicate join event, absence of join capability, bot source,
  answered forum question, project audience changes and personalized policy missing.
- [x] Run focused tests and observe the intended failures.
- [x] Implement pure planning and retained source hydration. No useful context means
  no starter. Missing gateway capability reports unavailable instead of scanning users.
- [x] Verify original forum/thread delivery and cancellation; run rate-limit tests
  with concurrent public and personalized candidates. Read public copy with reviewer.
- [x] Run focused gates; no commit. Do not claim community engagement until delivery
  and participant response are observed separately.

## Task 8: Mutually approved member introductions

**Files:** Create `src/engagement/introductions.rs`,
`src/commands_engage/introductions.rs`, introduction inline tests.

**Interfaces:** Consumes Introduction/IntroductionState declared in Task 1;
implements the approval and delivery lifecycle here.
- `Introduction { id:u64, revision:u64, scope:EngagementScope, members:[u64;2],
  approved_self_descriptions:[Option<String>;2], approvals:[Option<u64>;2],
  destination:u64, state:IntroductionState }` with Pending/Ready/Consumed/Cancelled.
- `/engage introduce member destination self_description` creates one proposal;
  each member approves their own exact description and common destination through
  invoker-bound components. Edits increment revision and clear BOTH approvals.
- `approve_introduction(&mut self,id:u64,member:u64,revision:u64)
  ->Result<bool,WorkError>` returns Ready only for both configured, opted-in members
  with matching revision. `withdraw_introduction` cancels and invalidates candidate.
- Descriptions are member-supplied, bounded to 300 Unicode characters each;
  previews disclose only the invoking member's own description before approval.
  Never mine private facts. Both have current access to common destination.

- [x] Test single/wrong-member approval, stale component, edited descriptions,
  withdrawn consent/stop after reservation, inaccessible destination, disallowed
  cross-guild source, capacity exhaustion and uncertain publication.
- [x] Implement invoker-bound private previews and fresh double-access validation;
  publication combines only approved descriptions. Use one introduction identity;
  bind Candidate.introduction_id to that record. Extend reserve to load both
  Introduction.members, validate both current policies and charge both capacities
  in the same owned commit; no partial charge/publication when either is blocked.
- [x] Run intro/lifecycle delivery tests. Reviewer checks no preapproval disclosure,
  no repeated invitation, and no publication from half-approved state. No commit.

## Task 9: Feedback, observability and compatibility qualification

**Files:** Modify `src/commands_engage/controls.rs`, `src/brain/style_signal.rs`,
`src/commands_brain/addenda.rs`, `src/observability.rs` closed events and tests;
modify README and deployment privacy/contract checks only where necessary.

**Interfaces:**
- Member feedback references their own Sent receipt. Useful/Dismissed is explicit;
  accepted/failed delivery and inferred relevance are separate typed outcomes.
- Feedback may reduce optional contact or suggest timing; it cannot enable contact,
  raise limits, clear stop, change timezone/destination or subscribe a member.
- Status exposes own pending/terminal receipt counts and reasons. Manager status
  contains aggregate counts and content-free timing; no recipient IDs or DM text.
- Measure queue, generation-first-visible-text, first successful Discord post,
  completion, and failures separately. Voice timings remain their own pipeline.

- [ ] Test wrong-member/duplicate feedback, quoted/code feedback exclusion,
  setting precedence, learned reduction, no authority escalation, help limits,
  redacted errors and no sensitive observability fields.
- [ ] Implement only existing bounded learning integration; preserve quorum,
  expiry/suppression and current eight quoted-feedback regressions.
- [ ] Run privacy/contract checks, focused style/addenda/engagement tests,
  `npm --prefix activity test`, and `python3 scripts/check-pages-liquid.py`.
- [ ] Review full domain boundary imports, file-size limits and exact rendered copy.
  Use no new dependency or migration that silently opts legacy members in.

## Task 10: Full gate, transactional rollout and requirement audit

**Files:** Evidence append only in `docs/MLAI-LIVE-ACCEPTANCE.md` and task ledger;
no unrelated edits. Owner-private receipt directory uses restrictive permissions.

- [ ] Freeze/check the source fingerprint and verify active checkout owners. Run
  `./check.sh` with exit status preserved and complete log. Resolve attributable
  failures; rerun after changes. Do not count unrelated failures as this release.
- [ ] Read the complete diff and get final independent review; check every spec
  requirement maps to an implemented behavior/test and every command is registered.
- [ ] Compare release and installed hashes. On existing operator authorization,
  run `./deploy/install-launchd.sh`; require exit0 and installation:ready, matching
  release/installed SHA256, and `python3 -I deploy/service-status.py` readiness.
- [ ] Use configured consenting participants for real Discord tests. One original
  channel follow-up and one explicitly chosen DM follow-up have bot-authored IDs;
  reply/resolution/snooze/stop cancel exact candidates; cross-guild simultaneous
  attempts respect one member budget; private content does not cross scopes.
- [ ] Exercise each new public behavior only after its guild toggle/allowlist is
  explicitly configured. Verify welcome unavailable when event capability absent;
  introduction requires two actual approvals. Do not change other servers' policies.
- [ ] Qualify Activity and human voice using Task6 receipts. Measure text/provider
  latency without presenting raw REST send time as generation or audible latency.
- [ ] Append a requirement-by-requirement ledger: Current/Partial/Blocked, evidence,
  exact gate result, installed identity, live receipts, remaining operator actions.
  Unverified human/Portal/hosting items keep overall release incomplete.

## Execution handoff and review status

Recommended: sequential subagent-driven development, one implementer and fresh
reviewer per task, because persistence, access and delivery races cross task
interfaces. Workers are not alone in this shared checkout and must preserve
others' edits. Native execution is also permitted if Donald chooses it.

The written specification was approved. Donald directed “finish up” on
2026-10-01 at09:17EDT after receiving this plan and the execution choices.
Execution proceeds sequentially with implementer/reviewer gates. Implementation
is in progress; task completion is tracked in the plan-specific SDD ledger.
Existing stability/Activity/outreach work is independent and must be revalidated,
not overwritten or repeated. No commit or push is authorized by this plan.
