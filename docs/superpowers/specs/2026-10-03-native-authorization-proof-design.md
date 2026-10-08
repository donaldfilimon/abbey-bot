# Native authorization proof repair design — 2026-10-03

Confirmed full-source review defects are repaired without changing valid public
commands, visibility, dependencies, schema or persistence. Native HTTP responses
must bind the requested member, guild and channel and contain @everyone plus
every assigned role before permission or hierarchy calculation. Final Engagement
recipient proof also binds current bot identity, thread parent/membership and
private human DM recipient. Existing bot-only community delivery remains valid.

Per-member/guild snapshots, moderation advice, selected permission diagnostics
and webhook guides retain their contracts; wrong or incomplete facts refuse
before rendering private metadata. Roleplay uses NSFW only from the exact
invocation guild/channel. A no-guild invocation proves BotDm by reading the exact
Private channel with a positive human recipient equal to the positive invoking
author. Missing, mismatched, bot, guild or unsupported channel facts are Unproved.

The internal transport-free RoleplayContext gains Unproved; RoleplayDecision
gains RefuseUnproved, with no persona and fixed honest copy. Existing BotDm and
Guild decisions remain unchanged. No serialized/public external API changes are
required. Admission remains entirely in roleplay_gate.rs; the shell translates
native facts only. Abbey Reviewer approved this compatible extension before
implementation. Actual registered loopback RED proves a wrong recipient activates
Aviva in the old shell; live group-DM bot-command reachability is unverified.
