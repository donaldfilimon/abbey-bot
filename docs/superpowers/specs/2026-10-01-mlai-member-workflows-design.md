# MLAI Member Workflows and Server Acceptance Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Make the redesigned server and bot discoverable and usable by ordinary members across complete task flows.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Activity and voice acceptance records determine truthful invitations/help; learning and Work features add surfaces only after their own qualification.

## Current evidence

Organization, onboarding links, forum layouts/tags and permission review have live receipts. Source help/forum suggestion improvements are verified. A bot with Administrator is not evidence of ordinary-member access. Existing role and channel history were preserved.

## Architecture

Keep the additive MLAI organization and catalog-owned authorization. Use classic controls and current REST permissions. Connect Help, Research, Showcase and project/voice/Activity flows with concise truthful guidance; no mass role/permission reconstruction.

## Requirements

MEMBER-WORKFLOWS-R1: Preserve all current channels, message history, AI LAB/VOICE categories, role IDs/order/permissions and unrelated overwrites. Server apply remains operator-only/dry-run by default and stops at first failure.
MEMBER-WORKFLOWS-R2: Ordinary members must discover relevant commands, open/reply to permitted forum posts, choose resolution status explicitly and reach supported voice/Activity guidance. Solved/Unresolved remain member/operator choices, never keyword auto-suggestions.
MEMBER-WORKFLOWS-R3: Current authorization is evaluated by snowflake/current REST facts, not display names or cached administrative privilege. Do not infer a CREATE_PUBLIC_THREADS grant is needed for forum creation.
MEMBER-WORKFLOWS-R4: Proposed /forum resolve changes only Solved/Unresolved tags on the invoking author’s own thread or a thread the actor can currently manage. Preserve every unrelated tag; never close/archive/delete the thread implicitly.
MEMBER-WORKFLOWS-R5: Help and workflow cards render within 2000 characters including footer/eligible commands and use classic controls. Members cannot see owner diagnostics; text accurately distinguishes source, installed, unavailable and human-gated capabilities.
MEMBER-WORKFLOWS-R6: Forum/search guidance references only accessible source posts and explicit source locations; no DM facts/private project summaries enter public workflows. Work projects/decisions reuse existing typed authority.
MEMBER-WORKFLOWS-R7: Acceptance witnesses one non-admin member, one manager and two isolated users in each supported server/DM path. No mass pings, unnecessary test-channel writes or credential/permission changes.
MEMBER-WORKFLOWS-R8: Current Telegram/Slack live operation is out of scope until explicitly enabled with owner credentials; source isolation/degraded tests still apply. Other installed guilds retain default-off unsolicited policy.

## Interfaces and data ownership

Proposed pure `ResolutionFacts { is_thread_author, can_manage_thread, current_tags, solved_tag_id, unresolved_tag_id }` feeds `resolution_tags(facts, desired: ResolutionState) -> Result<Vec<u64>, ResolutionError>`. ResolutionState is Solved or Unresolved; error is Denied or InvalidTags. Existing command_catalog guard and forum adapter own transport/access.

## Global constraints

- Rust 1.98.0 stable, edition 2024; retain --locked on every Cargo command.
- Canonical abbey-bot checkout; preserve unrelated dirty work; no checkout, clone, worktree, commit, push or PR without separate authorization.
- No new production dependency, credential rotation, permission escalation or deployment configuration change without the required operator decision.
- Pure policy receives time and seeds from callers; no Discord, network, randomness or wall-clock reads in pure modules.
- Production and external test-only Rust modules remain below 1,000 lines; production modules over 800 lines require review.
- Keep explicit ToolScope; voice, unsolicited generation and summaries remain read-only.
- Every DM uses network:dm:<user_id>; scoped persistence keys use U+001F; never widen a private scope into a guild or another user.
- Canonical state is atomic abbey-state.json; wdbx.seg.0.jsonl remains a rebuildable projection; no ABI/WDBX crate dependency.
- No automatic quarantine, contradiction or resolve edges; those remain explicit administrative actions.
- Service owners close admission, join observed work, freeze and attempt final persistence at most once; cancellation request is not cleanup.
- Managed JSONL contains only closed content-free events; no prompts, replies, raw feedback, recipient IDs, channel IDs, URLs, credentials or dynamic errors.
- Voice stores no raw audio or transcripts; personal durable Local policy-1 receipts, current roster, permissions and media epoch gate processing.
- Activity Portal URL mapping and human-witnessed acceptance remain operator-gated; no second Discord gateway beside the managed service.
- Preserve pipeline guard, cooldown and hourly-budget ordering; no fallback replay after visible output or mutating/uncertain effects.
- Defer interactions before network work, clamp rendered replies, preserve classic Action Rows; Components V2 remains crate-blocked.
- Run ./check.sh for each completed source slice, python3 scripts/check-pages-liquid.py for docs/Activity, and npm --prefix activity test for Court.
- Source tests, installed hash, provider qualification, Discord behavior and human acceptance are separate evidence layers; unknown evidence is not completion.

## Failures and acceptance

- Administrator bot succeeds where a non-admin member cannot; acceptance must catch it. Test owner: implementation Task 3.
- Same display name with different snowflake cannot gain resolution authority. Test owner: implementation Task 1.
- Changing resolution preserves unrelated tags and does not archive/delete. Test owner: implementation Task 1.
- Help expansion exceeds 2000 characters or leaks owner diagnostics. Test owner: implementation Task 2.
- Public workflow answer must not include inaccessible thread/DM evidence. Test owner: implementation Task 2.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-member-workflows.md](../plans/2026-10-01-mlai-member-workflows.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.
