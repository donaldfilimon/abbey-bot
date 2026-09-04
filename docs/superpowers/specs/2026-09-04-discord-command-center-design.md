# Discord Command Center Design

**Date:** 2026-09-04
**Status:** Approved and binding
**Scope:** Typed Discord command catalog, private help, privacy-aligned member
memory and image context menus, member-safe voice status, typed voice mode, and
the classic-component admin dashboard
**Implementation plan:**
`docs/superpowers/plans/2026-09-04-abbey-bot-full-modernization.md`

## Objective

Make Abbey's Discord surface discoverable and internally consistent without
changing its consent, privacy, or registration boundaries. One pure catalog is
the authority for command identity, context, access, capability, help grouping,
and user-facing summary. Poise remains the transport adapter, and Discord's
application-owned Entry Point remains an independently preserved registration.

This design is additive. Existing slash commands, `Abbey: profile`, `Ask
Abbey`, and the `launch` Entry Point remain available. New menus and views reuse
the existing pure memory, vision, persistence, and voice authorities rather than
creating parallel state or policy.

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

## Pure Catalog

The pure command module owns these types:

- `CommandKey`: a stable, exhaustive identifier for one leaf command or context
  menu. It is not the display name and is safe to use in content-free metrics.
- `CommandKind`: `Slash`, `UserContext`, or `MessageContext`.
- `InteractionContext`: a set drawn from `Guild` and `BotDm`.
- `DiscordPermission`: only permission bits Abbey tests: `ManageMessages`,
  `ModerateMembers`, `ManageWebhooks`, `ManageServer`, or `Administrator`.
- `AccessRule`: composable `Allow`, `Permission(DiscordPermission)`,
  `CallerPresentInVoice`, `SelfSubject`, `ApplicationOwner`, `All(rules)`, or
  `Any(rules)`. Empty `All`/`Any` values are invalid catalog data.
- `Capability`: `Generation`, `Vision`, `VoiceConfigured`, `VoiceLocal`, or
  `VoiceOpenAi`.
- `InputPredicate`: `FollowUpAbsent` or `ActionTargetResolved`.
- `ConditionRule`: composable `Always`, `Available(Capability)`,
  `Input(InputPredicate)`, `SelectedVoiceModeReady`,
  `HierarchyAllowsAction`, `All(rules)`, or `Any(rules)`. As with access,
  empty composites are invalid.
- `RegistrationPolicy`: Discord contexts plus the optional static
  `default_member_permissions` bit. This is registration metadata, not runtime
  authorization.
- `EligibilityRule`: one `AccessRule` and one `ConditionRule`, both of which
  must pass at invocation time.
- `HelpSection`: `Start`, `Conversation`, `Memory`, `Images`, `Moderation`,
  `Server`, `Voice`, or `Administration`.
- `CommandSpec`: key, kind, qualified Discord name, registration policy,
  runtime eligibility, section, short description, and whether the response
  is private.
- `registered_commands()`: the complete ordered slice of Abbey-owned leaf
  command and context-menu specifications.

The catalog is static application policy. It contains no Serenity or Poise
types, reads no environment or clock, performs no permission lookup, and does
not include provider names, endpoints, Discord IDs, or live state. Runtime code
supplies a plain `EligibilityInput` containing context, effective permissions,
whether actor equals subject, application-owner status, caller voice presence,
selected typed voice mode, available capability categories, optional-input
presence, and the already-resolved hierarchy result. One pure recursive
evaluator is used by command guards and help filtering; adapters may not
re-derive a named rule.

`RegistrationPolicy.default_member_permissions` is deliberately narrower than
runtime eligibility. It is `ModerateMembers`, `ManageWebhooks`, or
`ManageServer` only where every legal caller must have that permission. It is
unset for self-or-moderator and caller-present-or-manager rules so Discord does
not hide valid self/member paths. Administrator remains an effective-permission
input at runtime, not a second registration default.

Parent slash-command groups are structural and are checked recursively, but
the catalog enumerates leaves because leaves are what a person can invoke.
`launch` is not fabricated as an Abbey-owned `CommandSpec`: global registration
fetches the existing application command and merges its exact name, type,
handler, integration types, and contexts beside the catalog-derived Poise
commands.

## Binding Catalog

The following is the initial catalog. `private` means Discord ephemeral. An
optional member parameter is self by default; `memory moderator` means the
current caller is the subject or currently has Manage Messages, Manage Server,
or Administrator.

Every nontrivial row refers to these immutable typed rule constants:

| ID | Exact typed rule |
|---|---|
| `A0` | `AccessRule::Allow` |
| `A1` | `Any(SelfSubject, Permission(ManageMessages), Permission(ManageServer), Permission(Administrator))` |
| `A2` | `Permission(ModerateMembers)` |
| `A3` | `Permission(ManageWebhooks)` |
| `A4` | `Permission(ManageServer)` |
| `A5` | `All(Permission(ManageServer), CallerPresentInVoice)` |
| `A6` | `Any(CallerPresentInVoice, Permission(ManageServer))` |
| `A7` | `Any(ApplicationOwner, Permission(Administrator))` |
| `C0` | `ConditionRule::Always` |
| `C1` | `Available(Generation)` |
| `C2` | `Available(Vision)` |
| `C3` | `All(Available(Vision), Any(Input(FollowUpAbsent), Available(Generation)))` |
| `C4` | `Available(VoiceConfigured)` |
| `C5` | `All(Available(VoiceConfigured), SelectedVoiceModeReady)` |
| `C6` | `All(Available(VoiceConfigured), Available(VoiceLocal))` |
| `C7` | `HierarchyAllowsAction` |

`SelectedVoiceModeReady` is exact: Off is false, Local requires `VoiceLocal`,
and OpenAI requires `VoiceOpenAi`; every case also requires `VoiceConfigured`
through `C5`. `HierarchyAllowsAction` is evaluated only after the target and
roles are fetched and retains the existing owner/administrator/strict-role
rules. `C3` requires no generation capability when the optional follow-up is
absent and requires it when present.

| Section | Surface | Context | Access rule | Condition rule | Response |
|---|---|---|---|---|---|
| Start | `/help [section]` | guild, bot DM | `A0` | `C0` | private |
| Conversation | `/persona route` | guild, bot DM | `A0` | `C0` | existing visibility |
| Conversation | `/persona ask` | guild, bot DM | `A0` | `C1` | existing visibility |
| Server | `/whois <user>` | guild | `A0` | `C0` | existing visibility |
| Server | `Abbey: profile` USER menu | guild | `A0` | `C0` | private |
| Conversation | `Ask Abbey` MESSAGE menu | guild, bot DM | `A0` | `C1` | private |
| Server | `/perms <channel> <user>` | guild | `A0` | `C0` | existing visibility |
| Moderation | `/modcall <user> <severity> [warnings] [timeouts]` | guild | `A2` | `C7` | private |
| Server | `/server <kind>` | guild, bot DM | `A0` | `C0` | private |
| Server | `/webhook <channel>` | guild | `A3` | `C0` | private |
| Memory | `/remember <fact> [user] [replaces]` | guild, bot DM | `A1` | `C0` | private |
| Memory | `/forget <fact> [user]` | guild, bot DM | `A1` | `C0` | private |
| Memory | `/pending list [user]` | guild, bot DM | `A1` | `C0` | private |
| Memory | `/pending confirm <old_fact> [user]` | guild, bot DM | `A1` | `C0` | private |
| Memory | `/pending dismiss <old_fact> [user]` | guild, bot DM | `A1` | `C0` | private |
| Memory | `/recall [user]` | guild, bot DM | `A1` | `C0` | private |
| Memory | `/reputation [user]` | guild; bot DM for self only | `A1` | `C0` | private |
| Memory | `Abbey: memory` USER menu | guild | `A1` | `C0` | private |
| Conversation | `/summarize [count] [as]` | guild, bot DM | `A0` | `C1` | existing visibility |
| Images | `/see <image> [question]` | guild, bot DM | `A0` | `C3` | existing visibility |
| Images | `/ocr <image>` | guild, bot DM | `A0` | `C2` | existing visibility |
| Images | `Abbey: describe image` MESSAGE menu | guild, bot DM | `A0` | `C2` | private |
| Images | `Abbey: read image text` MESSAGE menu | guild, bot DM | `A0` | `C2` | private |
| Start | `/stats` | guild, bot DM | `A0` | `C0` | private |
| Administration | `/admin show` | guild | `A4` | `C0` | private |
| Administration | `/admin persona` | guild | `A4` | `C0` | private |
| Administration | `/admin learning` | guild | `A4` | `C0` | private |
| Administration | `/admin vision` | guild | `A4` | `C0` | private |
| Administration | `/admin cooldown` | guild | `A4` | `C0` | private |
| Administration | `/admin act` | guild | `A4` | `C0` | private |
| Administration | `/admin budget` | guild | `A4` | `C0` | private |
| Administration | `/admin brain` | guild | `A4` | `C0` | private |
| Administration | `/admin flush` | guild | `A4` | `C0` | private |
| Administration | `/admin export` | guild | `A4` | `C0` | private |
| Administration | `/admin reset` | guild | `A4` | `C0` | private |
| Administration | `/admin dashboard` | guild | `A4` | `C0` | private |
| Voice | `/voice consent` | guild | `A0` | `C4` | private |
| Voice | `/voice notice` | guild | `A4` | `C4` | private |
| Voice | `/voice join consent:true` | guild | `A5` | `C5` | private |
| Voice | `/voice resume consent:true` | guild | `A5` | `C5` | private |
| Voice | `/voice leave` | guild | `A6` | `C4` | private |
| Voice | `/voice status` | guild | `A0` | `C4` | private |
| Voice | `/voice diagnostics` | guild | `A4` | `C4` | private |
| Voice | `/voice mode [choice]` | guild | `A4` | `C4` | private |
| Voice | `/voice verify start` | guild | `A7` | `C6` | private |
| Voice | `/voice verify report` | guild | `A7` | `C6` | private |

The implementation must reconcile any current Poise/README context drift to
this table without silently removing an existing command. In particular,
`/reputation` becomes private, self-defaulting, and self-capable in bot DMs;
cross-member access adopts the existing memory authorization rule.

## Catalog and Registration Parity

Tests recursively flatten every Poise command and compare it with
`registered_commands()` by qualified name, command kind, registration
contexts, static default permission, typed access rule ID, typed condition rule
ID, and ephemerality. The test fails for duplicate leaves, missing catalog
entries, catalog-only entries, unknown/cyclic/empty policy expressions,
duplicate Discord names, or a parent whose child metadata weakens the catalog.
Table fixtures exercise every named rule against its truth table, including
self versus moderator, caller presence OR manager, caller presence AND manager,
Off/Local/OpenAI selection, and `/see` with and without a follow-up.

Runtime guard parity tests invoke every non-`A0` or non-`C0` command adapter and
prove it delegates to the catalog evaluator. Help uses the same evaluator in
`Discoverability` mode: current context, permission, subject, presence, and
capability facts are mandatory, while an invocation-only target predicate such
as `HierarchyAllowsAction` is treated as potentially satisfiable until a target
is supplied. Invocation mode fails closed if any required fact is unresolved.

A delimited generated region in `README.md` is rendered from the catalog and is
compared byte-for-byte in the gate. Handwritten voice, consent, provider, and
operational explanation remains outside that region.

Global registration has a separate pure merge fixture. It proves the fetched
`PRIMARY_ENTRY_POINT` command named `launch` retains its original handler,
integration types, and contexts after Abbey-owned commands are regenerated.
Guild-scoped development registration does not invent an Entry Point.

## Private Help Center

`/help [section]` always acknowledges ephemerally before any permission or
capability lookup. It then renders only entries that:

1. are legal in the current guild or bot-DM context;
2. are usable under the caller's current effective permissions and subject
   rules;
3. have every required runtime capability category available.

Unavailable capabilities are not advertised as usable. The help footer may say
that some commands are hidden because their deployment capability or permission
is unavailable, but it never reveals provider identity, configuration values,
hidden channels, or which higher privilege would expose a private operator
surface. `/help` itself always remains visible.

The default page is `Start`. A classic string select presents the eight
`HelpSection` values. Buttons or selects use at most five Action Rows and obey
Discord label, option, and message limits. Every body from the catalog/core is
passed through `clamp_message`.

Help custom IDs have exactly this versioned grammar:

```text
abbey:help:v1:<owner>:<expiry>:<section>
```

`owner` is the invoking Discord user snowflake, `expiry` is an absolute Unix
second, and `section` is the stable lowercase section slug. IDs are ASCII and
must be at most 100 characters. A help session expires 15 minutes after its
original command; interactions do not extend it. Only the owner may operate it.
Owner mismatch, malformed version/action, and expiry receive a private fixed
response and perform no lookup. Full custom IDs and their embedded snowflakes
are never logged or persisted.

Component dispatch is central rather than one collector per command. It
acknowledges the component before permissions, network access, or rendering.
Unknown `abbey:*` protocol versions fail closed with a private stale-control
message.

## Member Memory Card

One pure authorization function implements self-or-memory-moderator access for
`/remember`, `/forget`, `/pending`, `/recall`, `/reputation`, and `Abbey:
memory`. It takes plain actor/subject IDs and already-resolved permission bits;
the Discord adapter fetches current permissions only after acknowledgement.

One memory-card renderer is shared by `/recall` and `Abbey: memory`. It renders
the subject, bounded canonical facts, separately bounded pending replacements,
and the existing 0-to-1 standing. It cannot include another guild's or DM
namespace. The USER menu is guild-only because its target is a guild member;
DM self-service remains available through slash commands. The menu performs no
write, reward, learning, or transcript action.

`/reputation` changes from public guild-only lookup to private self-defaulting
lookup. A bot-DM caller may inspect only self. A guild caller may inspect
another member only under the shared memory authorization rule.

## Image Message Menus

`Abbey: describe image` and `Abbey: read image text` are private MESSAGE menus
available in guilds and a direct message with Abbey. They use one shared
resolved-attachment selector:

1. inspect message attachments in Discord order;
2. choose the first attachment that is a real supported JPEG, PNG, WebP, or
   GIF according to the existing bounded decoder, not merely a claimed MIME
   type or filename extension;
3. report a fixed no-supported-image result if none qualifies.

The selector never fetches embeds, stickers, message-body URLs, link previews,
or arbitrary remote URLs. It applies the same download ceiling, redirect
policy, byte limit, 8192 by 8192 pixel limit, 96 MiB decoded-allocation limit,
format validation, and first-frame GIF normalization as `/see` and `/ocr`.
Only Discord attachment URLs selected from the resolved message may be fetched.

The describe menu reuses `/see`'s description path without a generated
follow-up question. The text menu reuses `/ocr`. Results and fixed failures are
clamped. A context-menu invocation never appends the target message to a
transcript, writes memory, records a reward/experience, changes learning state,
or permits generated mentions.

## Voice Views and Typed Mode

`/voice status` becomes member-accessible. Its pure `MemberVoiceInput` to
`MemberVoiceView` projection has a strict allowlist:

- coarse state: off, presence, awaiting consent, active, or paused;
- processing category: Off, Local, or OpenAI;
- whether the caller's own saved choice covers the current mode;
- the configured/current channel only when Discord says the caller can view it;
- one next action appropriate to that caller and state.

It excludes model and voice-pack names, endpoints, provider errors, consent or
session epochs, participant identities/counts, queue/counter telemetry,
timestamps, and verification details. A hidden channel is rendered only as
`configured channel hidden`.

`/voice diagnostics` is the private Manage Server view. Its
`AdminVoiceInput`/`AdminVoiceView` may retain today's content-free operational
detail: exact phase, media gate, pending start, selected/configured modes,
consent/session epochs, aggregate participant and bounded-queue counters,
speech model labels, sidecar and text-backend readiness, and verifier state. It
still exposes no credentials, audio, transcripts, prompts, replies, or member
identities.

Discord `/voice mode` accepts `Option<VoiceModeChoice>`, whose displayed
choices are `Off`, `Local`, and `OpenAI`. It is not a free-form string. The
environment parser retains compatibility aliases: `disabled` and `off` map to
Off, `local` and `offline` map to Local, and `openai` maps to OpenAI. A mode is
selectable only when its complete backend configuration was retained at
startup. The displayed Off choice maps to the existing `VoiceMode::Disabled`
value; no serialized consent/configuration representation changes. Switching
while a join is pending or a call is connected remains refused because the
disclosure and saved consent name the processing mode.

Nothing in these views changes the voice-consent format or activation rules.
Join/resume still require saved individual agreement for every current
participant, public disclosure, final roster verification, and the existing
media epoch. `/voice leave` retains its unique safety ordering.

## Classic Admin Dashboard

`/admin dashboard` is a private Manage Server-only view implemented with
classic buttons and string selects. The pure model contains `AdminPage`,
`AdminAction`, `AdminEffect`, and a reducer/view pair. Pages are:

- `Overview`: current guild settings and safe capability categories;
- `Conversation`: default persona, vision, unsolicited action, and cooldown;
- `Learning`: learning state, hourly budget, epsilon, and the existing brain
  summary;
- `Operations`: truthful persist, ephemeral export, and reset entry point;
- `ConfirmReset`: an explicit second interaction naming this channel's
  transcript as the only reset scope.

Existing `/admin` leaf commands remain available. Dashboard actions call the
same pure clamps and state authorities; they do not become a second settings
implementation. Export remains an ephemeral attachment. Flush renders the
binding `PersistReport` result from the service/observability design and never
infers disk success from the presence of `ABBEY_DATA_DIR`.

Admin custom IDs have exactly this grammar:

```text
abbey:admin:v1:<owner>:<guild>:<expiry>:<action>
```

They are ASCII, at most 100 characters, owner- and guild-bound, and valid for
15 minutes from the original command without extension. Full IDs are never
logged or persisted. The dispatcher acknowledges first, checks the local
owner/guild/expiry envelope, then fetches the caller's current permissions and
reloads authoritative guild settings before every view and mutation. A caller
who lost Manage Server cannot act from an old page.

Mutations are reducers returning explicit `AdminEffect` values; Discord I/O is
an adapter concern. Stale page actions are idempotent against current state.
Reset requires navigation to `ConfirmReset` and a separate confirm component;
opening Operations or ConfirmReset does not clear anything. Confirmation
rechecks permission and current channel scope, clears only that channel's
multi-turn transcript, and reports `already clear` on a repeated/stale press.

## Acknowledgement and Delivery Rules

- Slash commands acknowledge unconditionally before REST, provider, disk, or
  other network work. Pure-only paths still acknowledge consistently.
- Central component dispatch acknowledges before permission refreshes or
  mutations. Fixed malformed/expired/foreign-owner guard replies are private.
- `/voice leave` remains the sole exception: it may authorize from interaction
  data and close the software media gate synchronously before its
  acknowledgement proceeds concurrently with the transition lock.
- No command parameter uses `GuildChannel`; use `ChannelId` and fetch after
  acknowledgement.
- Every rendered pure/model answer is clamped, and all generated/model output
  uses the existing empty allowed-mentions policy.

## Verification

Focused offline tests cover:

- recursive Poise/catalog/README parity and Entry Point merge preservation;
- catalog ordering, duplicate names/keys, context/access/capability matrices,
  and all Discord limits;
- help owner binding, expiry, malformed IDs, 100-character IDs, current
  permission/capability filtering, acknowledgement ordering, and clamping;
- memory self/cross-member/DM isolation authorization and shared card output;
- all four image formats, misleading MIME/extensions, attachment ordering,
  embeds/stickers/URLs excluded, fetch/size/decode/provider failures, no state
  mutation, clamping, ephemerality, and mention suppression;
- member voice allowlist, hidden channels, caller agreement, manager
  diagnostics, typed choice eligibility, connected-session refusal, and
  existing consent/leave ordering;
- dashboard permission changes, authoritative reloads, row/ID limits,
  stale-page idempotence, truthful flush, ephemeral export, and two-step
  channel-only reset.

These are source-contract tests only. They do not establish registered-command
propagation, live permissions, provider availability, installed identity,
Discord interaction behavior, or participant-consented voice acceptance.
