# Guided Discord UX and operator status

Date: 2026-09-06
Status: selected for implementation under the user's 03:21 EDT instruction to brainstorm and implement the ideas. The written design records the agent's selected approach; it does not claim a separate user review of this document.
Scope: the Abbey Rust bot and its deployment helpers. This extends the approved 2026-09-04 modernization; it does not replace its provider, lifecycle, privacy, consent, compatibility, or delivery requirements.

## User intent and approaches

The user requested full Rust/service modernization, better menus and UI/UX, logging throughout the codebase, parallel brainstorming, implementation, builds, and integration into main.

Three approaches were considered:

1. Presentation only: improve help instructions and fixed recovery copy. This is small, but the memory summary still hides longer and additional facts and operators still assemble status checks manually.
2. A cohesive read-only navigation and observation bundle, recommended: guided help, complete private fact browsing, scoped statistics/recovery guidance, and a local service-status reader. It makes existing capabilities usable without adding another storage or lifecycle authority.
3. New write wizards and a native menu-bar application: convenient in some workflows, but add mutation, packaging and permission contracts while duplicating the administration dashboard. These are alternatives, not part of the recommended implementation.

## 1. Guided help and menu discoverability

Keep the current private help center and section selector. On Start, add at most three text buttons: Talk with Abbey, Review memory, Use an image. Derive visibility from the existing catalog eligibility rules. Buttons navigate to the corresponding help section and never invoke a model, command, image request, or mutation.

Each command line identifies its invocation surface (slash command, member menu, message menu) and result visibility. In a guild use private or channel-visible; in a bot DM use private or reply in this DM. Show exact existing command/menu names without guessed Discord application command IDs. Do not advertise a guild-only menu inside a DM or unavailable operator commands to members.

Reuse the current owner-bound help v1 envelope, fixed original 15-minute expiry, central acknowledgement, current permission refresh and capability filtering. Support both Button and StringSelect navigation. No new stored UI session or event schema is needed. Keep the selector and optional button row within two action rows, IDs within 100 ASCII characters, and every rendered command and footer within 2,000 codepoints before the final clamp. Text labels carry meaning independently of color or emoji.

## 2. Complete private fact browsing

Preserve the bounded summary shared by /recall and Abbey: memory. Add a Browse facts button when stored facts exist. The reader shows four complete facts per page, including up to 300 characters per fact, a page count and total fact count. All 100 supported facts must be reachable. Empty and shrinking lists render honestly; navigation refreshes the canonical subject snapshot and clamps a previously valid page to the new last page.

Use the exact new component grammar:

    abbey:mem:v1:<owner>:<subject>:<scope>:<expiry>:<page>

Owner and subject are nonzero canonical decimal u64 IDs. Scope is a nonzero canonical decimal guild ID or the literal d for the invoking user's one-person bot DM. Expiry is a canonical decimal u64 Unix-second timestamp fixed at original creation plus 15 minutes with checked arithmetic. Page is canonical decimal 0 through 24. Even maximum numeric widths remain within the 100-character ASCII limit. Invalid envelopes fail closed. A DM requires owner equal to subject and the current invoking DM context; a guild requires an exact current guild match.

The dispatcher acknowledges first, checks owner/scope/expiry and bot-authored message, then refreshes current permissions and applies the shared A1 self-or-memory-moderator rule before reading any facts. Cross-member permission loss denies the next page. Navigation never extends expiry. All replies and updates are private, clamped and use empty allowed mentions.

The reader has previous/next navigation and no write/delete/confirm buttons. It reads canonical facts, not semantic recall. It does not persist facts or custom IDs as UI state, write transcripts, change learning, record rewards, or alter voice. Fact bodies, subjects and custom IDs are never logged.

## 3. Scoped statistics and recovery guidance

Keep /stats and its existing guild/DM registration and ephemerality. Render only the current scoped brain and budget facts. Remove the process-wide interaction, messages-seen and pending-reward totals currently mixed into this member view. State the scope in the heading. Do not invent per-guild historical command statistics, add durable platform IDs, or create a separate analytics store. This is a correction to existing privacy and presentation behavior and belongs with Task 10.

Add fixed guidance alongside truthful PersistReport component results in the administration Operations page. Distinguish Complete, Partial, Failed and MemoryOnly; account for publication followed by failed synchronization without claiming that old bytes necessarily survived. A flush result describes that operation and does not claim service or provider readiness. Use closed category mappings, not raw errors or paths.

For image/provider failures, offer a retry or appropriate server-manager route using existing eligible help/dashboard actions. An ordinary member is not told to inspect host logs. Error classification comes from the typed provider boundary, never string guesses. No new restart, repair, log viewer or diagnostic-upload control is introduced.

## 4. Local read-only service status

After the managed readiness and fake installer contracts are implemented, add deploy/service-status.py with its default read-only operation and --help. Exit 0 means a currently validated ready observation, exit 1 means not ready/unavailable/unknown, and exit 2 means invalid invocation or unsupported host. This helper has no alternate JSON output or new persisted schema.

Reuse the exact readiness/bootstrap decoder and fixed managed paths from Tasks 10/11. Share validation through a focused module if necessary while preserving the transaction checker's exact CLI. Read the exact local launchd service record with bounded bytes and timeout, extract only its PID, check process liveness and the fixed installed binary hash, validate private readiness, then sample identity again to reject a changing process. Use the same 30-second maximum age and two-second future-skew predicate. There is no installation transaction start or five-second acceptance claim in this observational flow.

Render only fixed readiness, Discord, scheduler, connector and persistence categories plus a fixed next action. Do not render PID, nonce, executable hash, raw launchctl output, paths, models, endpoints, errors or credentials. A missing readiness document means evidence unavailable, not proof that the service is stopped or absent. A matching bootstrap failure may select fixed guidance.

Do not open logs, production state, consent or the owner env. Reject unsafe file types, symlinks, owner/mode errors, duplicate/unknown keys, stale/dead/mismatched identity and oversized inputs. Never change permissions, remove files, restart services, probe a provider, or execute suggested recovery. All behavioral acceptance uses a fake HOME and fake host/process/clock primitives.

## Architecture and sequence

Reuse catalog eligibility, HelpSession, shared A1 authorization, canonical memory service, PersistReport, provider failure categories, and the exact readiness validator. New pure helpers own presentation/paging; Discord, local host and filesystem access remain thin adapters. Avoid a second settings, analytics, logging or service authority.

Guided help and fact browsing follow the reviewed Task 6 dispatcher. Scoped /stats and categorized recovery integrate with Task 10. Local status follows Task 11's stable shared validator. All additions finish before Task 12's final decomposition, documentation, fresh strict gate, whole-branch review, hosted CI and main integration.

## Acceptance

- Render help across guild member/moderator/manager and bot-DM contexts with capability changes. Every advertised action is eligible, command coverage fits before clamping, and visibility language is accurate.
- Invoke the actual Button and StringSelect dispatch paths against fakes. Prove acknowledgement first, no permission lookup after malformed/foreign/expired rejection, unchanged expiry and current permission filtering.
- Browse all 100 complete facts, Unicode and empty/shrinking data. Prove wrong actor/guild/DM subject and revoked permission cause no snapshot read. Prove no state, transcript, reward or voice mutation.
- Interleave another guild and two DMs' activity and prove the scoped /stats view does not change due to unrelated global traffic. Do not restore identifiers to the durable event schema.
- Test every persistence outcome and categorized provider recovery message, including post-publication sync failure. Use secret/content/path/ID canaries.
- Drive local status with fake launchd, installed binaries, readiness/bootstrap files, clocks and process liveness. Test current ready/degraded/partial, absent/invalid/stale evidence, identity changes, output bounds and write/probe spies.
- Preserve existing command registration, tool corpus, manifests, self-test CLI, voice consent and immediate leave ordering, exact schemas and fake installer checks. Run focused checks, the final strict required-WDBX gate in a fresh target, and exact-head hosted CI.

Source and fake-host checks do not establish live registration, installed identity, audible voice, participant consent, provider qualification or service acceptance. Those remain separate recorded layers.
